# rmc — an experimental Rust-subset ahead-of-time compiler

`rmc` compiles a small, statically-typed subset of Rust straight to x86-64
assembly, and then to a static ELF64 executable. It does **not** use LLVM, does
**not** use any external crate, and does **not** link a C runtime. By default it
does not even shell out to an assembler or linker: the assembly the code
generator emits is assembled and linked in-process, and the result runs against
a freestanding runtime that talks to the Linux kernel through raw syscalls.
`--binutils` keeps the old path through GNU `as` and `ld` for the cases where
the assembly text is what you want.

The point of the experiment is to measure the trade-off at the extremes:

| property                         | rmc                                   | rustc (LLVM)                     |
| -------------------------------- | ------------------------------------- | -------------------------------- |
| compile pipeline                 | lex → parse → typecheck → asm → ELF encoder | MIR → LLVM IR → LLVM opt → MC → `ld` |
| third-party code                 | none (Rust `std` only)                | LLVM + rustc                          |
| compiler peak RSS                | ~2.7 MB                               | ~188 MB                          |
| compile time (small programs)    | ~1.5 ms                               | ~2.1 s                           |
| artifact size                    | ~0.5–0.9 KB (static, no libc)         | ~290 KB (fat-LTO stripped); ~4.5 MB (plain `-O`); ~1.2 MB (static stripped) |
| generated-code quality           | peephole + top-five local promotion   | highly optimised                 |

`rmc` wins big on compile time, compiler memory, and artifact size, and those
three are the point of the experiment. Because it links in-process rather than
forking `as` and `ld`, a small program compiles in about 1.2–1.6 ms against
rustc's ~2.1 s. On **runtime** the result is mixed and program-dependent: rmc is
at parity or ahead on almost the whole suite (`hello` 3.9×, `forloops` 2.3×,
`refs` 2.2×, `sieve` 1.6×, `mandel` 1.14×, `stats` 1.13×, `heap` 1.05×, `fib`
1.02×), ties on `callrec` (0.99×), and trails on `mix` (0.92×). The loop-body
alignment below is what pulled `fib` back from an earlier 0.80× reading: its
code had not changed, the loop had simply landed on a worse fetch boundary.
Several of those are within run-to-run noise, and a call-heavy shape the
recursion-to-loop pass does not match would lose. It is not uniformly faster and
there is no single speedup number; `bench/RESULTS.md` reports the per-program
ratio.

The artifact-size comparison depends entirely on which rustc build is the
reference. Against a plain `rustc -O` (which statically links libstd, the panic
runtime and the allocator) rmc is ~4884–9492× smaller; against a stripped static
build, ~1273–2470×; against an aggressive fat-LTO stripped build, ~314–606×. The
first is the closest like-for-like, the last is the one that flatters rustc
least. The gap is this wide because the linker keeps only the runtime pieces a
program reaches: a program that never allocates carries no allocator, and one
that never indexes an array carries no bounds trap.

## Layout

```
compiler/            the rmc compiler (Rust, std only, no dependencies)
  src/lib.rs         library entry point: compile_to_asm(src, options)
  src/lexer.rs       source -> tokens
  src/parser.rs      tokens -> AST
  src/ast.rs         AST + type definitions
  src/sema.rs        type checking, struct layout, slot allocation, builtins
  src/codegen.rs     AST -> x86-64 assembly (System V, AT&T syntax)
  src/elf.rs         in-process assembler + ELF64 linker (no `as`, no `ld`)
  src/dce.rs         dead-function elimination (drop what main cannot reach)
  src/runtime.rs     freestanding runtime written in assembly
  src/main.rs        CLI driver: source -> asm -> ELF executable
  tests/compile.rs   unit tests for the front end and code generator
bench/
  rust/*.rs          benchmark programs (plain Rust, compiled by rmc)
  run.py             benchmark harness (compile time, CPU, RAM, size, runtime)
tools/
  fuzz.py            differential fuzzer and robustness checker
tests/
  run.py             differential test suite: rmc vs rustc on twin programs
  cases/*.rs         programs compiled by both rmc and rustc
  args.json          per-case argv vectors (for runtime inputs)
docs/
  language.md        the supported language subset
  design.md          compiler architecture and code generation notes
```

## Building

```sh
cargo build --release --manifest-path compiler/Cargo.toml
```

The compiler binary is `compiler/target/release/rmc`. Nothing else is required
to build a program: the default path needs no assembler and no linker. `as` and
`ld` from binutils are needed only for `--binutils`, and `--emit-asm`.

## Usage

```sh
rmc input.rs -o output             # compile and link in-process (default)
rmc input.rs --emit-asm            # print the generated assembly to stdout
rmc input.rs --binutils -o out     # assemble and link with `as`/`ld` instead
rmc input.rs --binutils --no-strip -o out    # keep the symbol table
rmc input.rs --binutils --min-size -o out    # drop the section headers too
rmc input.rs --overflow-checks     # trap on i64 +,-,* overflow (exit 101)
rmc input.rs --bounds-checks       # trap on an out-of-range array index (exit 101)
rmc input.rs --no-inline           # disable inlining (on by default)
rmc input.rs --no-licm             # disable loop-invariant code motion (on by default)
rmc input.rs --no-tco              # disable self-recursion-to-loop (on by default)
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

`tests/run.py` is a differential test suite. Every case is a single program,
`tests/cases/NAME.rs`, compiled by both rmc and rustc; the runner runs both and
fails if stdout or the exit status differs. `tests/args.json` optionally maps a case to argv vectors so the same
program is exercised with several runtime inputs. Each case is then recompiled
with the optimiser passes turned off in turn (`--no-inline`, `--no-licm`,
`--no-tco`, and all three at once), since a bug in one pass can hide behind
another or only appear on the unoptimised path; the Rust twin is built once and
reused across the five configurations.

```sh
python3 tests/run.py -v
```

The suite covers arithmetic and division semantics (including divisors that need
the magic-multiply add fixup and powers of two), every comparison operator with
the immediate on either side, string comparison by content (which a pointer
compare would get wrong), boolean short-circuiting, control flow, arrays,
calls and recursion, the output builtins (including `print!`/`println!`
formatting and the `assert!` family), `use` and `std` paths, `str`/`&str`
coercion, nested expressions, and arrays of `i64` and `bool` built
from either literal form. It is
deliberately adversarial: reverting the folded-comparison operand-order fix
makes the `cmp` case fail, dropping the sign bias from division by a power of
two makes the `div` case fail, and comparing string pointers instead of their
bytes makes the `strcmp` case fail.

`tools/fuzz.py` generates random programs and checks them against `rustc -O`
the same way, for the cases a hand-written suite does not think of. It has
several modes: `expr` for integer expressions (including comparisons) and
string comparisons, `stmt` for loops, conditionals, `match`, arrays and
aggregate calls (including argument
evaluation order), `opt` for a self-recursive function, an invariant loop and
small inlined helpers (the shapes the other two do not emit), `arith` for
overflowing arithmetic near `i64::MAX` (the only mode that makes the
`--overflow-checks` trap fire), `nest` for nested aggregates with a deep copy
and a nested-path write, `pat` for `match` expressions (multi-variant enums,
two-field payloads, explicit discriminants, `bool` matches, binding patterns,
struct patterns, nested matches, `if let`, string patterns, per-arm guards, and
payload variant patterns whose arguments take sub-patterns (`_`, `mut x`, a
trailing `..`, a nested struct/array pattern) and read through a `&E` scrutinee;
a side-effecting
scrutinee), `letdestr` for `let` struct destructuring (shorthand, renamed and
`mut` bindings, `_` and `..`, a call and a reference scrutinee, and a
struct-typed field destructured in turn), `fordestr` for `for S { .. } in arr`
destructuring (the head evaluated once, per-iteration rebinding, `break`, and
the same pattern grammar as `let`), `nestpat` for nested struct sub-patterns
(a chain of struct types so the pattern nests several levels deep, exercised in
`let`, `match`, `if let` and `for`), `arrpat` for array destructuring patterns
(`let [a, b, .., c] = arr`, `for [x, y] in pairs`, `match`/`if let` arms, with
`_` elements, `mut` bindings, nested and struct element patterns, `..` rests
in every position, and an array-typed struct field destructured in place),
`parampat` for aggregate parameter patterns (struct and array patterns, a
nested one, one mixed with plain parameters, and one on a method receiver),
`enumpat` for payload variant patterns with sub-patterns (`_`, `mut x`, a
trailing `..`, and a nested struct or array pattern inside `E::V(..)`, in
`match`, `if let` and a method, and read through a `&E` scrutinee), `atpat` for
`@` bindings (`n @ _`, `n @ m`, `t @ (a, b)`, a struct field `f: n @ _`, a `for`
head, a refutable `n @ 1..=5` in a `match` arm and an `if let`, and `n @ _` /
`mut n @ _` parameters), `tuple` for
tuples (construction, `.0` element access, destructuring in a `let`, a `match`
arm and a parameter, nesting, a tuple-typed struct field, an array of tuples,
and a tuple behind a reference, used as parameter and return types), `nestedfn`
for nested `fn` items (hoisting, a call before the declaration, a sibling called
after it, a nested `fn` inside another body, a nested `fn` shadowing a top-level
one of the same name, and the shadowed name called to confirm the inner one
wins), `typealias` for `type` aliases (a scalar, tuple, array and struct alias,
an alias as a parameter and return type, one alias naming another, an alias used
before it is declared, an alias behind a reference and in a cast), `printfmt`
for `print!`/`println!` (a literal format with `{}` filled by an `i64`, a
`bool`, a `char` or a `str`, literal `{{`/`}}` braces, escapes, a leading `\n`,
and a
bare `println!()` with no format string), `char` for `char` literals
(plain, escaped, `\xNN`, `\u{..}`), `as i64`, `char` ordering and equality, a
`char` `match` with literal, or-pattern and range arms, a `char` array, a
`const char`, a `char` parameter/return, and the `print_char` builtin, and
`order` for assignment evaluation order (a
side-effecting index and value on both `=` and every compound operator, the
shapes `stmt` never assigns through). `shift` drives `<<`, `>>` and the compound
`<<=`/`>>=` with a shift count from `argv`, so the count is opaque to the
compiler and can be out of range (negative or >= 64): every other generator
masks its counts with `& 63` to keep them valid, which left the shift-overflow
trap unreachable by fuzzing. Every program is recompiled with each optimiser pass off in turn.
Seeds are reproducible, so a failure can be re-run and shrunk.

```sh
python3 tools/fuzz.py --mode expr --n 500 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1
python3 tools/fuzz.py --mode opt --n 500 --seed 1
python3 tools/fuzz.py --mode arith --n 500 --seed 1
python3 tools/fuzz.py --mode nest --n 500 --seed 1
python3 tools/fuzz.py --mode nestedfn --n 500 --seed 1
python3 tools/fuzz.py --mode typealias --n 500 --seed 1
python3 tools/fuzz.py --mode printfmt --n 500 --seed 1
python3 tools/fuzz.py --mode pat --n 500 --seed 1
python3 tools/fuzz.py --mode order --n 500 --seed 1
python3 tools/fuzz.py --mode shift --n 500 --seed 1
python3 tools/fuzz.py --mode semi --n 500 --seed 1
python3 tools/fuzz.py --mode boolbit --n 500 --seed 1
# also exercise the trap codegen: --bounds-checks and --overflow-checks runs
python3 tools/fuzz.py --mode stmt --n 500 --seed 1 --bounds --overflow
python3 tools/fuzz.py --mode robust
python3 tools/fuzz.py --mode reject
python3 tools/fuzz.py --mode mutate --n 500 --seed 1
```

`robust` mode needs no rustc: it feeds a fixed corpus of malformed and
adversarial inputs (random bytes, unbalanced braces, and thousands of nested
parentheses, braces, unary operators or binary operators) and requires each to
produce a clean error rather than a panic or a signal. This is what catches an
unbounded parser recursion overflowing the stack on untrusted input. `reject`
mode is the soundness mirror: it holds a corpus of programs `rustc` rejects and
requires `rmc` to reject each one too, never accept it. `mutate` mode applies
random edits (splices, deletions, duplications) to the generators' programs and
the `robust` corpus, and requires every mutant to fail cleanly; it needs no
rustc and reaches error paths the fixed corpora miss. CI runs `robust` together
with a small fixed-seed differential fuzz.

A caveat for `expr`: an integer literal in a shift amount is unconstrained, so
rustc infers `i32` and masks the count to 5 bits, while rmc is `i64`-only and
masks to 6. The generator therefore keeps shift operands `i64`-typed, or the two
would disagree for a reason that is not a bug. For the same reason the
`i64::MIN` literal it emits is always a typed `let m: i64` binding: untyped,
rustc would infer `i32` and reject it while rmc accepts it.

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

Supported: `i64`, `bool`, `char`, `str`, fixed-size arrays, tuples `(A, B, ..)`,
and structs with named fields
(including field init shorthand, `P { x, y }`, struct update syntax,
`P { x: 9, ..base }`, struct patterns `P { x, y: z, .. }` in `match`/`if let`
(including nested sub-patterns `P { a: I { x }, .. }`), `let` destructuring
`let P { x, y: z, .. } = p;`, `for` destructuring
`for P { x, y: z, .. } in arr { .. }`, and array patterns
`[a, b, .., c]` (in `let`/`for`/`match`/`if let`, with `_`, `mut`, nested
patterns and `..` rests, also usable on an array-typed struct field,
`let P { a: [x, y], .. } = p;`), and tuple patterns
`(a, b)` / `(x, _, mut z)` (the same positions, and `.0`/`.1` element access),
and `@` bindings `n @ PAT` (`n @ _`, `n @ m`, `t @ (a, b)`, a struct field
`f: n @ _`, a `for` head, and `n @ _` / `mut n @ _` parameters, with the
sub-pattern required to be irrefutable in an irrefutable position));
functions with up to six parameters, which may be written as an aggregate
pattern (`fn f(P { x, y }: P)`, `fn f([a, b]: [i64; 2])`, nested and mixed with
plain parameters); `let` / `let mut` / `let _` (discard); assignment to variables,
array elements and struct fields (including compound `+=` and friends);
`if`/`else`, `while`, `loop`, `for i in a..b` / `a..=b` and `for x in arr`
(array iteration, by value); `match` on integers,
`bool`s, `char`s, enums and strings (literal, `true`/`false`, char, string,
`Enum::Variant`, struct and
`_` patterns, plus payload bindings and binding names, and a per-arm guard
`PAT if COND`) and `if let` with the
same patterns
(desugared to a two-arm `match`); enums with unit variants (an optional
explicit discriminant, `enum E { A = 5, B }`) and payload variants; `break`
(with a value in a `loop`, so `loop`
works as an expression), `continue`,
`return`; loop labels (`'a: loop` / `'a: while` / `'a: while let` / `'a: for`, with
`break 'a` / `break 'a value` / `continue 'a` targeting any enclosing labelled
loop); arithmetic, comparison, bitwise (`!`/`&`/`|`/`^`) and shift (`<<`/`>>`)
operators; `&&`/`||`; strings (a `str` literal or `str`-typed parameter, return
or field, compared by content with `==`/`!=`/`<`/`<=`/`>`/`>=`, with `.len()` for
the byte length); array `.len()` (a compile-time constant); built-in `i64` methods
(`x.abs()`, `x.signum()`, `x.min(y)`, `x.max(y)`, `x.clamp(lo, hi)`,
`x.is_positive()`, `x.is_negative()`, `x.div_euclid(y)`, `x.rem_euclid(y)`,
`x.midpoint(y)`,
`x.count_ones()`, `x.count_zeros()`, `x.leading_zeros()`, `x.trailing_zeros()`,
`x.leading_ones()`, `x.trailing_ones()`, `x.swap_bytes()`, `x.reverse_bits()`,
`x.rotate_left(n)`, `x.rotate_right(n)`, desugared to
operators or a small runtime helper, so `i64::MIN.abs()` maps to itself as in
`rustc -O`);
`mut` enforcement and definite
initialisation; `const` items (folded to a value and substituted at every use,
including array lengths and `match` patterns); `type` aliases (a transparent
second name for any type, resolved at parse time, usable before declaration);
references (`&T` / `&mut T`, `&e`, `*e`,
with auto-deref for `r.x`, `r[i]` and `r.m()`, and `&self` receivers); methods
(`self` / `&self` / `&mut self`) and associated functions (`fn name(..)` with no
`self`, called as `Type::name(..)`, the constructor idiom); nested `fn` items (a
`fn` declared in a block, hoisted to the top level since it captures nothing —
block-scoped, so it shadows a top-level `fn` of the same name and is visible
throughout its block, exactly as rustc); `as` casts at Rust's
precedence (looser than the unary operators, so `*r as i64` is `(*r) as i64`);
attributes such as `#[derive(Copy, Clone)]` (parsed and ignored); `use`
statements (parsed and discarded, since every name is resolved globally);
`print!`/`println!` with `{}` substitution, and `assert!`/`assert_eq!`/
`assert_ne!`/`panic!`/`unreachable!` (a failure exits with rustc's panic code
101); the `std::process::exit` and `std::hint::black_box` paths; and a small
set of runtime builtins. A `str` and a `&str` are the same one-word value, so
they coerce freely and a `&'static str` field or `&str` parameter may be
written `str`. Structs and arrays may be passed to and returned from
functions by value, or by reference to avoid the copy. Overflow checks are
opt-in with `--overflow-checks`, and array bounds checks with
`--bounds-checks`. See `docs/language.md` for the full grammar.

Runtime builtins (provided by `src/runtime.rs`, no imports needed):

| builtin         | signature            | behaviour                              |
| --------------- | -------------------- | -------------------------------------- |
| `print_i64(x)`  | `(i64) -> ()`        | print an integer followed by `\n`      |
| `print_str(s)`  | `(str) -> ()`        | print a string followed by `\n`       |
| `print_bool(b)` | `(bool) -> ()`       | print `1` or `0`                       |
| `print_char(c)` | `(char) -> ()`       | write a character as UTF-8, then `\n`  |
| `putchar(c)`    | `(i64) -> ()`        | write a single byte                    |
| `alloc(n)`      | `(i64) -> i64`       | allocate `n` bytes (first-fit free list) |
| `free(p)`       | `(i64) -> ()`        | return a block from `alloc` to the heap |
| `arg_i64(i)`    | `(i64) -> i64`       | parse `argv[i]` as a signed integer    |
| `exit(code)`    | `(i64) -> ()`        | terminate with `code` (raw syscall)    |

## Benchmarks

`bench/run.py` compares `rmc` against `rustc -O -C lto=fat -C codegen-units=1
-C panic=abort -C strip=symbols` on ten programs. It measures both halves of
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
LLVM 23.1.1, AMD EPYC 9B14, x86-64; both compile and run are the minimum of
their reps, 5 and 9 - the workload is deterministic, so the least-contended
sample is the program's own cost, which keeps the ratios stable on a shared
host):

| program | compiler | compile wall | compile CPU | compile RAM | artifact | run wall | run CPU | run RAM |
| ------- | -------- | -----------: | ----------: | ----------: | -------: | -------: | ------: | ------: |
| hello | rmc | 1.20 ms | 1.09 ms | 2.8 MB | 478 B | 0.21 ms | 0.12 ms | 24 kB |
| hello | rustc | 2109.66 ms | 2112.26 ms | 188.6 MB | 289416 B | 0.83 ms | 0.70 ms | 2012 kB |
| fib | rmc | 1.44 ms | 1.26 ms | 2.8 MB | 611 B | 7.12 ms | 7.00 ms | 24 kB |
| fib | rustc | 2070.83 ms | 2074.79 ms | 189.4 MB | 291696 B | 7.23 ms | 7.06 ms | 2084 kB |
| mandel | rmc | 1.39 ms | 1.27 ms | 2.8 MB | 918 B | 92.78 ms | 92.56 ms | 24 kB |
| mandel | rustc | 2114.50 ms | 2118.65 ms | 189.8 MB | 291776 B | 105.46 ms | 105.25 ms | 2028 kB |
| sieve | rmc | 1.31 ms | 1.19 ms | 2.8 MB | 692 B | 1.57 ms | 1.47 ms | 1584 kB |
| sieve | rustc | 2137.30 ms | 2141.19 ms | 186.9 MB | 292016 B | 2.47 ms | 2.35 ms | 3612 kB |
| mix | rmc | 1.35 ms | 1.24 ms | 2.8 MB | 853 B | 16.79 ms | 16.67 ms | 24 kB |
| mix | rustc | 2057.63 ms | 2061.97 ms | 190.9 MB | 291864 B | 15.43 ms | 15.29 ms | 2032 kB |
| stats | rmc | 1.36 ms | 1.25 ms | 2.8 MB | 767 B | 3.30 ms | 3.16 ms | 20 kB |
| stats | rustc | 2112.84 ms | 2109.91 ms | 187.5 MB | 292048 B | 3.73 ms | 3.61 ms | 2104 kB |
| refs | rmc | 1.54 ms | 1.38 ms | 2.8 MB | 890 B | 1.76 ms | 1.64 ms | 24 kB |
| refs | rustc | 2144.25 ms | 2147.20 ms | 192.6 MB | 292240 B | 3.85 ms | 3.72 ms | 2052 kB |
| heap | rmc | 1.35 ms | 1.23 ms | 2.8 MB | 932 B | 3.33 ms | 3.19 ms | 4712 kB |
| heap | rustc | 2151.64 ms | 2156.11 ms | 187.1 MB | 292384 B | 3.50 ms | 3.37 ms | 6692 kB |
| callrec | rmc | 1.30 ms | 1.19 ms | 2.8 MB | 615 B | 2.87 ms | 2.75 ms | 24 kB |
| callrec | rustc | 2125.38 ms | 2129.64 ms | 187.6 MB | 291680 B | 2.84 ms | 2.70 ms | 2108 kB |
| forloops | rmc | 1.34 ms | 1.21 ms | 2.8 MB | 726 B | 1.46 ms | 1.34 ms | 24 kB |
| forloops | rustc | 2167.46 ms | 2171.71 ms | 188.3 MB | 292480 B | 3.34 ms | 3.21 ms | 2028 kB |

Summary of the ratios (rustc ÷ rmc, so >1 means rmc is better):

| dimension         | ratio      |
| ----------------- | ---------: |
| compile wall      | ~1431–1658× |
| compile CPU       | ~1588–1868× |
| compiler peak RAM | ~67–69×    |
| artifact size     | ~314–606× vs fat-LTO stripped; ~1271–2470× vs static stripped; ~4860–9524× vs plain `-O` |
| run wall time     | 0.92–3.95× (rmc ahead on `hello`, `forloops`, `refs`, `sieve`, `mandel`, `stats`, `heap`, `fib`; at parity on `callrec` 0.99×; behind on `mix` 0.92×) |
| run peak RAM      | ~1.42× on `heap`, ~2.28× on `sieve`, ~84–105× elsewhere |

The compile-time win is the point of the project: `rmc` builds in about 1.2–1.6 ms
where rustc needs about 2.1 s, using ~2.8 MB of RAM against ~188 MB, and writing
a ~0.5–0.9 KB static binary against ~290 KB (fat-LTO stripped), ~1.2 MB (static
stripped) or ~4.5 MB (plain `-O`). The binary is small because it links only the
runtime pieces the program reaches: a program that never allocates carries no
allocator, and one that never indexes an array carries no bounds trap. The
run-time picture is mixed and remains the honest weak spot: rmc wins on `hello`
(3.29×, mostly process startup), `forloops` (2.26×), `refs` (2.19×), `sieve`
(1.61×), `mandel` (1.14×), `stats` (1.12×) and `heap` (1.06×), is at parity on
`callrec` (0.97×), and trails only on `mix` (0.92×), where the inner loop is
dominated by `x % y` in `gcd`. `fib` sits at parity too, but its ratio is the
noisiest in the suite — a run of this same binary read 0.80×, then 0.99× on a
re-run, with byte-identical code, so treat it as a coin flip rather than a loss. Both compilers inline
`gcd` into `main` (the inliner now accepts a small loop-bearing helper) and both
emit a 32-bit `div` fast path with a 64-bit `idiv` fallback, so the gap is the
code around the divide rather than the divide's checks: rmc reloads the bound and
the swapped operands from the frame each iteration where LLVM keeps them in
registers. The call-heavy cases (`fib`, `mix`,
`callrec`) used to lose; they no longer do because the recursion-to-loop pass now
folds the right spine of a shape like `1 + f(n-1) + f(n-2)`, leaving one call per
level the way rustc does. That pass is still narrow — one `i64` parameter, an
associative `+`/`*`, and a pure left operand — so a call-heavy shape it does not
match would lose again, since a stack-based generator with no register allocator
pays the argument setup and register save/restore on every call that LLVM avoids.
Run times are the noisiest column: they move ±20% between recordings on a shared
machine, and a program near parity (`fib`, `mix`, `callrec`) flips sign between
runs, so treat a single run's ratio as indicative and re-measure with many
samples before trusting a sub-5% difference. (An early `refs` run reported
0.14×; that was LLVM folding the *whole* loop into `128 * n` in the Rust twin,
not a measurement of the loop. The twin now `black_box`es a term.) The binary's
tiny footprint also shows at run time: an `rmc` program peaks at ~20–24 kB of
RAM, a rustc one at ~2 MB
— except on `heap` and `sieve`, where the arena each program allocates dominates
and the two converge. The ~2 MB floor is the dynamically linked C library
(`ld.so` + `libc`) that a rustc binary pulls in, not the generated code; rmc
links neither.

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

Two benchmarks need a word of caution when reading their ratio:

* **`heap`** builds its list with rmc's own `alloc`, which returns a payload 8
  bytes past a size header, so 16-byte nodes land 24 bytes apart. The Rust twin
  uses the same 24-byte stride (`base + 8 + i*24`) so both walks touch the same
  number of cache lines; a tighter Rust layout would shrink its working set and
  flatter the comparison. The two are still not identical — rmc's free-list
  allocator runs, glibc's does not — so the ratio is about the generated walk,
  not a claim that the allocators cost the same.
* **`refs`** needs the in-loop `black_box` to keep the Rust loop real (without
  it LLVM folds the whole thing to `128 * n`). rmc has no such barrier and
  instead hoists the loop-invariant method results with its own `licm`, so the
  two loops are not instruction-identical: both add a constant per iteration,
  but only the Rust loop pays the store/reload. Read the `refs` ratio as "two
  real loops doing the same arithmetic", not as an instruction-level tie.

## Honest limitations

rmc is a small, self-contained compiler, not a drop-in replacement for rustc.

- The language subset is small (see `docs/language.md`). Structs, arrays, `for`,
  aggregates across calls, overflow checks, integer `match`, enums,
  references, the `print!`/`println!` and `assert!` families, and `use`/`std`
  paths exist; traits, generics, borrowing, closures, floating point and the
  standard library beyond `std::process::exit` / `std::hint::black_box` do not.
  An aggregate is passed by value through a hidden
  pointer unless a reference (`&T` / `&self`) is used, which passes the address
  and copies nothing. References are not checked: nothing proves a reference
  outlives its referent, and `&mut` carries no exclusivity guarantee.
- Code generation is a stack machine for expressions. The hottest five scalar
  locals in each function — including the scalar fields of a struct that is
  never copied, returned or passed by value — are pinned to callee-saved
  registers; everything else is in memory. Temporaries are spilled to
  callee-saved registers (the first few) and then to frame slots, not the machine
  stack, so `%rsp` never moves and a call needs no alignment pad. The optimiser is an
  inlining pass (small, straight-line, scalar functions), a self-recursion-to-loop
  pass (the right-spine rewrite that turns `t(x) op f(h(x))` into a loop
  making one call per level, where `t` is any pure expression — so both
  `f(g(x)) + f(h(x))` and `1 + f(x-1) + f(x-2)` fold), a loop-invariant code
  motion pass, a dead-function pass (only what `main` reaches is emitted, which
  also drops any runtime piece only a dead function used), and a peephole
  pass (constant operands, folded comparisons, folded array addressing, `lea`
  strength reduction of `k*x+c` and of `x*k` for a small constant `k`, division
  by a constant, a 32-bit `div` fast path
  for a variable divisor, in-place accumulation of a running sum); there is no
  instruction scheduling and no live-range analysis. Inlining runs on the AST
  after semantic analysis and expands to a fixpoint, so a wrapper is inlined once
  its own callees are; the recursion pass then halves the call tree of a
  `fib`-shaped function, and loop-invariant code motion hoists the arithmetic that
  inlining exposed out of a loop body. `--no-inline`, `--no-tco` and `--no-licm`
  turn them off.
- The backend targets x86-64 Linux only, using the System V ABI and raw Linux
  syscalls. There is no Windows, macOS, ARM, or WebAssembly support.
- Array indexing is unchecked by default: an out-of-range index reads or writes
  adjacent stack memory, exactly as in C. `--bounds-checks` inserts a check
  before every index that traps with `rmc: index out of bounds` (exit 101); a
  constant index is decided at compile time.
- There is no stack probe and no stack-depth check in generated code, and rmc
  emits no stack-overflow guard. A function that recurses too deeply, or a
  program with a very large frame, runs off the end of the stack and dies with
  `SIGSEGV` rather than a clean `rmc:` message or a Rust-style "thread has
  overflowed its stack" abort. The runtime has no signal handler to turn that
  fault into a diagnostic. Untrusted input should be size-limited before it
  reaches a recursive rmc function.
- The parser bounds expression nesting at `MAX_DEPTH` (100, in `parser.rs`) and
  reports an error past that, rather than recursing until the native stack
  overflows. The limit is a fixed constant, not derived from the available
  stack, so a legitimately deep (but not pathological) expression near the limit
  is rejected.
- The heap is a simple first-fit free-list allocator; it does not return memory
  to the kernel and has no thread safety.
- Internal invariants between passes are enforced with `unreachable!()` (19 call
  sites), not by making the later pass handle every case. If a pass hands the
  next one a shape it did not expect, the compiler panics with a "this should be
  unreachable" message instead of generating wrong code. That is a deliberate
  trade - a loud compiler bug beats a silent miscompile - but it means an
  internal inconsistency aborts the compile rather than degrading.
- `rmc` is one person's project with a differential test suite and CI, not a
  funded product.

## Roadmap

What would move rmc from a serious toy toward something usable, roughly in order
of value per unit of effort:

1. **A real register allocator.** Spill temporaries now live in callee-saved
   registers (see design.md). The recursion-to-loop pass already closed the gap
   on the call-heavy benchmarks, but the calls that remain still pay argument
   setup and register save/restore that a register allocator would shrink. A full
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