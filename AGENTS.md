# AGENTS.md

Repository-specific notes for future sessions. Keep this file current.

## What this is

`rmc` is an experimental ahead-of-time compiler for a small Rust subset. It emits
x86-64 assembly directly, assembles with GNU `as`, and links with GNU `ld`
against a freestanding assembly runtime (raw Linux syscalls, no libc, no LLVM,
no external crates). The `bench/` directory compares it against real `rustc`.

## Build and run

```sh
export RUSTUP_HOME=/workspace/.rustup          # toolchain lives here
export PATH=/workspace/.cargo/bin:$PATH        # rustc/cargo are installed here
cargo build --release --manifest-path compiler/Cargo.toml

# compile a program
./compiler/target/release/rmc bench/programs/fib.rms -o /tmp/fib
/tmp/fib 30

# inspect generated assembly
./compiler/target/release/rmc bench/programs/fib.rms --emit-asm

# full benchmark suite (writes binaries to /tmp/rmcbench)
python3 bench/run.py

# differential test suite: rmc vs rustc on twin programs
python3 tests/run.py -v

# differential fuzzer: random programs vs rustc (reproducible seeds)
python3 tools/fuzz.py --mode expr --n 500 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1
```

## Source map

- `compiler/src/lib.rs` — library entry point: `compile_to_asm(src, options)`.
  The compiler is a library plus a thin CLI, so it can be embedded.
- `compiler/src/lexer.rs` — byte scanner, tokens carry byte positions.
- `compiler/src/parser.rs` — recursive descent; `is_block_like` allows
  `if`/blocks as statements without a trailing `;`. `parse_cond` sets
  `no_struct_lit` so a `{` after a condition starts the body, not a struct.
  Precedence, loosest to tightest: `||`, `&&`, comparison, `|`, `^`, `&`,
  `<< >>`, `+ -`, `* / %`. Note shifts bind *looser* than `+`/`-`, matching
  Rust: `parse_shift` consumes `parse_add`, and `parse_bitand` consumes
  `parse_shift`. Swapping these silently mis-parses `1 + 2 << 3`.
  `match` is parsed here and *desugared to a chain of `if`s* (`desugar_match`),
  so sema and codegen have no `match` case. A non-`Var`/`Int` scrutinee is bound
  to a hidden `let` first, so a call in the scrutinee runs once. An arm after
  `_` is unreachable and dropped (the `_` body becomes the `else`).
- `compiler/src/ast.rs` — AST and `Type` (`I64`, `Bool`, `Unit`, `Array`,
  `Struct`, and the parser-only placeholder `Named`).
- `compiler/src/sema.rs` — type checking, struct layout, stack-slot allocation,
  builtin sigs. Resolves `Type::Named`, fills `Field`/`FieldInit` offsets, and
  allocates the hidden slots for aggregate arguments and returns.
- `compiler/src/inline.rs` — AST inlining of small, straight-line, scalar
  functions, run after sema and before codegen. See the inlining gotcha below.
- `compiler/src/codegen.rs` — AST to AT&T x86-64 assembly.
- `compiler/src/runtime.rs` — freestanding assembly runtime (`_start`, I/O,
  `alloc`/`free`, `arg_i64`, the overflow trap).
- `compiler/src/main.rs` — CLI: reads source, calls the library, runs `as`/`ld`.
  Flags: `--emit-asm`, `--keep-asm`, `--overflow-checks`, `--bounds-checks`,
  `--no-inline`.
- `compiler/tests/compile.rs` — unit tests for the front end and codegen; these
  run without `as`/`ld`.

## Things that are easy to get wrong

- **A local whose address is taken must be forced into memory before promotion.**
  The promotion pass picks the hottest scalar locals and keeps them only in
  registers, so their frame slot goes stale. `&x` needs `x` to be in memory, so
  `count_expr` calls `mark_addressed` on an `AddrOf` operand to set `mem[]` for
  that slot *before* promotion runs. Miss this and `lea slot(%rbp)` reads a
  garbage word. `mark_addressed` walks `Field`/`Index` bases and marks the whole
  aggregate, and it must run in the dry pass too (it does: `count_expr` is shared
  by both passes).
- **A reference is a scalar, but an aggregate referent is named by its address.**
  `Type::Ref` is a scalar (one slot, one register) even when the referent is a
  struct or array. So `gen_expr` on a `Ref` local is a plain `mov`, while `Deref`
  of an aggregate is a no-op after evaluating the pointer — the pointer *is* the
  value. `gen_addr` on a `Deref` yields the pointer, and on a `Field` whose base
  is `Ref`-typed it adds the offset to the pointer (`ref_base`), never to the
  slot holding the pointer.
- **`&e` on a non-lvalue materialises a temporary.** `sema` gives `AddrOf` a
  hidden slot (`slot >= 0`) for a literal, call result, or aggregate-valued
  operand, and codegen stores the value there and yields that address. An
  aggregate operand is copied with `copy_words_rdi_rsi`, a scalar with one `mov`.
  `slot == -1` means the operand is an lvalue and `gen_addr` is used directly.
- **`&mut T` coerces to `&T`; the reverse does not.** `ref_coerces` is the only
  coercion and is checked in `let`, assignment and call arguments. Both are one
  pointer, so the coercion emits no code. `&mut` and `&` are otherwise distinct
  types, which is why `Sig` carries `ref_self_mut`.
- **A field read through a reference folds to one `disp(%reg)` load.**
  `field_operand` recognises a reference local kept in a register (`p.x` where
  `p: &T` and `p` is promoted) and a dereference of one (`(*q).x`), emitting
  `mov off(%reg), %rax` instead of `mov %reg,%rax; add $off,%rax; mov (%rax),%rax`.
  Before this, an `&self` accessor over an 8-field struct emitted ~3 instructions
  per field and the `refs` benchmark ran at 69 ms; the fold brought rmc's own
  runtime down ~1.4x to ~50 ms. Inlining the accessors then removed the calls
  entirely, taking `refs` to ~20 ms (rustc is still ~8x ahead on `refs` — it keeps
  the struct in registers across the loop, which rmc does not). The fold is
  deliberately *one* level deep: a `*p` used as a
  reference has its pointer in memory, so `ptr_reg` only matches a reference
  *local*, never a `Deref`. `addr_reg` (local or one `Deref` of a local) is the
  form `gen_addr` folds for a `lea`. Extending either to a deeper chain would
  read the inner pointer from a register it is not in — the bug that
  `(**ppr).x` in `tests/cases/references.rms` guards against.
- **A scalar `*p` read is a memory operand.** `simple_operand` returns `(%reg)`
  for a non-aggregate `Deref` whose pointer is a reference local, so `*p < 5`
  is one `cmpq $5, (%reg)` and `*p` in a comparison needs no load. An aggregate
  `Deref` is *not* a simple operand: its value is an address, not data.
- **`trim_dead` drops unreachable instructions and self-moves.** After each
  function body is generated it removes instructions following an unconditional
  `jmp`/`ret` up to the next label, and `mov %r, %r`. A `return` inside an
  expression emits `jmp epilogue` then the rest of the expression; the pass is a
  one-sweep fixpoint because removing a line creates neither a label nor a new
  terminator. It runs on the real body only (the dry spill pass's output is
  truncated anyway), so it costs nothing at compile time.
- **Inlining is a post-sema AST pass, and slots are frame indices.** Each
  inlined call gets a fresh region of the caller's frame (`base = f.nslots`,
  grown as it goes), the callee's slots are shifted by that base, and a read of
  a parameter's slot is replaced by the argument. Two expansions must not share
  a region or an inlined local would clobber another's. An argument with a side
  effect must be bound to one slot, never substituted into a body that reads the
  parameter twice — `f(sc(5))` with `f(x) = x + x` must call `sc` once. A
  candidate body must be call-free and straight-line; because it is, inlining it
  cannot expose new calls, but the pass still runs to a *fixpoint* because a
  function only becomes call-free after its own callees are inlined (the `refs`
  wrapper `sum_via_ref` needs round two). `--no-inline` disables it; the tests
  that assert on the calling convention (`call f`, argument setup) use it.
- **Stack slots grow downward.** A local in slot `s` lives at
  `-(s+1+nregs)*8(%rbp)`, where `nregs` is the number of registers holding
  promoted locals. Address them positively and you overwrite the saved
  `%rbp`/return address and segfault. See `Codegen::off`.
- **Spills live in frame slots, not on the stack.** `push_rax`/`pop_to` write a
  slot at `spill_base + sp_off` instead of `push`/`pop`, so `%rsp` never moves
  and no alignment pad is needed around a call. `spill_slots` is sized by a dry
  run of the body; if the real pass spilled deeper than the reservation, spills
  would overwrite locals. Keeping the two passes identical is what makes the
  reservation exact.
- **Two-pass codegen must reset every counter.** `gen_fn` runs the body twice and
  truncates the output between passes; the label counter, string table and spill
  high-water mark are part of that reset. Miss one and the two passes diverge.
- **A callee-saved temporary is not worth it here.** Holding a value across a
  call in `%r12`-`%r15` was implemented and measured *slower* (the save/restore
  in the prologue/epilogue costs more than the spill it removes). Keep the frame
  slot unless you re-measure.
- **Saved registers sit between `%rbp` and the frame.** The epilogue must point
  `%rsp` at the lowest saved register (or add the frame size back) before
  popping, otherwise `pop` reads the frame and the return address is lost.
  `regs.rms` and `calls.rms` fail if this regresses.
- **Only whole-value structs keep all their fields in memory.** A scalar field
  of a struct is promoted like any other local, *unless* the struct is ever
  copied, returned or passed by value, in which case the whole-value copy reads
  and writes memory and every field must stay there. `promote` tracks that
  during its walk (`copied` and the `let` targets) and zeroes those slots. A
  promoted field that stays in memory on the copy path but is read from its
  register on the field path is a silent wrong-answer bug, which is what
  `fieldpromo.rms` guards.
- **A struct's base slot is its last field and is never promoted.** Field `k`
  lives at `slot + fields - 1 - k`, so field `fields - 1` *is* `slot`, whose
  address is the struct's base; promoting it would break every `&struct`-style
  use. At most `fields - 1` fields can be promoted.
- **Field-to-slot uses the slot count, not the field count.** A struct with an
  array field has more slots than fields (`WithArr` is 2 fields, 4 slots), so
  `fields.len()` gives the wrong map. Use `size / 8`.
- **Arrays cannot be promoted.** An element is reached by `base + i*8` through
  memory, so no element slot may be promoted, and neither may an index base.
  `promote` excludes *every* slot an array spans, not just its first.
- **Aggregate element 0 is at the lowest address.** An array or struct in slots
  `s..s+n` has element/field 0 at `-(s+n+nregs)*8(%rbp)`, with item `i` at
  `base + i*8`. Mixing this up makes aggregate writes run upward past `%rbp`.
  `aggregate_base` returns this base for both arrays and structs; use it
  everywhere, including the struct-literal store in `gen_stmt` (an earlier bug
  used `off(slot)`, the wrong end of the range).
- **`for` is not a `while`.** `continue` must run the increment, so `for` keeps
  its own node with a dedicated continue label. Desugaring it to `while` would
  make `continue` skip the increment and loop forever.
- **Promotion counting must stay linear.** `counts` is built in slot order and
  `count_expr` indexes it directly by slot; do not reintroduce a linear `find`
  per reference or per excluded slot, which is O(n^2) and made a 200k-element
  array take seconds to compile. `large_array_compiles_in_linear_time` guards it.
- **Struct layout must agree between read and write.** `sema` resolves a field
  to an offset from the aggregate base; codegen must add that offset to the same
  base used for reads (`field_operand`) and writes.
- **Aggregates cross calls by hidden pointer.** An aggregate argument is copied
  into a `sema` slot and its address is passed; an aggregate result is written
  through a buffer the caller passes in `%r10`. Two orderings are load-bearing:
  the callee must save incoming pointers on the stack *before* any copy, because
  the copy clobbers `%rsi`/`%rdi` (which still hold later arguments). The caller
  evaluates arguments strictly left-to-right and pushes each one (aggregate: its
  slot address after the copy; scalar: its value), then pops them into the ABI
  registers right-to-left, so an earlier argument's source is already copied
  before a later argument runs. Evaluating all aggregates before the scalars
  would run side effects out of order: `f(g(), mk())` must print `2` then `1`,
  not the reverse. `aggfn.rms`/`aggbig.rms` and `argorder.rms` cover this.
- **`%r10` carries the return buffer.** It is caller-saved and not an argument
  register, so it survives argument setup. Do not use `%r10` as scratch anywhere
  argument setup can run after the return pointer is set.
- **The direct-argument shortcut must decline every clobbering argument.**
  `gen_call` writes scalar arguments straight into the ABI registers when each
  argument is a simple operand, or when there are at most two and none calls or
  copies an aggregate. `contains_call` must therefore recurse into *every*
  expression that can evaluate a child — including `EnumCtor`, `VariantTest`
  and `EnumPayload` — and `clobbers_arg_regs`/`stmt_clobbers` must flag every
  statement path that runs `copy_words_rdi_rsi` (whole-aggregate `let`, and
  aggregate `Var`/`Field`/`Index` stores). A missed case leaves an earlier
  argument in `%rdi`/`%rsi` that a later argument overwrites.
- **At most six parameters.** The System V integer argument registers are the
  only ones this compiler uses, so `sema` rejects a function (or a method after
  its receiver is inserted) with more than six parameters rather than letting
  `zip(ARG_REGS)` silently drop the seventh.
- **`mut` is enforced.** `Stmt::Let` carries a `mutable` flag and `FnDef` a
  `param_mut` vector; `sema` records both in its scope and rejects an assignment
  to a non-`mut` binding (the first assignment to an uninitialised binding is
  still allowed). Keep generated `let`s `mutable: true` so they never trip this.
- **A compound assignment evaluates its place once.** `desugar_compound` binds
  any component of the target that contains a call — the base or the subscript —
  to a temporary and reuses it for both the read and the write, so `make().x += 1`
  calls `make` once. A call-free `Var`/`Field`/`Index` is cloned directly.
- **`sema` persists the inferred type on every `let`.** Codegen selects the
  aggregate path from `Stmt::Let`'s `ty`, so leaving it `None` for `let a = [7; 2]`
  silently initialises only one word.
- **Struct-literal fields stay in source order.** Their layout offsets are
  attached per field, so reordering them into declaration order would change the
  order initializer side effects run in.
- **A compound assignment with a call in its subscript is bound to a temporary.**
  `a[next()] += 1` must evaluate `next()` once; the parser's `desugar_compound`
  binds the subscript when (and only when) it contains a call, so the common
  `i += 1` keeps the cheap two-instruction form.
- **Statements vs. tail expressions.** Assignments inside a block must be
  detected before the trailing-`;` check in `parse_block`, otherwise `x = ...;`
  is parsed as an expression and rejected.
- **Spill slots are frame slots.** `push_rax`/`pop_to` spill to a frame slot
  rather than `push`/`pop`, so `%rsp` is static and a call needs no alignment
  pad. This measured faster on every real program (`fib` +3.3%) because the pad
  costs two instructions per call.
- **Link flags.** The driver uses `ld -o <out> --build-id=none -n <obj>`; the
  assembly already carries `.note.GNU-stack` so no `-z noexecstack` is needed.
  `-n` (nmagic) drops the page alignment between sections and roughly halves the
  artifact size, which is one of the benchmark's measured dimensions.
- **A runtime function must be emitted in `.text`.** `-n` merges sections so a
  misplaced handler still runs under the CLI link, but a library client that
  links with a plain `ld` (no `-n`) maps `.rodata` without execute permission,
  so a handler left there faults. `__rmc_bounds` follows a `.rodata` message, so
  it re-opens `.text` first; check this whenever a trap is added after a
  `.section .rodata` block. `tests/run.py` links a few traps with a plain `ld`
  to catch it.
- **`array_len_of` must cover every array-typed base.** It reads the length from
  a `Var`/`Field`/`Call` type and from a block's or `if`'s tail. A base it does
  not recognise silently skips the bounds check; extend it alongside any new
  array-valued expression form.
- **Evaluate the base before checking bounds.** Rust runs the base and index
  before the bounds check, so a call or block base with a side effect must run
  even when the index is out of range. `check_bounds` uses no register, so
  emitting `gen_expr(base)` first leaves the address in `%rax` for the load.
- **A repeat literal `[v; n]` has no address.** Codegen only materialises an
  `ArrayLit` in `let`-initialiser position (the splat path); anywhere else it
  must be built into a buffer first, or it is emitted as a bare scalar and
  dereferenced as a pointer. Sema rewrites a value-position `[v; n]` to
  `{ let __rep = [v; n]; __rep }` via `check_let_init`, which reuses the splat.
  The list form `[a, b, c]` is unaffected: the parser already desugars it to a
  block. Keep `check_let_init` special-casing only the *direct* initialiser; a
  flag around the whole initialiser leaks into nested literals (a struct
  literal's fields are checked while it is set).
- **`gen_addr` must handle every aggregate-valued base.** A field or index
  whose base is a call result, a block, an `if`, or a struct literal is a
  *temporary*: its value is its own address. `gen_addr` matches those forms and
  evaluates the base, leaving the address in `%rax`. Only `Var`/`Field`/`Index`
  are true lvalues; the catch-all is `unreachable!`, so a new aggregate-valued
  expression form must be added here too or the compiler aborts on
  `mk().y`-style access.
- **Benchmark inputs must be opaque.** Programs read their workload via
  `arg_i64` (rmc) or `env::args()` + `black_box` (Rust), so LLVM cannot
  constant-fold the workload away. Do not "simplify" them to constants.
- **`print_bool` prints 0/1**, not `true`/`false`, despite the name.

## Benchmark gotchas

- Runtime is process-spawn dominated for small programs; use large enough
  workloads (see `ARGS` in `bench/run.py`) for a meaningful runtime comparison.
- `bench/run.py` measures compile time and peak RSS with `wait4()`, so `as`/`ld`
  and LLVM are included. It raises if a produced binary exits non-zero, which
  catches the classic "measured a nonexistent path" bug.

## Tests

`tests/run.py` compiles each `tests/cases/NAME.{rms,rs}` pair with both compilers
and diffs stdout and the exit status. `aggsem` (by-value aggregate assignment,
inferred aggregates, nested fields, indexing a returned array), `cmpsem`
(single evaluation of a side-effecting subscript in a compound assignment,
struct-literal field order) and `utf8` (non-ASCII string literals) guard the
front-end/codegen fixes below. When adding a case, add both twins, write
them to behave identically, and remember that `rustc` infers `i32` for unsuffixed
literals, so large literals need an `i64` suffix on the Rust side. `args.json`
maps a case name to a list of argv vectors. Verify a new case actually detects
the bug it guards: revert the fix and confirm the case fails.

The parser bounds nesting depth (`MAX_DEPTH` in `parser.rs`, currently 100). A
recursive-descent parser with no bound overflows the native stack and aborts on
deeply nested or very long input, which is a crash on untrusted input. The limit
must stay low enough for a 2 MiB thread stack (each expression level threads
through every precedence function, so it costs several KiB); `fuzz.py --mode
robust` and the depth tests in `tests/compile.rs` guard it. If you add a new
recursive construct, count a level with `bump_depth` on entry.

An enum is a set of unit variants represented by its `i64` tag (the declaration
index). `Expr::EnumVariant` keeps its node and its enum type so re-checking it
(as a method receiver is) stays consistent; codegen emits the tag. Enum names
resolve to `Type::Enum` through `resolve_ty`, which every type site uses, and
enum patterns are a `Pat::Variant` the parser desugars into an `EnumVariant`
comparison. `as` may not target an enum, matching rustc.

A method is an ordinary free function whose name is mangled to `Type$method` and
whose first parameter is `self` of the impl's struct type; the parser produces
it, sema inserts the `self` parameter before building signatures, and a method
call is rewritten by sema into a `Call` with the receiver prepended. Codegen
therefore has no method case — keep it that way. `Self` resolves through the
parser's `current_impl`.

An array's element type is always scalar (`i64` or `bool`); nested arrays and
arrays of structs are rejected. A list literal `[a, b, c]` is parsed as
`Expr::ArrayList` and rewritten by sema, which infers the element type from the
values — the parser must not guess it, because it does not type-check, and
guessing `i64` is what made `[true, false]` fail.

`tools/fuzz.py` is the differential fuzzer. `expr` mode builds random integer
expressions with calls and guarded division; `stmt` mode builds loops,
conditionals, `match`, arrays and aggregate calls, and deliberately places
side-effecting scalar and aggregate arguments in every position (this is what
found the argument-evaluation-order bug). It compares stdout and exit status
against `rustc -O`. A generated shift amount must stay `i64`-typed: rustc infers
`i32` for an unconstrained literal and masks the count to 5 bits, while rmc is
`i64`-only and masks to 6, so an untyped count reports a divergence that is not a
bug.

## Scope

This is a small, self-contained compiler, not a general Rust compiler. Do not
claim otherwise. It supports only the subset in `docs/language.md` (scalars,
arrays, structs, enums, `for`, aggregates across calls, references `&T`/`&mut T`
with no borrow checking). It has no *general* register allocator: whole-function
locals are promoted to callee-saved registers by mention count (`promote`), and
temporaries crossing a call use `SPILL_REGS`, but a dying temporary's register is
not reused for a later local (roadmap item 1). The optimiser is an AST inlining
pass (small, straight-line, scalar functions) plus a small peephole pass
(operand/comparison/array-address folding, constant folding, division by a
constant). Array indexing is unchecked by default (the index is used as-is, like
C), so an out-of-range index reads or writes adjacent stack memory;
`--bounds-checks` turns on a per-index check that traps (see the codegen notes).
The README's roadmap lists the intended next steps.

## Codegen notes

- Every expression leaves its result in `%rax`; `%rcx`/`%rdx` are the only
  scratch registers used. No other register is written by expression code, which
  is what makes the direct-argument call shortcut safe (see `gen_call`).
- **A destination address must survive the value it is assigned.** Assigning a
  whole aggregate to a field or element computes the destination first, and
  `gen_expr(value)` may clobber `%rdi` (a call, or another aggregate copy), so
  the address is pushed to a frame slot and popped back after the value. Writing
  `%rdi` before evaluating the value silently corrupts the destination.
- **The direct-argument call shortcut needs `%rdi`/`%rsi` preserved.**
  `contains_call` is not enough: a block's `let` initialiser can copy a whole
  aggregate (`copy_words_rdi_rsi`) with no call in sight. `clobbers_arg_regs`
  catches that case; add to it, not just to `contains_call`, when a new
  expression form can copy an aggregate.
- **A value-returning function must not fall off the end.** `check_fn` rejects a
  `Unit` body (no tail, or a tail like an `if` without an `else`) unless every
  path returns, using `block_diverges`. It is deliberately conservative: an
  `if` with no `else`, a `while`, and a `loop` are not treated as guaranteed
  returns.
- **With `--bounds-checks`, an array element is never a bare operand.**
  `simple_operand` returns `None` for `Expr::Index` when `self.bounds` is set, so
  every access is materialised through `gen_expr`/`gen_addr`, which emit the
  check. If you add a code path that folds an element into an instruction
  operand, it must also emit the check, or an out-of-range index can bypass it.
- **`lea_shape` must keep its displacement in the signed 32-bit range.** Each
  constant in a folded `k*x + c` chain is representable alone, but their sum is
  not; `disp_ok` guards the fold, and a rejection falls back to ordinary code.
- **Generated temporaries use unspellable names.** Compound-assignment
  desugaring introduces `let`s whose names contain a NUL byte, which no source
  identifier can contain, so they can never shadow a user variable. Do not
  switch them back to a spellable `__rmc_...` name.
- **`copy_words_rdi_rsi` must not touch `%rax`.** It copies through `%r8`, so a
  caller can hold a live value in `%rax` across the copy (a pending operand, or
  an argument already loaded into a register). It also clobbers `%rdi`/`%rsi`,
  which is why `contains_call` treats a struct/array literal as call-like: a
  literal is materialised through this copy, so it must not be selected by the
  direct-argument shortcut that requires register preservation.
- **`gen_expr_into_reg` folds arithmetic into one `lea`.** `lea_shape` recognises
  `k*x + c` for the scales `lea` can express (1, 2, 3, 4, 5, 8, 9) and a constant
  offset, so `2*i+3` becomes one instruction. It is only valid when overflow
  checks are off (a `lea` cannot trap); with `--overflow-checks` it falls through
  to `gen_expr`, which emits the `jo`. `gen_expr_into_reg(e, dst)` is also the
  safe way to materialise an expression straight into a destination register
  during argument setup, and a scalar `let`/assignment writes straight into a
  promoted local's register rather than bouncing through `%rax`.
- **`emit_cmp` folds a register operand.** `cmp %r, %r` and `cmp %r, mem` are
  legal, so a comparison between two promoted locals is one instruction; only a
  `mem, mem` compare must materialise one side into `%rax`.
- **Aggregates are copied, not aliased.** Whole-aggregate assignment, struct
  fields of aggregate type, array elements of aggregate type, and struct-literal
  fields all copy their words. A field or element read of aggregate type yields
  an *address*, not a value, and `simple_operand` returns `None` for it so no
  scalar path can treat the address as data.
- `simple_operand` returns operands usable with no register at all: immediates,
  scalar locals, constant-index array elements, and struct fields of a local.
  `array_elem_operand` returns the `disp(%rbp,%rax,8)` form whose index must
  first be loaded into `%rax`.
- Folding arithmetic memory operands into `add`/`imul` and the plain two-operand
  in-place `x = x + e` form were both tried and measured *slower* on the Zen
  cores this was tuned on, so they were removed. Keep the plain two-register
  form unless you re-measure on the target CPU.
  - The multi-term **accumulation fold** (`gen_accumulate`) is a narrower form
    that was measured neutral: it applies only when `x` is promoted, no term
    reads `x`'s register, and every term is already a single operand. It cuts
    instruction count (a running sum over locals drops to one `add`/`sub` per
    term; the `refs` loop goes 22 → 12) with no wall-clock regression, because
    these loops are latency-bound on the serial add chain and register renaming
    hides the extra moves. Folding *call-result* terms is a different story: it
    regressed `refs` by ~12%, which is why `gen_accumulate` bails out on any
    term that needs evaluating.
- Division or modulo by a literal divisor does not use `idiv`. `gen_div_const`
  emits the Hacker's Delight magic multiply (the same algorithm LLVM uses),
  with a sign fixup and, for roughly a quarter of divisors, an extra add of the
  dividend. Powers of two use a shift with a `2^k-1` sign bias.
  - **`imul` clobbers `%rax`** with the low product, so the fixups must read the
    dividend from the `%r8` copy taken before the multiply, not from `%rax`.
    Using `%rax` here silently produces wrong results for large dividends.
  - **The sign bias is required** for power-of-two divisors; without it a
    negative dividend rounds toward negative infinity instead of toward zero.
  - `small_imm` accepts a negated literal, so `x / -1000` takes the fast path.
  - `tests/cases/div.{rms,rs}` covers both variants, powers of two, negatives,
    and boundary dividends, and is the regression guard for all of the above.
