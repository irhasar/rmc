# AGENTS.md

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
    caller (TLS relocations went out of range when the guard was lifted). The
    `body.global_values.is_empty()` guard in `inliner.rs` is load-bearing.
- Extending SROA to **`&mut`/`RawPtr` whole-local borrows** (with `local_map`
  redirect) is a **miscompile** (p1/p3/p4 wrong). Shared-only is correct.
- The inliner is currently redundant: MIR inlining already folds the small
  helpers it targets, so it is correct but net-neutral on size/speed.
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
