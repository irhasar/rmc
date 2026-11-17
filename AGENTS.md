# AGENTS.md

## Repo purpose
Experiments comparing Rust codegen backends: LLVM (`rustc`) vs
`rustc_codegen_cranelift` (cg_clif) upstream vs a modified cg_clif fork.

## Layout
- `REPORT.md` — findings, numbers, root causes, recommendations.
- `build.sh` — rebuilds the modified cg_clif + its Cranelift std sysroot.
- `cg_clif_upstream/` — pristine cg_clif (commit 42ac63e).
- `cg_clif_mod/` — fork with the modifications (see commits
  `b6c94e4`…`4290fb6`).
- `bench/` — harness:
  - `compare3.sh` — 3-way benchmark (LLVM / cg_clif up / cg_clif mod) across 5
    sysroot configs; prints `C`=compile, `S`=size, `R`=run metrics.
  - `sizes2.sh` — sizes across opt-levels and stripped.
  - `measure.py` — wall/CPU/peak-RSS via `os.wait4` rusage.
  - `progs/p1..p6*.rs` — workloads (p5 inline, p6 scalar are the decisive microbenchmarks).

## Build environment
- Toolchain: `nightly-2026-11-17` (must match, cg_clif pins it).
- Need `rust-src` and `llvm-tools`. `$HOME/.cargo/bin` on PATH.
- Build: `build.sh` → `cg_clif_mod/dist/bin/rustc-clif`.
- Rebuilding Cranelift std takes ~1–2 min incremental.

## Key facts learned
- Cranelift is **Apache-2.0 WITH LLVM-exception**; forking/reworking is fine.
- cg_clif compiles ~20–45% faster than LLVM but produces ~26% larger binaries
  and (upstream) runs 2–9× slower; the modified fork closes most of that gap
  (p6: 902→61 ms; p2: 156→122 ms).
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
- `4290fb6` **borrow-aware SSA** (`src/analyze.rs`): a mutable/raw borrow now only
  forces its local to a stack slot when the borrow points *into* that local's own
  storage. A borrow that dereferences the local first (`&mut *p`, `&raw mut *p`)
  merely reads it, so the local may stay in a register. Before this, cg_clif
  emitted `_30 = &raw mut (*_30)` for a `Vec` indexing loop, which pinned the
  whole `Vec` header (ptr/len/cap) to the stack and reloaded it every iteration.
  mem micro-kernel 110→65 ms; p2_mem 156→122 ms. This supersedes the earlier
  note that extending SROA to `&mut`/`RawPtr` is always a miscompile: it is only
  unsafe for whole-local borrows, not for deref-through borrows.

## What did NOT work / open leads
- Extending SROA to **`&mut`/`RawPtr` *whole-local* borrows** (with `local_map`
  redirect) is a **miscompile** (p1/p3/p4 wrong). Shared-only is correct.
  (The `4290fb6` change is different and safe: it only relaxes *deref-through*
  borrows, where the local is read, never written, through the reference.)
- Remaining runtime gap on p2–p5 (LLVM 17–58 ms vs mod 106–365 ms) is **not**
  loops and **not** the `Vec` header anymore:
  - p2_mem is dominated by `HashMap::entry`/hashbrown: a bare
    `for i { m.entry(i % 65536).or_insert(0) }` loop is 55 ms (LLVM) vs 672 ms
    (mod), ~12×. The hashbrown probe loop spills almost every temporary to the
    stack (~20 slots, e.g. `movdqu xmm→stack` before `movemask`), so the gap is a
    pervasive mem2reg/aggregate-SSA limitation of cg_clif's MIR lowering, not a
    single missing pass.
  - p3_calls splits between `fib` recursion (LLVM rewrites the binary recursion
    into one recursive call per step; cg_clif emits both calls) and `format!`.
  - p5_inline/p6_scalar: same loop, but p5's iterator keeps an `Option<u64>` on
    the stack (mem2reg), blocking reassociation.
  - p4_simd has no `Vec`; it is plain memory-heavy code (spills/bounds checks).
  - Raising the inliner budget (`CG_CLIF_INLINE_MAX=256`) buys only ~4% on p2 and
    ~5% on p4, so cross-CGU inlining is not the lever.
- The inliner is currently redundant: MIR inlining already folds the small
  helpers it targets, so it is correct but net-neutral on size/speed.
- **Body-duplicating unrolling does not help.** Measured slower on p6 (357→423 ms):
  the win LLVM gets on p5/p6 is *affine reassociation* of the accumulator chain,
  not unrolling per se. Duplicating the body leaves the loop-carried dependency
  chain intact. Implemented anyway behind `CG_CLIF_UNROLL_CLONE` for completeness.
- Remaining runtime gap on p2–p5 (LLVM 17–58 ms vs mod 106–365 ms) is memory
  promotion: cg_clif spills aggregate temporaries to the stack where LLVM keeps
  them in registers (see the per-program breakdown above).

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

