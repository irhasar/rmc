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
| compiler peak RSS                | ~3.1 MB                               | ~188 MB                          |
| compile time (small programs)    | ~1.5 ms                               | ~2.1 s                           |
| artifact size                    | ~0.5–1.2 KB (static, no libc)         | ~290 KB (fat-LTO stripped); ~4.5 MB (plain `-O`); ~1.2 MB (static stripped) |
| generated-code quality           | peephole + top-five local promotion   | highly optimised                 |

`rmc` wins big on compile time, compiler memory, and artifact size, and those
three are the point of the experiment. Because it links in-process rather than
forking `as` and `ld`, a small program compiles in about 1.8 ms against
rustc's ~2.1 s. On **runtime** the result is program-dependent but, in the
current recording, in rmc's favour on all twelve programs: from `hello` 3.7×
(mostly process startup) and `forloops` 2.3×, through `sieve` 1.5×, the new
`heavy` (a 1e9-iteration steady-state loop) at 1.35×, `light` (a cache-resident
scan) at 1.26× and `refs` 1.17×, down to `mandel` 1.10×, `stats` 1.09×, `fib`
1.06×, `mix` 1.05×, `heap` 1.04× and `callrec` 1.04×. Several of those margins
— `fib`, `mix`, `heap`, `callrec` — sit inside run-to-run noise and flip sign
between recordings, so a single ratio is indicative, not a stable property.
`refs` used to be the one structural loss — LICM would not hoist a read through a
reference, because rmc's `&T` carries no guarantee that it is dereferenceable and
speculating the load out of a loop that might never run could fault. A provenance
pre-pass now marks a reference that is bound directly to a live local (`&x`,
`&x.field`) as provably frame-pointing, so a read through it *can* be hoisted;
that took `refs` from a structural loss to a 1.2× win. A call-heavy shape the
recursion-to-loop pass does not match would lose. It is not uniformly faster and
there is no single speedup number; `bench/RESULTS.md` reports the per-program
ratio and the harness's fairness rules.

The artifact-size comparison depends entirely on which rustc build is the
reference. Against a plain `rustc -O` (which statically links libstd, the panic
runtime and the allocator) rmc is ~3899–9166× smaller; against a stripped static
build, ~1016–2385×; against an aggressive fat-LTO stripped build, ~251–585×. The
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
  mir/*.rs           freestanding twins of the same programs (for run_mir.py)
  run.py             benchmark harness (compile time, CPU, RAM, size, runtime)
  run_fair.py        same-source, two-back-end comparison (rmc vs LLVM)
  run_mir.py         three-way comparison: rustc-MIR+rmc, rmc-AST, rustc-LLVM
tools/
  fuzz.py            differential fuzzer and robustness checker
tests/
  run.py             differential test suite: rmc vs rustc on twin programs
  cases/*.rs         programs compiled by both rmc and rustc
  args.json          per-case argv vectors (for runtime inputs)
docs/
  language.md        the supported language subset
  design.md          compiler architecture and code generation notes
  mir-roadmap.md     rustc-front-end/rmc-back-end: status, limits, roadmap
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
(plain, escaped, `\xNN`, `\u{..}`) and byte literals (`b'A'`, `b'\xFF'`),
`as i64`, `char` ordering and equality, a
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
python3 tools/fuzz.py --mode localconst --n 500 --seed 1
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
including array lengths and `match` patterns; also inside a function body, where
it is in scope for the whole block and may be used before its declaration) and an
immutable `static` (lowered to a `const`; `static mut` is rejected); `type`
 aliases (a transparent
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
statements (parsed and discarded, since every name is resolved globally); `pub`
visibility on an item, a struct field, an `impl` method or a nested `fn` (with
any `(crate)`/`(super)`/`(in ..)` restriction — dropped, since visibility has no
meaning in a single flat namespace); an inline `mod NAME { .. }` (skipped, since
rmc has no module system; a `mod NAME;` that loads another file is an error);
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
-C panic=abort -C strip=symbols` on twelve programs, with matching safety
semantics: rmc is invoked with `--bounds-checks` and rustc with
`-C overflow-checks=off`, so both carry bounds checks and neither carries
overflow checks (see below). The last two bracket the load spectrum: `light`
runs a short, cache-resident loop (a few ms), `heavy` a ~1e9-iteration
steady-state loop (hundreds of ms), so neither a startup-dominated nor a
throughput-dominated shape can hide a regression. It measures both halves of
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
| hello | rmc | 1.62 ms | 1.49 ms | 3.6 MB | 503 B | 0.21 ms | 0.12 ms | 24 kB |
| hello | rustc | 2110.17 ms | 2112.41 ms | 188.2 MB | 289416 B | 0.80 ms | 0.70 ms | 2104 kB |
| fib | rmc | 1.72 ms | 1.58 ms | 3.6 MB | 870 B | 6.72 ms | 6.61 ms | 24 kB |
| fib | rustc | 2146.25 ms | 2147.34 ms | 188.5 MB | 291696 B | 7.12 ms | 6.96 ms | 2040 kB |
| mandel | rmc | 1.89 ms | 1.77 ms | 3.7 MB | 1209 B | 95.75 ms | 95.50 ms | 24 kB |
| mandel | rustc | 2131.60 ms | 2135.78 ms | 186.9 MB | 291776 B | 105.17 ms | 104.95 ms | 2096 kB |
| sieve | rmc | 1.86 ms | 1.73 ms | 3.6 MB | 1137 B | 1.64 ms | 1.52 ms | 1584 kB |
| sieve | rustc | 2167.75 ms | 2168.35 ms | 186.7 MB | 292032 B | 2.39 ms | 2.26 ms | 3660 kB |
| mix | rmc | 1.91 ms | 1.77 ms | 3.6 MB | 1117 B | 14.71 ms | 14.56 ms | 24 kB |
| mix | rustc | 2174.82 ms | 2178.58 ms | 187.4 MB | 291880 B | 15.50 ms | 15.36 ms | 2076 kB |
| stats | rmc | 1.86 ms | 1.74 ms | 3.7 MB | 1104 B | 3.42 ms | 3.30 ms | 24 kB |
| stats | rustc | 2177.36 ms | 2180.97 ms | 187.9 MB | 292048 B | 3.72 ms | 3.59 ms | 2092 kB |
| refs | rmc | 2.04 ms | 1.92 ms | 3.8 MB | 1279 B | 4.68 ms | 4.56 ms | 24 kB |
| refs | rustc | 2149.20 ms | 2152.70 ms | 189.0 MB | 292512 B | 5.48 ms | 5.35 ms | 2104 kB |
| heap | rmc | 1.91 ms | 1.78 ms | 3.6 MB | 1335 B | 3.19 ms | 3.06 ms | 4712 kB |
| heap | rustc | 2183.21 ms | 2187.70 ms | 188.4 MB | 292384 B | 3.31 ms | 3.19 ms | 6716 kB |
| callrec | rmc | 1.75 ms | 1.63 ms | 3.7 MB | 873 B | 2.74 ms | 2.62 ms | 24 kB |
| callrec | rustc | 2150.84 ms | 2155.09 ms | 186.8 MB | 291680 B | 2.83 ms | 2.71 ms | 2088 kB |
| forloops | rmc | 1.81 ms | 1.69 ms | 3.7 MB | 1011 B | 1.46 ms | 1.35 ms | 24 kB |
| forloops | rustc | 2201.07 ms | 2205.34 ms | 187.9 MB | 292480 B | 3.31 ms | 3.18 ms | 2084 kB |
| light | rmc | 1.83 ms | 1.72 ms | 3.7 MB | 1209 B | 2.09 ms | 1.98 ms | 24 kB |
| light | rustc | 2154.45 ms | 2158.88 ms | 189.9 MB | 291904 B | 2.63 ms | 2.50 ms | 2092 kB |
| heavy | rmc | 1.76 ms | 1.65 ms | 3.6 MB | 969 B | 766.19 ms | 765.93 ms | 24 kB |
| heavy | rustc | 2207.43 ms | 2211.42 ms | 187.2 MB | 291792 B | 1030.97 ms | 1030.60 ms | 2060 kB |

Summary of the ratios (rustc ÷ rmc, so >1 means rmc is better):

| dimension         | ratio      |
| ----------------- | ---------: |
| compile wall      | ~1054–1303× |
| compile CPU       | ~1121–1418× |
| compiler peak RAM | ~50–53×    |
| artifact size     | ~219–575× vs fat-LTO stripped; ~887–2347× vs static stripped; ~3402–9020× vs plain `-O` |
| run wall time     | 1.04–3.73× (rmc ahead on all twelve in this recording; the four sub-1.10× margins are inside run-to-run noise) |
| run peak RAM      | ~1.4× on `heap`, ~2.3× on `sieve`, ~86–88× elsewhere |

The compile-time win is the point of the project: `rmc` builds in about 1.6–2.0 ms
where rustc needs about 2.1 s, using ~3.6 MB of RAM against ~188 MB, and writing
a ~0.5–1.3 KB static binary against ~290 KB (fat-LTO stripped), ~1.2 MB (static
stripped) or ~4.5 MB (plain `-O`). The binary is small because it links only the
runtime pieces the program reaches: a program that never allocates carries no
allocator, and one that never indexes an array carries no bounds trap. The
run-time picture is program-dependent but, in this recording, rmc's favour on
every program: `hello` (3.73×, mostly process startup), `forloops` (2.26×),
`sieve` (1.46×), `heavy` (1.35×), `light` (1.26×), `refs` (1.17×), `mandel`
(1.10×), `stats` (1.09×), `fib` (1.06×), `mix` (1.05×), `heap` (1.04×) and
`callrec` (1.04×). The last four sit within run-to-run noise, so treat them as
parity rather than a stable win. `refs` is no longer the structural loss it was —
see below. Both compilers inline `gcd` into `main` (the inliner now accepts a
small loop-bearing helper) and both emit a 32-bit `div` fast path with a 64-bit
`idiv` fallback, so the `mix` gap is the code around the divide rather than the
divide's checks: rmc reloads the bound and the swapped operands from the frame
each iteration where LLVM keeps them in registers. The call-heavy cases (`fib`,
`mix`, `callrec`) used to lose; they no longer do because the recursion-to-loop
pass now folds the right spine of a shape like `1 + f(n-1) + f(n-2)`, leaving one
call per level the way rustc does. That pass is still narrow — one `i64`
parameter, an associative `+`/`*`, and a pure left operand — so a call-heavy
shape it does not match would lose again, since a stack-based generator with no
register allocator pays the argument setup and register save/restore on every
call that LLVM avoids. `heavy` is the shape rmc's codegen handles best: a long
scalar loop the vectoriser cannot rewrite, where the loop the compiler emits is
what runs for hundreds of milliseconds. Run times are the noisiest column: they
move ±20% between recordings on a shared machine, and a program near parity
(`fib`, `mix`, `heap`, `callrec`) flips sign between runs, so treat a single
run's ratio as indicative and re-measure with many samples before trusting a
sub-5% difference. (An early `refs` run reported 0.14×; that was LLVM folding the
*whole* loop into `128 * n` in the Rust twin, not a measurement of the loop — the
fix was to keep the loop real without a barrier only one side pays for; see
"Reproducing the comparison honestly".)
The binary's
tiny footprint also shows at run time: an `rmc` program peaks at ~24 kB of
RAM, a rustc one at ~2 MB
— except on `heap` and `sieve`, where the arena each program allocates dominates
and the gap narrows to ~1.4× and ~2.3×. The ~2 MB floor is the dynamically
linked C library (`ld.so` + `libc`) that a rustc binary pulls in, not the
generated code; rmc links neither.

### Reproducing the comparison honestly

The `rustc` invocation enables fat LTO and a single codegen unit, which is the
closest equivalent to "compile everything, then link". The `rmc` time includes
spawning `as` and `ld`; the `rustc` time includes LLVM and the system linker.
Both compilers and both binaries are measured the same way, peak RAM comes from
each process's own `VmHWM`, and both programs read their workload from `argv` so
neither can fold the answer at compile time.

Two deliberate choices keep the comparison from flattering either side.

* **Matching safety semantics.** rmc defaults bounds checks *off* and overflow
  checks *off*; `rustc -O` keeps bounds checks *on* and overflow checks *off*.
  A run with rmc's defaults would have rmc skip a per-index check rustc emits.
  The harness closes the gap from the rmc side: `rmc --bounds-checks` and
  `rustc -C overflow-checks=off`, so both builds carry bounds checks and
  neither carries overflow checks. The check costs about 1% on the array-heavy
  programs (`sieve` is the largest, and its run ratio barely moves), so the
  choice does not manufacture any win; it removes a check rmc would otherwise
  not pay for.

* **No `black_box` inside a hot loop.** rmc lowers `std::hint::black_box` to the
  identity (it has no optimiser that could elide the value anyway); rustc makes
  it opaque, which forces a store/reload the optimiser cannot see through. An
  in-loop `black_box` therefore charges rustc work rmc never emits, and a
  benchmark whose ratio turns on it is measuring the barrier, not the code.
  A benchmark still has to stop LLVM from folding the loop away, so the
  workload is kept real in a way both compilers see the same: trip counts are
  read from `argv`, and where a loop body is otherwise loop-invariant (`refs`,
  `heap`) it is kept non-invariant by a data-dependent branch (`refs`) or by a
  pointer chase whose next address is the value just loaded (`heap`) rather than
  by a barrier.

The Rust twins do `black_box` their *inputs* — the trip count, the opaque seed
the struct is built from — because a workload the optimiser can see through is
not a workload. What they do not do is put a barrier *inside* the measured loop.

Three benchmarks need a word of caution when reading their ratio:

* **`light`** and **`heavy`** bracket the load spectrum. `light` is the short,
  cache-resident case: a 2 KiB table (so it stays in L1) scanned with a
  data-dependent branch, run for a few ms, where process startup and
  per-iteration codegen are a real share of the cost. `heavy` is the long,
  throughput-bound case: a ~1e9-iteration loop over two accumulators, run for
  hundreds of ms, where the measured time is the generated loop rather than a
  closed form (the data-dependent branch keeps it scalar and the second
  accumulator stops it collapsing to a running sum). This is the regime where
  rmc's codegen is strongest, and a startup-only win cannot flatter it.
* **`heap`** builds its list with rmc's own `alloc`, which returns a payload 8
  bytes past a size header, so 16-byte nodes land 24 bytes apart. The Rust twin
  uses the same 24-byte stride (`base + 8 + i*24`) so both walks touch the same
  number of cache lines; a tighter Rust layout would shrink its working set and
  flatter the comparison. The two are still not identical — rmc's free-list
  allocator runs, glibc's does not — so the ratio is about the generated walk,
  not a claim that the allocators cost the same.
* **`refs`** reads a fixed struct through `&self` accessors. The struct is built
  from an opaque seed and the accumulator is reduced by a data-dependent branch,
  so the loop is real for both compilers without an in-loop `black_box`. rmc's
  `licm` hoists a read through a reference only when a provenance pre-pass
  proves the reference points at a live local (`&x`, `&x.field`): rmc's `&T`
  carries no guarantee that it is dereferenceable, so a reference of unknown
  origin — a parameter, a cast, a mutable ref — must not have its load
  speculated out of a loop that might never run. `refs` binds its reference to a
  local, so the read is hoisted and rmc wins (1.17×); a loop that reads through a
  reference of unproven origin would still reload every iteration and lose, and
  that is the honest limit. An earlier `refs` design put `black_box` inside the
  loop and reported 2.15×; that number was withdrawn once the barrier asymmetry
  was found, because without the barrier rustc folded the loop to a constant and
  with it rustc paid a store/reload rmc did not.

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
   small programs actually need. (`match` on integers is done.) **Slices**
   (`&[T]`, and the slice literals and sub-slicing `&a[lo..hi]` that build them)
   are the other large gap: a slice is a *fat* pointer (a data pointer plus a
   length), so it needs a two-word value kind threaded through parameters,
   returns and indexing, unlike the one-word `&T` and `str` rmc has today.
4. **More backends.** A second target (ARM64 or a simple VM) would force the
   code generator's assumptions out into the open.
5. **Debug info.** Line tables so a debugger can point at source.

Anything involving borrowing, generics or the trait system is out of scope: it
would make rmc a different project rather than a better one.

## License

Provided as-is for experimentation.