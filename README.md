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
tools/
  fuzz.py            differential fuzzer and robustness checker
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
rmc input.rms --bounds-checks      # trap on an out-of-range array index (exit 101)
rmc input.rms --no-inline          # disable inlining (on by default)
rmc input.rms --no-licm            # disable loop-invariant code motion (on by default)
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
calls and recursion, the output builtins, nested expressions, and arrays of `i64` and `bool` built
from either literal form. It is
deliberately adversarial: reverting the folded-comparison operand-order fix
makes the `cmp` case fail, and dropping the sign bias from division by a power
of two makes the `div` case fail.

`tools/fuzz.py` generates random programs and checks them against `rustc -O`
the same way, for the cases a hand-written suite does not think of. It has two
modes: `expr` for integer expressions, and `stmt` for loops, conditionals,
`match`, arrays and aggregate calls (including argument evaluation order). Seeds
are reproducible, so a failure can be re-run and shrunk.

```sh
python3 tools/fuzz.py --mode expr --n 500 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1
python3 tools/fuzz.py --mode robust
```

`robust` mode needs no rustc: it feeds a fixed corpus of malformed and
adversarial inputs (random bytes, unbalanced braces, and thousands of nested
parentheses, braces, unary operators or binary operators) and requires each to
produce a clean error rather than a panic or a signal. This is what catches an
unbounded parser recursion overflowing the stack on untrusted input. CI runs it
together with a small fixed-seed differential fuzz.

A caveat for `expr`: an integer literal in a shift amount is unconstrained, so
rustc infers `i32` and masks the count to 5 bits, while rmc is `i64`-only and
masks to 6. The generator therefore keeps shift operands `i64`-typed, or the two
would disagree for a reason that is not a bug.

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
`if`/`else`, `while`, `loop`, and `for i in a..b`; `match` on integers (literal
patterns and `_`); `break`, `continue`,
`return`; arithmetic, comparison, bitwise (`&`/`|`/`^`) and shift (`<<`/`>>`)
operators; `&&`/`||`; string literals; references (`&T` / `&mut T`, `&e`, `*e`,
with auto-deref for `r.x`, `r[i]` and `r.m()`, and `&self` receivers);
attributes such as `#[derive(Copy, Clone)]` (parsed and ignored); and a small
set of runtime builtins. Structs and arrays may be passed to and returned from
functions by value, or by reference to avoid the copy. Overflow checks are
opt-in with `--overflow-checks`, and array bounds checks with
`--bounds-checks`. See `docs/language.md` for the full grammar.

Runtime builtins (provided by `src/runtime.rs`, no imports needed):

| builtin         | signature            | behaviour                              |
| --------------- | -------------------- | -------------------------------------- |
| `print_i64(x)`  | `(i64) -> ()`        | print an integer followed by `\n`      |
| `print_str(s)`  | `(i64) -> ()`        | print a string literal                 |
| `print_bool(b)` | `(bool) -> ()`       | print `1` or `0`                       |
| `putchar(c)`    | `(i64) -> ()`        | write a single byte                    |
| `alloc(n)`      | `(i64) -> i64`       | allocate `n` bytes (first-fit free list) |
| `free(p)`       | `(i64) -> ()`        | return a block from `alloc` to the heap |
| `arg_i64(i)`    | `(i64) -> i64`       | parse `argv[i]` as a signed integer    |

## Benchmarks

`bench/run.py` compares `rmc` against `rustc -O -C lto=fat -C codegen-units=1
-C panic=abort -C strip=symbols` on eight programs. It measures both halves of
the comparison, because "faster" means two different things here:

* **compile** — build wall time, compiler CPU time, compiler peak RAM, and the
  artifact size (the linked executable on disk);
* **run** — the wall time, CPU time and peak RAM of the compiled program.

Peak RAM is read from each process's own `VmHWM`, so the number is the process
itself, not the harness around it. Programs read their workload from `argv` via
`black_box` / `arg_i64`, so nothing is constant-folded away. The full, dated
result is recorded in [`bench/RESULTS.md`](bench/RESULTS.md); regenerate it with:

```sh
python3 bench/run.py --markdown bench/RESULTS.md
```

Representative run on the development machine (Debian 13, rustc 1.99.1 /
LLVM 23.1.1, AMD EPYC 9B14, x86-64; medians of 5 compiles and 9 runs):

| program | compiler | compile wall | compile CPU | compile RAM | artifact | run wall | run CPU | run RAM |
| ------- | -------- | -----------: | ----------: | ----------: | -------: | -------: | ------: | ------: |
| hello | rmc | 7.12 ms | 7.06 ms | 2.6 MB | 6616 B | 0.26 ms | 0.14 ms | 24 kB |
| hello | rustc | 2083.20 ms | 2086.46 ms | 185.7 MB | 289416 B | 0.85 ms | 0.73 ms | 2116 kB |
| fib | rmc | 6.78 ms | 6.70 ms | 2.6 MB | 6704 B | 10.21 ms | 10.04 ms | 28 kB |
| fib | rustc | 2166.56 ms | 2169.62 ms | 185.4 MB | 291696 B | 7.42 ms | 7.28 ms | 2048 kB |
| mandel | rmc | 6.88 ms | 6.81 ms | 2.6 MB | 6992 B | 96.17 ms | 95.96 ms | 28 kB |
| mandel | rustc | 2138.01 ms | 2142.62 ms | 187.9 MB | 291776 B | 105.56 ms | 105.30 ms | 2120 kB |
| sieve | rmc | 9.16 ms | 9.07 ms | 4.2 MB | 6744 B | 1.65 ms | 1.52 ms | 1588 kB |
| sieve | rustc | 2126.10 ms | 2131.05 ms | 185.7 MB | 292016 B | 2.51 ms | 2.39 ms | 3668 kB |
| mix | rmc | 6.87 ms | 6.80 ms | 2.6 MB | 6760 B | 16.85 ms | 16.70 ms | 24 kB |
| mix | rustc | 2135.36 ms | 2139.68 ms | 188.2 MB | 291864 B | 15.50 ms | 15.35 ms | 2108 kB |
| stats | rmc | 7.11 ms | 7.07 ms | 2.6 MB | 6832 B | 3.78 ms | 3.56 ms | 28 kB |
| stats | rustc | 2154.27 ms | 2154.25 ms | 185.4 MB | 292048 B | 3.74 ms | 3.61 ms | 2112 kB |
| refs | rmc | 7.04 ms | 6.95 ms | 2.7 MB | 7344 B | 4.99 ms | 4.77 ms | 28 kB |
| refs | rustc | 2145.37 ms | 2149.37 ms | 188.9 MB | 292240 B | 4.99 ms | 4.83 ms | 2116 kB |
| heap | rmc | 8.02 ms | 7.97 ms | 2.6 MB | 6736 B | 3.21 ms | 3.07 ms | 4712 kB |
| heap | rustc | 2157.17 ms | 2161.75 ms | 185.2 MB | 292368 B | 3.17 ms | 3.05 ms | 5124 kB |

Summary of the ratios (rustc ÷ rmc, so >1 means rmc is better):

| dimension         | ratio      |
| ----------------- | ---------: |
| compile wall      | ~232–319×  |
| compile CPU       | ~235–324×  |
| compiler peak RAM | ~45–74×    |
| artifact size     | ~40–44×    |
| run wall time     | 0.73–3.24× (rustc faster on call-heavy code; parity on the heap- and reference-heavy cases) |
| run CPU           | 0.73–5.04× |
| run peak RAM      | ~1.1× on the heap-heavy case, ~73–88× elsewhere |

The compile-time win is the point of the project: `rmc` builds in about 6–9 ms
where rustc needs about 2.1 s, using ~2.6 MB of RAM against ~186 MB and writing
a ~7 KB static binary against a ~290 KB one. The run-time picture is mixed and
close: `rmc` wins on `hello` (3.2×), `sieve` (1.5×) and `mandel` (1.1×) because
the workload is a tight loop or a single call and the static binary has no libc
startup, and it is at parity on `heap` (0.99×) and `refs` (1.00×) after the
allocator and store-folding work below. It loses on `fib` (0.73×, call-heavy —
rustc turns the right-spine recursion into a loop, rmc keeps two calls per
level) and on `mix` (0.92×, where LLVM inlines the whole loop-bearing `gcd` into
`main` while rmc's inliner deliberately leaves loops alone). Run times are the
noisiest column: they move ±20% between recordings on a shared machine, so treat
a single run's ratio as indicative and re-measure with many samples before
trusting a sub-5% difference. (An early `refs` run reported 0.14×; that was LLVM
folding the *whole* loop into `128 * n` in the Rust twin, not a measurement of
the loop. The twin now `black_box`es a term, and `refs` sits at ~1.00×.) The
binary's tiny footprint also shows at run time: an `rmc` program peaks at ~28 kB
of RAM, a rustc one at ~2 MB — except on `heap` and `sieve`, where the arena
each program allocates dominates and the two converge.

### Reproducing the comparison honestly

The `rustc` invocation enables fat LTO and a single codegen unit, which is the
closest equivalent to "compile everything, then link". The `rmc` time includes
spawning `as` and `ld`; the `rustc` time includes LLVM and the system linker.
Both compilers and both binaries are measured the same way, peak RAM comes from
each process's own `VmHWM`, and both programs read their workload from `argv` so
neither can fold the answer at compile time. The Rust twins additionally
`black_box` their input, because a workload the optimiser can see through is not
a workload: LLVM otherwise closes the `refs` loop into `128 * n`, which measured
its own constant folding rather than the loop both compilers are supposed to be
running. Every twin that has a loop now passes its trip count (or a term inside
it) through `black_box`.

## Honest limitations

rmc is a small, self-contained compiler, not a drop-in replacement for rustc.

- The language subset is small (see `docs/language.md`). Structs, arrays, `for`,
  aggregates across calls, overflow checks, integer `match`, enums and
  references exist; traits, generics, borrowing, closures, floating point and
  the standard library do not. An aggregate is passed by value through a hidden
  pointer unless a reference (`&T` / `&self`) is used, which passes the address
  and copies nothing. References are not checked: nothing proves a reference
  outlives its referent, and `&mut` carries no exclusivity guarantee.
- Code generation is a stack machine for expressions. The hottest five scalar
  locals in each function — including the scalar fields of a struct that is
  never copied, returned or passed by value — are pinned to callee-saved
  registers; everything else is in memory. Temporaries are spilled to frame slots, not the machine stack, so
  `%rsp` never moves and a call needs no alignment pad. The optimiser is an
  inlining pass (small, straight-line, scalar functions), a loop-invariant code
  motion pass, and a peephole pass (constant operands, folded comparisons, folded
  array addressing, `lea` strength reduction of `k*x+c`, division by a constant,
  in-place accumulation of a running sum); there is no instruction scheduling or
  other loop transformation, and no live-range analysis. Inlining runs on the AST
  after semantic analysis and expands to a fixpoint, so a wrapper is inlined once
  its own callees are; loop-invariant code motion then hoists the arithmetic that
  inlining exposed out of a loop body. `--no-inline` and `--no-licm` turn them
  off.
- The backend targets x86-64 Linux only, using the System V ABI and raw Linux
  syscalls. There is no Windows, macOS, ARM, or WebAssembly support.
- Array indexing is unchecked by default: an out-of-range index reads or writes
  adjacent stack memory, exactly as in C. `--bounds-checks` inserts a check
  before every index that traps with `rmc: index out of bounds` (exit 101); a
  constant index is decided at compile time.
- The heap is a simple first-fit free-list allocator; it does not return memory
  to the kernel and has no thread safety.
- `rmc` is one person's project with a differential test suite and CI, not a
  funded product.

## Roadmap

What would move rmc from a serious toy toward something usable, roughly in order
of value per unit of effort:

1. **A real register allocator.** Spill temporaries now live in callee-saved
   registers (see design.md), which is the part call-heavy code notices. A full
   linear scan over live ranges would also let a dying temporary's register go to
   a local born after it.
2. **Borrowing.** `&T` / `&mut T` now exist: a reference is one word, so a
   `&T` parameter or a `&self` receiver is passed by address rather than copied,
   and `*r = v` writes through a reference (see language.md). What is missing is
   the check — rmc does not prove a reference outlives its referent or that
   `&mut` is exclusive, so an aliasing bug is a wrong answer, not a compile
   error.
3. **More types.** `u*`/`i*` widths and `f64`, which together cover most of what
   small programs actually need. (`match` on integers is done.)
4. **More backends.** A second target (ARM64 or a simple VM) would force the
   code generator's assumptions out into the open.
5. **Debug info.** Line tables so a debugger can point at source.

Anything involving borrowing, generics or the trait system is out of scope: it
would make rmc a different project rather than a better one.

## License

Provided as-is for experimentation.
