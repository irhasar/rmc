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
```

## Source map

- `compiler/src/lib.rs` — library entry point: `compile_to_asm(src, options)`.
  The compiler is a library plus a thin CLI, so it can be embedded.
- `compiler/src/lexer.rs` — byte scanner, tokens carry byte positions.
- `compiler/src/parser.rs` — recursive descent; `is_block_like` allows
  `if`/blocks as statements without a trailing `;`. `parse_cond` sets
  `no_struct_lit` so a `{` after a condition starts the body, not a struct.
- `compiler/src/ast.rs` — AST and `Type` (`I64`, `Bool`, `Unit`, `Array`,
  `Struct`, and the parser-only placeholder `Named`).
- `compiler/src/sema.rs` — type checking, struct layout, stack-slot allocation,
  builtin sigs. Resolves `Type::Named`, fills `Field`/`FieldInit` offsets, and
  allocates the hidden slots for aggregate arguments and returns.
- `compiler/src/codegen.rs` — AST to AT&T x86-64 assembly.
- `compiler/src/runtime.rs` — freestanding assembly runtime (`_start`, I/O,
  `alloc`/`free`, `arg_i64`, the overflow trap).
- `compiler/src/main.rs` — CLI: reads source, calls the library, runs `as`/`ld`.
- `compiler/tests/compile.rs` — unit tests for the front end and codegen; these
  run without `as`/`ld`.

## Things that are easy to get wrong

- **Stack slots grow downward.** A local in slot `s` lives at
  `-(s+1+nregs)*8(%rbp)`, where `nregs` is the number of registers holding
  promoted locals. Address them positively and you overwrite the saved
  `%rbp`/return address and segfault. See `Codegen::off`.
- **Saved registers sit between `%rbp` and the frame.** The epilogue must point
  `%rsp` at the lowest saved register (or add the frame size back) before
  popping, otherwise `pop` reads the frame and the return address is lost.
  `regs.rms` and `calls.rms` fail if this regresses.
- **Aggregates cannot be promoted.** A promoted local is named by its register,
  so an array or struct (which needs an address) must stay in memory, and so must
  an index base. `promote` excludes *every* slot an aggregate spans, not just
  its first.
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
  the copy clobbers `%rsi`/`%rdi` (which still hold later arguments), and the
  caller must copy all aggregate arguments *before* evaluating any scalar
  argument, so a later argument cannot disturb an earlier one's source. Getting
  either wrong corrupts arguments silently; `aggfn.rms`/`aggbig.rms` catch it.
- **`%r10` carries the return buffer.** It is caller-saved and not an argument
  register, so it survives argument setup. Do not use `%r10` as scratch anywhere
  argument setup can run after the return pointer is set.
- **Statements vs. tail expressions.** Assignments inside a block must be
  detected before the trailing-`;` check in `parse_block`, otherwise `x = ...;`
  is parsed as an expression and rejected.
- **Link flags.** The driver uses `ld -o <out> --build-id=none <obj>`; the
  assembly already carries `.note.GNU-stack` so no `-z noexecstack` is needed.
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
and diffs stdout and the exit status. When adding a case, add both twins, write
them to behave identically, and remember that `rustc` infers `i32` for unsuffixed
literals, so large literals need an `i64` suffix on the Rust side. `args.json`
maps a case name to a list of argv vectors. Verify a new case actually detects
the bug it guards: revert the fix and confirm the case fails.

## Scope

This is a small, self-contained compiler, not a general Rust compiler. Do not
claim otherwise. It supports only the subset in `docs/language.md` (scalars,
arrays, structs, `for`, aggregates across calls), has no register allocator and
only a small peephole optimiser (operand/comparison/array-address folding,
constant folding, division by a constant), and no borrow checker. Aggregates
cross a call only by value; there are no references. The README's roadmap lists
the intended next steps.

## Codegen notes

- Every expression leaves its result in `%rax`; `%rcx`/`%rdx` are the only
  scratch registers used. No other register is written by expression code, which
  is what makes the direct-argument call shortcut safe (see `gen_call`).
- `simple_operand` returns operands usable with no register at all: immediates,
  scalar locals, constant-index array elements, and struct fields of a local.
  `array_elem_operand` returns the `disp(%rbp,%rax,8)` form whose index must
  first be loaded into `%rax`.
- Folding arithmetic memory operands into `add`/`imul` and the in-place
  `x = x + e` form were both tried and measured *slower* on the Zen cores this
  was tuned on, so they were removed. Keep the plain two-register form unless
  you re-measure on the target CPU.
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
