# AGENTS.md

## Session (2026-11-23, Fase 2 step 6) — narrow-type semantics + div_test
- New `rakit-ir/src/int_sem.rs`: `width`/`mask`/`wrap`/`sign_extend`/
  `shift_amount` plus shared `eval_bin`/`eval_icmp`. Values are zero-extended
  bit patterns per type; signed ops sign-extend, shifts mask by `width-1`.
- `interp.rs` and `opt.rs` now use `int_sem` for integers, so `I8/I16/I32`
  arithmetic wraps and signed compares are correct in both execution and
  constant folding.
- `div_test` now covers `I8/I16/I32/I64` and computes a width-specific inverse
  (`C^-1 mod 2^bits`) and threshold (`(2^bits-1)/odd`).
- Tests: `cargo test --manifest-path rakit-ir/Cargo.toml` -> 21/21 (6 div_test
  + 15 ir; new: narrow wrap/sign-extend, shift amount, I32 wrap, narrow
  constant folding, narrow inverse, narrow div_test).
- End-to-end: `RAKIT_RIR=1 RAKIT_NO_DIVTEST=1` on `u32 % 3` and `u8 % 3` logs
  `[rir] div_test fired` for both and prints `1 2 4294967295 1 2`, identical
  to the normal build.

## Session (2026-11-23, Fase 2 step 5) — `div_test` covers even divisors in RIR
- `rakit-ir/src/div_test.rs` now also handles `C = 2^s * odd` and powers of two
  for `I64`: mask `x & (2^s - 1)` must be zero, plus the odd-divisor
  multiply-compare; the two Bool tests combine with `select(a, b, false)` and
  `ne` inverts with `select(is_zero, false, true)`.
- The rewrite now inserts a prologue of new instructions and replaces the
  original `icmp` with `Copy(rr, Bool, is_zero)`; `optimize` then DCEs the
  `urem`.
- Tests: 15/15 (the even/power-of-two test now asserts the rewrite fires for
  `4`, `8`, `12` and for `ne` on `4`).
- End-to-end: `RAKIT_RIR=1 RAKIT_NO_DIVTEST=1` on `x % 12 == 0` and
  `x % 8 == 0` logs `[rir] div_test fired` for both and prints `1 2 1 1 2`,
  identical to the normal build.

## Session (2026-11-23, Fase 2 step 4) — `div_test` migrated to RIR
- `rakit-ir/src/div_test.rs`: RIR port of the CLIF divisibility fold for `I64`
  and odd `C > 1`: `icmp eq/ne (urem x, C), 0` -> `imul x, C^-1` +
  `icmp ule` (and `select` for `ne`). `run()` resolves `Copy` chains and finds
  the defining `urem`; `RirPass` calls it after `optimize` and runs `optimize`
  again to DCE the now-dead `urem`.
- Even divisors, narrower types and `brif`-on-remainder stay in the CLIF pass:
  RIR `brif` takes `Bool`, and RIR has no `uextend`/Bool-`band` yet.
- Tests: `cargo test --manifest-path rakit-ir/Cargo.toml` -> 15/15 (4 new:
  odd `eq`, odd `ne`, even/power-of-two skipped, inverse sweep).
- End-to-end: `RAKIT_RIR=1 RAKIT_NO_DIVTEST=1` (CLIF pass disabled) on
  `fn f(x: u64) -> u64 { if x % 3 == 0 {1} else {2} }` logs `[rir] div_test
  fired` + `[rir] optimized` and prints `1 2 1`, same as the normal build.

## Session (2026-11-23, Fase 2 step 3) — RIR-only folds that fire end-to-end
- `rakit-ir/src/opt.rs` now also folds, for integer types:
  `x - x -> 0`, `x ^ x -> 0`, `x & x -> x`, `x | x -> x`, `icmp x, x` to
  true/false per condition, `select(c, x, x) -> x`, and
  `brif(c, b, args, b, args) -> jump b`.
- Unit tests: `cargo test --manifest-path rakit-ir/Cargo.toml` -> 11/11
  (4 new: icmp same operand, select same arms, bin same operand, brif same
  target).
- End-to-end: `RAKIT_RIR=1` on `x & x` / `x | x` logs `[rir] optimized` and the
  binary prints the same output as the normal build (`1 2 0 0 5 6`). cg_clif
  already folds `x ^ x`, `x - x`, `x << 0` and `if c { x } else { x }`, so only
  `&`/`|` reach RIR here.
- Debug: `RAKIT_RIR_DEBUG=1` also dumps the CLIF of `no-change` functions,
  which is how the surviving `band`/`bor` were located.

## Session (2026-11-23, Fase 2 step 2) — CLIF<->RIR bridge + optional `RirPass`
- `rakit-opt/src/rir.rs`: `import(&Function) -> Option<rakit_ir::Function>` and
  `export(&rir, &Signature) -> Option<Function>` for a conservative scalar
  subset (constants, int/float binops, icmp/fcmp, select, block params,
  jump/brif/return). Calls, memory, stack slots, globals and multi-result
  instructions are refused, so unsupported functions are left untouched.
- **Aliases are the first blocker**: cg_clif emits `vN -> vM` heavily. Without
  `dfg.resolve_aliases` every real function fails to import (a value used in
  `return` has no defining instruction). `lookup`/`map_value` resolve both ways.
- **CLIF bools are `i8`**; RIR has a distinct `Bool`. `as_bool` inserts
  `icmp ne c, 0` for `brif`/`select` conditions, so functions with `i8`
  parameters import. A function *returning* a bool still fails the RIR verifier
  (Bool vs I8) and is skipped.
- `RirPass` runs first in the pipeline but is a no-op unless `RAKIT_RIR=1`.
  `RAKIT_RIR_FORCE=1` re-exports even when the optimizer changes nothing (this
  is how the export path is validated on real CLIF), `RAKIT_RIR_DEBUG=1` logs
  per-function skip/no-change/roundtrip decisions.
- Tests: `cargo test --manifest-path rakit-opt/Cargo.toml` -> 7/7 (5 bridge
  tests: add round-trip, branch folding, block params, floats, unsupported
  rejection). End-to-end: `RAKIT_RIR=1 RAKIT_RIR_FORCE=1` on a `while` loop and
  a bool-parameter `if` gives output identical to the normal build.
- The RIR optimizer only does constant folding, algebraic identities, branch
  folding and DCE -- all already done by Cranelift's egraph -- so `RirPass`
  usually reports `no-change`. This step delivers the bridge and the plumbing,
  not a speedup.

## Session (2026-11-23, Fase 2 step 1) — `rakit-ir` mid-level IR foundation
- New pure crate `rakit/rakit-ir` (no dependencies): SSA IR (`ir.rs`), builder,
  verifier (single definition, dominance, types), optimizer (`opt.rs`:
  constant folding + algebraic identities + branch folding + DCE), and a
  reference interpreter (`interp.rs`).
- Values keep stable indices, so folding rewrites an instruction in place and
  DCE turns dead instructions into `Inst::Nop`; no renumbering yet.
- `cargo test --manifest-path rakit-ir/Cargo.toml` → 7/7 pass (integer/float
  folding, branch folding, DCE, use-before-def rejection, block params/branch
  args, icmp+select).
- Not wired into the compiler yet: next step is the CLIF↔RIR bridge, then move
  passes onto RIR one at a time.

## Session (2026-11-23, later) — extract the optimization pipeline into `rakit-opt`
- New crate `rakit/rakit-opt/` (package `rakit-opt`) owns the CLIF-level
  optimization layer: `PassManager`, `Analyses` (CFG/dominator/loop, cached and
  invalidated per pass), `tunables`, `passtime`, and the seven first-party
  passes (inliner, rec_loop, store_forward, div_test, if_convert, unroll,
  sincos).
- `rakit/src/base.rs` now builds a `PassManager` and runs it, instead of calling
  the passes ad hoc. The inliner stays out of the pipeline because it uses
  `Context::inline`.
- Moved files had `rustc_data_structures::fx` replaced by `std::collections`
  aliases, and `pub(crate)` widened to `pub`, so `rakit-opt` has no
  rustc-private dependency and can be built/tested on its own.
- `Cargo.lock` regenerated (only the `rakit-opt` entry added). Verified:
  `./y.sh build --sysroot rakit` succeeds, `dist/bin/rustc-rakit` compiles and
  runs a sort/filter program (`sum=499500 div3=33`), and `RAKIT_PASS_TIME=1`
  shows store_forward/unroll/if_convert/div_test running via the manager.
- Branch `rakit-middle-end`, PR targets `replace-main-with-cgclif-experiments`.

## Session (2026-11-23) — rebrand the modified fork to Rakit
Only the **modified** backend was rebranded to **Rakit**; the upstream baseline
(`cg_clif_upstream/`) is intentionally left untouched as the original cg_clif /
Cranelift reference. The old engine, project, and IR tokens no longer appear in
the modified tree's source, docs, or scripts.

- **Layout**: the modified tree is `rakit/`; the reference tree stays
  `cg_clif_upstream/` (original cg_clif + Cranelift, unmodified).
- **Vendoring (modified tree only)**: `rakit/vendor/rakit-*` holds the full
  codegen dependency set (`rakit-codegen`, `rakit-frontend`, `rakit-module`,
  `rakit-native`, `rakit-jit`, `rakit-object`, plus the transitive
  `rakit-bforest`, `rakit-bitset`, `rakit-control`, `rakit-entity`,
  `rakit-codegen-shared`, `rakit-codegen-meta`, `rakit-isle`, `rakit-srcgen`,
  `rakit-assembler-x64`, `rakit-assembler-x64-meta`), wired as relative path
  deps. `cg_clif_upstream/` keeps its original registry dependencies.
- **Package/binary (modified tree)**: `rustc_codegen_rakit`, dylib
  `librustc_codegen_rakit.so`, wrappers `rustc-rakit` / `rustdoc-rakit` /
  `cargo-rakit`, dist `rakit/dist`.
- **Env knobs (modified tree)**: the old prefix is now `RAKIT_*` (e.g.
  `RAKIT_NO_LANES`, `RAKIT_UNROLL_DEBUG`, `RAKIT_INLINE_MAX`); the IR-time
  counter is `RAKIT_IR_TIME`.
- **Manifests (modified tree)**: the old `[patch.crates-io]` was removed; deps
  point directly at `vendor/rakit-*` via `path`.
- **Build verified** with `nightly-2026-11-17` + `rust-src,rustc-dev,llvm-tools`:
  `cargo check`/`cargo build --lib` clean in `rakit`, and
  `./y.sh build --sysroot rakit` produced `rakit/dist/bin/rustc-rakit` plus
  its Rakit-compiled std sysroot; a hello-world compiled and ran (`sum=5050`).

## Session (2026-11-22, "win everything") — why the remaining p1/p2/p3/p4/p8/p9 gaps are architectural
Ran the full per-benchmark bottleneck hunt (with RAKIT dumps via `--emit=llvm-ir`,
which writes `<out>.rakit/`). Every losing benchmark reduces to a deep compiler gap;
no knob closes them. **Nothing new landed beyond the already-pushed `a51c34b`
(simd_splat)**; tree verified clean.

- **p3_calls (1.62x)**: `rec_loop.rs` ALREADY converts `fib` to a single-call loop
  (verified in disasm: `cmp $2; jb base; call fib; sub $2; add; jmp`). The gap is
  the **`format!("item-{}-xyz", i)` string machinery** (std::fmt, allocations),
  not recursion. Hard (std-level).
- **p9_u64f32 (1.12x)** and **p4_simd (1.53x)**: the `sin()`/`cos()` loops are
  **multi-block**. Exact p9 shape: header `block2` (`icmp i<n; brif`) → `block6`
  (`call sinf`) → `block8` (`icmp` bounds check, `brif` hot→`block10` /
  cold→`block12` panic) → `block10` (store, `i++`, jump `block2`). `unroll::analyze`
  requires a *single-block* body whose jump goes straight to the header, so it
  rejects this (`then-arm is not body`). LLVM unrolls 4× for ILP on the
  independent `sinf` calls. Closing needs a **multi-block unroller** (clone a
  body block-set + remap inter-block args + remainder loop) — a real pass.
- **p8_minmax (1.15x)**: already unrolled **3×** (`ATOM_FACTOR=3`); LLVM 4×. Both
  use `cmovb`. rakit uses **memory-operand `imul 0x..(%rip)`**; LLVM hoists the
  constant to a register. `ATOM_FACTOR=4` **regressed p7 by 32%** (33.7→44.4 ms)
  and didn't help p8 — reverted. 3 is the tuned optimum.
- **p1_arith (1.25x)**: memory-operand `imul`/`mulsd` + branchy saturating
  `f as u64` (tested before, not worth it).
- **p2_mem (3.0–3.2x)**: regalloc spills (partition 83 rsp refs / 1072 B);
  SipHasher is scalar compute (3 rsp). Needs rakit regalloc work.
- **Bottom line**: rakit wins p5/p6 (~11×), p7 (0.82×), ties p8/p9 (~1.1×),
  loses p1/p2/p3/p4. Beating LLVM on *all* requires months of backend work:
  **multi-block unroller + reassociator**, **bounds-check elimination**,
  **regalloc improvements**, **autovectorization/sincos fusion**, and std-fmt
  speedups. Not session-sized knobs; a multi-block unroller alone is a 2–4h,
  correctness-critical change touching all code.

## Session (2026-11-22, later) — `simd_splat` scalarization fixed (LANDED, p2 −3.5%)
- **Found via disassembly of `hashbrown::raw::RawTableInner::find_inner`** (17% of
  p2): `_mm_set1_epi8(tag)` / `Simd::splat` was lowering to **N individual byte
  stores + a reload**, not a vector broadcast.
- **Path**: `_mm_set1_epi8` → `Simd::splat` → `std::intrinsics::simd::simd_splat`,
  lowered in `src/intrinsics/simd.rs`. The `sym::simd_splat` arm wrote the value
  to *each lane* (`for i in 0..lane_count { place_lane(i).write_cvalue }`) = N
  scalar stores. Replaced with a single `ins().splat(vec_ty, scalar)`.
- **Two guards are required** (both learned from real panics):
  1. `ret.layout().size.bytes() == 16` — the x64 backend only lowers `splat` for
     the natural 128-bit vector types. `i8x8` (64-bit) and `i8x32` (256-bit) hit
     `Compilation(Unsupported("should be implemented in ISLE: splat.i8x8"))`.
     So `<= 16` is wrong; it must be `== 16`.
  2. skip f16/f128 lanes (no RAKIT vector mapping → per-lane fallback).
- **`fx.rakit_type()` returns None for SIMD types** (`rakit_type_from_ty` has no
  SIMD arm — only scalars/pointers). The first attempt used it for the vector
  type, so the guard `if let Some(vec_ty) = fx.rakit_type(...)` silently failed and
  fell back to per-lane stores — the change looked correct but did nothing.
  Use `crate::intrinsics::rakit_vector_type(tcx, layout)` instead.
- **Measured** (`p2_slow`, 60x sort+hash loop): 2548 → 2387 ms (**−6.4%**);
  `p2_mem` 48.1 → 46.4 ms (**−3.5%**). `find_inner` 891 → 640 B with the 16
  byte-stores gone (0 `mov %sil` byte-splat stores left in the binary); also
  `simd_contains` closure −540 B, `Mask::to_*` −273 B, `memchr` −119 B,
  `prepare_rehash` −112 B. Binaries ~1.5 KB smaller.
- **Correctness**: `./y.sh test --sysroot rakit` → **65 suites, 0 failed (incl.
  portable-simd), exit 0**; p1–p9 byte-identical to LLVM; extbench crate tests
  (`exttests.sh`, openlocationcode/holidays-rs/stdnum-rs) all **ok for L/U/M**.
- **Post-fix p2 profile** (`p2_slow`): partition 32.9% (regalloc spills),
  `raw_entry` 12.6%, `SipHasher::write` 11.5% (697 B, **only 3 rsp refs / 0 calls**
  — pure scalar SipHash compute, not spill-bound, no easy win), `find_inner`
  9.8% (was 17.4%), `find` 7.8%, `swap_if_less` 5.5%. The remaining p2 gap is
  the partition's regalloc spilling (rakit-codegen) + scalar SipHash; both
  need deep changes.
- **Methodology gotcha (cost real time)**: editing the codegen backend does **not**
  invalidate cargo's std cache — the `.so` is loaded dynamically and is not part
  of the fingerprint, so `./y.sh build` recompiles libcore *incrementally* and
  keeps the **old** lowering baked into std. Force it with
  `rm -rf build/stdlib_target` before `./y.sh build` when validating a lowering
  change. Also: `nm | grep find_inner | tail -1` picked a stale/second instance
  and hid the win; diff function sizes (`nm -S` old-vs-new) to confirm.

## Session (2026-11-22, later) — Experiment A (VarN aggregate promotion): NEGATIVE
- **Goal**: keep the hot `PartitionState<u64>` (32-byte / 4-word aggregate, the
  46%-spill source in `partition_lomuto_branchless_cyclic`) in registers instead
  of stack slot `ss2`. rakit's `is_ssa` only accepted 1–2 word RAKIT types, so a
  4-word aggregate always became `new_stack_slot`.
- **Implemented** `CPlaceInner::VarN(Local, [Variable;4], u8)` + `word_aggregate_words`
  (flat struct/tuple, all fields i64-scalar or i64-pair, 1..=4 words) +
  `new_var_n` + arms in every `CPlaceInner` match (`to_cvalue`/`to_ptr`/`to_place`/
  `force_to_stack`/`place_field`/`write_cvalue`/lanes/unsized), and `is_ssa`
  extended to accept word aggregates. Committed… **no, reverted** (see below).
- **Two real bugs found and fixed while building it** (kept in notes in case it
  is retried):
  1. `place_field` word index must use `layout.fields.offset(i)/8`, **not** the
     sum of preceding field *sizes* — struct definition order ≠ offset order, and
     the size-sum gave the wrong var → **segfault** (p2_mem printed nothing).
  2. Restrict `word_aggregate_words` to `ty::Tuple` / `ty::Adt(def) if
     def.is_struct()` — enums/unions have a single "variant" field and broke
     `place_field` (`VarN field width` panic while compiling `core`).
- **Result: NEUTRAL, reverted.** After fixing both bugs: p1/p2/p3/p4 all
  byte-identical to LLVM, but `partition_lomuto_branchless_cyclic` spills are
  **unchanged at 57** (`(%rsp)` refs), fn size 525 B unchanged, and A/B
  (`RAKIT_NO_VARN` kill-switch) on p2/p3 shows no runtime or size change
  (p2 48.0 vs 48.1 ms; size 4,145,792 vs 4,145,656 — VarN marginally *bigger*).
- **Why it doesn't help**: the partition takes `&mut` of the `PartitionState`
  (or its fields), so `analyze` marks it `NotSsa` or codegen `force_to_stack`s
  it — the aggregate is materialised regardless of the VarN representation. The
  VarN'd path is only reached for locals that are already `MaybeSsa`, and those
  either aren't the hot ones or get materialised at the first `&mut`/address use.
- **Conclusion**: the 3x p2 gap is **not** the aggregate-local representation.
  Closing it needs either (a) lenient `&mut`-of-field handling in `analyze` so
  only the borrowed field spills, or (b) register allocation quality inside
  `rakit-codegen`. Neither is a quick `VarN` tweak. Do not re-attempt plain
  VarN without first solving the `&mut` materialisation.
- **Infra**: the toolchain was wiped mid-session again (no rustc); reinstalled
  `nightly-2026-11-17` + `rustc-dev,rust-src,llvm-tools,rustfmt`. `./y.sh build`
  rebuilds `.so` + std + `dist/bin` wrappers (~13 s incremental). If `dist/bin`
  looks empty, re-run `./y.sh build` (not `prepare`) to recreate the wrappers.

## Session (2026-11-22) — hot-callee inliner budget (LANDED, p2 −5%)
- **Experiment "rebuild std 1-CGU" disproved the cross-CGU hypothesis.** std is
  already 1 CGU (`ar t libstd-*.rlib` → 1 `.o`); the sort fns
  (`partition_*`/`swap_if_less`/`select_unpredictable`) are monomorphized in the
  **user crate** (mangled names end `_5p2big`); `-Ccodegen-units=1` on the program
  changes nothing (still 65 calls); removing the `is_final` collection filter
  changes nothing (all are `final=true`, already in `bodies`).
- **Real root cause: the per-caller growth budget.** Inliner debug
  (`reject_size id=35 size=48 cap=128 budget=5 self=36`) showed `sort9_optimal`
  inlines ~6 `swap_if_less` then `budget=5` rejects the remaining 65 calls.
- **Fix (`src/inliner.rs`): hot-callee budget.** Once a callee has been inlined
  `>= HOT_CALLS` times into one caller, further inlines may draw on a separate
  `HOT_BUDGET` even after the ordinary budget is spent. `HOT_CALLS=1`,
  `HOT_BUDGET=1024`. Fields: `hot_budget` on `CguInliner`, `use_hot` in `inline`.
- **Result**: p2 52.6 → 50.0 ms (**−5.0%**), 3.42× → 3.25×. p1/p3–p9 within noise
  (≤±1.6%), p9 no regression. Size +0.3%, compile same. `./y.sh test` → **25368
  passed / 65 suites / 0 failed**.
- `HOT_BUDGET=4096` gave p2 −4.1% **and** p3 −5.3% but p9 +4.3% → rejected; 1024
  is the safe point. So "p2 can't be improved" was wrong — sort inlining helps
  ~5%; the rest of p2 is partition codegen (28%, already 0-call, regalloc-bound).
- **Gotchas**: (a) compile-time measurements on this box are noisy — a p5
  "compile +18%" vanished with 8 reps (79 vs 80); always use many reps +
  min/median. (b) `CguBodies` is collected from *all* `codegened_functions` up
  front (not order-dependent), so the inliner already sees every callee; the
  blocker was purely the budget, not CGU or ordering. (c) `git checkout --`
  on the two files fully reverted the debug experiments; the only landed change
  is the ~20-line hot-budget patch in `inliner.rs`.
- **Real-crate validation (extbench / crateworks)** — hot-callee ON vs
  `RAKIT_NO_HOT_INLINE=1`: openlocationcode run −1.5%, holidays-rs −1.0%,
  stdnum +1.4% (noise), compile ~neutral (−0.4%/+3.6%/−0.9%), size ~neutral. So
  it generalizes and is **not** overfit to p2_mem. Contrast a prior
  `MIN_GROWTH=1024` attempt (separate session) that gave p2 −8.7% but +15%
  compile on real crates and was dropped: raising the *floor* for every caller
  costs compile, whereas the hot budget only targets repeatedly-inlined callees.
  `RAKIT_NO_HOT_INLINE` disables it (bisecting knob). extbench needs
  `extbench/crateworks` cloned (`git clone --depth 1
  https://github.com/cipansor/crateworks.git`).

## Session (2026-11-22) — `rec_loop`: recursion→loop (LANDED, p3 −38%)
- New pass `rakit/src/rec_loop.rs`: matches a self-recursive linear
  recurrence `F(x) = if x < base { x } else { F(x-c1) + F(x-c2) }` and rewrites
  it into a loop `acc=0; while x>=base { acc += F(x-c1); x -= c2 } return acc+x`.
  This is what LLVM's middle-end `TailCallElim` does on LLVM IR; rakit has no
  middle-end, so the naive two-call form reached RAKIT. Knob `RAKIT_NO_REC_LOOP`.
- Wired in `base.rs` **before** `store_forward` (early). Matcher is strict:
  exact shape, both calls must be **self-calls**, identity base, `iadd` of the
  two call results; otherwise skipped.
- **Gotcha**: the two self-calls use **distinct FuncRefs** (`fn0`/`fn1`, same
  function, different sigs). Do NOT test `fr1 == fr2`; test
  `is_self(fr1) && is_self(fr2)` via `func.params.user_named_funcs()[reff].index
  == func.name`'s index. This was the first bug (pass silently didn't fire).
- **Gotcha**: must call `crate::rec_loop::run` from `base.rs` — writing the module
  and adding `mod rec_loop;` is not enough (first build "passed" but never ran).
- RAKIT build recipe: `make_block()` does NOT insert into layout; `at_bottom`
  debug-asserts `is_block_inserted`, so `layout.append_block` first. `jump`/`brif`
  args need `BlockArg` (`value.into()`). `InstructionData::Return` has no such
  variant — detect via `opcode() == Opcode::Return` + `arguments()`. `Call`'s
  args via `arguments(&dfg.value_lists)`, not the `args` field. Add `..` to all
  `InstructionData` struct patterns (they carry an `opcode` field).
- **Result**: fib now 1 call/iteration (LLVM shape). p3_calls 92.4 → 57.2 ms
  (−38%), M/L 2.60× → 1.61×. Other benchmarks unchanged. fib(0..40) identical to
  LLVM; 9 bench progs identical; `./y.sh test` → **25368 passed / 65 suites / 0
  failed**.
- Answers the standing "why can't p3 be done" — it CAN; the transform is a
  middle-end pass, not a backend one. This recovers one slice of it.

## Session (2026-11-22) — loop-invariant load hoisting attempt (NEGATIVE, reverted)
- Tried `licm_loads` pass: mark loads from private stack slots at
  `stack_addr(slot)+const`, not stored in their loop, as `readonly+can_move+
  notrap`, so Rakit's LICM hoists them out of the loop (target: the `Vec`
  header `ptr`/`len` reloaded every iteration).
- **It fires (binaries of p2/p4/p9 change) and is correct, but gives 0 runtime
  win** (p2 median 54.4 on vs 53.8 off; p4/p9 reload counts identical).
- **Root cause 1**: Rakit LICM refuses to hoist any load when the loop has an
  *opaque store* (e.g. the `sinf` call in p9's loop):
  `safe = !any_opaque_store && !store_bases.contains(base)`. A private-slot load
  is still blocked even though the call cannot write the slot. Fixing this needs
  a `licm.rs` patch (allow private-slot loads past opaque stores).
- **Root cause 2 (decisive)**: the header reloads are NOT the bottleneck anyway.
  p9's loop is 85% `sinf`; p4 is vectorization-bound; p2 is sort/hash codegen.
  Removing two L1-hit reloads is invisible.
- Pass removed; REPORT §26 records it. `./y.sh build` clean.
- Lesson: before building a load-hoisting pass, confirm the loop is
  memory-reload-bound (disasm + profile), not libcall- or vectorization-bound.

## Session (2026-11-22) — bounds-check elimination attempt (NEGATIVE, reverted)
- Tried a `redundant_branch` RAKIT pass: eliminate `brif (icmp CC x y)` when a
  dominator has the same comparison and its matching edge dominates. Target: the
  redundant `i >= len -> panic_bounds_check` inside `for i in 0..v.len()`.
- **It fires on tiny cases but NOT on any of the 9 benchmarks (binary unchanged).**
  Root cause found by dumping RAKIT: the loop guard compares against the `Range`'s
  `end` value (threaded as a **block param**), while the bounds check compares
  against a **separate** `v.len()` load — different SSA values even though equal.
  Block-param substitution on the direct edge was added but still doesn't match
  because the two length loads are not CSE'd.
- **Prerequisite for a real bounds-check eliminator**: CSE/value-numbering of the
  two length loads (`Range::end` vs `v.len()`) + proof no store to the Vec header
  slot between them. That's alias/load analysis, not a branch pass. Don't retry
  the branch-only approach.
- **Also not worth it even if it worked**: `bcbig` is 134 vs LLVM 40 ms — a 4x
  **vectorization** gap, bounds checks are ~10-20%. So bounds checks are not the
  lever for p4/p2.
- Pass removed; REPORT §25 records this. `./y.sh build` clean, 9 bench progs match.
- Useful debugging trick: dump the loop RAKIT by iterating `layout.blocks()` +
  `dfg.display_inst` in a pass gated on an env var — there is no built-in RAKIT
  dump flag.

## Session (2026-11-22) — sincos fusion (LANDED, p4 100→86ms)
- **New pass `src/sincos.rs`** (runs in `base.rs` after `unroll`, has `module`
  context): fuses `sinf(x)`+`cosf(x)` (and `sin`/`cos`) with the same argument
  into one `sincosf`/`sincos` libcall — LLVM does this, rakit didn't. **p4
  100→86 ms (14%)**, LLVM 59. Knob `RAKIT_NO_SINCOS`.
- **Cross-block works**: sin's block need only **dominate** cos's block (not same
  block). p4's sin/cos ARE in different blocks (bounds-check branches between),
  so same-block-only would have missed it. Emit `sincos` at the sin position, load
  both outputs, `detach_inst_results` + `change_to_alias` to rewire.
- **BUG that cost the round**: the first version declared ONE func_ref from the
  *first* pair's type. A function with BOTH a f32 pair and a f64 pair (p4 loop 1
  does) then calls `sincos` (f64) for the f32 pair → wrong. 9 bench progs still
  passed (p4 is f32-only); only a dedicated f32+f64 differential test caught it.
  **Declare `sincosf` and `sincos` separately, pick per pair.** Lesson: never
  assume a function has a uniform float type across sin/cos sites.
- API gotchas: `Call { func_ref, args, .. }`; `args.get(0, &dfg.value_lists)`;
  `dfg.resolve_aliases` (plural); `MemFlagsData::trusted()` (not `MemFlags::new`);
  `StackSlotData::new(kind, size, align_shift)`; `func.create_sized_stack_slot`;
  `module.declarations().get_function_decl(FuncId::from_u32(index)).name` to get a
  callee's name (`FuncId` recovered from the `ExternalName::User` index).
- **Tests**: `sincostest`/`scloop1`/`scloop2`/`sc1` (same-block, cross-block,
  different args, f32, f64, in loops) → all byte-identical to LLVM. Suite 65/65.

## Session (2026-11-22) — remaining-gap investigation + fcvt local rules
- **fcvt narrowing extended** (`src/unroll.rs`): besides the dominator proof, two
  always-sound LOCAL rules now fire: `fcvt_from_uint(uextend(narrow))` (zero-extend
  → `< 2^32`) and `fcvt_from_uint(band(x, M<2^63))` (`≤ M`). **Important**: a
  narrow *type* alone is NOT enough — `fcvt_from_sint` on a raw `i32` is wrong for
  `u32 ≥ 2^31` (sign bit). Only `uextend` guarantees non-negative. Verified incl
  `u32::MAX`, `4e9`, `& 0xFFFF` masks; suite 65/65.
- **Where the remaining gaps really are (profiled this session — do NOT re-tread):**
  - **p2 (54 vs 17)**: 26% `partition_lomuto`, 31% hashing, 10% `swap_if_less`,
    all in the prebuilt **Rakit std**. Diffuse codegen quality (regalloc +
    cross-CGU inlining + no vectorizer). Not a pass-sized fix.
  - **p3 (94 vs 37)**: 84% in `fib`. LLVM rewrites `fib(n)=fib(n-1)+fib(n-2)` so
    the **second** recursive call becomes a loop iteration → ~**half** the calls
    (verified by disasm: L's loop calls fib(n-1), fib(n-3), …; M does both
    fib(n-1) and fib(n-2) → tree is ~2×). This is a MIR/algorithmic transform,
    out of scope for a RAKIT pass.
  - **p4 (100 vs 57)**: LLVM uses `sincosf` (M emits separate `sinf`+`cosf`), and
    the two remaining loops are vectorized by LLVM (M scalar). BUT the sin/cos
    calls are in **different basic blocks** (bounds-check `je`/`jae` between them),
    so same-block sincos fusion won't fire; cross-block fusion is much harder.
    Also the per-iteration bounds checks (i<a.len()) aren't eliminated because M
    can't prove a.len()==n. → vectorization + bounds-check-elimination wall.
  - **p1 (101 vs 82)**: branchy saturating `f64→u64` (`cvttsd2si` pair) — the
    documented negative-result area (branchless was slower; constants not
    hoistable). Not fixable without hoisting the constants to RAKIT values first.
  - **p8 (21 vs 19), p9 (37 vs 32)**: residual = rakit **regalloc** spills
    (lane loop 6 carried values; p9 reloads a.ptr/a.len from stack each iter).
- **sincos fusion is NOT worth attempting as a RAKIT pass**: declaring the
  `sincosf` symbol needs `module`/`tcx` (available in `base.rs`, not in a raw
  `&mut Function` pass), AND the calls are cross-block in real code. Abandoned.

## Session (2026-11-22) — u64→float narrowing via dominator (LANDED)
- **What**: `narrow_uint_fcvt` in `src/unroll.rs` rewrites `fcvt_from_uint(x)` →
  `fcvt_from_sint(x)` when `x` is provably `< 2^63`. On x86-64 that turns a
  branchy full-range u64→float sequence into a single `cvtsi2ss`/`cvtsi2sd`
  (LLVM does this via its range analysis; Rakit can't see the bound).
  p9_u64f32 **36 → 35 ms** (LLVM 31). Small win but **general**: every
  `for i in 0..n { … i as f32/f64 … }` with small const `n` benefits.
- **The trick is to NOT need loop analysis**: walk the `DominatorTree` up from
  the conversion's block; if a dominator `brif`s on `icmp ult x, C` (or `ule`,
  or `C > x` / `C >= x`) with `C < 2^63` and the **taken edge dominates** the
  block, then `x ≤ C < 2^63` there → safe. Works for **multi-block bodies**,
  which the `analyze`/`Plan` path (§19) rejects — and p9's `for i in 0..n` loop
  IS multi-block (range-iterator lowering splits it), so the Plan-based approach
  never fired; the dominator approach does.
- **API gotchas**: `func.layout.inst_block(inst)` returns `Option<Block>`;
  `Icmp` is `InstructionData::IntCompare { opcode, args, cond }` (NOT `Binary`);
  `Brif { opcode, arg, blocks }` needs `..` in the pattern; `dom.idom(block)`
  returns `Option<Block>`; build via
  `rakit_codegen::flowgraph::ControlFlowGraph::with_function` +
  `DominatorTree::new()` + `dom.compute(func, &cfg)`.
- **Soundness**: only narrows when `C < 2^63` AND the true-edge dominates, so
  `x ≤ C` holds on every path to the conversion. Exit blocks (where the guard is
  false) are not dominated by the true-edge, so they're skipped.
- **Knob**: `RAKIT_NO_NARROW_FCVT=1`. Debug: `RAKIT_UNROLL_DEBUG=1` prints
  `fcvt x=… bound=…`.
- **Tests**: `fcvttest.rs` (const bounds narrowed, dynamic bound falls back,
  `(1<<40)..` i≥2^31 correct) → byte-identical to LLVM; all 9 bench progs match;
  `./y.sh test` 65/65. This pass touches conversions everywhere in std, so the
  full suite is the real gate — it passed.
- **Remaining p9 gap (35 vs 31)** = per-iteration stack reloads of the array
  ptr / len (rakit regalloc keeps loop-invariants on the stack), not the
  conversion.

## Session (2026-11-22, latest) — LCG lane transform (LANDED, p8 29→21ms)
- **What**: new `apply_lanes` in `src/unroll.rs` rewrites `a = a*C + (i|1)` +
  min/max-reduction loops into `F=2` independent lanes → **1 imul/iter** (was
  2). p8_minmax **29 → 21 ms** (LLVM 18). Full suite 65/65 green.
- **The key insight (non-obvious)**: LLVM uses an **even** lane factor so each
  lane keeps a **fixed parity**; then `(i|1) = i + (lane even ? 1 : 0)` is
  **linear** in the lane step, so the forcing term advances by a **constant add**
  (`slope = F·Σ_{k<F}C^k`) instead of a per-step imul. Odd factors (3) can't do
  this. `F_L(k) = fbase + lane_const[L]`, `fbase = iv·S = k·slope`.
- **F=2 beats F=4** (21 vs 23 ms): F=4 carries 8 live values → spills; F=2
  doesn't. F=6=22, F=8=24. Don't assume more lanes = better.
- **The hard part was the exit, not the math.** rakit's exit block reads the
  header params **by dominance with 0 args** (`brif block7(0 args), block8(0
  args)`). So you cannot branch straight to the exit block. **Fix: branch the
  lane loop's exit back to the *old header*** with `(iv=B, a=a_cur, mn)`; it
  re-checks `iv<B` (false) and falls through to the real exit, which then reads
  correct params. Works for any exit shape. This also means the old header
  stays alive as a trampoline and the old body is simply dead.
- **`a_cur` must be carried**: the lane body advances all lanes past `a_B`
  before the header re-checks, so `a_B` is not recoverable from the lanes at
  exit; carry it (set `a_cur = a_last` each step).
- **Detection (`detect_lane`) is strict and conservative**: header params
  exactly `(iv, a, mn)`; `a' = C·a + (iv|1)` (one `Bor imm=1` atom over `iv`);
  `mn' = select(icmp(cc,l,r), arm1, arm2)` over `{a', mn}`; `iv` starts at 0;
  `bound` **constant** and `% F == 0`. Anything else → returns false → the old
  clone path runs. So extra loop state (n≠3) or a dynamic bound never risks
  correctness, only forgoes the speedup.
- **Constants (mod 2^64)**: `cpow[k]=C^k`; `pro_const[j]=Σ_{l<j}C^{j-1-l}(l|1)`;
  `lane_const[L]=Σ_{j<F}C^{F-1-j}((L+j)|1)`; `slope=F·Σ_{k<F}C^k`. Verified by
  hand for F=2,C=3 (const_1=6, const_2=12, F=8k+6).
- **Testing that matters**: compile-time-constant bounds via a macro
  (`for i in 0..4u64 {...}`) so the lane path actually fires; sweep bounds
  `{0,1,2,3,4,5,7,8,12,16,20,100,1000,12345,30M}` (divisible → lane, else
  fallback), min AND max, two multipliers, plus a case that **prints `a` after
  the loop** (=a_B), non-MAX initial `mn`, non-1 initial `a`. A bound that is a
  runtime variable does NOT trigger the path (bound must be `const_of`).
- **Knob**: `RAKIT_NO_LANES=1` disables it. Debug: `RAKIT_LANE_DEBUG=1`.
- **Commits**: detection → full transform → F=2 (4 commits). NOT yet pushed at
  time of writing (needs the user's `repo` PAT; `GITHUB_TOKEN` is read-only).
- **Remaining p8 gap (21 vs 18) = rakit regalloc spills**, not the pass.

## Session (2026-11-22, later) — constant-bound guard + atom factor 3 (LANDED)
- **Toolchain reset mid-session** (the documented failure): `$HOME/.cargo`,
  rustc, cargo, rustup all gone while `dist/` survived. Recovery that worked:
  `curl https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none`,
  then `rustup toolchain install` (reads `rust-toolchain.toml` =
  nightly-2026-11-17). The dist server DOES have 2026-11-17 (this sandbox's
  clock matches the rust dist server). `./y.sh build` then rebuilt in ~12 s
  (target/ survived).
- **p8_minmax / p7_branchy remaining gap = the per-iteration guard**, not the
  atom rebuild. `build_guard` now emits a **single `icmp ult(iv, limit)`**
  against a constant when the trip count is constant (and `bound+last` can't
  wrap); `limit = bound-last` (strict) or `bound-last+1` (`<=`), or 0 if
  `bound < last`. Falls back to the old `band(no_wrap,last_ok)` for
  non-constant bounds. Shared by `apply_partial` and `apply_reassoc`.
- **Two dead ends, both measured and reverted (do not re-try):**
  - Caching atom *values* in `build_affine` (share `(i+jδ)|1` across `pre[k]`):
    **netral**. The O(factor²) cost is the *coefficient* multiplies
    `C^k · a_j`, not the atom values.
  - De-duplicating the body's own single-step `acc_next` against `pre[0]`
    (remap body uses to `pre[0]`, make both branches use `pre[0]`): **netral**
    (35.1 vs 35.0). The redundant multiply is not on the critical path.
- **`ATOM_FACTOR` 2 → 3.** With the cheap guard the knee moved: p8 factor
  sweep 2=31.4, **3=29.1**, 4=33.5, 6=39.3, 8=51.3 ms; p7 2=38.0, **3=35.4**,
  4=45.1. Factor 4+ still loses to O(factor²) coefficient multiplies.
- **Result** (full 3-way, `bench/allaxes.sh` CREPS=3 RREPS=7): p7 40.3→**36**
  (LLVM 45), p8 35.0→**29** (LLVM 18). M beats LLVM on compile wall (most),
  size (~12% smaller everywhere), and runtime p5/p6 (~10×), p7. p9 near-tie
  (34 vs 30). p1/p2/p3/p4/p8 still lose on architectural grounds.
- **Tests**: `./y.sh test --sysroot rakit` → **65 suites, 0 failed** (EXIT 0).
  All bench outputs byte-identical to LLVM.
- **Commits**: `253dc80` (cheap guard), `8e49f2b` (factor 3), + docs. NOT
  pushed (token situation unchanged — needs a user-supplied `repo` PAT).
- **p8 lane restructure is the real remaining fix**: LLVM uses 4 independent
  lanes each advancing by `C^4` (O(f) imuls, full ILP); our `pre[k]` builds
  every step from the header (O(f²) imuls). Matching it needs per-lane carried
  state in the clones — a structural rewrite, out of scope for a safe change.

## Session (2026-11-22) — atom-aware affine reassociation + div_test alias ICE (LANDED locally)
- **Win**: loops of the shape `acc = acc*C + (i|1)` (LCG, p7/p8) used to be
  rejected by the unroller for two reasons: (a) the other state is a
  reduction (`min`) with no closed form, so `power` bailed, and (b) `i|1` is a
  bitwise op, not a linear form. Both are now handled.
- **Model** (`src/unroll.rs`): split into `Pure` (linear form, atom operand),
  `Atom { op, imm, operand: Pure }` (a *recomputable* non-linear term such as
  `i|1`), and `Affine { coeffs, c, atoms }`. `apply_step(step, state)` evaluates
  the one-step update at `state`, rebuilding each atom from its operand's value
  there (`subst_pure`) so `i|1` advances to `(i+delta)|1`. `build_affine`
  materialises every atom independently from the header params, so atoms stay
  off the carried `imul` chain.
- **Equivalence for pure loops**: `power`/`compose` were refactored to use
  `apply_step`; this is algebraically identical to the old composition
  (`F^{k+1} = F_j ∘ F^k`), so already-handled loops are unchanged.
- **Factors**: `ATOM_FACTOR = 2` (rebuilding atoms is O(factor^2), so a small
  factor wins), `PARTIAL_FACTOR = 6` for pure affine reductions
  (`RAKIT_PARTIAL` overrides). Sweep: atom loops best at 2, pure reductions at
  6.
- **Result** (`allaxes.sh`, full matrix in REPORT §19.5): `p7_branchy`
  U=100→**M=42** ms (LLVM 46) — now faster than LLVM; `p8_minmax` 38→**35** ms.
  Overall M now beats LLVM runtime on p5/p6 (6 vs 58 ms, ~9.7x) and p7, ties on
  p9 (35 vs 31), is ~11-12% smaller than LLVM, and compiles p2 1.66x faster.
  Verified output-identical to LLVM (differential test `t_atom`); full
  `./y.sh test --sysroot rakit` → **65 suites, 0 failed, exit 0** (incl. regex,
  rand, graviola, portable-simd, stdarch).
- **Pre-existing ICE fixed** (`src/div_test.rs`): compiling `env_logger 0.9.3`
  (a `regex` dev-dep) panicked in the egraph with
  `index out of bounds ... index is 4294967295` (`Value::reserved_value()`).
  Only `RAKIT_NO_DIVTEST=1` avoided it, so `div_test` was the cause — NOT the
  atom work (ICE persisted with `RAKIT_UNROLL=1`). Root cause: `div_test::collect`
  counted uses only via `InstructionData::arguments()`, which (1) excludes
  branch/block-call args and (2) does not resolve value aliases (`v420 -> v399`,
  which rakit emits heavily). A `urem` result used through an alias or as a
  phi arg looked single-use, so `remove_inst` left a dangling reference. Fix:
  resolve aliases when counting, count `branch_destination(...)` block args too,
  and resolve aliases in `classify_use`. Purely conservative (only rejects more
  candidates), so §18 results stand. `env_logger` now compiles clean.
- **Debugging recipe that worked**: the `#[cfg(false)] PrintOnPanic` block in
  `base.rs` (around line 221) prints the post-pass RAKIT of the panicking
  function when flipped to `#[cfg(true)]`; the dump shows aliases as `vX -> vY`.
- **Methodology note**: this ICE was invisible in earlier sessions because
  `regex`/`env_logger` were never actually compiled — the external crate
  download (`./y.sh prepare`) had not been done. Always run the full suite
  including `regex` before claiming "0 failures".
- **The unroller only fires on a compile-time-constant trip count.** A runtime
  bound (`for i in 0..n` with `n` from args) disables both `unroll` and the atom
  path entirely (atom/noatom binaries become byte-identical). All bench programs
  that show wins (p5/p6/p7/p8) use constant bounds. `bench/tests/t_atom.rs`
  therefore uses `const N`; its reduction states (min/xor/add) stop it folding
  to a closed form so it actually runs and exercises the atom path.

## Session (2026-11-20, latest) — `unroll` pass was quadratic (LANDED, `f6d2ae5`)
- **The per-pass instrumentation had never worked.** `passtime::report()` had no
  caller, and its accumulator was `thread_local!` while codegen runs on rustc's
  worker threads, so every number it could have printed was zero. Fixed both:
  `report("aot")` is now called from `codegen_crate`, and the accumulator is a
  shared `static [AtomicU64; 5]`. Enable with `RAKIT_PASS_TIME=1`.
- **First real numbers** (`p2_mem`, cgu.1): `store_forward=8.4ms unroll=18.9ms
  inline=3.6ms collect=1.9ms if_convert=1.9ms`. `unroll` was the most expensive
  pass, ahead of `store_forward` which does far more work.
- **Root cause**: `collapse_forwarders` → `find_forwarder` rebuilt three
  whole-function facts (incoming edge counts, edge sources, per-value reader
  counts for `params_escape`) on **every successful collapse**. `p2_mem` does
  1419 collapses, so the same function was rescanned 1419 times — quadratic in
  the block count.
- **Fix**: those facts moved into `struct Forwarders`, built once in
  `Forwarders::new` and updated incrementally in `Forwarders::removed` (drop the
  latch's edge, retarget the predecessor's edge to the latch's target). Trivial
  block candidates are also filtered once, not per iteration.
- **Measured**: `unroll` **18.9 → 2.4 ms** (7.9x), `collapse` sub-phase 18.8 →
  2.7 ms, iterations 1419 → 1093.
- **Tests**: `./y.sh test` → **25368 passed / 0 failed** (65 suites). p-suite
  8/8 and s-suite 10/10 output-identical to LLVM.
- **Note on remaining compile overhead**: mod is still ~13% slower than upstream
  to compile `p2_mem` (145 vs 128 ms). That is the honest price of four added
  passes, not a bug — `store_forward` is now the most expensive at 8–12 ms and
  is linear in instruction count (178 funcs → 10.1 ms total). On programs that
  do not trigger the passes the cost is zero.
- **Push/PR**: the injected `GITHUB_TOKEN` is a read-only GitHub App token
  (`ghu_`, "Resource not accessible by integration") and the OpenHands
  credential helper returns empty, so `git push` and the PR API both 403. Both
  the push and the PR edit succeed with a user-supplied `repo`-scoped PAT
  (`ghp_…`) passed inline in the URL. Pass it inline rather than storing it in
  the remote, then reset the remote:
  `git push "https://irhasar:$PAT@github.com/irhasar/rmc.git" <branch>` followed
  by `git remote set-url origin https://github.com/irhasar/rmc.git`.
- **PR**: #10 (`replace-main-with-rakit-experiments` → `main`), head `fd9396a`,
  `mergeable: true`, 888 files changed. Description updated with the §17
  compile-time findings.

## Session (2026-11-19) — small-switch compare chain (LANDED, `6cefd14`)
- **Methodology trap (important)**: `cargo build` in a driver crate does NOT use
  the Rakit backend. You must set `RUSTC=rustc-rakit` (with
  `rakit/dist/bin` on PATH), or you silently benchmark LLVM. Tell-tale
  sign: binary ~579 KB (LLVM) instead of ~1.63 MB (Rakit), and identical
  md5 across tunable values. All §9 numbers in REPORT.md use the correct build.
- **Disproved**: autovectorization is NOT the blur gap. LLVM with
  `-Cno-vectorize-loops -Cno-vectorize-slp` gives 763 insts / **0 SIMD** and
  still takes 1248 ms vs 1246 ms vectorized (551 insts / 71 SIMD). Also
  disproved: `RAKIT_UNROLL` (1/4/16/128 → identical binary; the pass only
  handles single-block constant-delta loops) and raising the inliner caps
  (10380 vs 10359 ms, size grew).
- **Root cause found**: `rakit_frontend::Switch::emit` picks `br_table` for
  any contiguous case range, so `match i { 0=>r, 1=>g, 2=>b }` becomes a jump
  table. Binary had **2714** jump-table sites vs LLVM 0.
- **Fix** (`rakit/src/base.rs`, `SwitchInt`): ≤8 cases and discriminant
  ≤64 bits → `icmp_imm_s` + `brif` chain. The 64-bit guard is REQUIRED:
  `icmp_imm_s` takes an i64 immediate, so i128/f128 discriminants truncate
  (that bug failed 13 coretests).
- **Measured**: blur 10329 → 10004 ms (**-3.1%**), binary 1629168 → 1500000 B
  (**-7.9%**), jump tables 2714 → 84. p1–p8 matrix did not regress (run 525 vs
  528 ms, compile 715 vs 719 ms).
- **Tests**: `./y.sh test --sysroot rakit` → **exit 0, 0 failures across 65
  suites** (6021 tests). Output byte-identical to LLVM.
- **Push**: `GITHUB_TOKEN`/`OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB` are read-only
  (403). Use `GH_PAT` with the explicit helper reset:
  `git -c credential.helper= -c credential.https://github.com.helper= -c "credential.https://github.com.helper=!f(){ echo username=irhasar; echo password=$GH_PAT; };f" push origin <branch>`
- **Remaining gap** (~8.3x on blur) is `rakit-codegen` quality: loop
  rotation / regalloc spilling (hot fn: 240 `(%rsp)` refs vs LLVM 66; LLVM
  concentrates 60.6% of samples at one offset, Rakit spreads over 213).
  Out of scope for this fork.

## Session (2026-11-17) — trivial-leaf inliner exemption (LANDED, `4d046cc`)
- **Root cause**: `MAX_CALLEE_USES = 3` applied to *all* callees, including
  one-operation bodies. `<f32 as Mul>::mul`, `<f32 as Add>::add`, and
  `__iterator_get_unchecked` (1 and 12-13 RAKIT insts) stayed out-of-line after
  3 uses, so hot loops kept real calls, each forcing spill/reload around it.
- **Fix**: leaves with `<= TRIVIAL_LEAF_INSTS` (16) insts are exempt from BOTH
  the use cap and the growth-budget charge. Safe: one op, and a leaf has no
  callees so no cascading; recursion still impossible. Knobs:
  `RAKIT_INLINE_TRIVIAL_MAX`, `RAKIT_INLINE_NO_TRIVIAL` (A/B off).
- **Threshold 8 is too low**: `__iterator_get_unchecked` is 12-13 RAKIT insts.
  Bisect on `filter_symmetric_column` calls: 8/9/11 -> 8 calls; 13/16/32 -> 0.
  Runtime 15.27 s (t=8) -> 12.80 s (t=16); 32 is a plateau. Default = 16.
- **Measured** (image blur 2048px, same compiler binary, knob only):
  22.323 s -> 12.790 s (**1.75x**), binary 1.647.648 -> 1.627.864 B, output
  identical. `filter_symmetric_row`: 581 insts/37 calls -> 359/**9** calls,
  now *smaller* than LLVM (446 insts/9 calls). LLVM still wins via `mulps`
  (9 vs 0).
- **Tests**: `./y.sh test --sysroot rakit` -> 25,368 passed, 0 failed.
  `bat` + `tokei` rebuild and run; tokei output identical per-file (md5 differs
  only by `reports` array order from hashmap iteration).
- **No suite regression**: baseline vs final all within noise. p7_branchy
  +9% is machine drift — the trivial-OFF config alone now measures 62 ms
  where the old log said 55 ms.
- **Corrects report 8.20**: that section claimed the remaining runtime gap was
  "not the inliner, must be inside rakit-codegen". Wrong here — the
  inliner IS a rakit pass and closed 42% of the blur gap. What remains in
  rakit-codegen is (a) vectorization, (b) regalloc spilling (column filter
  still 240 `(%rsp)` refs vs LLVM 66 with zero calls left).
- **Docs**: REPORT.md 8.27 (this change) + 8.28 (final 3-way matrix).
- **Gotcha**: `perf` is not installed in this container; use objdump/nm
  symbol-size analysis instead of profiling.

## Session (2026-11-18, latest) — if-conversion diamond → select/cmove (LANDED locally)
- New pass in the fork: `rakit/src/if_convert.rs`. Collapses a short,
  side-effect-free two-way diamond (`brif` head -> then/else -> common join)
  into a `select` in the head, which Rakit lowers to `cmove`. Runs
  **before** `unroll` (slot `IF_CONVERT` in `passtime.rs`; knob
  `RAKIT_NO_IFCONV`, default ON). Fork commit `7374000` (+ `8f88af2` docs).
- Two wins: a data-dependent in-loop branch mispredicts every iteration; and
  the fork's `unroll` only widens a single-block body, so a diamond blocks
  unrolling. Collapsing first gives the unroller its single block.
- **Cost model is essential** (the key finding): `cmove` computes both arms, so
  `is_cheap_to_speculate` rejects arms holding `FcvtToUintSat`/`FcvtToSintSat`
  (long branchy sequence on x86) or `Udiv/Sdiv/Urem/Srem/Fdiv`. Without it, p1's
  `f as u64` arm regressed p1 105 -> 112 ms; with it, p1 is neutral.
- **Measured** (wall, 30M iters): `p7_branchy` (hash%7 arm) 102 -> **56** ms
  (LLVM 44); `b1` (3-way cheap arms) 74 -> **61** ms (LLVM 60); p1/p4
  (`f as u64` arm) neutral; small 4% loss only on a near-always-false condition.
- **Tests**: `./y.sh test --sysroot rakit` -> 65 `test result: ok`, 0 failed.
- **Deliverable**: `rmc_clone` subtree commits `341a286` (change) + `5bce6fa`
  (docs) on `replace-main-with-rakit-experiments`; REPORT.md 8.22; new bench
  programs `p7_branchy`, `p8_minmax`.
- **PUSHED**: remote `replace-main-with-rakit-experiments` = `f084efe`; PR #10
  head `f084efe`, `mergeable: clean`, base `main`. The injected `GITHUB_TOKEN`
  (read-only GitHub App) and credential-helper token both fail; the push used a
  user-supplied `repo`-scoped PAT (not stored).

## Session (2026-11-18, latest) — affine unroll factor 8 → 128 (LANDED locally)
- `rakit` head **`6951858`** "Raise the affine loop unroll factor to 128
  with a checked step bound"; `rmc_clone` subtree commit **`193ec3e`** (PR #10
  branch `replace-main-with-rakit-experiments`), **not pushed** (see below).
- **Win**: reassociating an affine loop to closed form `S_k = M^k·S + c` removes
  the loop-carried `imul` chain, and the win keeps scaling with the factor.
  p5/p6 (300M iters): 61 ms @ f=8, 32 @ 16, 22 @ 32, 12 @ 64, **7 ms @ 128**,
  4.6 @ 256, 3.3 @ 512, 2.4 @ 2048 (LLVM = 58 ms). Output identical to LLVM at
  every factor; size +32 B, compile +0.4 ms. `UNROLL_FACTOR` default = 128.
- **Latent overflow fixed**: the wide-path guard checks `iv + (factor-1)*delta`,
  valid only while `delta*(factor-1)` does not wrap. Added `safe_factor(delta,
  factor)` shrinking the factor until `delta*(factor-1) <= 2^62`; both sites now
  `checked_mul` (was `wrapping_mul`). Large-step loops auto-use a smaller factor.
- **Environment note**: mid-session the runtime wiped `~/.rustup` + `~/.cargo`
  (no rustc anywhere). Reinstalled via rustup: `nightly-2026-11-17` +
  `rust-src,llvm-tools`. `dist/bin/rustc-rakit` was intact and works after PATH
  restore (`export PATH="$HOME/.cargo/bin:$PATH"` — the wrapper needs `rustc`).
- **Final 3-way matrix** (RC=5/RR=9, `bench/fullbench.sh`; ms, MB, bytes):
  | prog | rwall L/U/M | rcpu L/U/M | size L/U/M |
  |------|-------------|------------|------------|
  | p1_arith | 81/108/**105** | 82/108/**104** | 4.64/5.88/**4.40** MB |
  | p2_mem | 17/157/**69** | 17/157/**69** | 4.64/5.94/**4.42** MB |
  | p3_calls | 37/117/**98** | 37/117/**97** | 4.61/5.82/**4.33** MB |
  | p4_simd | 56/157/**139** | 56/157/**139** | 4.64/5.89/**4.41** MB |
  | p5_inline | 58/895/**7.3** | 57/892/**6.9** | 4.61/5.81/**4.32** MB |
  | p6_scalar | 58/905/**7.4** | 57/905/**7.0** | 4.61/5.81/**4.32** MB |
  Compile: M ≈ U (noise) and both ~1.7× faster than LLVM on p2. Size: M ~5%
  smaller than LLVM, ~25% smaller than U.
- **Root cause of the *remaining* runtime gaps (NOT autovectorization).**
  Disabling LLVM's vectorizer (`-Cno-vectorize-loops -Cno-vectorize-slp`) barely
  changes LLVM's times (p4 55.4 → 57.2 ms), so the gap is deep scalar codegen:
  - p4 trig: LLVM unrolls the `sinf` loop 6× for ILP on the calls (and fuses
    `sin`+`cos` of the same arg into one `sincosf`); M calls `sinf`/`cosf` once
    per iteration. 2.5× even for sin-only (`trig1`: L 30.9 / M 77.0 ms).
  - p2 mem: LLVM fully inlines the hashbrown SSE2 probe into `main`; M leaves it
    out of line (`find_insert_index_in_group` etc.). LLVM's `main` has 9 calls,
    M's has 12.
  - p3 fib: LLVM converts the double recursion to a loop; M keeps recursion.
- **Levers tested and found neutral** (do not re-try without new data):
  `-Ctarget-cpu=native`; `RAKIT_INLINE_MAX` 16/256; aggressive inliner
  (`INLINE_USES=100 INLINE_MAX=200`) → p1 108→105, others flat; Rakit
  `opt_level=speed` vs `speed_and_size` (no difference); clone-based body
  duplication on multi-block bodies (does not fire — `analyze_affine` requires a
  single-block body, so trig/sin loops are untouched).
- `./y.sh test --sysroot rakit` → 3011 passed / 0 failed (before the reset).
- **Push BLOCKED**: `irhasar/rmc` rejects every credential available in this
  environment (403 `Resource not accessible by integration`). Both the injected
  `GITHUB_TOKEN` (GitHub App installation, read-only) and the OpenHands
  credential-helper secret are rejected. To land, push
  `replace-main-with-rakit-experiments` with a user-supplied `repo`-scoped PAT;
  PR #10 then updates in place. Local commits `6951858` (mod) / `193ec3e` (subtree)
  are ready.

## Session (2026-11-18) — inliner growth cap (LANDED locally)
- `rakit` head **`41dc347`** "Bound inliner growth by caller size; speed up
  dead-store removal".
- **Root cause of the p2 compile gap vs U**: the inliner grew RAKIT by up to 62%
  (14449 → 23383 insts on p2_mem). Every later Rakit pass then walks the
  larger function, so the whole codegen slows down. This was the *entire* gap.
- **Fix**: cap the instructions a caller may absorb to a fraction of its own
  size. New tunable `RAKIT_INLINE_GROWTH` (default 50%), clamped to
  `[MIN_GROWTH=64, INLINE_BUDGET=512]`. `CguInliner::new` now takes `&Function`
  (reads `caller.dfg.num_insts()`); `base.rs` passes `&context.func`. Pareto
  knee at 50%: p2 compile 139.9 → 135.3 ms, runtime 61.3 → 67.0 ms.
- `store_forward::drop_dead_stores` now buckets loads per stack slot, so a store
  is compared only against reads of its own slot (was O(stores × loads)).
- **Dead ends measured and reverted** (do not re-try without new data):
  - loop-only (hot-path) inlining via `LoopAnalysis`: worse on *both* axes
    (p2 compile 139.8 vs 138.9 ms, runtime 69.3 vs 66.4 ms). Removed entirely.
  - one-level inlining (`visit_callee: false`): no compile win, hurts runtime
    (p2 69.3 → 74.5 ms). Kept `visit_callee: true`.
  - `-Ccodegen-units` / `-Zthreads`: threading hurts at this scale (p2 only has
    2 CGUs on a 4-core box, so there is little to overlap).
- **PUSHED** (2026-11-18, later session): `replace-main-with-rakit-experiments`
  is at `b521842` on the remote; PR #10 head is `b521842`, `mergeable: clean`,
  base `main`. The earlier credential blocker was worked around with a
  user-supplied `repo`-scoped PAT (used only for the push/PR edit; not stored).
- `./y.sh test --sysroot rakit` → **3011 passed / 0 failed** (+596 core benches).
- `REPORT.md` §8.16–8.17 and `bench/fullbench.sh` record the final matrix
  (RC=11/RR=11).

## Session (2026-11-18) — pass cost + SIMD lane ICE fix
- `rakit` head `97617a1`; `rmc_clone` subtree head `7249d45`.
- New `src/tunables.rs`: every env knob read once via `OnceLock` (passes run per
  function; `std::env::var` locks a global map each call).
- `store_forward` rewritten: one `analyze()` setup walk instead of three, and a
  single reverse-postorder sweep instead of a worklist dataflow (9.2ms → 5.1ms).
- New opt-in pass accounting: `src/passtime.rs` (`RAKIT_PASS_TIME=1`) and
  `src/rakitprof.rs` (`RAKIT_IR_TIME=1`).
- **Fixed the pre-existing SIMD-lane ICE**: `value_lane`/`value_typed_lane`/
  `value_lane_dyn` assumed a `ByRef` value and hit `unreachable!()` for the
  register-resident `ByVal` vector introduced by `a9d182a`; they now spill via
  `force_stack`. `./y.sh test` → **3011 passed / 0 failed** (+596 core benches);
  the `jit/aot.std_example` ICE is gone.
- `REPORT.md` §8.13–8.15 and `bench/fullbench.sh` record the full three-way
  matrix (RC=7/RR=9).
- **Push still blocked**: `irhasar/rmc` rejects every available credential (403
  `Resource not accessible by integration`). Commit `7249d45` is local only; to
  land it, push `replace-main-with-rakit-experiments` with a `repo`-scoped PAT
  (updates PR #10 in place).

## Repo purpose
Experiments comparing Rust codegen backends: LLVM (`rustc`) vs
cg_clif upstream (`rustc_codegen_cranelift`) vs a modified Rakit fork.

## Layout
- `REPORT.md` — findings, numbers, root causes, recommendations.
- `build.sh` — rebuilds the modified Rakit backend + its Rakit std sysroot.
- `cg_clif_upstream/` — pristine cg_clif (commit 42ac63e), unmodified.
- `rakit/` — fork with the modifications (see commits
  `b6c94e4`…`9ea4bbe`).
- `bench/` — harness:
  - `allaxes.sh` — **canonical all-axes 3-way harness** (LLVM / cg_clif
    upstream / Rakit modified). Every run MUST report all of these, per backend
    and per program; a benchmark is not complete without all of them:
    - compile: **wall time**, **CPU time** (user+sys), **peak RSS**, **artifact
      size** (bytes)
    - run: **wall time**, **CPU time** (user+sys), **peak RSS**
    Do not report a subset. `RREPS`/`CREPS` control repetitions.
  - `compare3.sh` — 3-way benchmark across 5 sysroot configs; prints
    `C`=compile, `S`=size, `R`=run metrics (including peak RSS).
  - `sizes2.sh` — sizes across opt-levels and stripped.
  - `measure.py` — wall/CPU/peak-RSS via `os.wait4` rusage.
  - `progs/p1..p6*.rs` — workloads (p5 inline, p6 scalar are the decisive microbenchmarks).

## Benchmark reporting rules (MANDATORY)
When comparing backends, always report the **full matrix**, never a partial one.
For each backend (LLVM / cg_clif upstream / Rakit modified) and each program,
record and present:

**Compile**
1. compile wall time (ms)
2. compile CPU time = user+sys (ms)
3. compile peak RSS (MB)
4. artifact size (bytes), plus section sizes (`.text`, `.data.rel.ro`,
   `.rodata`, `.eh_frame`) when relevant

**Run**
5. run wall time (ms)
6. run CPU time = user+sys (ms)
7. run peak RSS (MB)

Correctness: every backend's stdout must match LLVM's (or match the expected
value) before its numbers count. A run with `exit != 0` or a compile failure is
reported as such, never silently dropped.

Use `bench/allaxes.sh` as the single source of truth so no axis is ever omitted.
Add new programs under `bench/progs/` so they are picked up automatically.

## Build environment
- Toolchain: `nightly-2026-11-17` (must match, rakit pins it).
- Need `rust-src` and `llvm-tools`. `$HOME/.cargo/bin` on PATH.
- Build: `build.sh` → `rakit/dist/bin/rustc-rakit`.
- Rebuilding Rakit std takes ~1–2 min incremental.
- **The toolchain is not always present.** Some sandbox restarts come back with
  no `rustc`, `cargo`, `rustup`, or `$HOME/.cargo` at all (`find / -name rustc`
  returns nothing), while the prebuilt `dist/` trees survive. Measurements that
  need a fresh `rustc` then cannot be reproduced; the prebuilt binaries under
  `rakit_{mod,upstream}/dist/bin/rustc-rakit` still run. Check this before
  planning a rebuild, and read numbers off `bench/out/` artifacts instead
  (`stat -c%s` per program sums to the same totals the report cites).
- Git remotes may prompt for credentials and hang for the full timeout. Push with
  an explicit token URL and `-c credential.helper=''` rather than letting the
  prompt block; then `git update-ref refs/remotes/origin/<branch> <sha>` to keep
  the local remote-tracking ref honest, since `git fetch` will not refresh it.
- API writes to PRs fail with `Resource not accessible by integration` under the
  injected `GITHUB_TOKEN`; a user-supplied PAT is required for `PATCH /pulls/N`.

## Key facts learned
- Rakit is **Apache-2.0 WITH LLVM-exception**; forking/reworking is fine.
- rakit compiles ~20–45% faster than LLVM but produces ~26% larger binaries
  and runs 2–9× slower.
- **Size root cause:** `rakit-object` named every function section
  `.text.subsection` and every data object `.data.rel.ro.subsection`; the linker
  merges same-named sections so `--gc-sections` could not drop dead code.
  Fix: name sections after the linkage symbol (`add_subsection(kind, decl_name)`)
  and set `builder.per_data_object_section(true)`. Result: −15.6% raw, −26.5% stripped.
- **Runtime root cause:** rakit does not use Rakit's `Inline` API (the
  trait docstring names rakit explicitly), and MIR locals stay in stack slots
  (`add (%rax),%rdx` / `mov %rdx,(%rsp)` per loop iteration) with no unrolling/ILP.
  `-Zinline-mir=yes` grows functions but does not speed up runtime on its own.
- With LLVM-built std, rakit's binary size matches LLVM, proving the size gap
  comes from Rakit-built std and the runtime gap from the backend itself.

## Modifications landed in `rakit` (beyond upstream)
- `b6c94e4` per-symbol sections (size −15.6% raw, −26.5% stripped).
- `7ddf2d4` **SROA**: a scalar local that is only *shared*-borrowed is kept in a
  register; a stack slot is materialized lazily at the first real address use
  (`CPlace::to_ptr` → `force_to_stack`). Fixes the decisive `println!("{}", acc)`
  regression (p5/p6 894 ms → 356 ms) without regressing p2_mem.
- `2ddb545` **RAKIT inliner** (`src/inliner.rs`): per-CGU, inlines small (`≤64`
  RAKIT insts) *final* callees with a 512-inst budget. Guardrails: no self-inline,
  each callee at most once per caller, and **skip callees that reference any
  `global_values`** — Rakit's `inline_one` does not remap them and the
  resulting object file fails to link (`R_X86_64_TLSGD out of range`).
  `RAKIT_INLINE_MAX=0` disables it (bisecting knob).
- `9ea4bbe` **RAKIT affine-reassociation loop pass** (`src/unroll.rs`): recognises a
  canonical counted loop, derives the affine state update `S' = M·S + c` from the
  body, composes it `factor` times in closed form, and rewrites the back edge so
  one header check advances the whole state by `factor` iterations at once, with
  a guard that falls back to a single step for the tail. Exact modulo 2^64
  (iadd/imul wrap). Turns p6 from 902 ms → 61 ms (LLVM 58 ms). A body-duplicating
  unroller is also present but **off by default** (`RAKIT_UNROLL_CLONE`): it is
  correct but slower, because duplicating a body does not shorten the carried
  chain. Knob: `RAKIT_UNROLL` (default 8).
- **NEW** `src/store_forward.rs` **RAKIT stack store→load forwarding + dead-store
  elimination**: forwards a `store` into a private (non-escaping) stack slot to
  every later `load` of the same address, aliases the load result to the stored
  value and removes the load; then deletes stores whose bytes no surviving load
  reads. This is the real lever for p5/p6 (365 ms → 63 ms). It must run **before**
  `unroll`, because the affine loop pass rejects any body that still touches
  memory — the store/load round-trip rakit emits for an `Option`/iterator
  otherwise blocks the unroll. Knob: `RAKIT_NO_STORE_FWD`.
  - Why it is needed: Rakit's egraph only value-numbers loads that are
    `readonly`, `notrap` and `can_move`. A store to an explicit stack slot is not
    `readonly`, so a `store`/`load` round-trip of the same slot survives into the
    machine code, and inside a loop it lands on the carried dependency chain.
  - Soundness: only slots whose address is *never* used as anything but the
    address operand of a load/store are touched (`private_slots`), so no call or
    indirect store can observe the slot. Volatile and atomic accesses are skipped
    (a volatile store has no `Load`/`Store` `InstructionData`, and `store` to a
    private slot cannot be volatile because the address never escapes).
  - Overlapping (partial) stores are not forwarded: only an exact
    `(slot, offset, type)` match is.
  - **Miscompile fixed** (`343ff3f`): `private_slots` scanned only
    `InstructionData::arguments()`, which excludes block-call (branch)
    arguments. rakit threads a stack address through a block parameter (a phi),
    e.g. `jump block9(v29)` with `v29 = stack_addr ss4` and a later `load v42`;
    that reader was invisible, so `drop_dead_stores` deleted the store the load
    depended on. The BTreeMap insert path in libstd then read uninitialised
    memory and segfaulted (`std::process::Command::spawn`, `BTreeMap::insert`).
    Fix: treat any stack address that reaches a branch argument as escaping.
    Repro: `std::process::Command::new("true").env("c","d").spawn()`, and a
    64-entry `BTreeMap<OsString, OsString>` insert loop. Both now exit 0 and
    match LLVM output.
  - **Cross-block forwarding** (`43a30ac`): the pass was block-local, so a store
    in one block and the load reading it back in a dominated successor were never
    connected — yet rakit materialises aggregates *across* control flow (a
    tuple assigned in both arms of an `if`/`else` and read after the merge). In
    p2_mem alone there are 348 such cross-block round-trips. Added a
    must-analysis dataflow (`cross_block_entry`): a store fact reaches a block
    only if *every* predecessor agrees on the exact same SSA value, which makes
    the value well defined at the merge and dominating. The entry block is
    seeded empty because it is also entered from outside the function, an edge
    the CFG does not model. Result: find_inner stores 31→24, loads 13→12; binary
    size −2% more. Verified by rebuilding the whole std sysroot with the pass on
    and re-checking every program output against LLVM.
- `38923c0` **SIMD 128-bit constants → `vconst`** (`src/constant.rs`). A 16-byte
  read-only, pointer-free, fully-initialised `ConstValue::Indirect`
  (`Simd::ZERO`, splat masks, `_mm_set1_*` results) used to become an anonymous
  global plus a `load`. Now it lowers to a single `vconst`. Two effects: removes
  a memory round-trip on the hot path, and — the important one — removes the
  `GlobalValueData::Symbol` that blocks the inliner (it does not remap
  `ExternalName`). `_mm_*` call sites in `p2_mem` drop 31→3; the residual 3 are
  cross-CGU (`hashbrown::find_inner`).
- `0e6c2f0` **128-bit SIMD copy via one vector load/store**
  (`src/value_and_place.rs`). Writing a by-ref 128-bit SIMD value into a memory
  place fell through to `emit_small_memory_copy`, splitting 16 bytes into two
  `i64` loads and two `i64` stores. rakit bounces every SIMD op through such a
  slot, so the bitcast `__m128i → Simd<i8, 16>` alone was 1 store + 2 loads + 2
  stores. Now a single 16-byte vector load+store. Wider vectors (`__m256i`
  without AVX) have no single store and keep the `memcpy`.
- `8ff4779` **cross-type same-size vector store→load forwarding**
  (`src/store_forward.rs`). The pass only matched a load to a store of the
  *exact same* RAKIT type. A SIMD bitcast (`__m128i ↔ Simd<i8, 16>`) compiles to
  a store as one lane type and a load back as another, so those round-trips
  survived to machine code. When byte sizes match and both types are vectors,
  the stored value is now forwarded through a single `bitcast`; the target
  endianness is threaded in from the ISA because Rakit's verifier requires
  explicit endianness when lane counts differ. `as_i8x16` becomes one
  `bitcast.i8x16`. p2_mem 110→99 ms; size 4.41 MB (LLVM 4.64 MB).
- `c2e292b` **inline callees that reference globals** (`vendor/rakit-codegen-0.137.0`,
  `src/inliner.rs`). Rakit's inliner copied a callee's `GlobalValueData`
  verbatim, including `GlobalValueData::Symbol`, whose `ExternalName` indexes the
  *callee's* `user_named_funcs`/`ext_funcs` tables. Those indices were never
  translated, so inlining a callee that referenced a const/static/thread-local
  corrupted the caller (out-of-range `R_X86_64_TLSGD`). rakit worked around it
  by refusing to inline any callee with a non-empty `global_values`, which
  blocked the hot small helpers (`core::hash::sip`, the branchless sort
  partition closure). Fix: vendor rakit-codegen and translate the symbol's
  user name while copying global values (`translate_symbol_name`), reusing the
  same caller-side mapping `create_user_external_name_refs` installs — done
  inline there because global-value copying runs first. Also drop the
  one-inline-per-callee limit (`MAX_CALLEE_USES`, `RAKIT_INLINE_USES`): the
  partition closure is called 3× and was left out of line; the call-graph budget
  still bounds recursion. Effect: `slice::sort_unstable` quicksort calls 11→3,
  srt 28.5→23.1 ms; p2_mem 74.5→59 ms. TLS/atomics/statics verified correct.

## What did NOT work / open leads
- **p2_mem runtime gap (fixed)**: the 8× gap (125 ms vs LLVM 16 ms) came from
  rakit emitting the `core::core_arch::x86::sse2` intrinsics as **out-of-line
  calls** — `p2_mem_M` had 8 `_mm_*` symbols and 23 `call`s to them, while LLVM
  had 0. Root cause was *not* the inliner: `src/intrinsics/simd.rs` scalarized
  every integer SIMD comparison per-lane (load i8 → `icmp` → `bmask` → store,
  16×) and `simd_bitmask` into a per-lane shift/or loop, so `_mm_cmpeq_epi8`
  produced ~155 RAKIT instructions and never fit the inliner budget. Fixed by
  emitting one vector `icmp` (`pcmpeq*`/`pcmpgt*`) and `vhigh_bits`
  (`pmovmskb`) on x86_64; see `simd.rs` and REPORT.md §8.8. `_mm_cmpeq_epi8`
  body: 155→34 insts, `_mm_movemask_epi8`: 255→23; p2_mem 159→110 ms.
  - What did **not** help, recorded for the record: `-Ctarget-feature=+sse2`,
    `-Ctarget-cpu=native`, `-Zinline-mir=yes`, `RAKIT_INLINE_MAX=4096`
    (the last only trims in-crate callers), and forcing `-Ccodegen-units=1`.
  - Residual p2/p4 gap now comes from cross-CGU inlining of `memchr`/`hashbrown`
    helpers and the absence of autovectorization in Rakit; both are outside
    the reach of rakit-side changes. A real cross-CGU inliner would be the
    next step.
  - Rakit's `Inline` API clones `GlobalValueData` verbatim and does **not**
    remap the `ExternalName` indices inside `GlobalValueData::Symbol`, so
    inlining a callee that references a const/static/thread-local corrupts the
    caller (TLS relocations went out of range when the guard was lifted).
    **FIXED** in the vendored `rakit-codegen` (see the entry below): the
    symbol's user name is now translated into the caller's namespace, so the
    `body.global_values.is_empty()` guard in `inliner.rs` is gone.
- Extending SROA to **`&mut`/`RawPtr` whole-local borrows** (with `local_map`
  redirect) is a **miscompile** (p1/p3/p4 wrong). Shared-only is correct.
- The inliner is redundant **within a crate** (MIR inlining already folds those
  helpers), but load-bearing **across crates**: the hot helpers in `core::hash`
  and `slice::sort` live in the pre-built std, so only a RAKIT-level inliner can
  reach them. Once the global-values guard was removed it turns `slice::sort`'s
  partition closure from an out-of-line call into inlined code (quicksort calls
  11→3). It remains net-neutral on the p1–p6 microbenchmarks that MIR already
  folds.
- **Body-duplicating unrolling does not help.** Measured slower on p6 (357→423 ms):
  the win LLVM gets on p5/p6 is *affine reassociation* of the accumulator chain,
  not unrolling per se. Duplicating the body leaves the loop-carried dependency
  chain intact. Implemented anyway behind `RAKIT_UNROLL_CLONE` for completeness.
- Remaining runtime gap on p1–p4 (LLVM 17–58 ms vs mod 99–140 ms) is **not**
  loops or store/load round-trips — the cross-block pass left runtime unchanged.
  For p2 it is the out-of-line `core` SSE intrinsics (see the root-cause entry
  above). p1/p3/p4 gaps are smaller and stem from the same missing cross-CGU
  inlining plus incomplete memory-to-register promotion.

## Gotchas
- `measure.py`/`compare3.sh`: pass the rustc command as separate argv elements;
  the `--sysroot` handling in `run_backend` requires the sysroot as its own arg.
- Regenerating `rakit/Cargo.lock` needs dropping `--locked` because of the
  local `[patch.crates-io]` for `rakit-object`.
- `--gc-sections` works in this environment (verified with a hand-written asm test).
- **RAKIT rewriting:** `InstructionData` is `Copy` and its `BlockCall`s share the
  value pool with the source instruction. Before remapping a cloned instruction
  you must `deep_clone` its branch destinations, or the clone's branch args
  silently corrupt the original (this ICEs later in the egraph pass with
  `assertion failed: payload < (1 << 30)`).
- **Value aliases:** rakit emits `v6 -> v2` aliases heavily. Resolve with
  `dfg.resolve_aliases` before mapping header params to clone params, or the
  clone reads the wrong state.
- `func.layout` has no `num_blocks`; use `func.layout.blocks().count()`.



## Push status (T6) — DONE
- PR **#10** (`replace-main-with-rakit-experiments` → `main`) is open,
  **mergeable=True (clean)**. Local `rmc_clone` == remote.
- Latest fork commit on `rakit`: `8ff4779`
  `Forward same-size vector stores to loads of a different lane type`
  (on top of `0e6c2f0` SIMD copy via one vector load/store and `38923c0`
  SIMD constants → `vconst`). Squashed into one commit `4759d90` in
  `rmc_clone` and pushed; PR #10 head is `4759d90`, 10 commits.
- Earlier commits: `e101572` cross-block store forwarding + doc/bench-path sync;
  `628c73b` store-forward miscompile fix (branch args were not scanned in
  `private_slots`). Pushed with a user-supplied classic PAT with `repo` scope:
  `git push https://<user>:<PAT>@github.com/irhasar/rmc.git replace-main-with-rakit-experiments`.
- Note: the environment's injected `GITHUB_TOKEN` (GitHub App installation) is
  read-only for this repo — `git push` and all write APIs return 403
  (`Resource not accessible by integration`). `OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB`
  is also rejected (`Invalid username or token`), and there is no SSH key.
  Re-confirmed 2026-11-19: `POST /repos/irhasar/rmc/git/blobs` returns 403 while
  `GET /repos/irhasar/rmc` reports `push: true` — the repo permission and the
  token's granted scope disagree, so the 403 is a token-scope limit, not a
  repo-permission one. A user-supplied classic PAT with `repo` scope does work;
  pass it inline and reset the remote afterwards so it is not persisted:
  `git -c credential.helper= push https://<user>:<PAT>@github.com/irhasar/rmc.git
  HEAD:replace-main-with-rakit-experiments`, then verify `grep -c ghp_ .git/config`
  is 0.
- 2026-11-19 large-crate work is **landed**: remote head `960ae7e`, PR #10 at 46
  commits / 926 files, `mergeable: True`, `mergeable_state: clean`. Commits:
  `cd14e20` (treemeasure.py + cratebench.sh + REPORT 8.26), `df39c58` (push
  status note), `960ae7e` (three derived figures corrected).

## Session benchmark report (2026-11-18, latest)

Full three-way harness: `cd bench && REPS_COMPILE=3 REPS_RUN=5 ./compare3.sh`
(log `/tmp/compare3_final.log`) and `./sizes2.sh` (log `/tmp/sizes2_final.log`).
C = compile (wall ms, cpu ms, peak RSS MB, exit), S = size (raw bytes,
.text, .data.rel.ro, .rodata, .eh_frame, `size -A` total), R = run
(wall ms, cpu ms, peak RSS MB, exit). "up_llvmstd"/"mod_llvmstd" link the
stock LLVM std to isolate the backend from the Rakit-built std.

### Compile
| prog | backend | wall ms | cpu ms | RSS MB |
|------|---------|--------:|-------:|-------:|
| p1_arith | llvm | 89 | 108 | 106 |
| p1_arith | up | 80 | 102 | 108 |
| p1_arith | mod | 81 | 105 | 109 |
| p2_mem | llvm | 253 | 383 | 133 |
| p2_mem | up | 129 | 175 | 131 |
| p2_mem | mod | 138 | 192 | 132 |
| p3_calls | llvm | 103 | 126 | 116 |
| p3_calls | up | 90 | 111 | 117 |
| p3_calls | mod | 89 | 115 | 118 |
| p4_simd | llvm | 111 | 136 | 116 |
| p4_simd | up | 95 | 117 | 117 |
| p4_simd | mod | 98 | 123 | 118 |
| p5_inline | llvm | 85 | 106 | 105 |
| p5_inline | up | 78 | 102 | 107 |
| p5_inline | mod | 80 | 104 | 108 |
| p6_scalar | llvm | 85 | 103 | 101 |
| p6_scalar | up | 78 | 99 | 103 |
| p6_scalar | mod | 79 | 101 | 104 |

rakit (up and mod) compiles ~1.5–2x faster than LLVM on p2 and slightly
faster elsewhere, at equal peak RSS. The mod's extra RAKIT passes cost ~1 ms
wall / ~3 ms cpu and are size/run-neutral at compile time.

### Artifact size (raw bytes; section breakdown in the log)
| prog | llvm | up | mod |
|------|-----:|---:|----:|
| p1_arith | 4,635,232 | 5,878,688 | 4,376,376 |
| p2_mem | 4,639,888 | 5,936,880 | 4,404,664 |
| p3_calls | 4,612,368 | 5,820,872 | 4,317,616 |
| p4_simd | 4,636,096 | 5,893,112 | 4,384,824 |
| p5_inline | 4,609,368 | 5,814,440 | 4,309,064 |
| p6_scalar | 4,609,368 | 5,814,424 | 4,309,064 |

mod vs up: **−25% raw**, and −40% in `.text` (up 1.19 MB → mod 0.81 MB) with
`.rodata`/`.data.rel.ro` also ~4x smaller. mod vs LLVM raw: −5%, and `.text`
is 3x larger than LLVM's (0.81 MB vs 0.26 MB) — LLVM's `.text` is small
because much of its runtime lives in the shared LLVM-built std/panic
libraries, whereas the mod links its own Rakit-built std. With the LLVM
std (isolates the backend), mod is byte-for-byte the same size as up plus
only the SIMD lowering: p1 4,631,032 vs 4,631,880, p5/p6 4,603,608 vs
4,604,376 — i.e. the size win is in the Rakit-built std (the mod's
`--gc-sections` + store-forwarding let dead code go).

Stripped (p2_mem/p3_calls, opt-level 3 / z / cu1): up 1.92/1.85 MB →
mod 1.14/1.08 MB; llvm 0.36/0.34 MB. mod strips 40% smaller than up.

### Runtime
| prog | llvm ms | up ms | mod ms | mod/llvm |
|------|--------:|------:|-------:|---------:|
| p1_arith | 81 | 109 | 106 | 1.31x |
| p2_mem | 16 | 159 | 75 | 4.7x |
| p3_calls | 37 | 118 | 99 | 2.7x |
| p4_simd | 56 | 156 | 139 | 2.5x |
| p5_inline | 58 | 893 | 61 | 1.05x |
| p6_scalar | 58 | 900 | 61 | 1.05x |

mod vs up: **p2 159→75 ms (2.1x, the small-memcpy fix)**, p5 893→61 ms
(14.6x), p6 900→61 ms (14.8x). p5/p6 now match LLVM. Remaining gap is p2
(SipHash cross-crate inlining), p3 (closure calls), p4 (SSE intrinsics),
p1 (bounds/arith) — all cross-crate/cross-CGU inlining limits, not loops or
store/load round-trips.

### New work this session (on top of PR #10 head 4759d90)
- `badbf0e` Inline small constant-size memcpy/memmove instead of libc calls:
  `ptr::copy_nonoverlapping`/`ptr::copy` with a constant byte count ≤64
  (`count * elem_size` folded) become load/store pairs. `slice::sort_unstable`
  partition (8-byte element moves) went from a `%Memcpy`/`%Memmove` call per
  element to register moves: sort 44→24 ms on 300k u64 (LLVM 4.3 ms). Verified
  with a 61×5 fuzz over sizes 0..60, alignments, and both overlap directions
  (`bad=0`, identical to LLVM).
- `bc9b0f0` Honour `RAKIT_INLINE_MAX` when collecting inlineable bodies:
  `aot.rs` filtered bodies on the compile-time `MAX_CALLEE_INSTS` while the
  decision used `max_callee_insts()`, so the knob did nothing. Now both agree.
  Root-caused the p2 hash gap: `core::hash::sip::SipHasher::write`/`c_rounds`
  are cross-crate and round-trip the 32-byte `State` through the opaque
  `&mut self` pointer; even inlined they stay memory-bound (opaque-pointer
  aliasing), so raising the inline budget cannot close it. This is a genuine
  cross-crate inlining limit, not a tunable.

## Push status update (2026-11-18) — PUSHED
- PR **#10** open, `mergeable=True`, `mergeable_state=clean`, base `main`,
  remote head **`aee9ba3`**, 13 commits, 625 changed files.
- Pushed with a user-supplied `repo`-scoped classic PAT (the injected
  `GITHUB_TOKEN` is read-only for this repo, 403). New commits on the branch:
  `467f05c` "Inline small constant memcpy/memmove and honour RAKIT_INLINE_MAX"
  and `aee9ba3` "docs: record push status for the latest session".
- The branch replaces the whole `main` tree (subtree `rakit/` at repo
  root alongside `cg_clif_upstream/`, `bench/`, `REPORT.md`, `AGENTS.md`).

## Session benchmark report (2026-11-18, inliner-globals fix)

`cd bench && CREPS=3 RREPS=5 ./allaxes.sh` (log `/tmp/allaxes_final.log`).
L = stock LLVM nightly, U = cg_clif upstream, M = modified. Compile = wall ms,
run = wall ms, size = bytes.

| prog | Cwall L/U/M | Rwall L/U/M | size L/U/M |
|------|-------------|-------------|------------|
| p1_arith  |  90 /  81 /  83 |  82 / 108 / 105 | 4635232 / 5878688 / 4328824 |
| p2_mem    | 244 / 130 / 144 |  16 / 154 /  59 | 4639888 / 5936880 / 4352336 |
| p3_calls  | 103 /  88 /  90 |  37 / 116 /  97 | 4612368 / 5820872 / 4270352 |
| p4_simd   | 111 /  95 / 109 |  55 / 158 / 143 | 4636096 / 5893112 / 4336352 |
| p5_inline |  86 /  81 /  80 |  58 / 893 /  61 | 4609368 / 5814440 / 4261624 |
| p6_scalar |  86 /  80 /  80 |  58 / 900 /  61 | 4609368 / 5814424 / 4261624 |

M vs U: p2 154→59 ms, p5 893→61 ms, p6 900→61 ms; p1/p3/p4 unchanged
(they do not call global-referencing helpers). M compiles ~1.7–2x faster than
LLVM on p2 and is ~7% smaller than LLVM raw; peak RSS equal on all three.

### What limits the hash workloads (goal: parity with LLVM)
Micro-benchmarks (M vs L, run wall): `slice::sort_unstable` 23.1 / 8.2 ms,
`HashMap` insert 35.4 / 10.0 ms, `DefaultHasher` (SipHash) 136.6 / 20.2 ms.
The inliner-globals fix shrank `quicksort` from 11 out-of-line calls to 3, but
the residual 3–7x is **Rakit codegen quality, not inlining**:
- Forcing `RAKIT_INLINE_MAX=200` to inline the SipHash rounds *slows* the
  hash benchmark (136.6→154 ms): after inlining, the `SipState` round-trip goes
  through the opaque `&mut self` pointer and Rakit spills/reloads it every
  round, while LLVM keeps it in registers. rakit's `store_forward` only
  forwards through *non-escaping* pointer bases, and the `Hasher::write`
  parameter escapes into `c_rounds`/`d_rounds`, so the round-trip survives.
- LLVM's advantage on these loops is its register allocator plus
  autovectorization (the SipHash tail, `u8to64_le`), which Rakit does not
  do. These are backend-internal; a rakit-side pass cannot reach them.

### Push status (2026-11-18, second update) — PUSHED
- Pushed to PR **#10** (`replace-main-with-rakit-experiments` → `main`), head
  now **`8f84035`**, 16 commits, `mergeable=True`, `mergeable_state=clean`.
  Commits this session: `64b3760` "Inline callees that reference globals; allow
  repeated inlining" (vendors `rakit/vendor/rakit-codegen-0.137.0` +
  the `translate_symbol_name` fix) and `8f84035` (docs).
- The environment's injected `GITHUB_TOKEN` is read-only for this repo (all
  writes 403 `Resource not accessible by integration`); a user-supplied
  `repo`-scoped classic PAT was used to push. (History note below kept for the
  blocked attempt.)
- Historical note — BLOCKED ON CREDENTIALS
- `rakit` head is `c2e292b` "Inline callees that reference globals; allow
  repeated inlining"; it vendors `vendor/rakit-codegen-0.137.0` (added to
  `[patch.crates-io]`) with the `translate_symbol_name` fix.
- `rmc_clone` (subtree layout) is committed as **`64b3760`** on branch
  `replace-main-with-rakit-experiments`, which is PR **#10**'s head branch
  (`102ec7b` on remote). The commit is **local only** — it is not pushed.
- **Push is blocked**: every token in this environment is read-only for
  `irhasar/rmc`. `GITHUB_TOKEN` authenticates as user `irhasar` and the repo API
  reports `permissions.push=true`, but all writes return 403
  (`Resource not accessible by integration`): `git push` (helper token),
  `git push` with `$GITHUB_TOKEN` embedded, `gh auth token`, and
  `POST /git/refs` all fail. `OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB` is
  `401 Bad credentials`. The earlier session pushed with a user-supplied
  `repo`-scoped classic PAT that is no longer available.
- To finish: supply a `repo`-scoped PAT (or push manually):
  `git -C rmc_clone push https://<user>:<PAT>@github.com/irhasar/rmc.git replace-main-with-rakit-experiments`
  This updates PR #10 in place; no new PR is needed.
- **Push is no longer blocked**: it landed with a user-supplied `repo`-scoped
  classic PAT. The earlier failures were *not* only a read-only token: passing
  `-c credential.helper=` does **not** clear the global
  `credential.https://github.com.helper` in `~/.gitconfig`, so the read-only
  helper still answered first and every attempt returned 403. Working recipe
  (token from env, never in the remote URL, so `.git/config` stays clean):

      cd rmc_clone && GIT_TERMINAL_PROMPT=0 git \
        -c credential.helper= \
        -c credential.helper='!f() { echo username=<user>; echo "password=$GH_PAT"; }; f' \
        push origin replace-main-with-rakit-experiments

- **Pushed**: `c64e31b` is on the remote; PR #10 is `mergeable_state: clean`.
  Confirmed via the contents API that `rakit/src/intrinsics/mod.rs` on
  the remote contains the native `round` lowering.

## Session 2026-11-19 (later) — the trunc libcall finding

**Biggest single win so far: −15.8% runtime on the image blur benchmark, and
fully portable (no `-Ctarget-cpu=native` needed).**

### The finding

`ftrunc` only lowers to `roundss`/`roundsd` when the target has **SSE4.1**
(`vendor/rakit-codegen-0.137.0/src/isa/x64/lower.isle`, the `x64_round_*`
rules). On the x86-64 baseline it falls through to a libcall into
`compiler_builtins`' soft-float `truncf`. `round` in rakit was built on
`ins().trunc(x)`, so it inherited the same libcall.

### Methodology lessons (both cost real time)

1. **A first patch that changes nothing is a signal, not a dead end.** Patching
   only `sym::trunc` gave 10091 ms vs 10053 ms — no change. The hot caller was
   `round`, which called `ins().trunc(x)` internally. When a plausible fix
   measures as noise, grep for *every* emission site of the instruction, not
   just the intrinsic you assumed.
2. **Never validate a lowering against itself.** The first correctness test
   compared `v.trunc()` with `f64::trunc(v)` — both compiled by the lowering
   under test. It "passed" while the code was wrong. Only a **cross-backend
   differential** test is valid: hash the results in a RAKIT-built binary and in
   an LLVM-built binary, then compare the hashes.
3. **A 2^32-iteration test must stream.** Buffering one line per f32 pattern
   into a `String` OOM-killed the sandbox and wiped `~/.cargo` + `~/.rustup`.
   Fold results into a running hash instead. Recovery: reinstall rustup, then
   `rustup toolchain install nightly-2026-11-17 --profile minimal -c
   rust-src,rustc-dev,llvm-tools,rustfmt` (per `rust-toolchain.toml`).

### The fix

`trunc_via_int` in `rakit/src/intrinsics/mod.rs`: `fcvt_to_sint_sat` →
`fcvt_from_sint` → `fcopysign` (to keep the sign of zero), guarded by
`|x| < 2^mantissa_bits` with `select`. SSE2 only, never traps (the saturating
conversion cannot).

Correctness argument: any float at or above 2^mantissa_bits is already
integral, so the round-trip is exact exactly where it is applied; below that
bound the value is small enough to convert without overflow. NaN/infinity fail
the magnitude test and pass through, which is what truncation returns.

**The sign-of-zero bug is easy to miss**: the integer round-trip returns `+0.0`
for `trunc(-0.4)`, which must be `-0.0`. The bundled test suite catches it
(`num::floats::{trunc,round,fract}`), so always run the full suite — 6 tests
failed on this alone.

### Results

| backend | wall (ms) | size (bytes) |
|---|---|---|
| LLVM | 1237.7 | — |
| RAKIT baseline | 10257.5 | 1500000 |
| RAKIT + small-switch | 10002.5 | 1500000 |
| RAKIT + small-switch + trunc | **8422.0** | **1499568** |
| RAKIT + `-Ctarget-cpu=native` | 8241.8 | 1498832 |

`compiler_builtins` went from ~13% of profile samples to **zero**.

Verified: differential hash identical to LLVM (`4de7d090cdfce77d`) over all
2^32 f32 patterns plus an f64 sweep; `./y.sh test --sysroot rakit` → 65 suites,
0 failures; benchmark output byte-identical at n = 512/1024/2048.

### Remaining gap

8422 ms vs 1238 ms LLVM. Now dominated by codegen quality: `try_from_fn`,
`wrap_mut_1`, `Pixels`, `Zip` never inlined (~18% of samples) and register
spilling in the hot loop. §9.9's claim that this was "out of scope" is wrong —
`rakit-codegen` is vendored and patchable.

### Push status — PUSHED

Commit `5e92648` is on the remote; PR #10 head is `5e92648` (54 commits),
`mergeable: True`, `mergeable_state: clean`, base `main`.

The environment token situation is worth remembering: `GH_PAT` is **not**
persisted to disk and dies with a sandbox restart, and `GITHUB_TOKEN` is a
GitHub App token with **read-only contents** (`PUT /contents` → 403,
`git push` → 403). With no SSH key, a fresh push needs a PAT supplied by the
user. Use the credential-helper form (never the remote URL) so `.git/config`
stays clean:

    git -c credential.helper= \
      -c "credential.https://github.com.helper=!f(){ echo username=<user>; echo password=<pat>; };f" \
      push origin <branch>

Verified after push: `trunc_via_int` and the `fcopysign` sign fix are present in
`rakit/src/intrinsics/mod.rs` on the remote branch.

### Follow-up: the remaining gap is auto-vectorization, not inlining

Re-profiled the final binary and then compared disassembly of the hot function
`filter_symmetric_column` across both backends (same baseline x86-64 target):

| metric | LLVM | RAKIT (mod) |
|---|---|---|
| total instructions | 904 | 733 |
| packed SIMD (`ps`/`pd`) | **75** | **1** |
| breakdown | `movups`×34, `mulps`×17, `addps`×10, `shufps`×6, `xorps`×4, `movaps`×4 | `xorps`×1 |

LLVM vectorizes the convolution 4 `f32` lanes at a time; Rakit emits fully
scalar code. **Rakit has no loop vectorizer** — only local RAKIT-level
patterns (`simd_*`, i.e. hand-written `std::simd`), not automatic loop
analysis. Closing this needs a real loop vectorizer in `rakit-codegen`
(dependence analysis, widening, tail handling), which is months of work and
out of scope. `-Ctarget-cpu=native` only buys ~2% (8241.8 vs 8422.0 ms), which
confirms SSE2 was always available — nothing uses it without a vectorizer.

**Inlining is empirically saturated** — don't re-open that lead. Raising the
existing tunables makes things worse:

| config | runtime (ms) |
|---|---|
| default | 8440 |
| `RAKIT_INLINE_MAX=200` | 8533 |
| `RAKIT_INLINE_LEAF_MAX=200` | 8451 |
| `RAKIT_INLINE_USES=10` | 8564 |

So the gap is now fully explained: §9 (small switch) + §10 (trunc libcall) are
landed wins; the rest is architectural. Documented as §11 in REPORT.md.

### Session: floor/ceil were the same libcall class as trunc

`floor`, `ceil`, `nearest` and `trunc` **all** reach the same `x64_round` rules
in `vendor/rakit-codegen-*/src/isa/x64/lower.isle`, which select a libcall
when `has_sse41` is false. Fixing only `trunc`/`round` in the previous session
left `floor` and `ceil` as soft-float calls. Measured on a compute-bound loop
they were **6.3x / 6.6x** slower than LLVM; both now match.

Two measurement mistakes cost real time here — avoid both:

1. **`RUSTC=rustc-rakit rustc ...` does nothing.** `RUSTC` is read by *cargo*,
   not by `rustc`. Both "backends" were LLVM and the binaries were
   byte-identical (`md5sum` proves it). Always invoke `rustc-rakit` directly, and
   sanity-check that the two binaries differ before trusting any timing.
2. **A 16 MB array is memory-bound**, not compute-bound, which hides rounding
   cost entirely. Keep the working set in L1 (16 KB) with many repetitions.

Gating matters: the integer round-trip is ~3 instructions vs 1 `roundss`, so
applying it unconditionally regressed `-Ctarget-cpu=native` by 4%. `FunctionCx`
now caches `has_sse41` (read once per function — `isa_flags()` allocates a
`Vec`, so never call it per call site).

**`nearest` must stay as-is.** Its baseline libcall masks the mantissa rather
than doing soft-float arithmetic, so it is already fast; an integer round-trip
measured **15% slower**. "Libcall" does not imply "slow" — measure per case.
Use interleaved A/B with many samples; single runs on this box vary ~15%.

Verification pattern that works: print only a hash so memory stays constant,
compile with both backends, and require identical hashes over all 2^32 f32
patterns. Also confirm in the disassembly (`cvttss2si` vs `roundss`) that the
intended code was actually emitted — the symbol table alone cannot tell you.

## LICM in rakit-codegen

`vendor/rakit-codegen-0.137.0/src/licm.rs` is the only loop pass in the
vendored crate. It runs from `Context::compile` **after** the egraph pass, and
rebuilds `cfg`/`domtree`/`loops` from scratch each round because creating a
preheader mutates the CFG. Do not "optimise" that into incremental updates: the
earlier attempt reused a stale dominator tree and surfaced as a regalloc2 ICE
(`ion/moves.rs:658 unwrap None`), not as a wrong answer.

The three ways this pass was silently wrong, in the order they were found:

1. Splitting an edge discarded the header's phi arguments — forward the edge's
   block args into the new block.
2. `BlockArg` is not always a plain `Value`; `TryCallRet`/`TryCallExn` carry
   exception payloads. Restrict splitting to plain values and give up on EH
   edges rather than guessing.
3. `is_pure_for_egraph` accepts **trapping** ops (`sdiv`, `fcvt_to_sint`).
   Purity is not the same as safety to speculate, so LICM also requires
   `!op.can_trap()`.

Header block parameters are phi nodes and must never be treated as invariant;
`bench/tests/t_licm.rs` pins that and the other shapes.

Two knobs exist for bisecting: `RAKIT_NO_LICM` and `RAKIT_NO_COLLAPSE`
(the unroller's forwarding-latch collapse, which only unblocks one extra loop
and is runtime-neutral).

Reading env vars inside a per-function pass is measurable: `std::env::var`
locks a global map on every call. Cache every knob in a `OnceLock`, as
`src/tunables.rs` does. The vendored crate cannot see that module, so `licm.rs`
keeps its own cached copies.

Measurement notes: `/usr/bin/time` does not exist in this image — use
`bench/measure.py`. Running `./y.sh test` rebuilds `dist/`, so any binary
compiled from `dist/bin/rustc-rakit` disappears mid-run; never benchmark while a
test suite is in flight.

### Never trust a stale `dist/`

`rustc-rakit` falls back to the LLVM sysroot when its own sysroot has no
`libstd`, and it does so **silently**. The only symptom is that the "Rakit"
binary is ~4.6 MB and behaves exactly like LLVM. This invalidated a whole
benchmark matrix once (upstream looked 12% smaller than it is).

Before trusting any U-vs-L row, check all three:

```bash
ls cg_clif_upstream/dist/lib/rustlib/*/lib/libstd-*.rlib   # must exist
ls rakit/dist/lib/rustlib/*/lib/libstd-*.rlib        # must exist
stat -c%s /tmp/fb/p1_arith_U /tmp/fb/p1_arith_L             # must differ
```

`fullbench.sh` passes no `--sysroot`, which is correct — each `rustc-rakit`
must use its own. Do not add one; pointing `--sysroot` at
`dist/lib/rustlib/<triple>` produces `E0463: can't find crate for std`.
Rebuild with `./y.sh build --sysroot rakit` in both trees before measuring.

## Session 2026-11-17 — divisibility folding pass

**New pass: `rakit/src/div_test.rs`** rewrites `x % C == 0` / `!= 0` /
`brif` on a remainder into a multiply-compare, dropping the `urem` entirely.
Commit `8aa1dfb` in `rmc_clone` (subtree layout, branch
`replace-main-with-rakit-experiments` = PR #10's head).

Identity for odd `C`: `x % C == 0 <=> (x * C^-1 mod 2^64) <= (2^64-1)/C`.
Even `C` peels its power of two as a mask first. `srem` is left alone
(truncating division rounds toward zero, so negative `x` breaks the identity).

Two traps worth remembering:

1. **Two use-site shapes need different rewrites.** `brif rem, ..` uses the
   remainder *as* the condition — swap `blocks` targets and feed the new test
   in. `icmp eq rem, 0` produces a *separate value* that other code consumes —
   redirect that result with `detach_inst_results` + `change_to_alias`, never
   the comparison's operands. Getting this wrong is silent and type-checks:
   the icmp ends up testing its own boolean result, so `filter(|x| x % 3 == 0)`
   keeps the complement and `t_iter` prints exactly 2x the right number.
   Eight benchmarks all passed while this bug was live — `t_iter` is the
   regression test that catches it.
2. **`RAKIT_*` tunables are read at compile time, not run time.**
   `RAKIT_NO_DIVTEST=1 ./binary` disables nothing; it must prefix the
   `rustc-rakit` invocation.

Result: `p1_arith` 104.8 -> 96.7 ms (LLVM 82.2), `mulq` gone from the hot loop,
isolated kernel 108.7 -> 55.9 ms vs LLVM's 61.3 ms (faster than LLVM because
`cmovbe` beats `setb`+`xor`). Remaining `p1_arith` gap is the branchy saturating
`f64->u64` conversion in `rakit-codegen`, not the pass.

Also gated `store_forward`'s timing counters behind `RAKIT_SF_TIME`; they
called `Instant::now()` twice per block unconditionally.

**Loop folding can look impossible.** `p5_inline` (300M iterations) runs in
6.8 ms = 48 G iter/s. Not a miscompile: unroll-128 plus strength reduction
collapses it to a closed form, and the value matches Python. Passing `n` as a
runtime argument restores linear scaling (364 ms for 300M), which proves the
folding is compile-time.

**PUSHED.** The injected `GITHUB_TOKEN` is read-only for this repo (403 on both
`git push` and `PATCH /pulls/10`), but a user-supplied `repo`-scoped classic PAT
worked. Recipe that keeps the token off disk and out of `.git/config` — write it
to a 600 file, read it in the helper, delete it after:

    printf '%s' "$PAT" > /tmp/.ghpat && chmod 600 /tmp/.ghpat
    git -c credential.helper= \
        -c credential.helper='!f(){ echo username=<user>; echo "password=$(cat /tmp/.ghpat)"; }; f' \
        push origin replace-main-with-rakit-experiments
    rm -f /tmp/.ghpat

Remote head is now `5e7730e`; PR #10 is `mergeable=True`, `mergeable_state=clean`,
77 commits, 889 files, and its description was updated to the final numbers.

## Negative result: branchless saturating `f64->u64` is not worth it

The "remaining `p1_arith` gap is the branchy saturating conversion" note above
was tested and is wrong. A branchless sequence was implemented in
`rakit-codegen`'s `Inst::CvtFloatToUintSeq` emit arm (two `cvttsd2si`, one
`subsd`, `sar`/`and`/`or` for the `+2^63`, `cmovs` for `>=2^64`, `ucomisd`+
`cmovb` for `<0`/NaN), verified bit-exact against LLVM on 81188 inputs, and
benchmarked A/B against the identical tree with the path disabled:

* `p1_arith` 98 -> 109 ms (**+10%, a regression**), `cast3` +3%, everything else
  within noise, `pure` conversion loop a wash.
* LLVM's loop wins because it keeps both float constants in registers outside
  the loop. In `rakit-codegen` the constants are materialized in `emit`,
  which runs after LICM, so they are reloaded every iteration and the extra
  `movabs`+`movq` outweigh the saved branch. The branchy version is also
  predicted almost perfectly on the monotonic values real code produces.

Lesson: a "predictable branch" in a hot loop is cheaper than six extra
instructions. Before attacking one, check that the constants the replacement
needs are already loop-invariant at the level where you are generating code.
Do not re-attempt this without first making the constants RAKIT values so LICM
can hoist them; that is the prerequisite, not the sequence itself.

## Three more negative results, and the real shape of the remaining gap

Chased the two worst runtime gaps (p8_minmax 2.2x, p2_mem 3.2x vs LLVM). All
three candidate fixes measured, all rejected:

1. **Block-order / RPO successor swap** (`dominator_tree.rs`, `ForwardGraph::
   successors`). Hypothesis: RPO emits `header, exit, body`, so each iteration
   takes two branches (`jae` out + `jmp` back). Swapping the two-way successor
   order compiles and produces *different bytes*, but the loop shape is
   unchanged: Rakit lowers `brif` + back edge to test-at-top plus an
   unconditional back jump regardless of which arm is laid out first, and the
   layout code picks fall-through by its own rules. Runtime unchanged on all 8
   programs (only p4_simd moved 96->87 ms, which did not reproduce as a
   structural change; the jump count was identical). Reverted.

2. **Inliner budget/caps** (`RAKIT_INLINE_GROWTH`, `INLINE_MAX`,
   `INLINE_LEAF_MAX`, `INLINE_USES`). Instrumented `CguInliner::inline` and
   found 206 rejects and 170 not-in-map on the p2 workload; the dominant reason
   is the *growth budget*, not the size cap (`REJECT size=116 cap=128
   budget=66`). But raising the budget 150 -> 1000 with every cap lifted only
   moves p2big 527 -> ~490 ms (~5%) and leaves 37 `call`s inside
   `partition_lomuto_branchless_cyclic`. The closures that matter are reached
   through the sort machinery's generic layers, so the budget is not the binding
   constraint at the sites that are hot. Not worth the compile-time cost
   (146 -> 165 ms). Reverted.

3. **`RAKIT_UNROLL_CLONE`** on p8: 38 -> 46 ms. Actively worse.

### Where the gap actually comes from

`p2_mem` is dominated by `core::slice::sort` and `hashbrown`, not by the loop
bodies the passes above touch. Profiling (tools/sampler.c + sym.py) shows LLVM
folding `find_inner`, `Hasher::write`, `swap_if_less` into their callers while
rakit keeps them out of line: the LLVM binary has 601 local symbols on the
same program, the rakit one has 1680. In rakit's `partition_lomuto_
branchless_cyclic` the per-element closure is a real `call`, and the iterator
state is spilled to `0x30(%rsp)` / `0x38(%rsp)` and reloaded each iteration.

Closing that needs cross-crate inlining (the bodies are not in the same CGU, and
`-C codegen-units=1` does not change it: 533 vs 531 ms, same 41 calls), or a
MIR-level inliner in rakit itself. Neither is a `rakit-codegen` change, so
no amount of ISA work fixes p2_mem.

Lesson: before touching codegen, profile. Both gaps that looked like codegen
(block layout, saturation branch) were in fact missing inlining and missing loop
invariance, i.e. MIR/RAKIT-level pass gaps.

## Session 2026-11-17 (later) — the frame pointer trap, and what p3 really measures

Toolchain was reinstalled from scratch (rustup, `nightly-2026-11-17`, components
`rust-src,rustc-dev,llvm-tools,rustfmt`). The prebuilt `dist/` still worked
against the fresh toolchain, so all measurements below were done before any
rebuild.

### `opt_level` is inert on x86_64

`RAKIT_OPT_LEVEL=speed` produces a **byte-identical** binary to the default
`speed_and_size` on all nine programs (same sizes to the byte, run time within
noise). So the `speed_and_size`-vs-`speed` distinction that the comment in
`build_isa` worries about does not actually reach the x64 backend. Do not spend
time on it again.

### `-Z min-function-alignment=16` is a regression

Totals over p1/p2/p3/p7/p8: 340.2 ms at the default vs 353.3 ms at 16-byte
alignment, and the binaries get slightly larger (20429784 → 20430776). Padding
costs I-cache and buys nothing here. Rejected.

### `p3_calls` is a tail-call problem, not an inlining problem

Profiling p3 (60 Hz sampler) shows `fib` at **84% of samples** in the rakit
build. Time spent inside `fib` itself is 80 ms vs 28 ms for LLVM — 2.85x on a
function with no callees to inline into it. The disassembly explains it:

- LLVM rewrites `fib` into a **loop** with a single recursive call per
  iteration (tail-recursion elimination with an accumulator), 13 instructions
  of body.
- rakit emits the naive shape: **two** recursive calls per activation, plus
  `push %rbp; mov %rsp,%rbp; sub $0x10,%rsp`, two spill/reload pairs of
  `r12`/`r13` around the calls, and the matching epilogue.

The single biggest lever here is turning the second recursive call into a loop
iteration. That is a MIR-level transformation (LLVM does it in the middle end),
not something the Rakit backend can do at the RAKIT level, and it is
therefore out of scope for a backend-only patch.

### The frame pointer is a trap: Rakit x64 ignores `preserve_frame_pointers`

This cost the most time this session, so record it precisely.

`rakit` computes `preserve_frame_pointer` from
`sess.target.options.frame_pointer.ratchet(sess.opts.cg.force_frame_pointers)`.
For the default target spec `frame-pointer` is **None**, i.e. `MayOmit`, so
rakit sets `preserve_frame_pointers=false`. Nevertheless every function gets
`push %rbp; mov %rsp,%rbp`, including leaf functions.

Cause: in `rakit-codegen-0.137.0`,
`Callee::gen_prologue` (`machinst/abi.rs:2288`) calls
`M::gen_prologue_frame_setup` **unconditionally**, and the x64 implementation
(`isa/x64/abi.rs:538`) emits `push %rbp; mov %rsp,%rbp` with no check of the
flag at all. Likewise `compute_frame_layout` hardcodes
`setup_area_size = 16; // RBP, return address`. The flag is consulted only for
unwind info and for the aarch64/riscv/s390x backends. So on x64 the flag is
dead code.

Two patches were tried and **both must be reverted if re-attempted**:

1. Guard the prologue on the flag only. Result: `rbp` disappears from the
   prologue but the epilogue still does `mov %rbp,%rsp; pop %rbp`, so the
   stack is unbalanced → **segfault** on p2_mem (exit 139).
2. Guard prologue *and* epilogue and shrink `setup_area_size` 16 → 8.
   Result: prologue/epilogue look right and small programs print correct
   output, but p2_mem aborts with
   `memory allocation of 435733609449752 bytes failed` → a **miscompile**.
   `setup_area_size` is not only prologue bookkeeping: it is the
   caller-SP-relative offset used by `gen_clobber_save`/`gen_clobber_restore`
   and by the unwinder. Changing it without auditing those paths silently
   corrupts callee-save slots.

Conclusion: making omit-frame-pointer work on x64 means auditing every use of
`setup_area_size` and the `offset_upward_to_caller_sp` computations, not
flipping a boolean. Both attempts are reverted; the tree is clean. If this is
picked up again, the honest first step is a Rakit-side test that checks
`rsp` is balanced across a function with callee-saves and a `return_call`.

Important sanity check learned here: after any ABI-level patch, verify output
of **p2_mem** specifically. It is the program that surfaced both failures,
while p1/p3/p4 still printed plausible-looking numbers.

### Correctness baseline (clean build, 2026-11-17)

All nine benchmark programs produce byte-identical stdout for
LLVM (`L`), cg_clif upstream (`U`) and modified Rakit (`M`). Any mismatch is
a bug in a patch, not a pre-existing backend defect — the clean tree is sound.

### p2_mem: the remaining 3.3x

Run time 54.8 ms (M) vs 16.8 ms (L). Samples are spread thin: 31%
`partition_lomuto_branchless_cyclic`, 14% `hashbrown::find_inner`, 10%
`HashMap::raw_entry`, 9% SipHash `write`, 8% `swap_if_less`. Unlike p3 there is
no single hot function to fix; the cost is diffuse across hash and sort code,
which is consistent with the already-documented conclusion that the remaining
gap is broad code-quality (no loop rotation, weak regalloc on the partition
loop, no autovectorizer) rather than one missing optimization.

### `regalloc_algorithm=single_pass` is not a compile-time win

Regalloc is a plausible place to look for compile-time savings, so both
algorithms were measured on all nine programs (correctness verified on p2_mem
first; both produce identical output).

| metric (sum of 9) | backtracking | single_pass |
| --- | --- | --- |
| compile wall | 843.8 ms | 839.6 ms |
| compile cpu | 1088.0 ms | 1089.6 ms |
| compile peak RSS | 1009 MB | 1006 MB |
| run wall | 478.9 ms | 629.5 ms |
| artifact size | 36803872 | 36853216 |

Compile time is unchanged (the difference is inside run-to-run noise), because
for these small single-crate programs regalloc is not the dominant cost — the
frontend and the rest of the pipeline are. Runtime regresses by 31% and the
binaries get larger. Rejected; the experimental `RAKIT_REGALLOC` tunable was
removed again rather than left in as dead code.

This also closes out the "compile-time" axis of the investigation: the knobs
that are actually available (`opt_level`, `regalloc_algorithm`, CGU count,
inliner budget) all move compile time by at most a few percent. Compile time is
dominated by work shared with LLVM (parsing, type checking, MIR), which is why
rakit's advantage there is structural rather than tunable.

## Next step (NOT done): LCG lane transform for p8 — exact math

Goal: p8 (`a = a*C + (i|1)`, min) 29 ms → ~20 ms, matching LLVM's 4-lane loop.
Current `apply_partial` builds each `pre[k]` from the header → O(factor²)
`imul`s (factor 3 = 6 imuls / 3 iters = 2/iter). LLVM does ~1 imul/iter.

**Why LLVM wins: even factor ⇒ fixed parity per lane.** With `f` lanes, lane `L`
visits `i = L, L+f, L+2f, …`. For **even** `f`, `i` has the *same parity* as `L`,
so `(i|1) = i + (L even ? 1 : 0)` is **linear in the lane step** `k`. Hence each
lane's forcing term is affine in `k` and advances by a **constant add** (no
`imul`). Lane recurrence:
  `a_L(k+1) = C^f · a_L(k) + F_L(k)`,   `F_L(k+1) = F_L(k) + slope`,
  `slope = f · (C^{f-1} + C^{f-2} + … + C + 1)  (mod 2^64)`, same for all lanes;
  `F_L(0) = Σ_{j=0}^{f-1} C^{f-1-j} · ((L+j)|1)`  — a **compile-time constant**;
  `a_L(0) = C^L · a_0 + Σ_{j<L} C^{L-1-j}·(j|1)` — one `imul` (C^L·a_0) + const.
Loop body = f × (1 imul + 1 add + 1 forcing add + 1 min) per f iters ⇒ 1 imul/iter.
The min over all lane values covers every inner iteration (lanes partition `i`).

**Why factor 3 can't use it:** odd `f` ⇒ `i`'s parity alternates per lane ⇒
`(i|1)` is not linear. So the lane transform must use an **even** factor (4).

**Hard part = the tail/epilogue.** The lane loop's carried state is f separate
accs, not "the current acc". When `BOUND % f != 0`, the remainder must fall back
to a scalar loop, and the scalar loop needs the true `a` at iteration
`f·⌊BOUND/f⌋` — i.e. one specific lane's last value. Options: (a) run the scalar
tail from that lane's value (pick lane = `(f·n_last) mod f`); (b) keep the
existing guard+narrow single-step but only enter the lane path when
`iv + f <= BOUND`, and after the lane body recompute the scalar acc from the
lane that is `f-1` steps ahead. Getting this wrong is a silent miscompile, so it
needs the full differential suite (2^32 f32-style sweeps are overkill here; a
range of BOUND mod f values × small BOUNDs, compared byte-for-byte to LLVM, is
the minimum).

**Scope:** only fires for a single LCG state + a single associative reduction
(min/max/add) with atom `iv | const` and even factor. Everything else keeps the
current (correct) clone path. This is a *new* emission path in `apply_partial`,
~200–300 lines, higher risk than the guard/factor tweaks. Deferred because the
sandbox reset twice this session; land it only with incremental commits + the
full `./y.sh test` green gate.
