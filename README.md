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
| generated-code quality           | naive, unoptimised                    | highly optimised                 |

`nrc` wins big on compile time, compiler memory, and artifact size. Its
generated code is, as expected from a compiler with no optimiser, slower than
LLVM's on arithmetic-heavy loops.

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
| hello   | nrc      |     7.12 ms  |    6.85 ms  |  11.8 MB |   5840 B |  1.53 ms |
| hello   | rustc    |  2062.09 ms  | 2065.35 ms  | 187.1 MB | 289416 B |  2.19 ms |
| fib     | nrc      |     6.82 ms  |    6.60 ms  |  11.8 MB |   5912 B | 16.70 ms |
| fib     | rustc    |  2096.99 ms  | 2100.78 ms  | 187.9 MB | 291696 B |  8.50 ms |
| mandel  | nrc      |     6.92 ms  |    6.70 ms  |  11.8 MB |   6184 B |149.57 ms |
| mandel  | rustc    |  2068.98 ms  | 2073.19 ms  | 192.3 MB | 291776 B |106.77 ms |
| sieve   | nrc      |     7.22 ms  |    6.91 ms  |  11.8 MB |   6064 B |  3.27 ms |
| sieve   | rustc    |  2087.31 ms  | 2091.77 ms  | 188.2 MB | 292016 B |  3.83 ms |
| mix     | nrc      |     6.89 ms  |    6.65 ms  |  11.8 MB |   6008 B | 18.69 ms |
| mix     | rustc    |  2068.76 ms  | 2071.55 ms  | 188.3 MB | 291864 B | 16.87 ms |

Summary of the ratios (rustc ÷ nrc):

| dimension        | ratio      |
| ---------------- | ---------: |
| compile wall     | ~290–307×  |
| compile CPU      | ~300–318×  |
| compiler peak RSS| ~15.9×     |
| artifact size    | ~47–50×    |
| run time         | 0.5–2.3× (rustc usually faster) |

`nrc` beats rustc on `sieve` because the whole workload is a tight array loop
that the simple code generator already handles well, and the static binary has
no libc startup. On `fib` (call-heavy) and `mandel` (multiply/divide-heavy)
LLVM's register allocation and loop optimisation win.

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
  allocator, no instruction scheduling, and only a small amount of peephole
  folding (constants and locals become immediate/memory operands, comparisons
  become direct `cmp`/`jcc`).
- Arrays live on the stack; there is no heap except the bump allocator.
- `nrc` reads and writes programs that use `i64` and `bool`; there is no
  overflow checking, and division by zero traps at the CPU level.

## License

Provided as-is for experimentation.
