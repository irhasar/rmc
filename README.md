# nrc — an experimental Rust-subset ahead-of-time compiler

`nrc` compiles a small, statically-typed subset of Rust straight to x86-64
assembly. It does **not** use LLVM, does **not** use any external crate, and
does **not** link a C runtime. The emitted assembly is assembled with GNU `as`
and linked with GNU `ld` against a freestanding runtime that talks to the Linux
kernel through raw syscalls.

The point of the experiment is to measure the trade-off at the extremes:

| property                         | nrc                                   | rustc (LLVM)                     |
| -------------------------------- | ------------------------------------- | -------------------------------- |
| compile pipeline                 | lex → parse → typecheck → asm → `as` → `ld` | MIR → LLVM IR → LLVM opt → MC → `ld` |
| third-party code                 | none (Rust `std` only)                | LLVM + rustc                          |
| compiler peak RSS                | ~12 MB                                | ~188 MB                          |
| compile time (small programs)    | ~7 ms                                 | ~2.1 s                           |
| artifact size                    | ~6 KB (static, no libc)               | ~290 KB                          |
| generated-code quality           | small peephole optimiser              | highly optimised                 |

`nrc` wins big on compile time, compiler memory, and artifact size. On runtime
it is now within a few percent on divide-heavy and array-heavy loops, and still
loses on call-heavy code where LLVM's register allocator and loop
transformations (for example turning the naive double-recursive `fib` into a
loop) do the work a stack-machine code generator cannot.

## Layout

```
compiler/            the nrc compiler (Rust, std only, no dependencies)
  src/lexer.rs       source -> tokens
  src/parser.rs      tokens -> AST
  src/ast.rs         AST + type definitions
  src/sema.rs        type checking, slot allocation, builtin signatures
  src/codegen.rs     AST -> x86-64 assembly (System V, AT&T syntax)
  src/runtime.rs     freestanding runtime written in assembly
  src/main.rs        driver: source -> lex -> parse -> sema -> asm -> as -> ld
bench/
  programs/*.nrs     benchmark programs in the nrc subset
  rust/*.rs          the same programs written in real Rust
  run.py             benchmark harness (compile time, CPU, RAM, size, runtime)
tests/
  run.py             differential test suite: nrc vs rustc on twin programs
  cases/*.nrs, *.rs  paired programs that must behave identically
  args.json          per-case argv vectors (for runtime inputs)
docs/
  language.md        the supported language subset
  design.md          compiler architecture and code generation notes
```

## Building

```sh
cargo build --release --manifest-path compiler/Cargo.toml
```

The compiler binary is `compiler/target/release/nrc`. It needs `as` and `ld`
from binutils on `PATH`; no other tools are required.

## Usage

```sh
nrc input.nrs -o output          # compile, assemble and link
nrc input.nrs --emit-asm         # print the generated assembly to stdout
```

Run the produced binary directly; it is a static ELF executable with no
dynamic dependencies.

## Testing

`tests/run.py` is a differential test suite. Every case is a pair of programs,
`tests/cases/NAME.nrs` and `tests/cases/NAME.rs`, written to behave identically;
the runner compiles both, runs them, and fails if stdout or the exit status
differs. `tests/args.json` optionally maps a case to argv vectors so the same
program is exercised with several runtime inputs.

```sh
python3 tests/run.py -v
```

The suite covers arithmetic and division semantics (including divisors that need
the magic-multiply add fixup and powers of two), every comparison operator with
the immediate on either side, boolean short-circuiting, control flow, arrays,
calls and recursion, the output builtins, and nested expressions. It is
deliberately adversarial: reverting the folded-comparison operand-order fix
makes the `cmp` case fail, and dropping the sign bias from division by a power
of two makes the `div` case fail.

## Language subset

A short program that exercises most of the supported surface:

```rust
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

fn main() -> i64 {
    let mut n: i64 = arg_i64(1);   // read argv[1] as an integer
    if n == 0 {
        n = 30;
    }
    print_i64(fib(n));
    return 0;
}
```

Supported: `i64`, `bool`, fixed-size arrays `[i64; N]`, functions with up to six
parameters, `let` / `let mut`, assignment, `if`/`else`, `while`, `loop`, `break`,
`continue`, `return`, arithmetic and comparison operators, `&&`/`||`, string
literals, and a small set of runtime builtins. See `docs/language.md` for the
full list.

Runtime builtins (provided by `src/runtime.rs`, no imports needed):

| builtin         | signature            | behaviour                              |
| --------------- | -------------------- | -------------------------------------- |
| `print_i64(x)`  | `(i64) -> ()`        | print an integer followed by `\n`      |
| `print_str(s)`  | `(i64) -> ()`        | print a string literal                 |
| `print_bool(b)` | `(bool) -> ()`       | print `true` or `false`                |
| `putchar(c)`    | `(i64) -> ()`        | write a single byte                    |
| `alloc(n)`      | `(i64) -> i64`       | anonymous zero-filled `mmap` of `n` bytes |
| `arg_i64(i)`    | `(i64) -> i64`       | parse `argv[i]` as a signed integer    |

## Benchmarks

`bench/run.py` compares `nrc` against `rustc -O -C lto=fat -C codegen-units=1
-C panic=abort -C strip=symbols` on five programs. It measures each compile with
`wait4()` so CPU time and peak RSS cover the real process (including `as`/`ld`
for nrc and LLVM for rustc). Programs read their workload from `argv` via
`black_box` / `arg_i64`, so nothing is constant-folded away.

```sh
python3 bench/run.py
```

Representative run on the development machine (Debian 13, gcc 14.2, rustc
1.99.1 / LLVM 23.1.1, x86-64, 16 GB RAM; medians of 5 compiles and 9 runs):

| program | compiler | compile wall | compile CPU | peak RSS | artifact | run time |
| ------- | -------- | -----------: | ----------: | -------: | -------: | -------: |
| hello   | nrc      |     7.80 ms  |    7.03 ms  |  11.8 MB |   5848 B |  1.87 ms |
| hello   | rustc    |  2079.99 ms  | 2082.03 ms  | 182.9 MB | 289416 B |  2.40 ms |
| fib     | nrc      |     7.95 ms  |    7.49 ms  |  11.8 MB |   5888 B | 15.72 ms |
| fib     | rustc    |  2115.85 ms  | 2119.49 ms  | 183.6 MB | 291696 B |  8.68 ms |
| mandel  | nrc      |     7.01 ms  |    6.75 ms  |  11.8 MB |   6184 B |105.17 ms |
| mandel  | rustc    |  2116.81 ms  | 2121.21 ms  | 184.6 MB | 291776 B |107.12 ms |
| sieve   | nrc      |     6.99 ms  |    6.70 ms  |  11.8 MB |   6008 B |  3.15 ms |
| sieve   | rustc    |  2089.10 ms  | 2092.67 ms  | 184.9 MB | 292016 B |  4.03 ms |
| mix     | nrc      |     6.87 ms  |    6.60 ms  |  11.8 MB |   5976 B | 18.82 ms |
| mix     | rustc    |  2107.53 ms  | 2111.46 ms  | 184.2 MB | 291864 B | 16.89 ms |

Summary of the ratios (rustc ÷ nrc):

| dimension        | ratio      |
| ---------------- | ---------: |
| compile wall     | ~266–307×  |
| compile CPU      | ~283–320×  |
| compiler peak RSS| ~15.6×     |
| artifact size    | ~47–50×    |
| run time         | 0.9–1.8× (rustc faster on call-heavy code) |

`nrc` beats rustc on `sieve` because the whole workload is a tight array loop
that the simple code generator already handles well, and the static binary has
no libc startup. `mandel` is now a dead heat after division by a constant became
a multiply-and-shift. On `fib` (call-heavy) and `mix` (register-pressure-heavy)
LLVM's register allocation and loop optimisation still win.

### Reproducing the comparison honestly

The `rustc` invocation enables fat LTO and a single codegen unit, which is the
closest equivalent to "compile everything, then link". The `nrc` time includes
spawning `as` and `ld`; the `rustc` time includes LLVM and the system linker.
Both binaries are measured the same way, and both read their workload from
`argv` so neither can fold the answer at compile time.

## Honest limitations

This is a research toy, not a drop-in Rust compiler.

- The language subset is tiny (see `docs/language.md`); no structs, enums,
  traits, generics, `match`, references, borrowing, or the standard library.
- Code generation is a straightforward stack machine. There is no register
  allocator and no instruction scheduling. The optimiser is a small peephole
  pass: constants and locals become immediate/memory operands, comparisons
  become a single `cmp`/`setcc`, array addressing folds into `disp(%rbp,%rax,8)`,
  call arguments load straight from their operands, and division by a constant
  becomes a magic multiply and shift instead of `idiv`.
- Arrays live on the stack; there is no heap except the bump allocator.
- `nrc` reads and writes programs that use `i64` and `bool`; there is no
  overflow checking, and division by zero traps at the CPU level.

## License

Provided as-is for experimentation.
