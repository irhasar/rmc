# rmc compiler design

`rmc` is a single-pass-per-stage, ahead-of-time compiler for a Rust subset. It
is deliberately small: no external crates, no LLVM, no libc. The whole pipeline
is `source -> lexer -> parser -> sema -> codegen -> asm text`, after which the
driver shells out to GNU `as` and `ld`.

## Pipeline

1. **Lexer** (`lexer.rs`) — byte-oriented scanner producing `Token { tok, pos }`.
   Tracks byte offsets so parse and type errors can report a position.
2. **Parser** (`parser.rs`) — recursive descent producing the AST in `ast.rs`.
   Precedence climbing handles binary operators; `is_block_like` lets `if`,
   `while`, `loop` and bare blocks appear as statements without a trailing `;`.
3. **Semantic analysis** (`sema.rs`) — checks types, resolves every variable to
   a stack slot, lays out structs, and records the frame size (`nslots`) per
   function. It rewrites `Expr::Var` with the resolved slot and type and
   `Expr::Field` with the field's byte offset, so codegen never looks up a name.
   Runtime builtins are registered as ordinary signatures here, so calls to them
   type-check like any other function.
4. **Code generation** (`codegen.rs`) — emits AT&T-syntax x86-64 assembly. Every
   expression leaves its result in `%rax`; the result type is returned to the
   caller so callers know how to use it.
5. **Assemble and link** (`main.rs`) — writes the assembly, runs `as --64`, then
   `ld -o <out> --build-id=none <obj>`, with the runtime text prepended.

## Stack layout

Each function prologue is `push %rbp; mov %rsp, %rbp; sub $N, %rsp`. Slots are
numbered from zero and addressed as `-(slot+1)*8(%rbp)`, so they grow downward
and never collide with the saved frame pointer or return address above `%rbp`.
`sub $N` rounds the frame up to a multiple of 16 to keep the stack 16-byte
aligned for calls.

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
element with a constant index. A struct literal is reordered into declaration
order by `sema` so codegen can store the fields sequentially.

Structs live in memory and are named by their address. They cannot be function
parameters or return values, which is the main limitation to lift next: the
System V ABI passes large aggregates via a hidden pointer.

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
- Division by a constant: `a / d` and `a % d` for a literal `d` become a magic
  multiply plus a shift instead of `idiv`. See below.
- Comparison folding: `a < b` emits a single `cmp` plus `setcc` when one side is
  a register-free operand, so no stack traffic is needed at all.
- Array access: for a local array, `a[i]` with a variable index is addressed as
  `disp(%rbp,%rax,8)` in one operand (the index is loaded into `%rax` first), and
  `a[3]` with a constant index needs no register. Stores use the same operand.
- Deep or nested operands fall back to pushing the left result on the stack
  while the right is evaluated, then popping into `%rax`.

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
bias. Two details are easy to get wrong and are covered by `tests/cases/div`:

- `imul` overwrites `%rax` with the low product, so the fixups read the dividend
  from a `%r8` copy taken before the multiply.
- The sign bias is mandatory for power-of-two divisors, because an arithmetic
  shift rounds toward negative infinity while integer division rounds toward
  zero.

Modulo reuses the quotient: `q * d - dividend` gives the remainder.

## Registers

There is no allocator in the usual sense, but the hottest locals in a function
are promoted. `promote` counts how often each scalar local is mentioned, takes
the top five, and assigns them `%rbx`, `%r12`–`%r15` for the whole function.
These are callee-saved, so a promoted value stays valid across a call and never
needs spilling. Aggregates are skipped because they need a memory address, and so
is any slot used as an index base. Everything else stays in a stack slot, now
placed below the saved registers so the two never overlap.

The frame is padded so that the saved registers plus the slots together keep
`%rsp` 16-byte aligned at every call, and the epilogue points `%rsp` at the
lowest saved register before popping, which also discards the frame.

Selection is whole-function, not per loop, so a local hot in one loop competes
with one hot in another; `fib`, whose only variable is the parameter, sees no
change. The benchmarks are call-dominated, so the win is small, but it is free:
compile time, peak RSS and binary size are unchanged.

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

## Calls

Arguments are evaluated right-to-left and pushed, then popped into the argument
registers in order. If an odd number of pushes is live, an extra 8-byte pad is
inserted before the `call` so `%rsp` stays 16-byte aligned. Caller-saved
registers are not preserved across calls; temporaries always live on the stack.

Two shortcuts avoid the push/pop round trip: when every argument is a constant,
scalar, or string literal it is loaded straight into its argument register, and
when there are at most two arguments and none contains a call, they are
evaluated directly into `%rdi`/`%rsi`. Both are safe because expression
evaluation never writes `%rdi`/`%rsi`; only the `%rcx`/`%rdx` scratch registers
are clobbered, which is why the simple-operand test matters for wider calls.

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

### Overflow checks

With `--overflow-checks`, every `+`, `-` and `*` is followed by `jo
__rmc_overflow`. The trap writes a fixed message to stderr and exits 101, the
same status rustc uses for a panic. The check is off by default because it costs
a branch per arithmetic operation.

## What is intentionally missing

There is no optimiser beyond the folding and local promotion described above.
No global register allocation, no common-subexpression elimination, no inlining,
no loop transforms, no instruction scheduling. The compiler is optimised for
compile speed and simplicity, not for the speed of the code it produces. The
non-trivial passes are division by a constant (1.4x on `mandel.rms`) and the
top-five local promotion (a few percent on `mix` and `sieve`); everything else is
local and cheap to compute.

On the language side the notable absences are: aggregates across function
boundaries, `u*`/`i*` widths other than `i64`, floating point, `match`, enums,
methods, and anything involving borrowing or generics. See the roadmap in the
README for the order in which these might arrive.
