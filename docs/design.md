# rmc compiler design

`rmc` is a single-pass-per-stage, ahead-of-time compiler for a Rust subset. It
is deliberately small: no external crates, no LLVM, no libc. The whole pipeline
is `source -> lexer -> parser -> sema -> inline -> tco -> licm -> codegen
-> asm text`, after which the
driver shells out to GNU `as` and `ld`.

## Pipeline

1. **Lexer** (`lexer.rs`) — byte-oriented scanner producing `Token { tok, pos }`.
   Tracks byte offsets so parse and type errors can report a position.
2. **Parser** (`parser.rs`) — recursive descent producing the AST in `ast.rs`.
   Precedence climbing handles binary operators; `is_block_like` lets `if`,
   `while`, `loop` and bare blocks appear as statements without a trailing `;`.
   Recursion is bounded: `bump_depth` counts one level per nested expression,
   block, type, unary operator and postfix step, and refuses past `MAX_DEPTH`
   (100). Without it, deeply nested or very long input overflows the native
   stack and aborts — a crash on untrusted input. The bound is kept low enough
   for a 2 MiB thread stack, which is what `cargo test` runs on.
3. **Semantic analysis** (`sema.rs`) — checks types, resolves every variable to
   a stack slot, lays out structs, and records the frame size (`nslots`) per
   function. It rewrites `Expr::Var` with the resolved slot and type and
   `Expr::Field` with the field's byte offset, so codegen never looks up a name.
   Runtime builtins are registered as ordinary signatures here, so calls to them
   type-check like any other function. Three surface constructs are lowered here
   rather than in codegen: a list literal (`Expr::ArrayList`) becomes an
   array-building block with an inferred element type, a method call
   (`Expr::MethodCall`) becomes a `Call` to the mangled `Type$method` with the
   receiver prepended, and an `Enum::Variant` (`Expr::EnumVariant`) keeps its
   node and records its `i64` tag. A method is itself just a free function with
   a leading `self` parameter, which sema inserts before it builds signatures.
   An enum name resolves to `Type::Enum` through `resolve_ty`, which every type
   site (parameters, returns, `let`, struct fields, arrays) goes through, so an
   enum may be used wherever a scalar is. An enum with a payload resolves to
   `Type::EnumData(name, size)`, an aggregate, so codegen reuses its struct
   machinery: a constructor is materialised into a frame buffer and yields that
   address, and a `match` binds a payload by reading a word out of that buffer.
   Struct and enum layouts are computed together (`resolve_layouts`), because a
   struct may have an enum field and an enum variant may have a struct payload.
   Sema also holds the checks that keep rmc from accepting programs rustc
   rejects: each scope entry is a `Binding` with a `mutable` and an `init` flag,
   so a write to a non-`mut` binding (`E0384`), a read of a binding not
   definitely initialized on every path (`E0381`), a constant index outside an
   array, and a string literal used as an `i64` are all rejected here. See
   "Soundness checks" in `AGENTS.md`.
4. **Inlining** (`inline.rs`) — rewrites the AST so a call to a small,
   straight-line, scalar function is replaced by the function's body. Runs after
   sema, so every name already carries its slot and no lookup is needed, and
   before codegen, so the peephole pass sees the inlined arithmetic. See
   "Inlining" below.
5. **Self-recursion to loop** (`tco.rs`) — rewrites a function of the shape
   `f(x) = t(x) op f(h(x))` (with `op` associative, `+` or `*`, and `t` any pure
   expression that may itself contain a self-call) so the `h` chain becomes a
   loop and only one recursive call per level remains. Runs after
   inlining and before codegen. See "Self-recursion to loop" below.
6. **Loop-invariant code motion** (`licm.rs`) — hoists a pure, invariant scalar
   subexpression out of a loop body so it is computed once instead of every
   iteration. Runs after inlining, because inlining is what exposes the
   invariant arithmetic. See "Loop-invariant code motion" below.
7. **Code generation** (`codegen.rs`) — emits AT&T-syntax x86-64 assembly. Every
   expression leaves its result in `%rax`; the result type is returned to the
   caller so callers know how to use it.
8. **Assemble and link** (`main.rs`) — writes the assembly, runs `as --64`, then
   `ld -o <out> --build-id=none -z noseparate-code <obj>`, with the runtime text
   prepended. `-z noseparate-code` packs the read-only and executable sections
   into one read+execute `LOAD` segment and leaves the writable data in its own
   read+write one, so no page is ever both writable and executable. It is also
   the smallest layout `ld` produces here (3304 bytes vs 7344 for `-n`, which
   packs *everything* into one read+write+execute segment).

## Stack layout

Each function prologue is `push %rbp; mov %rsp, %rbp; sub $N, %rsp`. Slots are
numbered from zero and addressed as `-(slot+1)*8(%rbp)`, so they grow downward
and never collide with the saved frame pointer or return address above `%rbp`.
`sub $N` rounds the frame up to a multiple of 16 to keep the stack 16-byte
aligned for calls.

`N` is not `nslots`: the parser sizes `nslots` for every local a function could
declare, but a function that promotes its locals into registers and never
materialises an aggregate touches none of them. `off`/`array_off` record the
deepest slot the emitted body actually names, and the prologue allocates just
that plus the spill slots — so an all-register leaf like `fib` gets no `sub` at
all. (The measurement is a `Cell` filled during the dry codegen pass, since
`off` takes `&self` and the prologue is emitted before the body it sizes.)

Arrays and structs occupy `n` consecutive slots. Because slot addresses decrease
as the slot index rises, an aggregate's element 0 / field 0 is at the *lowest*
address, `-(slot+n)*8(%rbp)`, and element or field `i` is `base + i*8`. Getting
this layout consistent between the fill, `gen_addr` and value loading was the
source of an early miscompile where array writes ran upward past `%rbp`.
`aggregate_base` computes this base for both arrays and structs.

## Structs

A struct is a name, a list of fields, and a size. `sema` assigns each field an
offset in declaration order and no padding is needed because every field is 8
bytes. A field access is then just a `disp(%rbp)` operand, exactly like an array
element with a constant index. `sema` attaches each field's offset to the literal
but leaves the fields in source order, so initializer side effects run in the
order written; codegen stores each field at its own offset. A field of aggregate
type is copied word by word, and a read of such a field yields its address.

Structs live in memory and are named by their address, which is also how they
cross a call: a struct literal is materialised into a `sema`-allocated frame
buffer, so it has an address in every context (a `let` initialiser, an argument,
a `return`, or a tail expression).

## Expression convention

- The result of every expression is in `%rax`.
- Binary operators evaluate the left operand into `%rax` and the right into
  `%rcx`, then apply the operation.
- Operand folding: if an operand is a constant or a scalar local, it is emitted
  directly as an immediate (`$5`) or memory operand (`-8(%rbp)`) instead of
  being materialised through the stack. This removes most push/pop traffic.
- Constant folding: a small integer literal on the right of `+`, `-`, `*`, `/`
  or `%` folds into the instruction itself, so `x - 200` is `mov; sub $200`
  rather than three instructions. `small_imm` also accepts a negated literal.
- Register-operand folding: when the right operand of `+`, `-`, `*`, `&`, `|`
  or `^` is a promoted local, it folds in as a register operand, so `x + y`
  skips the `mov y, %rcx` bounce and emits `mov y, %rax; add x, %rax` (or one
  `imul %reg, %rax`). Only a register folds, never a memory operand: a memory
  operand measured ~19% *slower* on the `mandel` benchmark, because the load
  lands in the instruction's dependency chain, which is why the constant path
  folds immediates only.
- Commutative spill: when neither operand of a commutative op (`+`, `*`, `&`,
  `|`, `^`) is a simple operand, `eval_operands` is told the op does not care
  about order and pops the saved left operand straight into `%rcx` instead of
  routing it back through `%rax`. The op then reads `%rax` (the right operand's
  result) and `%rcx`, which is one instruction fewer than the order-preserving
  path. Comparisons and `-`/`/`/`%` keep the order-preserving path.
- Division by a constant: `a / d` and `a % d` for a literal `d` become a magic
  multiply plus a shift instead of `idiv`. See below.
- Comparison folding: `a < b` emits a single `cmp` plus `setcc` when one side is
  a register-free operand, so no stack traffic is needed at all.
- Bitwise and shift: `&`, `|`, `^` and `<<`, `>>` fold a small constant operand
  into the instruction (`and $k, %rax`). A shift count is masked to 6 bits
  before folding, matching the hardware and Rust. With a variable count the
  count must be in `%cl`, which `eval_operands` already arranges, so the shift
  is a single `sal %cl`/`sar %cl` with no extra move.
- Array access: for a local array, `a[i]` with a variable index is addressed as
  `disp(%rbp,%rax,8)` in one operand (the index is loaded into `%rax` first), and
  `a[3]` with a constant index needs no register. Stores use the same operand.
- In-place accumulation: `x = x + a + b - c ...`, where `x` is promoted to a
  register, no term reads `x`'s own register, and every term is already a single
  operand (an immediate, a local, or an array element), folds into one
  `add`/`sub` per term on that register instead of a tree that bounces each
  partial result through `%rax` and the spill slots. The chain is flattened
  first, so a nested sum counts: `x + a + (b + c)` is four adds, not one add of
  a separately-evaluated `%rax` sum. That shape is what LICM leaves behind, since
  it hoists each invariant term into its own local and the surviving expression
  is a sum of sums. A term that reads the accumulator on the right (`x = x + x`)
  falls back to the tree. A term that needs evaluating (a call, an index) is
  computed into `%rax` and added with one `add %rax, dst` when overflow checks
  are off; with them on it falls back, since that `add` cannot trap. Terms are
  emitted left to right, so a call that writes a local an earlier term read still
  sees the read happen first.
- Deep or nested operands fall back to spilling the left result while the right
  is evaluated, then reloading it into `%rax`. The first few spills go to
  callee-saved registers (`%r12`–`%r14`), so a value held across a call costs a
  register move rather than a store and a load; only a spill deeper than that
  reaches a frame slot. Slots are used rather than `push`/`pop` so `%rsp` never
  moves and a call needs no alignment pad.
- A promoted-local divisor of `a / y` or `a % y` stays in its callee-saved
  register and `idiv` reads it directly, instead of bouncing it through `%rcx`.
  `gen_expr(a)` uses only `%rax`/`%rcx`/`%rdx` as scratch and `cqto` writes only
  `%rdx`, so the divisor register is live across both — the same reasoning that
  lets the `add`/`imul` register fold read a promoted local.

### Division by a constant

`idiv` is one of the slowest x86 instructions, roughly 20–40 cycles and not
fully pipelined, and `mandel.rms` divides three times per inner iteration. When
the divisor is a literal, `gen_div_const` replaces it with the Hacker's Delight
"magic number" method, the same algorithm LLVM uses:

```text
t = high64(signed(m) * x);  if add { t += x }  t >>= s
q = t + (x >> 63)                       # sign fixup
```

`m`, `s`, and the `add` flag come from `sdiv_magic`. For roughly a quarter of
divisors the multiplier is large enough that an extra add of the dividend is
needed; powers of two take a separate path that is a shift plus a `2^k-1` sign
bias. For `%` the quotient is computed in `%rdx`, and the remainder is taken
from it directly (`imul d, %rdx; sub %rdx, %r8`); only `/` copies the quotient
into `%rax`, so the modulo path has no `%rdx`→`%rax`→`%rdx` round-trip. Two
details are easy to get wrong and are covered by `tests/cases/div`:

- `imul` overwrites `%rax` with the low product, so the fixups read the dividend
  from a `%r8` copy taken before the multiply.
- The sign bias is mandatory for power-of-two divisors, because an arithmetic
  shift rounds toward negative infinity while integer division rounds toward
  zero.

Modulo reuses the quotient: `q * d - dividend` gives the remainder.

### Division by a run-time divisor

A variable divisor cannot use a magic number, but it can often use a narrower
`div`. `emit_div` first tests both operands unsigned against 2^32; when both fit,
`xor %edx, %edx` plus a 32-bit `div <reg>d` computes the exact result in about 20
cycles instead of the 30–40 of a 64-bit `idiv`. The test must be unsigned
(`jae`): a negative operand has the sign bit set and compares as a huge unsigned
value, so it correctly falls through to `idiv` rather than being truncated.
`mix`'s `gcd` has small operands, so it takes the narrow path every iteration.

Two divisors have no quotient and must trap rather than fault: zero, and `-1`
with an `i64::MIN` dividend. The zero check runs before the split, because zero
fits in 32 bits and would otherwise reach `div`. The `MIN / -1` check runs only on
the `idiv` path, because a dividend that fits in 32 bits is never `MIN`; putting
it before the split would cost every division a `movabs` plus a branch for
nothing.

## Registers

There is no live-range allocator in the usual sense, but two kinds of value are
kept in callee-saved registers, because a callee-saved register survives a call
where a frame slot does not.

**Hot locals** are promoted: `promote` counts how often each scalar local is
mentioned, takes the top five, and assigns them `%rbx`, `%r12`–`%r15` for the
whole function.

**Spill temporaries** — the values that must outlive a subexpression, such as
the first recursive result in `fib`'s `fib(n-1) + fib(n-2)` — go to registers
too. `gen_fn` runs the body once with no spill registers reserved, which reports
the deepest spill the function ever reaches, and reserves that many of the
callee-saved registers the locals did not take. So the pool is shared and
disjoint: a function that promotes five locals reserves no spill registers, and
a function that promotes none can hold three spills in registers. Deeper spills
than that fall back to frame slots as before.

This is where a register allocator pays for itself. `fib` has one memory
operation in its whole body, the store and reload of the first recursive result
around the second call; putting it in a register is worth about 23% on the
*recursive* `fib` (13.1 ms down to 10.1 ms, measured with `--no-tco` so the
self-recursion rewrite does not remove the very operation being tested). In the
default pipeline the win is invisible: `tco` has already turned `fib`'s
recursion into a loop with no cross-call temporary, so the benchmark is at
parity with or without the spill register. Every other benchmark is unchanged
within noise (±1.5%). A true linear scan over live ranges would do more — reuse
a register for a temporary that dies before a local is born — but the spill
register is the part the call-heavy code actually notices.

Scalar struct fields take part in promotion too, so a hot `acc.sum` stays in a
register. A field is named by its register only where its slot is known: field
`k` of a struct at `slot` lives at `slot + fields - 1 - k` (field 0 is the
highest slot). Two kinds of field stay in memory. The struct's base slot is its
last field, whose address *is* the struct's base, so it must stay addressable —
this is why at most `fields - 1` fields of a struct can ever be promoted. And a
struct that is ever read or written as a whole (copied, returned, passed by
value, or declared by a `let` whose initialiser is a whole-value copy) is
accessed through memory, so all of its fields stay there. `promote` records
those cases during its walk and zeroes the whole struct's slots.

Arrays are skipped entirely: an element is reached by `base + i*8` through
memory, so no element slot may be promoted, and neither may any slot used as an
index base. A `let` of an array is promoted element by element only when its
initialiser writes each element (a splat `[v; n]` or a zero-fill), which
`fill_aggregate` handles by writing promoted elements to their registers.
Everything else stays in a stack slot, placed below the saved registers so the
two never overlap.

The frame is padded so that the saved registers plus the slots (locals and
spill slots together) keep `%rsp` 16-byte aligned, and because spills go to
frame slots rather than the stack, `%rsp` is static for the whole body and no
per-call alignment pad is needed. The epilogue points `%rsp` at the lowest saved
register before popping, which also discards the frame.

The number of spill slots is not known until the body is generated, so `gen_fn`
runs the body twice: the first pass measures the deepest spill, the second sizes
the frame and emits the final code.

Local selection is whole-function, not per loop, so a local hot in one loop
competes with one hot in another. The remaining gap to rustc on `fib` is the
frame-based argument passing and the unoptimised prologue, not the spill.

## Control flow

- `while`, `loop` and `for` push `(continue_label, break_label)` onto a loop
  stack so `continue` and `break` resolve to the innermost loop.
- `for i in a..b` is *not* desugared to a `while`: `continue` must still run the
  increment, so the increment gets its own label and `continue` targets that,
  not the loop test. Both bounds are evaluated once, the end into a hidden slot,
  so `for i in 0..f()` calls `f` exactly once.
- `if`/`while` conditions are emitted as a direct `cmp` + `jcc` when the
  condition is a comparison, instead of materialising a 0/1 and testing it. The
  negated condition codes are in `invert_cc`.
- A `while` is rotated: the test is emitted once before the loop as an entry
  jump and again at the bottom, and the bottom copy is the back edge
  (`jl`/`jne`, via `gen_branch_true`), so an iteration is four instructions
  instead of five and the loop has one jump rather than two. The entry jump runs
  once, so a false first condition still skips the body. `continue` targets the
  bottom test and `break` targets the label the test falls into. `for` is *not*
  rotated: its increment sits between the body and the guard, so the first guard
  must see the un-incremented start.

## References

A reference is a single word: the address of its referent. It is a scalar, so it
travels in one register whether it points at an `i64` or a 64-byte struct, and a
`&self` method call passes the receiver's address instead of copying the receiver
into the callee. `&e` on a place (a variable, field, element or dereference)
yields that place's address directly; `&e` on anything else has no address, so
`sema` allocates a temporary slot, the value is stored there, and the temporary's
address is taken. `*e` reads the referent: for a scalar that is a load from the
pointer, for an aggregate the pointer itself, since an aggregate value *is* its
address.

Two facts keep the code correct around references:

- A local whose address is taken must live in memory. The code generator's
  promotion pass is told this before it promotes anything, so `&x` is always a
  valid `lea` of `x`'s frame slot and never a stale read of a register the
  variable no longer keeps up to date.
- A reference-typed base is a pointer, not a place. `r.x` reads through the
  pointer; the address of the field is the pointer plus the field offset, not the
  address of the slot that holds the pointer. `gen_addr` and the field operand
  path both special-case a `Ref` base.

Reading a field, indexing, or calling a method through a reference auto-derefs,
so `r.x`, `r[i]` and `r.m()` need no explicit `*`. There is no borrow checker:
nothing tracks lifetimes, and `&mut` is not enforced to be exclusive.

## Calls

Scalar arguments are evaluated right-to-left and spilled to frame slots, then
loaded into the argument registers in order. Because spills are frame slots and
`%rsp` is already 16-byte aligned, no pad is inserted before the `call`.
Caller-saved registers are not preserved across calls; a temporary that must
survive one is spilled to a frame slot.

### Aggregates across a call

Structs and arrays are passed by value through a hidden pointer, following the
shape of the System V ABI. An aggregate argument is first copied into a frame
slot allocated by `sema`; that slot's address then travels in the argument
register, and the callee copies it into a parameter buffer it owns, so a
mutation inside the callee never reaches the caller. An aggregate result is
written through a buffer the caller points `%r10` at — a caller-saved register
that is not an argument register, so it survives argument setup.

Two ordering details matter. In the callee, the incoming pointers are saved on
the stack before any copy runs, because the copy clobbers `%rsi`/`%rdi`, which
still hold later arguments. In the caller, all aggregate arguments are copied up
front, before any scalar argument is evaluated, so a later argument expression
cannot disturb an earlier one's source. Copies of eight words or fewer are
unrolled; larger ones use a counted loop. `--overflow-checks` is irrelevant
here: a copy never traps.

Two shortcuts avoid the push/pop round trip: when every argument is a constant,
scalar, or string literal it is loaded straight into its argument register, and
when there are at most two arguments and none contains a call, they are
evaluated directly into `%rdi`/`%rsi`. Both are safe because expression
evaluation never writes `%rdi`/`%rsi`; only the `%rcx`/`%rdx` scratch registers
are clobbered, which is why the simple-operand test matters for wider calls.

## Inlining

A call to a small, straight-line, scalar function is replaced by the function's
body, so an accessor like `p.sum()` costs the same as the `add` it wraps and no
argument setup, jump or return runs. This is what makes `refs` competitive: its
hot loop reads a struct through three `&self` methods, and after inlining the
loop contains no call at all. The pass runs on the AST after sema — every `Var`
already carries its slot and every call its mangled name, so no lookup is needed
— and before codegen, so the peephole pass sees the arithmetic that was behind
the call.

A function is a candidate when its return and parameters are scalar (a reference
counts as scalar; aggregates are passed and returned by pointer and are left to
the call path), it takes no `self` by value, and its body is call-free and
straight-line — no loop, no `break`/`continue`, and no `return` except as the
last statement, which is folded into the body's value. The body must also be at
most 32 expression nodes, so a large function is never spliced in. Because a
candidate is call-free, inlining it never adds work that could itself grow; the
pass still runs to a fixpoint, because a function only becomes call-free *after*
its own callees are inlined, and a wrapper like `sum_via_ref` should follow its
accessors in.

Expansion is a slot remap. Each inlined call is given a fresh region of the
caller's frame, the callee's slots are shifted into it by a base offset, and a
read of a parameter's slot is replaced by the argument. A trivial argument (a
name, a literal, a string) is substituted in place; anything else is evaluated
once into a fresh slot and the slot is substituted, so a side effect runs exactly
once and the left-to-right order is preserved. A parameter the body *assigns* is
a local copy, so it is always given a fresh slot even when the argument is
trivial: substituting a caller variable for it would make the assignment write
through to the caller. Two expansions get disjoint regions, so an inlined body's
locals never alias the caller's or each other's. A whole-program budget of 8192
expansions bounds compile time and code size; a loop body is inlined once, not
once per iteration. `--no-inline` turns the pass off, which is what the tests
that assert on the calling convention use.


## Self-recursion to loop

The naive `fib` makes two recursive calls per level:

```text
fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); }
```

LLVM folds the right spine — the `fib(n - 2)` chain — into a loop, leaving one
call per level, and that constant factor is why `fib` used to be the one
benchmark `rmc` lost. `tco.rs` performs the same rewrite, in the shapes where it
is exact. Given

```text
f(x) = t(x) op f(h(x))             op is `+` or `*`
```

where `t` is any pure expression — it may itself contain a self-call, which is
what `tree(n) = 1 + tree(n - 1) + tree(n - 2)` needs (parsed left to right that
is `t op f(h)` with `t = 1 + f(n - 1)` and `h = n - 2`; when both operands are
self-calls, `t` is just `f(g)`). Associativity lets the `h` chain be unrolled:

```text
f(x) = t(x) op t(h(x)) op t(h(h(x))) op ... op guard(h^k(x))
```

which is exactly the loop

```text
acc = identity                     // 0 for `+`, 1 for `*`
while !cond(x) { acc = acc op t(x); x = h(x); }
return acc op guard(x)
```

The recursive call inside `t` stays a call; only the second call per level is
removed, halving the call tree (2.0x on `fib`). `t` is evaluated once per level
in both forms, so it may call the function being transformed but no other
function (that call could be duplicated or reordered), and it may not contain a
`return` (which would leave the loop instead of the recursive frame). The pass
runs after inlining and before LICM, and is turned off by `--no-tco`. It is also
skipped entirely under
`--overflow-checks`: reassociation is only exact for wrapping arithmetic, and the
checked trap fires on the first overflowing intermediate, which the loop forms in
a different order than the recursion.

The shape test is deliberately narrow, because a wrong match is a wrong program.
The function must take exactly one `i64` and return `i64` (so `x` is the only
state), be a free function rather than a method, and consist of exactly two
statements: a base-case `if cond { return guard; }` with no `else`, and a
`return t op f(h)` whose right operand is a call to the function itself. `op`
must be `+` or `*` — subtraction and division are not associative, and a
recursion that uses them is left alone. Finally `cond`, `guard` and `h` must be
call-free and return-free, and `t` must be return-free and call only the function
being transformed, so that evaluating them a different number of times cannot
change behaviour: `rmc` has no side-effecting user function, but a `print` nested
in an argument would otherwise be duplicated by the loop, and a `return` nested
in an argument would leave the transformed loop instead of the recursive frame,
dropping the accumulator.

The transform is validated by `tests/cases/tco.{rms,rs}`, which diffs `fib` and
`fact` against rustc for `n` in `0..20`, and `tests/cases/callrec.{rms,rs}`,
which covers the `1 + f(n-1) + f(n-2)` shape, plus unit tests that check a
non-associative recursion keeps both calls and a foreign call in the left operand
is not folded.

## Loop-invariant code motion

Inlining exposes arithmetic that reads only loop-invariant data. After inlining,
`refs`'s hot loop recomputes the same struct-field sum and the same two-product
sum every iteration, even though the struct is fixed. `licm.rs` computes each
such subexpression once, before the loop, and reads the result inside. It runs
after inlining and before codegen.

A sum needs one extra step. `acc + a + b + c` parses left-associatively, so no
sub-expression is invariant and the pass would hoist `a`, `b` and `c`
separately, leaving one dependent add per term in the loop. The pass first
regroups the invariant terms of a `+` chain into a single sub-sum
(`acc + (a + b + c)`), which is one hoistable value and one add per iteration.
That is the reassociation a backend such as LLVM performs, and it is what makes
the `refs` loop match the Rust one; it is exact for the wrapping arithmetic the
pass runs on.

The pass only rewrites a loop *body*; a `while` condition and a `for`'s bounds
run once per iteration by design, so hoisting them would change the loop. Inside
a body it hoists a scalar subexpression when every local it reads is untouched
by the loop and it cannot trap or write: no call, no division or modulo, no
array index or dereference, and no block or `if`. A struct-field read is a
memory read, so it is only hoisted when the loop writes no memory at all. A loop
that contains any call is skipped entirely, because a call can write anything.

The effect scan has to see writes that do not name a slot directly. A `while`
condition and a `for`'s start/end expressions run as part of the loop, so their
calls count too. Taking a local's address (`&mut x`) marks that slot as
written, because an inlined callee's deref store or a later `*p = ...` reaches it
through the pointer even though no statement assigns `x`. An assignment target is
scanned as an expression, so `a[next(&mut x)] = 0` sees the call. And assigning a
whole aggregate (`p = P { .. }`) marks the loop as writing memory, since it
rewrites the words a reference to `p` reads. A `for`'s variable and bound are
assigned by the loop itself rather than in the body, so they are treated as
loop-carried.

Hoisting a pure, non-trapping value earlier is otherwise safe: it can only waste
a computation when the loop does not run, never change a result. The one
exception is an arithmetic trap, so the pass is disabled under
`--overflow-checks`, where a hoisted `+`/`-`/`*` would trap before the loop
instead of at the iteration that overflows. `--no-licm` turns the pass off.

## Runtime

`runtime.rs` holds a freestanding assembly runtime. `_start` captures `argc` and
`argv` from the initial stack, calls `main`, and exits via the `exit` syscall
with `main`'s return value. I/O (`print_i64`, `print_str`, `print_bool`,
`putchar`), memory (`alloc`/`free`), `arg_i64` and the overflow trap are all
implemented with raw syscalls. Because there is no libc, the resulting ELF is
static and tiny.

### Heap

`alloc` is a first-fit allocator. Memory comes from `mmap`'d chunks of at least
1 MiB; a bump pointer carves blocks out of the current chunk, and each block
carries an 8-byte size header immediately before the pointer handed to the
program. `free` inserts a block into an address-ordered free list and coalesces
it with an adjacent neighbour. The list is address-ordered (rather than a plain
LIFO) precisely so coalescing is possible; the cost is a linear scan on both
`alloc` and `free`, which is fine at this scale. Memory is never returned to the
kernel.

### Bounds checks

With `--bounds-checks`, every array index is preceded by a compare against the
array's length and a `jae` to a runtime trap; `jae` catches a negative index too,
since it is an enormous unsigned value. A constant index is folded: in range
emits nothing, out of range is an unconditional `jmp` to the trap. When the flag
is on, an array element is never used as a bare memory operand, so no access can
bypass the check. The base is evaluated before the check, matching Rust: a base
that is a call or block runs its side effects even when the index is out of
range. The check itself uses no register, so the base address survives in `%rax`
for the load.

### Overflow checks

With `--overflow-checks`, every `+`, `-` and `*` is followed by `jo
__rmc_overflow`. The trap writes a fixed message to stderr and exits 101, the
same status rustc uses for a panic. The check is off by default because it costs
a branch per arithmetic operation.

## What is intentionally missing

There is no optimiser beyond the folding, local promotion, inlining, self-
recursion to loop, loop-invariant code motion and dead-instruction trim described
above. No global register allocation, no common-subexpression elimination, no
other loop transforms, no instruction scheduling. The compiler is optimised for compile
speed and simplicity, not for the speed of the code it produces. The non-trivial
passes are division by a constant (1.4x on `mandel.rms`), the top-five local
promotion (a few percent on `mix` and `sieve`), the reference-operand folds plus
`trim_dead` (1.4x on the `refs` benchmark), inlining (2.6x on `refs`, by removing
the three accessor calls from its hot loop), and loop-invariant code motion (a
further 2.9x on `refs`, by hoisting the now-inlined struct reads out of the
loop), the self-recursion rewrite (2.0x on `fib`, by halving its call tree);
everything else is local and cheap to compute. Flattening the accumulator
fold over a nested sum, plus unwrapping the inliner's spliced block so the fold
can see the accessor arithmetic, is what turns LICM's three hoisted values into
a six-instruction loop (`refs` 6.4 → 4.7 ms of rmc's own runtime, within ~20% of
an honest rustc twin).

On the language side the notable absences are: `u*`/`i*` widths other than
`i64`, floating point, traits and generics, closures, `for` over arbitrary
iterators, and anything involving real borrow checking (`&T`/`&mut T` exist but
are unchecked, and `&mut T` is not exclusivity-checked). Aggregate arguments and
returns work by value; a `&T` parameter or `&self` receiver is passed by address
so no copy is made. See the roadmap in the README for the order in which these
might arrive.
