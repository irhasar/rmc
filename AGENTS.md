# AGENTS.md

## Session (2026-11-18, latest) — leaf-tier inlining closes most of the real-crate runtime gap (LANDED)

- New tiered inliner in `src/inliner.rs`: `MAX_CALLEE_INSTS = 64` (non-leaf,
  unchanged) plus `MAX_LEAF_INSTS = 192` (`CG_CLIF_INLINE_LEAF_MAX`) for leaf
  callees. A leaf (no `call`/`call_indirect` of its own) cannot cascade, so a
  larger cap is safe; that is the whole reason the tier exists.
- `is_leaf` ignores blocks ending in `trap`: those are panic paths (`trap` +
  a `call` that formats the message). Without this, a helper with a bounds
  check looks non-leaf. This is what finally admits `itoa` (83 CLIF insts).
- Leaves also get their own growth budget, *added* to the normal one
  (`leaf_budget = inline_budget(caller) + max_leaf_insts`). A hot loop body is
  often small, so the plain 50%-of-caller growth budget rejected the very
  helper called every iteration. `src/driver/aot.rs` collects bodies up to the
  larger of the two caps so leaf-only bodies are available to inline.
- `CG_CLIF_INLINE_MAX=0` still means "inlining off" for both tiers, so the knob
  can still bisect it away.
- **Measured** (isolated A/B, leaf tier off -> on, `--reps 5`): runtime
  openlocationcode 19461 -> 14019 ms (-28.0%), holidays-rs 27872 -> 22115 ms
  (-20.7%), stdnum-rs 11423 -> 7819 ms (-31.6%). Size *drops* 0.2-0.5% (inlining
  removes call overhead and enables more DCE); compile cost <= +3.5%.
- **Final 3-way on real crates** (L=LLVM, U=upstream, M=modified): runtime
  M vs U improves to 1.9-2.2x faster (was 1.6-1.9x in 8.24); openlocationcode
  M vs L goes from 7.3x to 3.5x slower.
- **Tests**: `./y.sh test --sysroot clif` -> 55 suites `test result: ok`,
  0 failed (incl. 3011 core benches, 1355 std). Driver outputs identical
  across L/U/M. Full std rebuilt with the pass active.
- REPORT.md section 8.25 documents it. Fork commits: `163ab57` (code) + `caecf4f`
  (docs).
- **PUSHED**: `replace-main-with-cgclif-experiments` is now at `3814539` on the
  remote (commits `e90bf60` code+REPORT 8.25+AGENTS, `96b4941` blocker note,
  `3814539` A/B script). PR #10 head is `3814539`, `mergeable: MERGEABLE`,
  `mergeStateStatus: CLEAN`, base `main`. PR description updated via the REST
  API to add the leaf-tier and if-conversion results.
- Push history: the injected GITHUB_TOKEN / OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB
  and the git credential helper are all read-only for `irhasar/rmc` (403). The
  push and PR edit used a user-supplied `repo`-scoped PAT, used only for the
  push/PR edit and not stored anywhere.

## Session (2026-11-18) — if-conversion diamond → select/cmove (LANDED, `7374000`)
- New pass `src/if_convert.rs`: collapses a short, side-effect-free two-way
  diamond (`brif` head -> then/else -> common join) into a `select` in the
  head, which Cranelift lowers to `cmove`. Runs **before** `unroll` in
  `src/base.rs` (slot `IF_CONVERT` in `src/passtime.rs`; knob
  `CG_CLIF_NO_IFCONV`, default ON; tunable `tunables::if_convert_enabled`).
- Why it matters twice: (1) a data-dependent in-loop branch mispredicts every
  iteration; (2) the fork's `unroll` only widens a single-block body, so a
  diamond blocks unrolling. Collapsing first gives the unroller its single block.
- Guards: head is a `brif` with two successors; each arm is single-pred /
  single-succ, ends in `jump` to the *same* join, join has no block params, no
  escaping defs, every arm inst `is_speculatable`, arm <= `MAX_ARM_INSTS = 8`.
  Rewrites to a fixed point so nested diamonds also collapse.
- **Cost model is essential**: `cmove` computes both arms, so
  `is_cheap_to_speculate` rejects arms holding `FcvtToUintSat`/`FcvtToSintSat`
  (expands to a long branchy sequence on x86) or `Udiv/Sdiv/Urem/Srem/Fdiv`.
  Without it, p1's `f as u64` arm regressed p1 105 -> 112 ms.
- **Measured** (wall ms): `w_mod` (hash%7 arm) 62.7 -> **38.1** (LLVM 29.8);
  `b1` (3-way, cheap arms) 73.8 -> **61.1** (LLVM 60.0); `b2` (predictable cond)
  39.0 -> 40.7 (small, expected loss); `v_full`/p1 (arm holds `f as u64`)
  104.6 -> 104.7 (neutral thanks to the cost model).
- **Tests**: `./y.sh test --sysroot clif` -> 65 `test result: ok`, 0 failed
  (incl. 1400 + 825 + 3011 core benches). p1-p6 outputs identical to LLVM.
- REPORT.md section 8.22 documents it; subtree `cg_clif_mod/src` mirrors the
  fork commit `7374000`.
- **PUSHED**: `replace-main-with-cgclif-experiments` is at `f084efe` on the
  remote (commits `341a286` pass, `5bce6fa` docs, `f084efe` REPORT 8.23).
  PR #10 head is `f084efe`, `mergeable: clean`, base `main`. The injected
  `GITHUB_TOKEN` (read-only GitHub App) and the credential-helper token both
  fail; the push used a user-supplied `repo`-scoped PAT (used only for the
  push, not stored).

## Session (2026-11-18, latest) — store-forwarding miscompile fix (LANDED locally, `4121be3`)
- Found a real miscompile in the fork: `rust-random/rand` test
  `rngs::std::test::test_stdrng_construction` FAILED in M but passed in
  upstream cg_clif AND LLVM.
- Isolation: `CG_CLIF_NO_STORE_FWD=1` → ok; `CG_CLIF_SF_NO_CBE=1` → ok;
  `CG_CLIF_UNROLL=1` → still FAILED. So the bug is the cross-block
  store-forwarding dataflow, not the unroller. Upstream has no such pass, so
  it is a fork-only regression.
- Root cause: `record_store` returned early (store not in `loaded`, or atomic
  store) BEFORE the overlap invalidation. Such a store still writes bytes, so a
  stale fact survived it and a later load was forwarded the old value.
- Fix: move the `avail.retain(...)` invalidation ahead of both early returns.
  Strictly more conservative (can only drop forwards), so it cannot introduce a
  new miscompile. `src/store_forward.rs` only; no knob added in the final commit.
- Verified: rand workspace all ok (0 failed); `./y.sh test --sysroot clif`
  0 failed; p1–p6 all match LLVM/upstream; 3-way benchmark unchanged
  (p5/p6 7 ms, p2 67 ms) — the soundness fix costs nothing.
- Commits: `cg_clif_mod` `4121be3`; `rmc_clone` subtree `74b78f6`.
- Debug technique that worked: temporary env knobs to bisect the pass
  (`CG_CLIF_NO_STORE_FWD`, `CG_CLIF_SF_NO_CBE`, `CG_CLIF_UNROLL`), plus a
  temporary `CG_CLIF_NO_DEAD_STORE` knob and an `eprintln!` trace guarded by
  `CG_CLIF_SF_TRACE` in the forward site. Both temporary knobs were removed
  before the final commit. `CG_CLIF_SF_TIME=1` dumps per-function pass stats.
- CLIF dump: `should_write_ir` keys off `--emit llvm-ir`, so `-Cllvm-args` is
  not needed; the `.opt.clif`/`.unopt.clif` files land next to the output.
- Push status: **DONE**. PR #10 head is now `2d5a77f` on branch
  `replace-main-with-cgclif-experiments` (open, mergeable, `mergeable_state`
  clean). Earlier sessions were blocked by a read-only installation token; a
  user-supplied `repo`-scoped PAT unblocked it. The PAT was used only for the
  push and is not stored anywhere in the repo or git config.

## Session (2026-11-18) — affine unroll factor 8 → 128 (LANDED locally, superseded)
- `cg_clif_mod` head **`6951858`** "Raise the affine loop unroll factor to 128
  with a checked step bound"; `rmc_clone` subtree commit **`193ec3e`** (PR #10
  branch `replace-main-with-cgclif-experiments`), **not pushed** (see below).
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
  `rust-src,llvm-tools`. `dist/bin/rustc-clif` was intact and works after PATH
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
  `-Ctarget-cpu=native`; `CG_CLIF_INLINE_MAX` 16/256; aggressive inliner
  (`INLINE_USES=100 INLINE_MAX=200`) → p1 108→105, others flat; Cranelift
  `opt_level=speed` vs `speed_and_size` (no difference); clone-based body
  duplication on multi-block bodies (does not fire — `analyze_affine` requires a
  single-block body, so trig/sin loops are untouched).
- `./y.sh test --sysroot clif` → 3011 passed / 0 failed (before the reset).
- **Push BLOCKED**: `irhasar/rmc` rejects every credential available in this
  environment (403 `Resource not accessible by integration`). Both the injected
  `GITHUB_TOKEN` (GitHub App installation, read-only) and the OpenHands
  credential-helper secret are rejected. To land, push
  `replace-main-with-cgclif-experiments` with a user-supplied `repo`-scoped PAT;
  PR #10 then updates in place. Local commits `6951858` (mod) / `193ec3e` (subtree)
  are ready.

## Session (2026-11-18) — inliner growth cap (LANDED locally)
- `cg_clif_mod` head **`41dc347`** "Bound inliner growth by caller size; speed up
  dead-store removal".
- **Root cause of the p2 compile gap vs U**: the inliner grew CLIF by up to 62%
  (14449 → 23383 insts on p2_mem). Every later Cranelift pass then walks the
  larger function, so the whole codegen slows down. This was the *entire* gap.
- **Fix**: cap the instructions a caller may absorb to a fraction of its own
  size. New tunable `CG_CLIF_INLINE_GROWTH` (default 50%), clamped to
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
- **PUSHED** (2026-11-18, later session): `replace-main-with-cgclif-experiments`
  is at `b521842` on the remote; PR #10 head is `b521842`, `mergeable: clean`,
  base `main`. The earlier credential blocker was worked around with a
  user-supplied `repo`-scoped PAT (used only for the push/PR edit; not stored).
- `./y.sh test --sysroot clif` → **3011 passed / 0 failed** (+596 core benches).
- `REPORT.md` §8.16–8.17 and `bench/fullbench.sh` record the final matrix
  (RC=11/RR=11).

## Session (2026-11-18) — pass cost + SIMD lane ICE fix
- `cg_clif_mod` head `97617a1`; `rmc_clone` subtree head `7249d45`.
- New `src/tunables.rs`: every env knob read once via `OnceLock` (passes run per
  function; `std::env::var` locks a global map each call).
- `store_forward` rewritten: one `analyze()` setup walk instead of three, and a
  single reverse-postorder sweep instead of a worklist dataflow (9.2ms → 5.1ms).
- New opt-in pass accounting: `src/passtime.rs` (`CG_CLIF_PASS_TIME=1`) and
  `src/clifprof.rs` (`CG_CLIF_CLIF_TIME=1`).
- **Fixed the pre-existing SIMD-lane ICE**: `value_lane`/`value_typed_lane`/
  `value_lane_dyn` assumed a `ByRef` value and hit `unreachable!()` for the
  register-resident `ByVal` vector introduced by `a9d182a`; they now spill via
  `force_stack`. `./y.sh test` → **3011 passed / 0 failed** (+596 core benches);
  the `jit/aot.std_example` ICE is gone.
- `REPORT.md` §8.13–8.15 and `bench/fullbench.sh` record the full three-way
  matrix (RC=7/RR=9).
- **Push still blocked**: `irhasar/rmc` rejects every available credential (403
  `Resource not accessible by integration`). Commit `7249d45` is local only; to
  land it, push `replace-main-with-cgclif-experiments` with a `repo`-scoped PAT
  (updates PR #10 in place).

## Repo purpose
Experiments comparing Rust codegen backends: LLVM (`rustc`) vs
`rustc_codegen_cranelift` (cg_clif) upstream vs a modified cg_clif fork.

## Layout
- `REPORT.md` — findings, numbers, root causes, recommendations.
- `build.sh` — rebuilds the modified cg_clif + its Cranelift std sysroot.
- `cg_clif_upstream/` — pristine cg_clif (commit 42ac63e).
- `cg_clif_mod/` — fork with the modifications (see commits
  `b6c94e4`…`9ea4bbe`).
- `bench/` — harness:
  - `allaxes.sh` — **canonical all-axes 3-way harness** (LLVM / cg_clif up /
    cg_clif mod). Every run MUST report all of these, per backend and per
    program; a benchmark is not complete without all of them:
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
For each backend (LLVM / cg_clif upstream / cg_clif modified) and each program,
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
- Toolchain: `nightly-2026-11-17` (must match, cg_clif pins it).
- Need `rust-src` and `llvm-tools`. `$HOME/.cargo/bin` on PATH.
- Build: `build.sh` → `cg_clif_mod/dist/bin/rustc-clif`.
- Rebuilding Cranelift std takes ~1–2 min incremental.

## Key facts learned
- Cranelift is **Apache-2.0 WITH LLVM-exception**; forking/reworking is fine.
- cg_clif compiles ~20–45% faster than LLVM but produces ~26% larger binaries
  and runs 2–9× slower.
- **Size root cause:** `cranelift-object` named every function section
  `.text.subsection` and every data object `.data.rel.ro.subsection`; the linker
  merges same-named sections so `--gc-sections` could not drop dead code.
  Fix: name sections after the linkage symbol (`add_subsection(kind, decl_name)`)
  and set `builder.per_data_object_section(true)`. Result: −15.6% raw, −26.5% stripped.
- **Runtime root cause:** cg_clif does not use Cranelift's `Inline` API (the
  trait docstring names cg_clif explicitly), and MIR locals stay in stack slots
  (`add (%rax),%rdx` / `mov %rdx,(%rsp)` per loop iteration) with no unrolling/ILP.
  `-Zinline-mir=yes` grows functions but does not speed up runtime on its own.
- With LLVM-built std, cg_clif's binary size matches LLVM, proving the size gap
  comes from Cranelift-built std and the runtime gap from the backend itself.

## Modifications landed in `cg_clif_mod` (beyond upstream)
- `b6c94e4` per-symbol sections (size −15.6% raw, −26.5% stripped).
- `7ddf2d4` **SROA**: a scalar local that is only *shared*-borrowed is kept in a
  register; a stack slot is materialized lazily at the first real address use
  (`CPlace::to_ptr` → `force_to_stack`). Fixes the decisive `println!("{}", acc)`
  regression (p5/p6 894 ms → 356 ms) without regressing p2_mem.
- `2ddb545` **CLIF inliner** (`src/inliner.rs`): per-CGU, inlines small (`≤64`
  CLIF insts) *final* callees with a 512-inst budget. Guardrails: no self-inline,
  each callee at most once per caller, and **skip callees that reference any
  `global_values`** — Cranelift's `inline_one` does not remap them and the
  resulting object file fails to link (`R_X86_64_TLSGD out of range`).
  `CG_CLIF_INLINE_MAX=0` disables it (bisecting knob).
- `9ea4bbe` **CLIF affine-reassociation loop pass** (`src/unroll.rs`): recognises a
  canonical counted loop, derives the affine state update `S' = M·S + c` from the
  body, composes it `factor` times in closed form, and rewrites the back edge so
  one header check advances the whole state by `factor` iterations at once, with
  a guard that falls back to a single step for the tail. Exact modulo 2^64
  (iadd/imul wrap). Turns p6 from 902 ms → 61 ms (LLVM 58 ms). A body-duplicating
  unroller is also present but **off by default** (`CG_CLIF_UNROLL_CLONE`): it is
  correct but slower, because duplicating a body does not shorten the carried
  chain. Knob: `CG_CLIF_UNROLL` (default 8).
- **NEW** `src/store_forward.rs` **CLIF stack store→load forwarding + dead-store
  elimination**: forwards a `store` into a private (non-escaping) stack slot to
  every later `load` of the same address, aliases the load result to the stored
  value and removes the load; then deletes stores whose bytes no surviving load
  reads. This is the real lever for p5/p6 (365 ms → 63 ms). It must run **before**
  `unroll`, because the affine loop pass rejects any body that still touches
  memory — the store/load round-trip cg_clif emits for an `Option`/iterator
  otherwise blocks the unroll. Knob: `CG_CLIF_NO_STORE_FWD`.
  - Why it is needed: Cranelift's egraph only value-numbers loads that are
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
    arguments. cg_clif threads a stack address through a block parameter (a phi),
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
    connected — yet cg_clif materialises aggregates *across* control flow (a
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
  `i64` loads and two `i64` stores. cg_clif bounces every SIMD op through such a
  slot, so the bitcast `__m128i → Simd<i8, 16>` alone was 1 store + 2 loads + 2
  stores. Now a single 16-byte vector load+store. Wider vectors (`__m256i`
  without AVX) have no single store and keep the `memcpy`.
- `8ff4779` **cross-type same-size vector store→load forwarding**
  (`src/store_forward.rs`). The pass only matched a load to a store of the
  *exact same* CLIF type. A SIMD bitcast (`__m128i ↔ Simd<i8, 16>`) compiles to
  a store as one lane type and a load back as another, so those round-trips
  survived to machine code. When byte sizes match and both types are vectors,
  the stored value is now forwarded through a single `bitcast`; the target
  endianness is threaded in from the ISA because Cranelift's verifier requires
  explicit endianness when lane counts differ. `as_i8x16` becomes one
  `bitcast.i8x16`. p2_mem 110→99 ms; size 4.41 MB (LLVM 4.64 MB).
- `c2e292b` **inline callees that reference globals** (`vendor/cranelift-codegen-0.137.0`,
  `src/inliner.rs`). Cranelift's inliner copied a callee's `GlobalValueData`
  verbatim, including `GlobalValueData::Symbol`, whose `ExternalName` indexes the
  *callee's* `user_named_funcs`/`ext_funcs` tables. Those indices were never
  translated, so inlining a callee that referenced a const/static/thread-local
  corrupted the caller (out-of-range `R_X86_64_TLSGD`). cg_clif worked around it
  by refusing to inline any callee with a non-empty `global_values`, which
  blocked the hot small helpers (`core::hash::sip`, the branchless sort
  partition closure). Fix: vendor cranelift-codegen and translate the symbol's
  user name while copying global values (`translate_symbol_name`), reusing the
  same caller-side mapping `create_user_external_name_refs` installs — done
  inline there because global-value copying runs first. Also drop the
  one-inline-per-callee limit (`MAX_CALLEE_USES`, `CG_CLIF_INLINE_USES`): the
  partition closure is called 3× and was left out of line; the call-graph budget
  still bounds recursion. Effect: `slice::sort_unstable` quicksort calls 11→3,
  srt 28.5→23.1 ms; p2_mem 74.5→59 ms. TLS/atomics/statics verified correct.

## What did NOT work / open leads
- **p2_mem runtime gap (fixed)**: the 8× gap (125 ms vs LLVM 16 ms) came from
  cg_clif emitting the `core::core_arch::x86::sse2` intrinsics as **out-of-line
  calls** — `p2_mem_M` had 8 `_mm_*` symbols and 23 `call`s to them, while LLVM
  had 0. Root cause was *not* the inliner: `src/intrinsics/simd.rs` scalarized
  every integer SIMD comparison per-lane (load i8 → `icmp` → `bmask` → store,
  16×) and `simd_bitmask` into a per-lane shift/or loop, so `_mm_cmpeq_epi8`
  produced ~155 CLIF instructions and never fit the inliner budget. Fixed by
  emitting one vector `icmp` (`pcmpeq*`/`pcmpgt*`) and `vhigh_bits`
  (`pmovmskb`) on x86_64; see `simd.rs` and REPORT.md §8.8. `_mm_cmpeq_epi8`
  body: 155→34 insts, `_mm_movemask_epi8`: 255→23; p2_mem 159→110 ms.
  - What did **not** help, recorded for the record: `-Ctarget-feature=+sse2`,
    `-Ctarget-cpu=native`, `-Zinline-mir=yes`, `CG_CLIF_INLINE_MAX=4096`
    (the last only trims in-crate callers), and forcing `-Ccodegen-units=1`.
  - Residual p2/p4 gap now comes from cross-CGU inlining of `memchr`/`hashbrown`
    helpers and the absence of autovectorization in Cranelift; both are outside
    the reach of cg_clif-side changes. A real cross-CGU inliner would be the
    next step.
  - Cranelift's `Inline` API clones `GlobalValueData` verbatim and does **not**
    remap the `ExternalName` indices inside `GlobalValueData::Symbol`, so
    inlining a callee that references a const/static/thread-local corrupts the
    caller (TLS relocations went out of range when the guard was lifted).
    **FIXED** in the vendored `cranelift-codegen` (see the entry below): the
    symbol's user name is now translated into the caller's namespace, so the
    `body.global_values.is_empty()` guard in `inliner.rs` is gone.
- Extending SROA to **`&mut`/`RawPtr` whole-local borrows** (with `local_map`
  redirect) is a **miscompile** (p1/p3/p4 wrong). Shared-only is correct.
- The inliner is redundant **within a crate** (MIR inlining already folds those
  helpers), but load-bearing **across crates**: the hot helpers in `core::hash`
  and `slice::sort` live in the pre-built std, so only a CLIF-level inliner can
  reach them. Once the global-values guard was removed it turns `slice::sort`'s
  partition closure from an out-of-line call into inlined code (quicksort calls
  11→3). It remains net-neutral on the p1–p6 microbenchmarks that MIR already
  folds.
- **Body-duplicating unrolling does not help.** Measured slower on p6 (357→423 ms):
  the win LLVM gets on p5/p6 is *affine reassociation* of the accumulator chain,
  not unrolling per se. Duplicating the body leaves the loop-carried dependency
  chain intact. Implemented anyway behind `CG_CLIF_UNROLL_CLONE` for completeness.
- Remaining runtime gap on p1–p4 (LLVM 17–58 ms vs mod 99–140 ms) is **not**
  loops or store/load round-trips — the cross-block pass left runtime unchanged.
  For p2 it is the out-of-line `core` SSE intrinsics (see the root-cause entry
  above). p1/p3/p4 gaps are smaller and stem from the same missing cross-CGU
  inlining plus incomplete memory-to-register promotion.

## Gotchas
- `measure.py`/`compare3.sh`: pass the rustc command as separate argv elements;
  the `--sysroot` handling in `run_backend` requires the sysroot as its own arg.
- Regenerating `cg_clif_mod/Cargo.lock` needs dropping `--locked` because of the
  local `[patch.crates-io]` for `cranelift-object`.
- `--gc-sections` works in this environment (verified with a hand-written asm test).
- **CLIF rewriting:** `InstructionData` is `Copy` and its `BlockCall`s share the
  value pool with the source instruction. Before remapping a cloned instruction
  you must `deep_clone` its branch destinations, or the clone's branch args
  silently corrupt the original (this ICEs later in the egraph pass with
  `assertion failed: payload < (1 << 30)`).
- **Value aliases:** cg_clif emits `v6 -> v2` aliases heavily. Resolve with
  `dfg.resolve_aliases` before mapping header params to clone params, or the
  clone reads the wrong state.
- `func.layout` has no `num_blocks`; use `func.layout.blocks().count()`.



## Push status (T6) — DONE
- PR **#10** (`replace-main-with-cgclif-experiments` → `main`) is open,
  **mergeable=True (clean)**. Local `rmc_clone` == remote.
- Latest fork commit on `cg_clif_mod`: `8ff4779`
  `Forward same-size vector stores to loads of a different lane type`
  (on top of `0e6c2f0` SIMD copy via one vector load/store and `38923c0`
  SIMD constants → `vconst`). Squashed into one commit `4759d90` in
  `rmc_clone` and pushed; PR #10 head is `4759d90`, 10 commits.
- Earlier commits: `e101572` cross-block store forwarding + doc/bench-path sync;
  `628c73b` store-forward miscompile fix (branch args were not scanned in
  `private_slots`). Pushed with a user-supplied classic PAT with `repo` scope:
  `git push https://<user>:<PAT>@github.com/irhasar/rmc.git replace-main-with-cgclif-experiments`.
- Note: the environment's injected `GITHUB_TOKEN` (GitHub App installation) is
  read-only for this repo — `git push` and all write APIs return 403
  (`Resource not accessible by integration`). `OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB`
  is also rejected (`Invalid username or token`), and there is no SSH key.

## Session benchmark report (2026-11-18, latest)

Full three-way harness: `cd bench && REPS_COMPILE=3 REPS_RUN=5 ./compare3.sh`
(log `/tmp/compare3_final.log`) and `./sizes2.sh` (log `/tmp/sizes2_final.log`).
C = compile (wall ms, cpu ms, peak RSS MB, exit), S = size (raw bytes,
.text, .data.rel.ro, .rodata, .eh_frame, `size -A` total), R = run
(wall ms, cpu ms, peak RSS MB, exit). "up_llvmstd"/"mod_llvmstd" link the
stock LLVM std to isolate the backend from the Cranelift-built std.

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

cg_clif (up and mod) compiles ~1.5–2x faster than LLVM on p2 and slightly
faster elsewhere, at equal peak RSS. The mod's extra CLIF passes cost ~1 ms
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
libraries, whereas the mod links its own Cranelift-built std. With the LLVM
std (isolates the backend), mod is byte-for-byte the same size as up plus
only the SIMD lowering: p1 4,631,032 vs 4,631,880, p5/p6 4,603,608 vs
4,604,376 — i.e. the size win is in the Cranelift-built std (the mod's
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
- `bc9b0f0` Honour `CG_CLIF_INLINE_MAX` when collecting inlineable bodies:
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
  `467f05c` "Inline small constant memcpy/memmove and honour CG_CLIF_INLINE_MAX"
  and `aee9ba3` "docs: record push status for the latest session".
- The branch replaces the whole `main` tree (subtree `cg_clif_mod/` at repo
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
the residual 3–7x is **Cranelift codegen quality, not inlining**:
- Forcing `CG_CLIF_INLINE_MAX=200` to inline the SipHash rounds *slows* the
  hash benchmark (136.6→154 ms): after inlining, the `SipState` round-trip goes
  through the opaque `&mut self` pointer and Cranelift spills/reloads it every
  round, while LLVM keeps it in registers. cg_clif's `store_forward` only
  forwards through *non-escaping* pointer bases, and the `Hasher::write`
  parameter escapes into `c_rounds`/`d_rounds`, so the round-trip survives.
- LLVM's advantage on these loops is its register allocator plus
  autovectorization (the SipHash tail, `u8to64_le`), which Cranelift does not
  do. These are backend-internal; a cg_clif-side pass cannot reach them.

### Push status (2026-11-18, second update) — PUSHED
- Pushed to PR **#10** (`replace-main-with-cgclif-experiments` → `main`), head
  now **`8f84035`**, 16 commits, `mergeable=True`, `mergeable_state=clean`.
  Commits this session: `64b3760` "Inline callees that reference globals; allow
  repeated inlining" (vendors `cg_clif_mod/vendor/cranelift-codegen-0.137.0` +
  the `translate_symbol_name` fix) and `8f84035` (docs).
- The environment's injected `GITHUB_TOKEN` is read-only for this repo (all
  writes 403 `Resource not accessible by integration`); a user-supplied
  `repo`-scoped classic PAT was used to push. (History note below kept for the
  blocked attempt.)
- Historical note — BLOCKED ON CREDENTIALS
- `cg_clif_mod` head is `c2e292b` "Inline callees that reference globals; allow
  repeated inlining"; it vendors `vendor/cranelift-codegen-0.137.0` (added to
  `[patch.crates-io]`) with the `translate_symbol_name` fix.
- `rmc_clone` (subtree layout) is committed as **`64b3760`** on branch
  `replace-main-with-cgclif-experiments`, which is PR **#10**'s head branch
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
  `git -C rmc_clone push https://<user>:<PAT>@github.com/irhasar/rmc.git replace-main-with-cgclif-experiments`
  This updates PR #10 in place; no new PR is needed.

## Session note — external-crate validation (REPORT 8.24)

- Added `extbench/` (harness `extbench.sh`, `exttests.sh`, `drivers/`) and
  REPORT section 8.24. Three real crates from `github.com/cipansor/crateworks`
  are cloned and built with L=LLVM, U=cg_clif upstream, M=modified fork.
- Crates: `openlocationcode` (fp + string), `holidays-rs` (date search),
  `stdnum-rs` (checksum/parsing). All pure Rust, no deps, `#![forbid(unsafe_code)]`.
- Results: compile wall 1.7–2.1× faster than LLVM, M≈U within 6%; artifact
  7–24% smaller than U; runtime 1.6–1.9× faster than U on all three; run RSS
  identical (~10.7 MB). All crate test suites pass (0 failed) on L/U/M; driver
  outputs identical across backends.
- `dist/` is not committed, so the harness takes `RUSTC_U`, `RUSTC_M`,
  `CRATEWORKS`, `BENCH_DIR` env overrides. Pushed as `a4278b0`; PR #10 head.

## Session (2026-11-19) — large-repo validation on cipansor/Verif

Benchmarked against **`github.com/cipansor/Verif`** (4.1 MB Rust, 84 files, zero
external deps, `tests/mir.rs` 588 KB, `src/mir/exec.rs` 570 KB) to check the
optimizations hold on a codebase much larger and more complex than the earlier
microbenchmarks. Scratch harness in `/workspace/project/bigbench`
(`verif_full.sh` = 3-way compile/run/equivalence, `deep.sh` = MIR-heavy
examples, `ab_passes.sh` = per-pass A/B).

### Correctness: clean
All seven runnable examples produce **byte-identical** stdout across
L (LLVM) / U (upstream cg_clif) / M (this fork), including the two that shell
out to rustc and parse MIR (`comparison_check`, `mir_coverage`).

### Results (`--features std`, `-C opt-level=3`, best of 3)

| backend | compile lib | rlib | binary | run wall | run RSS |
|---|---|---|---|---|---|
| L LLVM | 11719 ms | 32.10 MB | 10.00 MB | 4 ms | 10.7 MB |
| U upstream | 3242 ms | 27.40 MB | 13.43 MB | 7 ms | 10.8 MB |
| M fork | 3439 ms | 29.70 MB | **9.74 MB** | 5 ms | 10.7 MB |

- Compile: **3.4x faster than LLVM**, +6% vs upstream.
- Binary: **27% smaller than upstream**, **2.6% smaller than LLVM**.
- `flight_control` (the only real compute loop): M 8920 ms vs U 9387 ms vs
  L 4484 ms — M is 5% faster than U, LLVM still 2x ahead.

### Root cause of the +6% compile vs upstream: the passes, not the vendor patch
A/B with `CG_CLIF_NO_STORE_FWD=1 CG_CLIF_NO_IFCONV=1 CG_CLIF_INLINE_MAX=0
CG_CLIF_UNROLL=0` gives **3212 ms ≈ U's 3208 ms**, so the vendored
`cranelift-object`/`cranelift-codegen` patches cost nothing. Per-pass on this
crate: store_forward 380-400 ms (dominant), inline 153-158 ms, if_convert 87-98
ms, collect 75-80 ms, unroll 5 ms. `store_forward` runs on 8060 functions at
~48 us each and is close to flat per function, i.e. fixed overhead.

### Why the rlib is *bigger* than upstream's but the binary is much smaller
`vendor/cranelift-object` names each function's section after its symbol
(`.text.<sym>`) and gives each data object its own section, instead of
upstream's single `.text.subsection` / `.data.rel.ro.subsection`. On this crate
that is 30,465 sections vs 18,550, of which ~9,283 are per-data-object and cost
~2 MB of `.shstrtab`. Payload is unchanged (`.text` is actually 3.20 MB vs
3.75 MB). Measured tradeoff with `-Zfunction-sections=no`:

| variant | rlib | binary |
|---|---|---|
| M sections ON | 29.70 MB | **9.74 MB** |
| M sections OFF | 26.46 MB | 16.42 MB |
| U upstream | 27.40 MB | 13.43 MB |

Per-function sections **save 6.7 MB of final binary (41%) for 3.2 MB of rlib**.
That is the right trade for a shipped artifact; do not "fix" the rlib size by
turning them off.

### Changes landed this session
- `inliner`: new `CguBodies` computes each callee's `is_leaf` once at collection
  time instead of re-scanning the body at every call site that names it.
- `store_forward`: O(1) early-out when the function has no sized/dynamic stack
  slot (nothing to forward, nothing to drop), plus the same guard inside
  `drop_dead_stores`. Compile 3590 -> 3439 ms on Verif, output unchanged.

### Gotcha: rlibs are not portable across backends
A `verif_arithmetic` rlib built by U cannot be read by M (`E0463: can't find
crate`), because the metadata hash embeds the backend. Each backend must build
its own dependency rlibs; a benchmark harness that shares one rlib between
backends silently measures a failed compile as a 126 ms "fast" build.

## Push to `irhasar/rmc` — needs a user-supplied PAT (RESOLVED 2026-11-19)

The sandbox tokens cannot write to `irhasar/rmc`:
- `GITHUB_TOKEN` is a GitHub App installation token with read-only contents.
  `GET /repos/irhasar/rmc` misleadingly reports `push: true, admin: true`, but
  `POST /git/refs` returns **403 "Resource not accessible by integration"** and
  `git push` returns **403** even for a throwaway branch. `x-oauth-scopes` is
  empty; `x-accepted-github-permissions: allows_permissionless_access=true`.
- `OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB` is **401 Bad credentials**.
- `gh auth setup-git` does not help; it reuses the same token.

A **`repo`-scoped classic PAT** from the user does work. Use it inline, once,
and never persist it:
```
git push "https://x-access-token:<PAT>@github.com/irhasar/rmc.git" \
    HEAD:replace-main-with-cgclif-experiments
```
Afterwards confirm `git remote get-url origin` still has no token and
`.git/config` has no `ghp_` string. Pushed this way: `17e2c56..63fd27a`
(PR #10 head, 42 commits, 923 files, mergeable, clean).

Deleting main's contents also deleted its root `LICENSE` and `README.md`.
Both are restored: `LICENSE` byte-identical to main's (Apache-2.0, and the same
bytes as `cg_clif_mod/LICENSE-APACHE`), `README.md` rewritten for the
experiment repo. `git merge-tree --write-tree origin/main <branch>` now equals
the branch tree exactly, so the merge keeps them. The old rmc README stays in
main's history.
