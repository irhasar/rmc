# AGENTS.md

## Session (2026-11-20, latest) — `unroll` pass was quadratic (LANDED, `f6d2ae5`)
- **The per-pass instrumentation had never worked.** `passtime::report()` had no
  caller, and its accumulator was `thread_local!` while codegen runs on rustc's
  worker threads, so every number it could have printed was zero. Fixed both:
  `report("aot")` is now called from `codegen_crate`, and the accumulator is a
  shared `static [AtomicU64; 5]`. Enable with `CG_CLIF_PASS_TIME=1`.
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
  to compile `p2_mem` (148 vs 130 ms). That is the honest price of four added
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
- **PR**: #10 (`replace-main-with-cgclif-experiments` → `main`), head `fd9396a`,
  `mergeable: true`, 888 files changed. Description updated with the §17
  compile-time findings.

## Session (2026-11-19) — small-switch compare chain (LANDED, `6cefd14`)
- **Methodology trap (important)**: `cargo build` in a driver crate does NOT use
  the Cranelift backend. You must set `RUSTC=rustc-clif` (with
  `cg_clif_mod/dist/bin` on PATH), or you silently benchmark LLVM. Tell-tale
  sign: binary ~579 KB (LLVM) instead of ~1.63 MB (Cranelift), and identical
  md5 across tunable values. All §9 numbers in REPORT.md use the correct build.
- **Disproved**: autovectorization is NOT the blur gap. LLVM with
  `-Cno-vectorize-loops -Cno-vectorize-slp` gives 763 insts / **0 SIMD** and
  still takes 1248 ms vs 1246 ms vectorized (551 insts / 71 SIMD). Also
  disproved: `CG_CLIF_UNROLL` (1/4/16/128 → identical binary; the pass only
  handles single-block constant-delta loops) and raising the inliner caps
  (10380 vs 10359 ms, size grew).
- **Root cause found**: `cranelift_frontend::Switch::emit` picks `br_table` for
  any contiguous case range, so `match i { 0=>r, 1=>g, 2=>b }` becomes a jump
  table. Binary had **2714** jump-table sites vs LLVM 0.
- **Fix** (`cg_clif_mod/src/base.rs`, `SwitchInt`): ≤8 cases and discriminant
  ≤64 bits → `icmp_imm_s` + `brif` chain. The 64-bit guard is REQUIRED:
  `icmp_imm_s` takes an i64 immediate, so i128/f128 discriminants truncate
  (that bug failed 13 coretests).
- **Measured**: blur 10329 → 10004 ms (**-3.1%**), binary 1629168 → 1500000 B
  (**-7.9%**), jump tables 2714 → 84. p1–p8 matrix did not regress (run 525 vs
  528 ms, compile 715 vs 719 ms).
- **Tests**: `./y.sh test --sysroot clif` → **exit 0, 0 failures across 65
  suites** (6021 tests). Output byte-identical to LLVM.
- **Push**: `GITHUB_TOKEN`/`OH_GIT_CREDENTIAL_ACCESS_TOKEN_GITHUB` are read-only
  (403). Use `GH_PAT` with the explicit helper reset:
  `git -c credential.helper= -c credential.https://github.com.helper= -c "credential.https://github.com.helper=!f(){ echo username=irhasar; echo password=$GH_PAT; };f" push origin <branch>`
- **Remaining gap** (~8.3x on blur) is `cranelift-codegen` quality: loop
  rotation / regalloc spilling (hot fn: 240 `(%rsp)` refs vs LLVM 66; LLVM
  concentrates 60.6% of samples at one offset, Cranelift spreads over 213).
  Out of scope for this fork.

## Session (2026-11-17) — trivial-leaf inliner exemption (LANDED, `4d046cc`)
- **Root cause**: `MAX_CALLEE_USES = 3` applied to *all* callees, including
  one-operation bodies. `<f32 as Mul>::mul`, `<f32 as Add>::add`, and
  `__iterator_get_unchecked` (1 and 12-13 CLIF insts) stayed out-of-line after
  3 uses, so hot loops kept real calls, each forcing spill/reload around it.
- **Fix**: leaves with `<= TRIVIAL_LEAF_INSTS` (16) insts are exempt from BOTH
  the use cap and the growth-budget charge. Safe: one op, and a leaf has no
  callees so no cascading; recursion still impossible. Knobs:
  `CG_CLIF_INLINE_TRIVIAL_MAX`, `CG_CLIF_INLINE_NO_TRIVIAL` (A/B off).
- **Threshold 8 is too low**: `__iterator_get_unchecked` is 12-13 CLIF insts.
  Bisect on `filter_symmetric_column` calls: 8/9/11 -> 8 calls; 13/16/32 -> 0.
  Runtime 15.27 s (t=8) -> 12.80 s (t=16); 32 is a plateau. Default = 16.
- **Measured** (image blur 2048px, same compiler binary, knob only):
  22.323 s -> 12.790 s (**1.75x**), binary 1.647.648 -> 1.627.864 B, output
  identical. `filter_symmetric_row`: 581 insts/37 calls -> 359/**9** calls,
  now *smaller* than LLVM (446 insts/9 calls). LLVM still wins via `mulps`
  (9 vs 0).
- **Tests**: `./y.sh test --sysroot clif` -> 25,368 passed, 0 failed.
  `bat` + `tokei` rebuild and run; tokei output identical per-file (md5 differs
  only by `reports` array order from hashmap iteration).
- **No suite regression**: baseline vs final all within noise. p7_branchy
  +9% is machine drift — the trivial-OFF config alone now measures 62 ms
  where the old log said 55 ms.
- **Corrects report 8.20**: that section claimed the remaining runtime gap was
  "not the inliner, must be inside cranelift-codegen". Wrong here — the
  inliner IS a cg_clif pass and closed 42% of the blur gap. What remains in
  cranelift-codegen is (a) vectorization, (b) regalloc spilling (column filter
  still 240 `(%rsp)` refs vs LLVM 66 with zero calls left).
- **Docs**: REPORT.md 8.27 (this change) + 8.28 (final 3-way matrix).
- **Gotcha**: `perf` is not installed in this container; use objdump/nm
  symbol-size analysis instead of profiling.

## Session (2026-11-18, latest) — if-conversion diamond → select/cmove (LANDED locally)
- New pass in the fork: `cg_clif_mod/src/if_convert.rs`. Collapses a short,
  side-effect-free two-way diamond (`brif` head -> then/else -> common join)
  into a `select` in the head, which Cranelift lowers to `cmove`. Runs
  **before** `unroll` (slot `IF_CONVERT` in `passtime.rs`; knob
  `CG_CLIF_NO_IFCONV`, default ON). Fork commit `7374000` (+ `8f88af2` docs).
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
- **Tests**: `./y.sh test --sysroot clif` -> 65 `test result: ok`, 0 failed.
- **Deliverable**: `rmc_clone` subtree commits `341a286` (change) + `5bce6fa`
  (docs) on `replace-main-with-cgclif-experiments`; REPORT.md 8.22; new bench
  programs `p7_branchy`, `p8_minmax`.
- **PUSHED**: remote `replace-main-with-cgclif-experiments` = `f084efe`; PR #10
  head `f084efe`, `mergeable: clean`, base `main`. The injected `GITHUB_TOKEN`
  (read-only GitHub App) and credential-helper token both fail; the push used a
  user-supplied `repo`-scoped PAT (not stored).

## Session (2026-11-18, latest) — affine unroll factor 8 → 128 (LANDED locally)
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
- **The toolchain is not always present.** Some sandbox restarts come back with
  no `rustc`, `cargo`, `rustup`, or `$HOME/.cargo` at all (`find / -name rustc`
  returns nothing), while the prebuilt `dist/` trees survive. Measurements that
  need a fresh `rustc` then cannot be reproduced; the prebuilt binaries under
  `cg_clif_{mod,upstream}/dist/bin/rustc-clif` still run. Check this before
  planning a rebuild, and read numbers off `bench/out/` artifacts instead
  (`stat -c%s` per program sums to the same totals the report cites).
- Git remotes may prompt for credentials and hang for the full timeout. Push with
  an explicit token URL and `-c credential.helper=''` rather than letting the
  prompt block; then `git update-ref refs/remotes/origin/<branch> <sha>` to keep
  the local remote-tracking ref honest, since `git fetch` will not refresh it.
- API writes to PRs fail with `Resource not accessible by integration` under the
  injected `GITHUB_TOKEN`; a user-supplied PAT is required for `PATCH /pulls/N`.

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
  Re-confirmed 2026-11-19: `POST /repos/irhasar/rmc/git/blobs` returns 403 while
  `GET /repos/irhasar/rmc` reports `push: true` — the repo permission and the
  token's granted scope disagree, so the 403 is a token-scope limit, not a
  repo-permission one. A user-supplied classic PAT with `repo` scope does work;
  pass it inline and reset the remote afterwards so it is not persisted:
  `git -c credential.helper= push https://<user>:<PAT>@github.com/irhasar/rmc.git
  HEAD:replace-main-with-cgclif-experiments`, then verify `grep -c ghp_ .git/config`
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
- **Push is no longer blocked**: it landed with a user-supplied `repo`-scoped
  classic PAT. The earlier failures were *not* only a read-only token: passing
  `-c credential.helper=` does **not** clear the global
  `credential.https://github.com.helper` in `~/.gitconfig`, so the read-only
  helper still answered first and every attempt returned 403. Working recipe
  (token from env, never in the remote URL, so `.git/config` stays clean):

      cd rmc_clone && GIT_TERMINAL_PROMPT=0 git \
        -c credential.helper= \
        -c credential.helper='!f() { echo username=<user>; echo "password=$GH_PAT"; }; f' \
        push origin replace-main-with-cgclif-experiments

- **Pushed**: `c64e31b` is on the remote; PR #10 is `mergeable_state: clean`.
  Confirmed via the contents API that `cg_clif_mod/src/intrinsics/mod.rs` on
  the remote contains the native `round` lowering.

## Session 2026-11-19 (later) — the trunc libcall finding

**Biggest single win so far: −15.8% runtime on the image blur benchmark, and
fully portable (no `-Ctarget-cpu=native` needed).**

### The finding

`ftrunc` only lowers to `roundss`/`roundsd` when the target has **SSE4.1**
(`vendor/cranelift-codegen-0.137.0/src/isa/x64/lower.isle`, the `x64_round_*`
rules). On the x86-64 baseline it falls through to a libcall into
`compiler_builtins`' soft-float `truncf`. `round` in cg_clif was built on
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
   differential** test is valid: hash the results in a CLIF-built binary and in
   an LLVM-built binary, then compare the hashes.
3. **A 2^32-iteration test must stream.** Buffering one line per f32 pattern
   into a `String` OOM-killed the sandbox and wiped `~/.cargo` + `~/.rustup`.
   Fold results into a running hash instead. Recovery: reinstall rustup, then
   `rustup toolchain install nightly-2026-11-17 --profile minimal -c
   rust-src,rustc-dev,llvm-tools,rustfmt` (per `rust-toolchain.toml`).

### The fix

`trunc_via_int` in `cg_clif_mod/src/intrinsics/mod.rs`: `fcvt_to_sint_sat` →
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
| CLIF baseline | 10257.5 | 1500000 |
| CLIF + small-switch | 10002.5 | 1500000 |
| CLIF + small-switch + trunc | **8422.0** | **1499568** |
| CLIF + `-Ctarget-cpu=native` | 8241.8 | 1498832 |

`compiler_builtins` went from ~13% of profile samples to **zero**.

Verified: differential hash identical to LLVM (`4de7d090cdfce77d`) over all
2^32 f32 patterns plus an f64 sweep; `./y.sh test --sysroot clif` → 65 suites,
0 failures; benchmark output byte-identical at n = 512/1024/2048.

### Remaining gap

8422 ms vs 1238 ms LLVM. Now dominated by codegen quality: `try_from_fn`,
`wrap_mut_1`, `Pixels`, `Zip` never inlined (~18% of samples) and register
spilling in the hot loop. §9.9's claim that this was "out of scope" is wrong —
`cranelift-codegen` is vendored and patchable.

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
`cg_clif_mod/src/intrinsics/mod.rs` on the remote branch.

### Follow-up: the remaining gap is auto-vectorization, not inlining

Re-profiled the final binary and then compared disassembly of the hot function
`filter_symmetric_column` across both backends (same baseline x86-64 target):

| metric | LLVM | CLIF (mod) |
|---|---|---|
| total instructions | 904 | 733 |
| packed SIMD (`ps`/`pd`) | **75** | **1** |
| breakdown | `movups`×34, `mulps`×17, `addps`×10, `shufps`×6, `xorps`×4, `movaps`×4 | `xorps`×1 |

LLVM vectorizes the convolution 4 `f32` lanes at a time; Cranelift emits fully
scalar code. **Cranelift has no loop vectorizer** — only local CLIF-level
patterns (`simd_*`, i.e. hand-written `std::simd`), not automatic loop
analysis. Closing this needs a real loop vectorizer in `cranelift-codegen`
(dependence analysis, widening, tail handling), which is months of work and
out of scope. `-Ctarget-cpu=native` only buys ~2% (8241.8 vs 8422.0 ms), which
confirms SSE2 was always available — nothing uses it without a vectorizer.

**Inlining is empirically saturated** — don't re-open that lead. Raising the
existing tunables makes things worse:

| config | runtime (ms) |
|---|---|
| default | 8440 |
| `CG_CLIF_INLINE_MAX=200` | 8533 |
| `CG_CLIF_INLINE_LEAF_MAX=200` | 8451 |
| `CG_CLIF_INLINE_USES=10` | 8564 |

So the gap is now fully explained: §9 (small switch) + §10 (trunc libcall) are
landed wins; the rest is architectural. Documented as §11 in REPORT.md.

### Session: floor/ceil were the same libcall class as trunc

`floor`, `ceil`, `nearest` and `trunc` **all** reach the same `x64_round` rules
in `vendor/cranelift-codegen-*/src/isa/x64/lower.isle`, which select a libcall
when `has_sse41` is false. Fixing only `trunc`/`round` in the previous session
left `floor` and `ceil` as soft-float calls. Measured on a compute-bound loop
they were **6.3x / 6.6x** slower than LLVM; both now match.

Two measurement mistakes cost real time here — avoid both:

1. **`RUSTC=rustc-clif rustc ...` does nothing.** `RUSTC` is read by *cargo*,
   not by `rustc`. Both "backends" were LLVM and the binaries were
   byte-identical (`md5sum` proves it). Always invoke `rustc-clif` directly, and
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

## LICM in cranelift-codegen

`vendor/cranelift-codegen-0.137.0/src/licm.rs` is the only loop pass in the
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

Two knobs exist for bisecting: `CG_CLIF_NO_LICM` and `CG_CLIF_NO_COLLAPSE`
(the unroller's forwarding-latch collapse, which only unblocks one extra loop
and is runtime-neutral).

Reading env vars inside a per-function pass is measurable: `std::env::var`
locks a global map on every call. Cache every knob in a `OnceLock`, as
`src/tunables.rs` does. The vendored crate cannot see that module, so `licm.rs`
keeps its own cached copies.

Measurement notes: `/usr/bin/time` does not exist in this image — use
`bench/measure.py`. Running `./y.sh test` rebuilds `dist/`, so any binary
compiled from `dist/bin/rustc-clif` disappears mid-run; never benchmark while a
test suite is in flight.

### Never trust a stale `dist/`

`rustc-clif` falls back to the LLVM sysroot when its own sysroot has no
`libstd`, and it does so **silently**. The only symptom is that the "Cranelift"
binary is ~4.6 MB and behaves exactly like LLVM. This invalidated a whole
benchmark matrix once (upstream looked 12% smaller than it is).

Before trusting any U-vs-L row, check all three:

```bash
ls cg_clif_upstream/dist/lib/rustlib/*/lib/libstd-*.rlib   # must exist
ls cg_clif_mod/dist/lib/rustlib/*/lib/libstd-*.rlib        # must exist
stat -c%s /tmp/fb/p1_arith_U /tmp/fb/p1_arith_L             # must differ
```

`fullbench.sh` passes no `--sysroot`, which is correct — each `rustc-clif`
must use its own. Do not add one; pointing `--sysroot` at
`dist/lib/rustlib/<triple>` produces `E0463: can't find crate for std`.
Rebuild with `./y.sh build --sysroot clif` in both trees before measuring.
