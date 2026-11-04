# rmc roadmap: from serious toy to a compiler that can compete

This document is a critical audit of `rmc` on the `feature/rmc-compiler`
branch, and a concrete plan to move it forward. It was written after building
the compiler, running the full test suite, the differential fuzzer and the
benchmark harness, and reproducing every finding below by hand.

Everything here is grounded in a measurement or a reproduction. Where a claim
is not yet verified it is marked **unverified**.

## Verdict

`rmc` is not a toy. It is a working compiler: 13,873 lines, no dependencies, a
complete pipeline (`lex -> parse -> sema -> inline -> tco -> licm -> codegen
-> as -> ld`), a differential suite against `rustc`, and a seven-mode fuzzer.
That is a real achievement for one person.

The honest answer to "can it compete with `rustc` and Cranelift?" is: **not on
their axis, and it never will be on their axis.** This is a 14k-line solo
project; Cranelift is about a decade of work by a funded team, and `rustc`
carries LLVM. The project has to win on a *different* axis - compile latency,
static binary size, zero dependencies - not on general codegen quality.

What was verified during the audit:

| check | result |
| --- | --- |
| `cargo build --release` | 7.3 s |
| unit tests | 219 pass |
| differential suite (`tests/run.py`) | 55 cases x 5 optimiser configs pass |
| `tools/fuzz.py --mode robust` | pass |
| `tools/fuzz.py --mode reject` | pass |
| large fuzz run (400 x 7 modes x 3 seeds) | all pass, **but the generator cannot reach the bugs in section 1** |
| benchmarks | compile-time win confirmed |

## What is genuinely strong

1. **Zero dependencies, no libc, raw syscalls.** A 6.6 KB static ELF. Real.
2. **Compile latency.** Measured here: `rmc` ~8 ms, `rustc -O` default
   ~118 ms (14x), `rustc -O -C lto=fat -C codegen-units=1` ~2255 ms (280x).
   The core claim holds.
3. **Test discipline.** Differential suite against `rustc`, a seven-mode
   fuzzer, a `robust` mode for adversarial input, a `reject` mode for parity
   with `rustc` rejections, and every case recompiled with each optimiser pass
   off in turn. Better than many funded projects.
4. **Soundness checks that mirror `rustc`.** `E0384` (mut), `E0381` (definite
   init), `E0594`/`E0596`, constant-folded index and out-of-range constants.
5. **Real optimisations.** Self-recursion-to-loop (makes `fib` match LLVM),
   AST inlining, LICM, magic-number division, in-place accumulation.
6. **Documentation.** README, `docs/design.md`, `docs/language.md`, and a
   63 KB `AGENTS.md` of pitfalls. Rare.
7. **Decent diagnostics** for the type layer.

## Section 1: correctness bugs

### 1.1 `--overflow-checks` does not trap on shift overflow

Reproduction:

```rust
// run with n = 81
fn main() -> i64 {
    let n: i64 = arg_i64(1);
    let y: i64 = 1 << n;
    print_i64(y);
    return 0;
}
```

```
rmc --overflow-checks          -> 131072  exit=0
rustc -C overflow-checks=on    -> panic "attempt to shift left with overflow"  exit=101
```

The same holds for `x <<= n` and for `a[i] <<= 81` inside a loop. The overflow
pass only wraps `+`, `-` and `*` (see `check_overflow` in
`compiler/src/codegen.rs`), whereas `rustc` with `overflow-checks=on` also
checks shift counts. `docs/language.md` only promises `+,-,*`, but the
project's pitch is parity with Rust, and the fuzzer's `--overflow` mode
compares against `-C overflow-checks=on`.

**Worse: the fuzzer hides the entire class.** In `tools/fuzz.py` the generator
forces every shift count through `& 63`, so an overflowing count is never
produced. This is a deliberate coverage hole, not an accident.

### 1.2 `i64::MIN / -1` returns a wrong value instead of trapping

```
rmc                            -> -9223372036854775808  exit=0
rustc                          -> panic "attempt to divide with overflow"  exit=101
```

The generated code for `x / -1` is a bare `neg %rax` (the magic-division path
for divisor `-1`). The constant path is rejected at compile time, but the
runtime path is not. This is a **silent wrong answer**, the most dangerous
class of bug.

### 1.3 The `LOAD` segment is RWX (a W^X violation)

```
LOAD ... RWE 0x8
ld: warning: ... has a LOAD segment with RWX permissions
```

The `ld ... -n` flag in `link()` (`compiler/src/main.rs`) merges `.text` and
`.data` into one read-write-execute segment. This violates W^X and is rejected
on systems that enforce it (SELinux, some kernel hardening). Dropping `-n`, or
adding `-z separate-code`, yields correct `R E` and `RW` segments at a cost of
about 3 KB on a 6.5 KB binary. The "6.6 KB artifact" is currently bought with
hardening.

### 1.4 No borrow checking

Three programs that `rustc` rejects outright are accepted and run by `rmc`:

```rust
fn dangle() -> &i64 { let x: i64 = 7; return &x; }      // rmc prints 7
let a: &i64 = &x; let b: &mut i64 = &mut x; *b = 2;     // rmc allows the aliasing
let r: &i64 = { let x = 5; &x };                        // rmc allows the dangle
```

This is documented as a limitation, but the consequence should be stated
plainly: **without a borrow checker, `rmc` is not a "Rust subset", it is C
with Rust syntax.** Rust without ownership guarantees loses its whole value
proposition. This is both the biggest gap versus `rustc` and the moat that
makes the project worth building.

### 1.5 Large frames have no stack probe; recursion has no diagnostic

- A 200k-element array emits `sub $1600000, %rsp` with no guard-page probe.
  `rustc` always inserts a stack probe. On Linux with a small stack this can
  jump the guard page and produce a confusing segfault.
- Unbounded recursion segfaults silently. `rustc` prints `"thread 'main' has
  overflowed its stack"`.

### 1.6 `MAX_DEPTH = 100` is low and tuned to a 2 MiB stack

`compiler/src/parser.rs` bounds nesting to 100 levels, hardcoded to the 2 MiB
thread stack `cargo test` runs on. Machine-generated nested expressions fail
to compile. The fix is not a bigger number but converting the recursive
descent to an explicit stack, or making the limit a runtime function of the
actual stack size.

### 1.7 `unreachable!()` in codegen turns internal gaps into panics

There are more than ten `unreachable!()` sites in `compiler/src/codegen.rs`
(748, 749, 1045, 1111, 1513, 1519, 1520, 2052, 2148, 2181, ...). If sema
misses one new expression form, the compiler panics instead of emitting an
error. `fuzz.py --mode robust` only feeds parser-level garbage, never a
program that passes sema and then hits an `unreachable!`. This is
architectural debt that will bite on every language extension.

### 1.8 Stale module doc comment

`compiler/src/codegen.rs` lines 1-8 still say temporaries are "pushed on the
machine stack", while `docs/design.md`, `AGENTS.md` and the `SPILL_REGS`
constant all say spills go to frame slots precisely so `%rsp` never moves.
The top-of-file comment misleads a new reader.

## Section 2: benchmark methodology

### 2.1 The headline 231-311x is achieved with the worst `rustc` configuration

The harness uses `-C lto=fat -C codegen-units=1` (`bench/run.py`). Measured
here on the same machine:

| `rustc` configuration | compile time | ratio vs `rmc` (~8 ms) |
| --- | --- | --- |
| `rustc -O` (the default people use) | 118 ms | 14x |
| `rustc -O -C lto=fat -C codegen-units=1` (used by the harness) | 2255 ms | 280x |

Fat LTO plus one codegen unit is 19x slower than the default. The README does
name the flags, but the number quoted in the headline (and by anyone who
repeats it) is 231-311x. That is misleading. Honest reporting needs two
columns: default `rustc -O` and fat-LTO.

### 2.2 Eight toy programs, all inside the `rmc` subset

Nothing stresses the stack machine: deep expressions, many live temporaries
across calls, large functions. A call-heavy program written for this audit:

```
rmc  : 75 ms
rustc: 64 ms   -> rmc is 17% slower
```

The README claims code is "within a few percent on the call- and pointer-heavy
cases". That does not survive a program outside the tiny-loop shape.

### 2.3 Runtime ratios are not reproducible, yet are quoted as direction

Running the same benchmark here gave `mandel` at **rmc 0.94x (losing)**, where
`bench/RESULTS.md` records **rmc 1.10x (winning)** - a 17% swing. The README
admits +/-20% noise, yet still writes "rmc wins on mandel (1.10x)". The suite
reports a single median with no confidence interval and no significance test.
Sub-5% claims are not trustworthy from this harness.

### 2.4 Artifact-size comparison is apples to oranges

| | rmc | rustc (dynamic) | rustc (static, `+crt-static`) |
| --- | --- | --- | --- |
| `hello` | 6.6 KB | 289 KB | 1.12 MB |

The 6.6 KB vs 290 KB comparison mostly measures "did you link libc", not code
quality. The honest figure is versus static `rustc`: about 170x, not 43x.
Still a large win, but the "43x" framing overstates it.

### 2.5 The runtime-RAM claim is also a libc artefact

`rustc`'s ~2 MB is `libc` and `ld.so` mapped in. Once a program actually
computes (a large arena), the gap collapses: `sieve` 2.3x, `heap` 1.4x. The
README notes this, but the summary table still highlights 72-86x.

## Section 3: architecture and competitiveness

| # | issue | impact |
| --- | --- | --- |
| 3.1 | Stack-machine codegen; every expression through `%rax`; only 5 locals + 3 spill registers; no live-range analysis, no instruction scheduling | the ceiling on code quality; the root of every runtime loss |
| 3.2 | No IR/SSA. Passes (inline/tco/licm) work on the AST | optimisations are fragile and order-sensitive; cannot grow into serious ones |
| 3.3 | No borrow checker (see 1.4) | not Rust; not safe |
| 3.4 | Minimal type system: `i64`/`bool`/unit-enum only; no `u*`/`i*`, no float, no generics, no traits, no tuples, no slices | cannot compile any real Rust program |
| 3.5 | x86-64 Linux only, raw syscalls, no Windows/macOS/ARM/WASM | not portable |
| 3.6 | No debug info (no DWARF line tables) | cannot be debugged; no `gdb` |
| 3.7 | Errors use byte offsets, not `line:col` | not user friendly |
| 3.8 | No incremental, parallel or cached compilation | 200k statements -> 490 ms (good, but linear) |
| 3.9 | Bus factor of one; the repo is two days old, the default branch holds only `LICENSE`, PR #1 is open | calibrate expectations: a solo experiment, not a product |

## Section 4: the plan

The strategic premise first, because it decides everything else: do not fight
`rustc` and Cranelift on their axis (general optimised codegen). That race is
lost by a decade. Win on the other axis:

> compile in microseconds, a static binary under 10 KB, zero dependencies.

That axis is real for embedded and kernel modules, JIT and build scripts, hot
reload, WASM, and for **teaching compilers**. There, `rustc`'s two seconds and
300 KB genuinely do not fit.

### Phase 0: correctness and honesty

Cheap, high impact, finishable in under a week.

1. Close the shift-overflow gap. Emit a trap for `<<`/`>>` under
   `--overflow-checks`, or reject a runtime count `>= 64`. Match
   `-C overflow-checks=on`.
2. Fix `i64::MIN / -1`. Detect the `idiv` overflow and trap with exit 101.
3. Fix the RWX segment. Drop `-n` or use `-z separate-code`; accept about
   +3 KB for W^X.
4. Widen the fuzzer. Stop forcing `& 63`; generate full counts (`0..=127`) and
   compare against `-C overflow-checks=on`. This is what catches 1.1 in CI.
5. Replace the `unreachable!()` sites with proper errors.
6. Emit `line:col` in diagnostics instead of byte offsets.
7. Add a stack probe and a stack-overflow diagnostic instead of a segfault.
8. Fix the stale `codegen.rs` header comment.
9. Fix the benchmark: add a default `rustc -O` column, run N samples with a
   confidence interval, compare artifact size against **static** `rustc`, and
   add programs that stress the stack machine (call-heavy, more than six
   locals, deep expressions).

### Phase 1: codegen quality

The largest single lever.

10. Introduce an IR between sema and codegen (a simple SSA or linear IR).
    Everything else depends on this.
11. A linear-scan register allocator over live ranges. Replace "five hottest
    locals plus three spill registers".
12. Instruction selection and peephole at the IR level, not on assembly text.
13. Basic instruction scheduling.

Target: close the call-heavy gap (2.2) from -17% to within +/-2%.

### Phase 2: language

So that "Rust subset" is true.

14. A borrow checker (NLL-lite). This is the moat. Without it the project
    should not call itself a Rust subset. Minimum: reference lifetime checks
    and `&mut` exclusivity.
15. Wide integer types (`u8..u64`, `i8..i32`) and `f64`.
16. Tuples, slices, arrays of aggregates.
17. Generics by monomorphisation (a prerequisite for real code).

### Phase 3: infrastructure

18. A target-agnostic IR with a second backend (ARM64 or a simple VM). This
    forces the code generator's assumptions into the open.
19. DWARF line tables.
20. Incremental, parallel and cached compilation.

### A strategic note

If the real goal is "emit code as fast as `rustc`/Cranelift with a fast
compiler", there is an honest shortcut: use Cranelift as the backend and
compete on the front end and the orchestration. That discards the "no LLVM,
no deps" thesis. So the real choice is:

- **Stay purist** (write the backend) - accept the code-quality ceiling, win
  on compile speed, size and teaching.
- **Go pragmatic** (use Cranelift) - beat `rustc` on compile time, but it is
  no longer a from-scratch project.

There is no third path where a 14k-line project beats `rustc` + Cranelift on
every axis.

## How to reproduce the audit

```sh
cargo build --release --manifest-path compiler/Cargo.toml
cargo test --manifest-path compiler/Cargo.toml
python3 tests/run.py
python3 tools/fuzz.py --mode robust
python3 tools/fuzz.py --mode reject
for m in expr stmt opt arith nest pat order; do
  python3 tools/fuzz.py --mode $m --n 400 --seed 7
done
python3 bench/run.py
```

The three correctness bugs in section 1 are reproduced by the small programs
shown next to them; each was run against both `rmc` and a `rustc` built with
the matching flags.
