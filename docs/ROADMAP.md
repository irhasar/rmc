# rmc roadmap

Where the project is going, in priority order, and how each step will be
judged. The current state, with measurements, is in [`AUDIT.md`](AUDIT.md);
this file is about what comes next.

## The goal, stated honestly

rmc can be two useful things, and the roadmap keeps them apart.

1. **A very fast compiler for a declared Rust subset** (rmc's own front end).
   It already builds a program in milliseconds, roughly the cost of rustc's
   front end alone and an order of magnitude below an LLVM build. Its limit is
   language coverage, and a hand-written front end will not reach rustc's
   (traits, generics, inference, borrow checking). The target is therefore "all
   of the subset in `docs/language.md`, rustc-identical, ten times faster to
   build", not "all of Rust".
2. **A development back end for rustc** (the `-Zcodegen-backend` dylib). The
   reference point is `rustc_codegen_cranelift`, the official fast-compile back
   end, which compiles every program in the survey. To be worth using, rmc's
   back end has to cover what Cranelift covers and build faster than it. It is
   not a candidate to replace LLVM for release builds: on code it was not tuned
   for it runs well behind LLVM (see `AUDIT.md`), and LLVM's value is exactly
   the optimisation and target breadth rmc does not have.

## 1. Correctness infrastructure (mostly done in this round)

Done:

* a differential fuzzer for the MIR routes (`fuzz.py --mode mir`, both
  `--pipeline mir` and `cb`, with and without overflow checks) in CI;
* every MIR case built through `as`+`ld` as well as the in-process encoder;
* `tests/std` in CI; ten realistic workloads as regression cases;
* one MIR pass pipeline shared by every route;
* bisecting switches (`RMC_SKIP_MIR_PASSES`, `RMC_SKIP_PEEPHOLE`,
  `RMC_TRACE_PEEPHOLE`, `RMC_TIME`).

Next:

* run the MIR fuzzer with `--keep` on a schedule and add each finding as a case;
* extend `--mode mir` to **unsigned and float** values (it generates only
  `i64`, which is why it never found the signed-branch and float-branch
  miscompiles `tests/mir/cmpkinds` now covers), then structs passed by value,
  enums, nested arrays and `u8`/`i32` arithmetic;
* a stress mode that runs the suites and the fuzzer under deliberately unusual
  register allocations: one such experiment exposed the `coalesce_scratch`
  miscompile, which ordinary allocations hid;
* an encoding oracle (`--encoding`, as the AST route has) for the MIR route.

## 2. The MIR route's architecture (the largest single lever)

The back end receives MIR as **text** (`write_mir_pretty`, re-parsed by
`mir/mod.rs`), and only the local crate's MIR at that. Every generic standard
library function a program instantiates (`Vec<T>`, `Option::map`, iterator
adapters, `core::fmt`) has no body to compile, so the front end recognises a
growing list of std items by name and maps them to runtime helpers. That list
is brittle (one nightly can change an inlined shape, as `env::args().nth`
showed) and it is the root cause of most refusals.

Replace it with what `rustc_codegen_cranelift` and `rustc_codegen_gcc` do:

1. collect work with `tcx.collect_and_partition_mono_items(())`, and lower each
   `MonoItem::Fn(instance)`'s `tcx.instance_mir(..)` directly, with generics
   substituted and layouts from `tcx.layout_of`;
2. materialise constants from the interpreter's allocations (bytes plus
   relocations) instead of parsing their printed form; this alone covers every
   `Vec` program, which currently fails on a `Layout` constant;
3. use `tcx.fn_abi_of_instance` at every call boundary, so calls into
   precompiled `std` follow the real Rust ABI (rmc's internal convention, with
   an aggregate return buffer in `%r10`, does not);
4. emit relocatable objects per codegen unit and let `rustc_codegen_ssa` link
   them against the sysroot, which resolves non-generic `std` functions
   (`std::env::args`, `from_ascii_bytes_radix_impl`, ...). Keep the in-process
   ELF writer as a fast path for `no_std` programs;
5. keep the printed-MIR route only as a test input format.

Measure: survey coverage (currently 83/117 through the dylib) toward
Cranelift's 117/117, with no wrong results.

## 3. Coverage of rmc's own front end

In order of snippets unlocked per unit of work (`tools/mir_survey.py` lists
the exact refusals): format specifiers beyond `{}`; integer-width methods
(`wrapping_*`, `checked_*`) on every width; `std::mem::{swap, replace, take}`;
more than six parameters; module paths; closures (as hoisted functions plus an
environment struct); iterator chains desugared to loops; slices; generics by
monomorphisation; floats. Packing `[u8; N]` into bytes (rather than one 8-byte
slot per element) removes the frame-size limit `check_frame_sizes` enforces.

## 4. Run-time code quality

Measured on the held-out `real` set, where rmc's code is behind LLVM. Done in
the second round (`AUDIT.md`, section 9): an interference-based register
allocator with one priority order, arguments kept in their incoming
registers, branch alignment off 32-byte boundaries, immediate shift counts,
one-instruction element addressing through pointers, out-of-line bounds-check
failure paths. Next, in expected order of payoff:

* **live-range splitting**. With nine allocatable registers, a value live
  through an inner loop it never touches (an outer counter) still holds its
  register there. Splitting it around the loop (store before, reload after)
  frees the register for the loop's temporaries. Re-weighting cannot do this:
  three weightings were measured and each moved the loss between programs
  (`design-notes.md`, "Register sharing").
* **more allocatable registers**. `%r10`/`%r11` are scratch for division,
  block copies and the return buffer, and several peepholes assume they are
  free; freeing them per function needs those lowerings and the peepholes'
  `SCRATCH` lists to agree.
* **bounds-check elimination by range analysis**. A version that bounds
  `(y & 127) * 128 + (x & 127)` and signed counters under `while x < N`
  removed every check in `life`, but ran slower: the index lost its second use
  and with it its register. It should land together with splitting; the
  soundness half (stale facts) is already in.
* if-conversion to `cmov` for data-dependent branches, with a heuristic that
  keeps a predictable branch;
* induction-variable strength reduction, generalising the single-pattern
  `close_counted_loops`.

Judge every change on `real`, not only on `core`, with many interleaved
repetitions: programs under ~5 ms move 5-15% with code placement alone.

## 5. Compile time at scale

`bench/scale.py` shows the codegen backend's cost growing faster than linearly
with function size. This round removed the worst case (an O(n^3) reachability
closure); profiling the remainder points at:

* the assembler's branch relaxation (`x86/elf.rs::relax_branches`), which
  re-sums alignment padding for every branch;
* the text peepholes (`mir/codegen.rs`), which re-parse each line in every pass
  and scan forward to the end of the function, quadratic in a long function.

The durable fix for the second is to run the peepholes on a small structured
instruction list instead of on strings.

## 6. Benchmark integrity

* record results on at least two microarchitectures, with repetitions and a
  confidence interval, before a ratio goes into the README;
* keep `bench/real` out of tuning; add programs to it, never remove a losing one.

## History

The previous version of this file tracked the MIR route's early milestones
(freestanding `memset`/`memcpy`, clean refusals instead of panics, the `light`
addressing-mode fold, function pointers, and the move to a `-Zcodegen-backend`
dylib). Those are done and described in `docs/design-notes.md`. Two of its
claims did not hold up and are corrected here and in `AUDIT.md`: the dylib
renders MIR to text and re-parses it rather than receiving `mir::Body`, and the
run-time parity with LLVM held on the twelve tuned programs but not on held-out
code.
