# nrc compiler design

`nrc` is a single-pass-per-stage, ahead-of-time compiler for a Rust subset. It
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
   a stack slot, and records the frame size (`nslots`) per function. Runtime
   builtins are registered as ordinary signatures here, so calls to them
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

Arrays occupy `n` consecutive slots. Because slot addresses decrease as the slot
index rises, an array's element 0 is at the *lowest* address,
`-(slot+n)*8(%rbp)`, and element `i` is then `base + i*8`. Getting this layout
consistent between `fill_array`, `gen_addr` and value loading was the source of
an early miscompile where array writes ran upward past `%rbp`.

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
fully pipelined, and `mandel.nrs` divides three times per inner iteration. When
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

## Control flow

- `while` and `loop` push `(continue_label, break_label)` onto a loop stack so
  `continue` and `break` can resolve to the innermost loop.
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
with `main`'s return value. I/O (`print_i64`, `print_str`, `putchar`), memory
(`alloc`, an anonymous zero-filled `mmap`), and `arg_i64` are all implemented
with raw syscalls. Because there is no libc, the resulting ELF is static and
tiny.

## What is intentionally missing

There is no optimiser beyond the folding described above. No register
allocation, no common-subexpression elimination, no inlining, no loop
transforms. The compiler is optimised for compile speed and simplicity, not for
the speed of the code it produces. The one non-trivial peephole is division by a
constant, which was worth a 1.4x speedup on `mandel.nrs`; everything else is
local and cheap to compute.
