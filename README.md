# rmc — an experimental Rust-subset compiler without LLVM

`rmc` compiles a statically-typed subset of Rust to x86-64 and links a static
ELF64 executable itself: no LLVM, no external crates, no libc, and by default
no assembler or linker process. It has two front ends sharing one back end:

| route | front end | use it for |
|---|---|---|
| `rmc prog.rs` | rmc's own lexer, parser and type checker | the subset in [`docs/language.md`](docs/language.md); builds in milliseconds |
| `rmc --rustc-frontend prog.rs`, or the `-Zcodegen-backend` dylib in [`backend/`](backend) | rustc (parsing, type checking, MIR optimisation) | more of the language; rmc replaces LLVM |

It is a research compiler. It is fast to build with and produces tiny
binaries; it compiles a fraction of real Rust, and on code it was not tuned for
its output runs at about two thirds of LLVM's speed, on par with Cranelift's. [`docs/AUDIT.md`](docs/AUDIT.md) has the
measured state and [`docs/ROADMAP.md`](docs/ROADMAP.md) the plan.

## Where it stands

Measured on one Intel Xeon (2.1 GHz, family 6 model 207; raw tables in
`bench/RESULTS-*.md`, method in `docs/AUDIT.md`). Every pipeline compiles the
*same* source.

**Build time** (one build, generated programs; [`RESULTS-SCALE.md`](bench/RESULTS-SCALE.md)):

| lines | rmc | rustc + rmc back end | rustc + Cranelift | rustc + LLVM -O0 | rustc + LLVM -O3 | rustc type-check only |
|---:|---:|---:|---:|---:|---:|---:|
| 973 | **13 ms** | 205 ms | 85 ms | 76 ms | 158 ms | 65 ms |
| 9 253 | **115 ms** | 1 029 ms | 406 ms | 428 ms | 1 559 ms | 249 ms |
| 46 053 | **1 476 ms** | 6 472 ms | 2 702 ms | 2 233 ms | 16 753 ms | 1 182 ms |

rmc's own front end builds a program in about the time rustc needs just to
type-check it: 11-14x faster than an LLVM release build and 1.5-6x faster than
a debug build. The rustc + rmc route is not a fast-compile route yet: it is
faster than LLVM -O3 only on large programs and about 2.5x slower than
Cranelift or LLVM -O0, because its back end works on assembly text.

**Run time** (geometric mean of LLVM time / pipeline time, so below 1 is
slower than LLVM; [`RESULTS-4WAY.md`](bench/RESULTS-4WAY.md)):

| | 12 programs rmc was tuned on | 10 held-out realistic programs |
|---|---:|---:|
| rustc + rmc back end | 1.13 | **0.69** |
| rmc front end + back end | 0.97 | 0.63 |
| rustc + Cranelift | 0.81 | 0.71 |

On the programs it was tuned for, rmc's code matches LLVM's and beats
Cranelift's (the tuned geometric mean includes `forloops`, where LLVM is 2.5x
slower; without it the MIR back end is at about 1.05). On held-out code the
MIR back end runs at 0.69x LLVM, up from 0.52x before its second round of
work, and at 0.97x Cranelift: register pressure in loops with many live values
(`bfs`, `life`, `particles` at about 0.5x) is the remaining gap
(`docs/AUDIT.md`, section 9).

**Binary size**: a few hundred bytes to a few kB, against ~17 kB for the same
`no_std` program through LLVM (the runtime keeps only the pieces a program
uses).

**Coverage**: of 117 realistic snippets rustc accepts
([`tools/mir_survey.py`](tools/mir_survey.py)), rmc's own front end compiles
66 and the rustc route 83, with no wrong output and no crash; Cranelift
compiles all 117. Neither rmc route compiles idiomatic code built on `Vec`, iterators,
closures, traits or `String` yet.

## Building

```sh
cargo build --release --manifest-path compiler/Cargo.toml
```

The compiler is `compiler/target/release/rmc`. Nothing else is needed to build
a program; `as` and `ld` are used only by `--binutils`.

## Usage

```sh
rmc input.rs -o output             # compile and link in-process (default)
rmc input.rs --emit-asm            # print the generated assembly
rmc input.rs --binutils -o out     # assemble and link with `as`/`ld` instead
rmc input.rs --overflow-checks     # trap on +,-,* overflow (exit 101)
rmc input.rs --no-bounds-checks    # drop the per-index bounds check (unsafe opt-out)
rmc input.rs --no-inline | --no-licm | --no-tco    # disable an optimisation pass
rmc --rustc-frontend input.rs -o output            # rustc's front end, rmc's back end
rmc --rustc-frontend --keep-mir input.rs -o output # also keep the MIR text
```

The output is a static executable with no dynamic dependencies. Bounds checks
are on by default, as in Rust; an out-of-range index exits with status 101.

## Using rmc as a rustc codegen backend

`backend/` builds rmc as a `rustc_private` `CodegenBackend` dylib that rustc
loads with `-Zcodegen-backend`. It links one exact nightly, pinned in
`backend/rust-toolchain.toml`:

```sh
rustup toolchain install "$(sed -n 's/^channel *= *"\(.*\)"/\1/p' backend/rust-toolchain.toml)" \
    --component rustc-dev --component llvm-tools --component rustc-codegen-cranelift
./backend/build.sh
backend/rmc-rustc prog.rs -o prog
```

It targets x86-64 Linux executables only and refuses anything else. It
currently renders MIR to text and re-parses it, and sees only the local crate's
MIR, which is why it cannot yet compile code that relies on generic `std`
functions (`Vec`, iterator adapters, `env::args`); see the roadmap. A program
it cannot compile is refused with a message naming the construct.

## Using rmc as a library

```rust
let asm = rmc::compile_to_asm(
    "fn main() -> i64 { let mut x: i64 = 0; for i in 0..10 { x += i; } return x; }",
    &rmc::CompileOptions::default(),
)?;
let elf = rmc::elf::assemble(&asm)?;
```

`rmc::mir_text_to_asm` does the same for `rustc --emit=mir` text, running the
same pass pipeline as the CLI and the dylib.

## Layout

```
compiler/src/front/   rmc's front end: lexer, parser (+ desugaring), ast, sema, consts
compiler/src/opt/     AST passes: inline, tco, licm, dce
compiler/src/mir/     the rustc route: MIR parser and passes, MIR -> x86-64
compiler/src/x86/     shared back end: AST -> x86-64, runtime, ELF assembler/linker
backend/              the -Zcodegen-backend dylib
tests/                differential suites against rustc (cases/, mir/, std/, cb/)
tools/                fuzzers and the coverage survey
bench/                benchmark harnesses and recorded results
docs/                 language subset, design, audit, roadmap, design notes
```

## Testing

```sh
cargo test --release --manifest-path compiler/Cargo.toml   # unit tests, no as/ld needed
python3 tests/run.py          # rmc's front end vs rustc, 5 optimiser configs, both link paths
python3 tests/mir/run.py      # rustc route, no_std, vs rustc+LLVM, in-process and --binutils
python3 tests/std/run.py      # rustc route with std (formatting, black_box)
python3 tests/cb/run.py       # the dylib
python3 tools/mir_survey.py   # 121 realistic snippets: compile or refuse cleanly, never crash
python3 tools/fuzz.py --mode mir --n 300 --seed 1    # random programs through the rustc route
python3 tools/fuzz.py --mode stmt --n 500 --seed 1   # ... through rmc's front end (many modes)
python3 tools/fuzz.py --mode reject                  # programs rustc rejects must be rejected
```

Every suite compares stdout and exit status with rustc. CI runs all of them,
plus bounded fuzzing of both front ends and both MIR pipelines. `AGENTS.md`
lists the rules a change must follow.

## Benchmarks

```sh
python3 bench/run_four.py --set all     # rmc, rustc+rmc, Cranelift, LLVM on one source
python3 bench/scale.py                  # build time at ~1k/10k/50k lines
python3 bench/run.py                    # rmc vs rustc on the std sources
```

`run_four.py` has two sets: `core` (`bench/uni`, the programs the optimisers
were tuned on) and `real` (`bench/real`, held out). Quote both; a result that
holds on one only is not a result. Run time near parity varies by several
percent between runs and hosts.

## Limitations

* Language: the subset in `docs/language.md` for rmc's front end; for the rustc
  route, what the local crate's MIR can express without generic `std` code (no
  `Vec`, iterator adapters, closures as values, `dyn Trait`, `env::args`).
* Code quality: the register allocator shares registers by interference but
  does not split live ranges, and only nine registers are allocatable; no
  `cmov`, no induction-variable strength reduction. Held-out code runs at
  about two thirds of LLVM's speed.
* rmc's own front end treats `u64`/`usize` as `i64` (`docs/language.md`), so a
  comparison or division of values at or above 2^63 differs from rustc.
* rmc's front end stores each array element in an 8-byte slot, so a large local
  byte array can exceed the stack; it is rejected with a message.
* x86-64 Linux only.

## License

See [`LICENSE`](LICENSE).
