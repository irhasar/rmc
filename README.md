# rmc — an experimental Rust-subset ahead-of-time compiler

`rmc` compiles a small, statically-typed subset of Rust straight to x86-64
assembly. It does **not** use LLVM, does **not** use any external crate, and
does **not** link a C runtime. The emitted assembly is assembled with GNU `as`
and linked with GNU `ld` against a freestanding runtime that talks to the Linux
kernel through raw syscalls.

The point of the experiment is to measure the trade-off at the extremes:

| property                         | rmc                                   | rustc (LLVM)                     |
| -------------------------------- | ------------------------------------- | -------------------------------- |
| compile pipeline                 | lex → parse → typecheck → asm → `as` → `ld` | MIR → LLVM IR → LLVM opt → MC → `ld` |
| third-party code                 | none (Rust `std` only)                | LLVM + rustc                          |
| compiler peak RSS                | ~12 MB                                | ~188 MB                          |
| compile time (small programs)    | ~7 ms                                 | ~2.1 s                           |
| artifact size                    | ~6 KB (static, no libc)               | ~290 KB                          |
| generated-code quality           | peephole + top-five local promotion   | highly optimised                 |

`rmc` wins big on compile time, compiler memory, and artifact size. On runtime
it is now within a few percent on divide-heavy and array-heavy loops, and still
loses on call-heavy code where LLVM's register allocator and loop
transformations (for example turning the naive double-recursive `fib` into a
loop) do the work a stack-machine code generator cannot.

## Layout

```
compiler/            the rmc compiler (Rust, std only, no dependencies)
  src/lib.rs         library entry point: compile_to_asm(src, options)
  src/lexer.rs       source -> tokens
  src/parser.rs      tokens -> AST
  src/ast.rs         AST + type definitions
  src/sema.rs        type checking, struct layout, slot allocation, builtins
  src/codegen.rs     AST -> x86-64 assembly (System V, AT&T syntax)
  src/runtime.rs     freestanding runtime written in assembly
  src/main.rs        CLI driver: source -> asm -> as -> ld
  tests/compile.rs   unit tests for the front end and code generator
bench/
  programs/*.rms     benchmark programs in the rmc subset
  rust/*.rs          the same programs written in real Rust
  run.py             benchmark harness (compile time, CPU, RAM, size, runtime)
tests/
  run.py             differential test suite: rmc vs rustc on twin programs
  cases/*.rms, *.rs  paired programs that must behave identically
  args.json          per-case argv vectors (for runtime inputs)
docs/
  language.md        the supported language subset
  design.md          compiler architecture and code generation notes
```

## Building

```sh
cargo build --release --manifest-path compiler/Cargo.toml
```

The compiler binary is `compiler/target/release/rmc`. It needs `as` and `ld`
from binutils on `PATH`; no other tools are required.

## Usage

```sh
rmc input.rms -o output            # compile, assemble and link
rmc input.rms --emit-asm           # print the generated assembly to stdout
rmc input.rms --keep-asm -o out    # keep the intermediate .s file
rmc input.rms --overflow-checks    # trap on i64 +,-,* overflow (exit 101)
```

Run the produced binary directly; it is a static ELF executable with no
dynamic dependencies.

## Using rmc as a library

`rmc` is both a binary and a library. `compile_to_asm` turns a program into a
complete x86-64 assembly unit (program plus runtime); you can assemble and link
it yourself, or just inspect it.

```rust
let asm = rmc::compile_to_asm(
    "fn main() -> i64 { let mut x: i64 = 0; for i in 0..10 { x += i; } return x; }",
    &rmc::CompileOptions::default(),
)?;
std::fs::write("out.s", asm)?;
```

## Testing

`tests/run.py` is a differential test suite. Every case is a pair of programs,
`tests/cases/NAME.rms` and `tests/cases/NAME.rs`, written to behave identically;
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

Supported: `i64`, `bool`, fixed-size arrays, and structs with named fields;
functions with up to six parameters; `let` / `let mut`; assignment to variables,
array elements and struct fields (including compound `+=` and friends);
`if`/`else`, `while`, `loop`, and `for i in a..b`; `break`, `continue`,
`return`; arithmetic and comparison operators; `&&`/`||`; string literals;
attributes such as `#[derive(Copy, Clone)]` (parsed and ignored); and a small
set of runtime builtins. Structs and arrays may be passed to and returned from
functions by value. Overflow checks are opt-in with `--overflow-checks`. See
`docs/language.md` for the full grammar.

Runtime builtins (provided by `src/runtime.rs`, no imports needed):

| builtin         | signature            | behaviour                              |
| --------------- | -------------------- | -------------------------------------- |
| `print_i64(x)`  | `(i64) -> ()`        | print an integer followed by `\n`      |
| `print_str(s)`  | `(i64) -> ()`        | print a string literal                 |
| `print_bool(b)` | `(bool) -> ()`       | print `true` or `false`                |
| `putchar(c)`    | `(i64) -> ()`        | write a single byte                    |
| `alloc(n)`      | `(i64) -> i64`       | allocate `n` bytes (first-fit free list) |
| `free(p)`       | `(i64) -> ()`        | return a block from `alloc` to the heap |
| `arg_i64(i)`    | `(i64) -> i64`       | parse `argv[i]` as a signed integer    |

## Benchmarks

`bench/run.py` compares `rmc` against `rustc -O -C lto=fat -C codegen-units=1
-C panic=abort -C strip=symbols` on six programs. It measures each compile with
`wait4()` so CPU time and peak RSS cover the real process (including `as`/`ld`
for rmc and LLVM for rustc). Programs read their workload from `argv` via
`black_box` / `arg_i64`, so nothing is constant-folded away.

```sh
python3 bench/run.py
```

Representative run on the development machine (Debian 13, gcc 14.2, rustc
1.99.1 / LLVM 23.1.1, x86-64, 16 GB RAM; medians of 5 compiles and 9 runs):

| program | compiler | compile wall | compile CPU | peak RSS | artifact | run time |
| ------- | -------- | -----------: | ----------: | -------: | -------: | -------: |
| hello   | rmc      |     7.61 ms  |    7.33 ms  |  12.1 MB |   6504 B |  1.57 ms |
| hello   | rustc    |  2079.03 ms  | 2079.19 ms  | 185.1 MB | 289416 B |  2.19 ms |
| fib     | rmc      |     7.48 ms  |    7.21 ms  |  12.1 MB |   6608 B | 15.82 ms |
| fib     | rustc    |  2117.46 ms  | 2122.32 ms  | 186.5 MB | 291696 B |  8.48 ms |
| mandel  | rmc      |     8.49 ms  |    7.55 ms  |  12.1 MB |   6912 B |103.87 ms |
| mandel  | rustc    |  2124.81 ms  | 2128.81 ms  | 187.0 MB | 291776 B |108.42 ms |
| sieve   | rmc      |    10.71 ms  |   10.46 ms  |  12.1 MB |   6672 B |  2.97 ms |
| sieve   | rustc    |  2087.45 ms  | 2091.65 ms  | 185.5 MB | 292016 B |  3.82 ms |
| mix     | rmc      |     7.41 ms  |    7.17 ms  |  12.1 MB |   6704 B | 18.58 ms |
| mix     | rustc    |  2162.71 ms  | 2166.82 ms  | 186.2 MB | 291864 B | 17.08 ms |
| stats   | rmc      |     7.54 ms  |    7.29 ms  |  12.1 MB |   6784 B |  5.61 ms |
| stats   | rustc    |  2147.54 ms  | 2151.90 ms  | 186.6 MB | 292048 B |  5.20 ms |

Summary of the ratios (rustc ÷ rmc):

| dimension        | ratio      |
| ---------------- | ---------: |
| compile wall     | ~195–292×  |
| compile CPU      | ~200–302×  |
| compiler peak RSS| ~15.4×     |
| artifact size    | ~42–45×    |
| run time         | 0.54–1.39× (rustc faster on call-heavy code) |

`rmc` beats rustc on `hello` (1.39×), `sieve` (1.29×) and `mandel` (1.04×)
because the workload is a tight loop or a single call and the static binary has
no libc startup. On `fib` (0.54×, call-heavy), `mix` (0.92×) and `stats` (0.93×)
LLVM's loop transformations and register allocation still win. `stats` holds its
accumulators in a struct, which rmc keeps in memory, so it is a fair illustration
of where a real register allocator would pay off.

### Reproducing the comparison honestly

The `rustc` invocation enables fat LTO and a single codegen unit, which is the
closest equivalent to "compile everything, then link". The `rmc` time includes
spawning `as` and `ld`; the `rustc` time includes LLVM and the system linker.
Both binaries are measured the same way, and both read their workload from
`argv` so neither can fold the answer at compile time.

## Honest limitations

rmc is a small, self-contained compiler, not a drop-in replacement for rustc.

- The language subset is small (see `docs/language.md`). Structs, arrays, `for`,
  aggregates across calls and overflow checks exist; enums, traits, generics,
  `match`, references and borrowing, methods, closures, floating point and the
  standard library do not. Aggregate values are passed by value through a hidden
  pointer, but there is no way to pass one *by reference*, and a copy is made on
  every call.
- Code generation is a stack machine for expressions. The hottest five scalar
  locals in each function are pinned to callee-saved registers; everything else
  is in memory. The optimiser is a peephole pass (constant operands, folded
  comparisons, folded array addressing, division by a constant); there is no
  instruction scheduling, inlining, or loop transformation.
- The backend targets x86-64 Linux only, using the System V ABI and raw Linux
  syscalls. There is no Windows, macOS, ARM, or WebAssembly support.
- The heap is a simple first-fit free-list allocator; it does not return memory
  to the kernel and has no thread safety.
- `rmc` is one person's project with a differential test suite and CI, not a
  funded product.

## Roadmap

What would move rmc from a serious toy toward something usable, roughly in order
of value per unit of effort:

1. **A real register allocator.** Linear scan over live ranges, replacing the
   whole-function top-five heuristic. This is the single biggest win for the
   call-heavy benchmarks that rmc still loses.
2. **References and borrowing.** `&T` / `&mut T` parameters would remove the
   copy an aggregate argument makes today, and are the prerequisite for most
   data-structure code.
3. **More types.** `u*`/`i*` widths, `f64`, and `match` on integers, which
   together cover most of what small programs actually need.
4. **Enums and `impl` methods.** The next step after structs toward expressing
   real data structures.
5. **More backends.** A second target (ARM64 or a simple VM) would force the
   code generator's assumptions out into the open.
6. **Debug info.** Line tables so a debugger can point at source.

Anything involving borrowing, generics or the trait system is out of scope: it
would make rmc a different project rather than a better one.

## License

Provided as-is for experimentation.
