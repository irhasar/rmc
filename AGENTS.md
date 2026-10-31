# AGENTS.md

Repository-specific notes for future sessions. Keep this file current.

## What this is

`nrc` is an experimental ahead-of-time compiler for a small Rust subset. It emits
x86-64 assembly directly, assembles with GNU `as`, and links with GNU `ld`
against a freestanding assembly runtime (raw Linux syscalls, no libc, no LLVM,
no external crates). The `bench/` directory compares it against real `rustc`.

## Build and run

```sh
source /workspace/.cargo/env          # rustc/cargo are installed here
cargo build --release --manifest-path compiler/Cargo.toml

# compile a program
./compiler/target/release/nrc bench/programs/fib.nrs -o /tmp/fib
/tmp/fib 30

# inspect generated assembly
./compiler/target/release/nrc bench/programs/fib.nrs --emit-asm

# full benchmark suite (writes binaries to /tmp/nrcbench)
python3 bench/run.py
```

## Source map

- `compiler/src/lexer.rs` — byte scanner, tokens carry byte positions.
- `compiler/src/parser.rs` — recursive descent; `is_block_like` allows
  `if`/`while`/`loop`/blocks as statements without a trailing `;`.
- `compiler/src/ast.rs` — AST and `Type` (`I64`, `Bool`, `Unit`, `Array`).
- `compiler/src/sema.rs` — type checking, stack-slot allocation, builtin sigs.
- `compiler/src/codegen.rs` — AST to AT&T x86-64 assembly.
- `compiler/src/runtime.rs` — freestanding assembly runtime (`_start`, I/O,
  `alloc`, `arg_i64`).
- `compiler/src/main.rs` — driver: lex -> parse -> sema -> asm -> `as` -> `ld`.

## Things that are easy to get wrong

- **Stack slots grow downward.** A local in slot `s` lives at `-(s+1)*8(%rbp)`.
  Address them positively and you overwrite the saved `%rbp`/return address and
  segfault. See `Codegen::off`.
- **Array element 0 is at the lowest address.** An array in slots `s..s+n` has
  element 0 at `-(s+n)*8(%rbp)`, with element `i` at `base + i*8`. Mixing this up
  makes array writes run upward past `%rbp`. See `Codegen::array_off`.
- **Statements vs. tail expressions.** Assignments inside a block must be
  detected before the trailing-`;` check in `parse_block`, otherwise `x = ...;`
  is parsed as an expression and rejected.
- **Link flags.** The driver uses `ld -o <out> --build-id=none <obj>`; the
  assembly already carries `.note.GNU-stack` so no `-z noexecstack` is needed.
- **Benchmark inputs must be opaque.** Programs read their workload via
  `arg_i64` (nrc) or `env::args()` + `black_box` (Rust), so LLVM cannot
  constant-fold the workload away. Do not "simplify" them to constants.
- **`print_bool` prints 0/1**, not `true`/`false`, despite the name.

## Benchmark gotchas

- Runtime is process-spawn dominated for small programs; use large enough
  workloads (see `ARGS` in `bench/run.py`) for a meaningful runtime comparison.
- `bench/run.py` measures compile time and peak RSS with `wait4()`, so `as`/`ld`
  and LLVM are included. It raises if a produced binary exits non-zero, which
  catches the classic "measured a nonexistent path" bug.

## Scope

This is a research toy. Do not claim it is a general Rust compiler. It supports
only the subset in `docs/language.md`, has no optimiser beyond operand and
comparison folding, and no borrow checker.
