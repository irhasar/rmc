# AGENTS.md

Rules for working on this repository. Keep it short and current: a rule goes
here, the story behind it goes in [`docs/design-notes.md`](docs/design-notes.md)
(the long form, ~3 300 lines, searchable by keyword).

## What this is

`rmc` is an experimental ahead-of-time compiler for a Rust subset, with two
routes to the same x86-64 back end and no LLVM:

* **rmc's own front end** (`rmc prog.rs`): lexer, parser, sema and AST passes,
  then the x86 code generator. Compiles in milliseconds; accepts only the subset
  in [`docs/language.md`](docs/language.md).
* **rustc's front end** (`rmc --rustc-frontend prog.rs`, a `.mir` file, or the
  `-Zcodegen-backend` dylib in `backend/`): rustc parses, type-checks and
  optimises MIR; rmc lowers MIR to x86-64.

Both routes assemble and link in-process (`x86/elf.rs`) against a freestanding
runtime (`x86/runtime.rs`: raw syscalls, no libc). `--binutils` uses `as`+`ld`
instead, and the two must always agree. See [`docs/AUDIT.md`](docs/AUDIT.md)
for the current state and [`docs/ROADMAP.md`](docs/ROADMAP.md) for what is next.

## Layout

```
compiler/src/
  lib.rs            public API: compile_to_asm, MirOptions, optimize_mir, mir_text_to_asm
  main.rs           the `rmc` CLI
  front/            rmc's front end: lexer, parser (+ desugaring), ast, sema, consts
  opt/              AST passes: inline, tco, licm, dce
  mir/mod.rs        MIR text parser, MIR model and MIR passes
  mir/codegen.rs    MIR -> x86-64 lowering and the assembly peepholes
  x86/codegen.rs    AST -> x86-64 lowering
  x86/elf.rs        in-process assembler and static ELF64 linker
  x86/runtime.rs    freestanding runtime, linked piece by piece
compiler/examples/  rmcmir (MIR text -> ELF), mirdump
compiler/tests/     unit tests that need no `as`/`ld`
backend/            the -Zcodegen-backend dylib + rmc-rustc wrapper (pinned nightly)
tests/cases/        AST route: differential cases vs rustc (tests/run.py)
tests/mir/          MIR route, no_std: differential vs rustc+LLVM, + real_* workloads
tests/std/          MIR route with std (println!, black_box)
tests/cb/           the dylib route
tools/fuzz.py       differential + robustness fuzzers (incl. --mode mir)
tools/mir_survey.py 121 realistic snippets: coverage and crash survey
bench/              run.py (vs rustc), run_four.py (4 back ends, core + real sets),
                    scale.py (compile time vs size), pipelines.py (shared helpers)
```

The crate root re-exports the old flat module names (`crate::ast`,
`rmc::mir_codegen`, `rmc::elf`, ...), so older paths in notes still resolve.

## Build, test, measure

```sh
cargo build --release --manifest-path compiler/Cargo.toml --bins --examples
./backend/build.sh                        # needs the nightly in backend/rust-toolchain.toml

cargo test --release --manifest-path compiler/Cargo.toml
python3 tests/run.py                      # AST route, 5 optimiser configs, both link paths
python3 tests/mir/run.py                  # MIR route, no_std, in-process and --binutils
python3 tests/std/run.py                  # MIR route with std
python3 tests/cb/run.py                   # the dylib
python3 tools/mir_survey.py               # must report no crash

python3 tools/fuzz.py --mode mir --n 300 --seed 1             # MIR route
python3 tools/fuzz.py --mode mir --pipeline cb --n 200 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1            # AST route (see the
python3 tools/fuzz.py --mode robust; --mode reject; --mode mutate  # mode list in fuzz.py)

python3 bench/run_four.py --set all --markdown bench/RESULTS-4WAY.md
python3 bench/scale.py --markdown bench/RESULTS-SCALE.md
python3 bench/run.py --markdown bench/RESULTS.md
```

CI (`.github/workflows/ci.yml`) runs all of the above at bounded sizes. A
change is done when every suite passes on the pinned nightly *and* on stable
(the `test` job uses stable; MIR shapes differ between toolchains).

## Debugging switches

| switch | effect |
|---|---|
| `RMC_SKIP_MIR_PASSES=a,b` | skip named MIR passes (names: `lib.rs::optimize_mir`) |
| `RMC_SKIP_PEEPHOLE=a,b` | skip named assembly peepholes (`mir/codegen.rs::optimize`) |
| `RMC_TRACE_PEEPHOLE=1` | print every peephole pass that changed a function, and why asmopt gave up on one |
| `RMC_TRACE_ALLOC=1` | every register candidate in priority order, where it went, and what blocked it |
| `RMC_TIME=1` | per-pass and per-phase timings (MIR route, `rmcmir`) |
| `--no-inline`, `--no-licm`, `--no-tco` | disable a pass on either route |
| `--keep-mir`, `--emit-asm`, `--binutils`, `--no-strip` | inspect intermediate output |

A miscompile on the MIR route is bisected by turning everything off, checking
the output is right, then re-enabling passes one at a time (see `docs/AUDIT.md`
for worked examples).

## Ground rules for every change

1. **A fix ships with a test that fails without it.** Revert the fix locally
   and watch the test fail before committing. Tests that read a value the
   program cannot vary prove nothing: in `tests/mir`, `arg(0)` is `argv[0]`
   (the program path); the first argument is `arg(1)`. A runtime value must
   reach the code under test, or rustc folds it away (`black_box`, `arg(1)`,
   and indexing at runtime positions so GVN cannot read through an array).
2. **Never skip, weaken or delete a test to get green.**
3. **Both link paths must agree.** The in-process encoder and `as`+`ld` must
   accept the same assembly and produce the same behaviour; `tests/run.py` and
   `tests/mir/run.py` check both. Never emit an instruction whose width only
   one of them infers (an unsuffixed immediate-to-memory op).
4. **A new fuzz mode is checked for validity, not just a clean run.** Count how
   many generated programs rustc accepts and rmc compiles; then reintroduce a
   known bug and confirm the mode finds it. A mode with a 100% skip rate is
   silent, not clean.
5. **Measure before claiming.** Compare on the same source (`run_four.py`), name
   the CPU and the baseline, report geometric means, and report both sets:
   `core` (what the optimisers were tuned on) and `real` (held out). A win on
   `core` that is a loss on `real` is over-fitting, not a result. Differences
   under ~5% are noise unless repeated.
6. **Keep commits focused and the tree clean.** No build artifacts; stage files
   explicitly (`git add <paths>`), never with a stale index.

## Invariants, by area

The bold phrase is greppable in `docs/design-notes.md` for the full reasoning.

### Front end (`front/`)

* `match`, `if let`, `while let`, guards, or-, range-, struct-, tuple-, array-
  and `@`-patterns are **desugared in the parser**; sema and codegen have no
  pattern case. Marker nodes (`PatScrutinee`, `TupleLit`, `PatArity`) are
  rewritten by sema and never reach a later pass.
* Precedence, loosest to tightest: `||`, `&&`, comparison, `|`, `^`, `&`,
  `<< >>`, `+ -`, `* / %`, `as`, unary. Shifts bind looser than `+`.
* An integer literal takes the declared width, through `expected_int` and
  `expected_elem_int`; **a repeat literal's value does too** (`[0; n]` in a
  `[u8; n]`).
* Generated temporaries use names containing a NUL byte, so they never collide
  with user identifiers. Nested `fn`s are hoisted with `rmc$nf<n>$name`.
* Every recursive construct counts a level with `bump_depth` (`MAX_DEPTH`).
* Soundness: `mut`, definite initialization, constant bounds, `str` typing,
  casts and constant arithmetic are checked as rustc does; a program rustc
  rejects must be rejected (`fuzz.py --mode reject`).

### AST passes (`opt/`)

* **Inlining** gives each expansion fresh frame slots; an argument with a side
  effect, or a parameter the body assigns or takes the address of, is bound to
  a fresh slot rather than substituted.
* **LICM never hoists a read through a reference** unless provenance proves it
  frame-pointing; it is off under `--overflow-checks`; `for` slots are
  loop-carried.
* **TCO** applies only when exact (associative op, pure guard); off under
  overflow checks.
* **DCE** over-approximates reachability; a program without `main` is untouched.

### AST code generator (`x86/codegen.rs`)

* Every expression leaves its value in `%rax`; only `%rcx`/`%rdx` are scratch.
* **Stack slots grow downward**; aggregate element 0 is at the lowest address;
  a wide element's stride is `size()/8` slots.
* Two-pass codegen (**dry pass** then real) must reset every counter.
* A local whose address is taken is forced into memory before promotion;
  arrays are never promoted.
* Aggregates cross calls by hidden pointer; `%r10` carries the return buffer;
  arguments are evaluated left to right; at most six parameters.
* **Every array element takes an 8-byte slot**, so a function whose frame would
  exceed 6 MiB is rejected with a message (`lib.rs::check_frame_sizes`) rather
  than segfaulting on entry.
* Bounds checks are on by default (`--no-bounds-checks` opts out).

### Assembler and linker (`x86/elf.rs`)

* **A symbol defined twice is an error**, as in `as`.
* Operand width follows the register named; an unsuffixed `mov $imm, mem` is
  32-bit (as `as` does); ALU immediate-to-memory ops must carry a suffix.
* `.p2align n` aligns to 2^n and pads executable sections with `nop`.
* **Branches stay off 32-byte boundaries**: a `jmp`/`jcc`, or a `cmp`/`test`/
  `add`/`sub`/`and`/`inc`/`dec` fused with the `jcc` after it, is padded so it
  neither crosses nor ends on one (a straddling loop test cost 4x on the
  benchmark host). `--binutils` passes `as` the same rule, `nop` padding only.
* Two `PT_LOAD` segments (R+X, R+W); never a writable+executable segment.

### Runtime (`x86/runtime.rs`)

* The runtime is a table of pieces; **every piece names every symbol it
  defines** and re-opens `.text` first. Only pieces a program reaches are
  linked.

### MIR route (`mir/`)

* **One pass pipeline**: `lib.rs::optimize_mir` is called by the CLI, the dylib
  and `rmcmir`. Never copy the pass list.
* **A destination place reads locals** (`(*_p)[i] = x` reads `_p` and `i`);
  every use/def analysis must count them (`count_place_reads`), and so must
  the promotion weights.
* **Split assembly operands with `split_ops`** (last comma outside
  parentheses), never at the first comma: scaled-index addresses carry commas.
* A peephole that replaces an instruction with `lea` must check
  `flags_used_by` on the next instruction; **never fold `mov $k, %t; sub %s,
  %t`** (it is `k - s`).
* **Local labels carry the function name** (`.Lrmc_<fn>_<n>`, `.Lbb<fn>_<n>`).
* **`[v; n]` stores with the element's size** (bytes, words, or whole aggregates).
* `ArgumentType::Count(n)` is the niche layout `[0, n]`.
* `black_box::<T>` for a one-word `T` is the runtime identity call.
* No memory-to-memory `cmp`: load one side first.
* **A branch on a comparison uses the operands' kind**: `jb`/`jbe`/`ja`/`jae`
  when either side is unsigned or a pointer; a float comparison is never folded
  into a branch (it goes through `ucomisd`, which handles NaN).
* **Register sharing needs every read in the liveness**: `return` reads `_0`,
  an indirect call its callee, a written place its base and index locals, a
  folded comparison its operands at the terminator. Locals sharing a
  caller-saved register share its spill slot. A local live across a call takes
  only a callee-saved register.
* **`coalesce_scratch` folds only copies out of a scratch register**; a copy
  into one (`mov %r14, %rcx; imul %rcx`) must stay.
* **A dropped assertion rests only on facts that still hold**: a call kills
  its destination, no fact is kept about an address-taken local, and a folded
  overflow check keeps its comparisons when another block reads them.
* **`split_loop_ranges` is an identity**: a save follows the entry block's
  last mention of its value, a restore precedes the exit's first mention, and
  a new edge block joins every enclosing loop. It sinks a constant assignment
  only for a local whose address was never taken: a read through a pointer
  does not name it.
* **asmopt runs once, after every text peephole**, never before one, and
  leaves a function untouched when it meets a line it does not model. It
  trusts a call's `#argregs N` note (written by `gen_call`) for which argument
  registers the call reads; an undercount deletes an argument's setup.
* **Speculated statements must be speculatable**: a select or a folded
  condition moves only register arithmetic and comparisons (no deref, no
  computed index, no `/` or `%`) that define locals nothing else reads and
  that the moved code does not read before writing (a value carried between
  runs, `c || { t += 1; t > 5 }`), and a condition chain is folded only when
  it ends in constant arms.
* **`fold_range_checks` runs after `drop_redundant_asserts`**, which reads
  bounds off the original tests.
* **A range bound is for the destination's width**: `rv_range` takes the
  destination type, and a signed result narrower than 64 bits keeps a bound only
  when it fits (`126 + 2` in an `i8` is `-128`, not 128).
* **A tuple has one layout** (`tuple_layout`): constant, field offset, element
  stride, copy length and drop glue all come from it; field `k` starts at
  `max(end of the previous field, 8k)` and the size runs to the end of the last.
* **Call arguments are one parallel assignment**: operand evaluation writes
  only `SCRATCH_REGS`; a cycle goes through `%r11`/`%r10`, never promoted.
* The dylib links one exact nightly (`backend/rust-toolchain.toml`); every
  harness reads the pin from there. Panic stubs are derived from `nm -u`, never
  hard-coded mangled names.
* An unsupported construct must exit 1 with a message naming it, never panic.

### Tests and benchmarks

* `tests/mir/real_*` and `bench/real/*` are the same ten realistic workloads;
  each found a back-end bug when written. Keep them out of tuning.
* Benchmark inputs must be opaque (`arg_i64`, `black_box`); a rustc twin whose
  time equals its `hello` time has been constant-folded.
* `heap`, `refs`, `callrec` are the cases most sensitive to the generated code;
  near-parity programs flip sign between hosts.
* A run under ~5 ms (`fib`, `stats`, `light`, `heap`) moves by 5-15% with code
  placement alone; compare such programs on many repetitions, interleaved.

## Git workflow

* Develop on a branch; one focused commit per fix, with its test.
* A review fix lands on the branch the finding names, then merges upward.
  Resolve conflicts by hand: `-X ours`/`-X theirs` silently drops code.
* Before every push: build, `cargo test`, and the four suites above.
