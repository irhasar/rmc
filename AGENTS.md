## Project kickoff (2026-11-24, branch `rakit-exp5`) — p4 loop autovectorizer (milestone plan)

Started "opsi 2" (the big backend project) targeting `p4_simd` (M ~83 ms vs LLVM
~57 ms). p4 has three loops; only two are vectorizable:
- L1 `a[i]=sin(i as f32); b[i]=cos(i as f32)` — **libcall-bound, not
  vectorizable**; sincos fusion already helps. Ignore.
- L2 `s += a[i]*b[i] + abs(a[i]-b[i])` — elementwise + horizontal reduction.
- L3 `c[i] = (a[i].to_bits() ^ b[i].to_bits()).wrapping_mul(2654435761)` —
  elementwise map, `u32`. Read Rakit CLIF now: the loops **are unrolled 128x**
  (huge single-block bodies) but remain **scalar** — LLVM's win is real SIMD
  (`mulps/addps/andps/pmulld`), not unrolling.

**Why this is a project, not a pass:** a correct loop vectorizer needs, in
order:
1. **Alias/no-aliasing proof** for the three distinct `Vec` data pointers
   (`a`,`b`,`c` in L2/L3). Without `noalias` the loads/stores of different Vecs
   may alias; a vectorized read-ahead or reordered store would be wrong. Rust
   does mark `&mut` `noalias`, but these are raw `Vec` derefs through bounds
   checks — the evidence isn't in the CLIF today. (This is the same wall as
   §25 bounds-check elimination.)
2. **Bounds-check elimination** for `for i in 0..n { v[i] }` with `v.len()==n`:
   §25 showed the guard compares the `Range::end` block param against a
   *separately reloaded* `v.len()`, so the two must first be CSE'd (needs a
   load-value-numbering that proves no store to the Vec header between them).
3. **Loop legality + widening**: single induction var (step 1), pure elementwise
   body, no calls/traps, contiguous access, then build `i32x4`/`f32x4`
   loads/stores, splat the constant, vector `bxor`/`imulx`/`fmulx`/`fabsx`, and
   a horizontal reduce for L2 **only if it preserves the f32 addition order**
   (see the ordering caveat below); scalar tail for `n % lanes`.
4. **Codegen support check**: `pmulld` is SSE4.1 — must be feature-gated;
   verify vector `bxor`/`imul` (`pmulld`) lower in `rakit-codegen`.

**Float-reduction ordering is a hard constraint (not optional).** p4's L2 is
`s += a[i]*b[i] + (a[i]-b[i]).abs()` on `f32`. Rust does **not** permit
reassociating floating-point additions, so a lane-wise (horizontal) reduction
that regroups them is **not a legal transform** — it can change the printed
result. L2 may only be vectorized in a way that keeps the exact addition order
(e.g. no regroup across lanes), or left scalar. The safe first real target is
**L3** (`u32` xor-mul), which has no ordering constraint; L1 (sin/cos) is
libcall-bound.

**Milestones (each its own commit, each validated by the full gate: build,
15/15 stdout identical to LLVM, suite 65/25,368/0, allaxes 45/45):**
- M1: load-value-numbering / redundant-load CSE + `v.len()==n` proof (enables
  bounds-check elimination) — also helps p2.
- M2: bounds-check elimination for `0..v.len()` loops (retire §25's blocker).
- M3: legality analysis + a **map-only (no reduction)** vectorizer for L3
  (`u32` xor-mul map) as the first real target (simplest body, no reduction).
- M4: L2 reduction (horizontal), tails, `f32x4`.
- M5: tune; measure vs LLVM.

Each milestone is independently landable and correctness-gated; none is
attempted blindly. Do **not** land a vectorizer without the alias proof (M1) —
reordering loads/stores of possibly-aliasing Vecs silently corrupts results.

### M1 first attempt — blocked by debug value-labels (reverted)
Wrote `rakit-opt/src/passes/load_cse.rs`: block-local redundant-load elimination
(merge a `load` with an earlier same-address/type/offset/flags `load` in the same
block when no store/call/side-effect lies between; this IR has **no volatile
flag**, so it is semantically safe in a store-free stretch). Note the egraph has **two** load-elimination paths: the
flags-gated GVN (`readonly+notrap+can_move`, which Rust `Vec` loads are not) and
a separate **alias-analysis** path (`OptResult::AliasedLoad`,
`egraph/mod.rs:549-565`) that replaces a load with an earlier load or stored
value regardless of those flags — the latter is why the block-local pass was
redundant. It
**ICE'd compiling `core`**: `change_to_alias` inserts a
`ValueLabelAssignments::Alias` entry keyed by the *removed* value
(`dfg.rs:270-271`), and the machinst value-label emitter then does
`self.value_regs[val]... .only_reg().unwrap()` on an empty reg set
(`machinst/lower.rs:1064`). Guarding on a pre-existing label is not enough —
`change_to_alias` creates the entry itself. Safe load elimination that removes a
value therefore needs the value-label machinery to follow/relocate alias labels
(or debuginfo to be dropped for the merged value), which is a real M1 sub-task,
not a drop-in pass. **Reverted** (no ICE shipped); this is the first concrete
blocker to solve for M1.
- **Resolved & then found redundant.** The ICE is fixable: after
  `change_to_alias`, drop the merged value's debug label and
  `layout.remove_inst` the now-dead load so the value-label scan never visits a
  value with no register. That builds `core` and passes output checks — but the
  resulting binaries are **byte-identical with and without the pass** on
  p1–p9 (value neutral). So the egraph/MIR already eliminates *same-block*
  redundant loads; the remaining redundancy is **cross-block** (the guard's
  length reload vs the bounds-check length reload). Block-local load CSE is
  therefore a no-op pass that only costs compile time → **reverted**. Conclusion
  for M1: it must be a **cross-block** load CSE / value numbering with dominance
  plus a no-store-on-path proof — the same alias/analysis wall as §25, i.e. the
  genuinely hard part. Do not re-add a block-local-only version.

## Session (2026-11-24, branch `rakit-exp5`) — x64 omit-FP: execution-correct but UNWIND-INVALID, RUNTIME-NEUTRAL (reverted)

- **Outcome:** implemented full omit-frame-pointer for x64 (mirroring RISC-V),
  built the whole sysroot with it, validated **execution output only** (unwind
  is NOT correct — see the unwind bullet), measured **neutral on runtime**, and
  **reverted**. This closes the biggest open runtime lever from the `rakit-exp4`
  blocker map — don't re-attempt it for runtime.
- **Recipe (execution works; unwind still needs the extra change below; kept for
  reference, reverted):**
  - `isa/x64/abi.rs` `compute_frame_layout`: `setup_area_size = if
    flags.preserve_frame_pointers() {16} else {8}`, and **pad `clobber_size +=
    8`** when omitting. Dropping the 8-byte `push %rbp` leaves the frame 8 bytes
    short of the 16-byte call-site alignment, so add the missing 8 to the
    clobber area — it is reserved *above* the spill/fixed slots, so no slot
    offset shifts.
  - `isa/x64/abi.rs` `gen_prologue_frame_setup` / `gen_epilogue_frame_restore`:
    early-return an empty vector when `!flags.preserve_frame_pointers()`.
  - **Unwind must ALSO change — the recipe as written is not a correct build.**
    Even with no frame setup, `gen_clobber_save` still emits
    `UnwindInst::DefineNewFrame` in `gen_clobber_save` (`x64/abi.rs:652`; `DefineNewFrame` at line 707) and the System V encoder
    sets `CallFrameInstruction::CfaRegister(fp)` when `mr.fp()` exists
    (`isa/unwind/systemv.rs:209-220`) — it tells the unwinder the CFA is in RBP
    even though the function never set RBP. So the omit-FP build would produce
    **wrong backtraces / broken unwinding**; a correct build must also suppress
    `DefineNewFrame`/CFA-register for non-FP frames (e.g. clear `mr.fp()`).
    Because of this the omit-FP experiment is **not fully correct**, and the
    "outputs identical" check below proves only execution, never unwind.
  - Incoming stack args RBP → RSP in **both** lowering paths (they must agree):
    `args.rs` `SyntheticAmode::finalize` (`sp_offset = tail_args_size +
    setup_area_size + clobber_size + fixed_frame_storage_size +
    outgoing_args_size; addr = rsp + sp_offset - offset`) and
    `external.rs` `From<SyntheticAmode> for asm::Amode` (`base: Gpr::RSP`, same
    `-offset`); `emit.rs` `incoming_arg_offset = tail_args_size +
    setup_area_size + clobber_size + fixed_frame_storage_size +
    outgoing_args_size`. (Same formula as `isa/riscv64/inst/args.rs:142-151`.)
- **Correctness — EXECUTION ONLY (unwind is NOT correct, see the unwind bullet
  above).** After `rm -rf build/stdlib_target`
  (the backend `.so` is not fingerprinted, so std keeps the old lowering
  otherwise) and a full rebuild, **all 15 bench outputs are byte-identical to
  LLVM**; a 10-arg `i64` fn and a 10-arg `f64` fn (stack-passed args) print
  `883500 784500`, identical to LLVM — i.e. the new RSP-relative incoming-arg
  path is right. Hot user/core fns confirmed FP-less in disasm: `fib` and
  `SipHasher::write` compile to `sub $N,%rsp; ...` with **no** `push %rbp` /
  `mov %rsp,%rbp`.
- **Runtime A/B (cpu ms, 21 reps, FP → omit-FP):** p1 100→102, p3 55→56,
  p4 83→83, p2 48→48, p8 21→20, p9 33→34. Most are within the ±3% band, **but
  p8's −4.8% (21→20) is above it**; with only 15–21 reps a single clamped A/B
  does **not** establish that as a real win — treat p8 as inconclusive/noise
  until measured with many more reps and unrounded medians. Artifact size
  ±0.0% (e.g. p2 4143368→4141728 B).
- **Why reverted:** neutral on every axis yet it carries real correctness risk
  for code we don't exercise: `emit.rs:1893` `emit_return_call_common_sequence`
  **asserts** `preserve_frame_pointers()` (so `return_call` would ICE), and
  `verifier/mod.rs:660` rejects `get_frame_pointer`/`get_return_address` without
  FP. Net: no upside, real downside — not worth landing.
- **Conclusion:** the x64 frame-pointer prologue/epilogue (~6 instructions per
  call: `push rbp; mov rsp,rbp` … `mov rbp,rsp; pop rbp`) is **not** a
  bottleneck on these workloads — they are compute/memory/vectorization bound,
  and even call-heavy `p3` (`fib`) is call-*latency* bound, so the prologue
  instructions hide under the call latency. Remaining backend levers are
  a loop vectorizer (p4), which is large.
- **Addendum — p8 is already near-optimal (so "regalloc cycle" was a red
  herring).** Disassembling `p8_minmax`'s hot loop: state `(i, a, mn)` is carried
  entirely in registers (`rax`=i, `r10`=a, `r8`=mn) with **no stack traffic** —
  `imul a*C; or $1,%rax; add a; cmp; mov %r10,%rax; cmovb %r8,%rax; mov
  %rax,%r8; mov %rcx,%rax; jmp` (~10 instrs/iter). The only copies are the
  min-reduction (`mov/cmovb/mov`), which are needed because `a` (in `r10`) must
  survive for the next iteration while `mn` is updated. So the residual p8 gap
  (M ~20 ms vs LLVM ~18 ms) is not a removable spill/cycle — it is inherent to
  the scalar recurrence; LLVM's edge is elsewhere. Do not chase a "parallel-copy
  cycle" in p8.
- **Standing scoreboard, grounded in the full matrix below (`prog | bk |
  Cwall | Ccpu | Crss | size(B) | Rwall | Rcpu | Rrss`):** M's compile is faster
  than **LLVM** but **slower than upstream cg_clif (U)** — `h_arith` Cwall U **88**
  ms vs M **100** ms. M's **artifact size is smallest** in that matrix (e.g.
  `h_arith` size L 4617128 / U 5839120 / M **4078672** B). Runtime M wins only
  **p5/p6/p7**, **loses p1/p2/p3/p4**, and the `h_*` rows are **not uniformly
  ties** — `h_mem` Rcpu L **3** ms vs M **4** ms (M loses runtime CPU to LLVM).
  Do not state "M wins compile" or "ties h_*" without the full L/U/M matrix
  (Cwall/Ccpu/Crss/size + Rwall/Rcpu/Rrss) to back it. Closing p1 (memory-operand constants + branchy saturating cast +
  serial int↔float chain), p2 (hashbrown/sort codegen depth), p3 (call-latency /
  middle-end), p4 (a real loop autovectorizer) is a **months-scale backend
  effort** comparable to LLVM's own middle-end, not an incremental pass.

## Session (2026-11-24, branch `rakit-exp4`) — runtime probe (no pass-level win), x64 omit-FP blocker map

- **Branch:** from `replace-main-with-cgclif-experiments` after PR #14 merged
  (`7ffa4b6`).
- **`p1_arith` — full all-axes (matrix below):** run wall/cpu = LLVM **81/81**,
  cg_clif upstream **108/107**, Rakit **101/101** ms. Rakit **beats upstream**
  and loses to LLVM. **Knob A/B is supplementary (not an L/U/M comparison):**
  disabling a single pass barely moves it — `RAKIT_NO_IFCONV` 101/101,
  `RAKIT_NO_DIVTEST` 105/105, `RAKIT_NO_STORE_FWD` 103/103, `RAKIT_NO_LANES`
  102/101, `RAKIT_NO_SINCOS` 101/101 (default 103/101). So the residual is
  **codegen, not a pass**: Rakit keeps constants as memory operands
  (`imul 0xf2(%rip)`, `cmp 0xe6(%rip)`, `mulsd 0xd7(%rip)`, `movabs $0x43e0...`
  reloaded each iteration) where LLVM hoists them once to registers, and the
  saturating `f as u64` is a **branchy** multi-`jmp` sequence vs LLVM's
  two-`cvttsd2si` branchless form (matches the earlier negative result:
  branchless `emit` regressed because the constants are materialised after
  LICM).
- **`p2_mem` cross-block budget is not the limiter:** full row L 17/16,
  U 157/157, M 47/49 ms; `RAKIT_SF_CBE_MAX=1e9` gives the same M 49/48 as the
  default 16384. `cross_block_entry` is already an RPO single sweep with an
  early-meet bail.
- **No RSS axis to win (from the matrix):** run peak RSS ties across backends
  (p2 11.1/11.1/10.8 MB, p1 11.1/10.8/11.1 MB); compile RSS is M ~ U and
  slightly above L (p1 106.0/108.8/109.2 MB).
- **x64 omit-frame-pointer blocker map (NOT attempted; all must be fixed
  together or it miscompiles — two earlier naive attempts segfaulted):**
  - `vendor/rakit-codegen/src/isa/x64/abi.rs:955` — `let setup_area_size = 16`
    is **hardcoded** (`// RBP, return address`); must be 8 with no frame
    pointer.
  - `isa/x64/abi.rs:538-568` `gen_prologue_frame_setup` unconditionally emits
    `push %rbp; mov %rsp,%rbp` (+ `Unwind::PushFrameRegs`); `:571-588`
    `gen_epilogue_frame_restore` unconditionally emits `mov %rbp,%rsp; pop
    %rbp`.
  - **Incoming stack arguments are RBP-relative — ordinary calls, not just
    `return_call`:** `isa/x64/inst/external.rs:338-356` lowers
    `SyntheticAmode::IncomingArg { offset }` to an `Amode::ImmReg` with
    `base: Gpr::RBP` and `simm32 = -offset`, whereas local
    `SyntheticAmode::SlotOffset` uses `base: Gpr::RSP`. So **any callee whose
    arguments spill to the stack (e.g. >6 register args, or any stack-passed
    arg) reads them via RBP**; omitting the frame pointer breaks such
    **ordinary calls**, not only `return_call`. This is **fixable by mirroring
    RISC-V**, which already lowers `AMode::IncomingArg` **SP-relative** from the
    frame layout (`isa/riscv64/inst/args.rs:142-151`:
    `sp_offset = tail_args_size + setup_area_size + clobber_size +
    fixed_frame_storage_size + outgoing_args_size`, then `sp_offset - off`).
    The earlier claim that "RSP moves within a function" is **wrong for
    clobbers/slots**: `isa/x64/abi.rs:714-729` (`gen_clobber_save`) reserves
    `fixed_frame_storage_size + clobber_size + outgoing_args_size` **once in the
    prologue** with a single `sub rsp`, then stores clobbers at fixed RSP
    offsets, so those SP offsets are constant. RSP is only adjusted
    transiently for outgoing call args and for the `tail_args_size` growth of a
    `return_call` — both folded into the RISC-V formula above.
  - `isa/x64/abi.rs:673` `gen_clobber_save` additionally does rbp-relative
    surgery when a `return_call` grows the incoming-arg area.
  - `isa/x64/inst/emit.rs:1893` `emit_return_call_common_sequence` **asserts**
    `preserve_frame_pointers()` — `return_call` needs FP (latent ICE if
    default-on and tail calls are emitted).
  - `verifier/mod.rs:660` rejects `get_frame_pointer`/`get_return_address`
    without FP (backtrace machinery).
  - `rbp` is callee-save (`is_callee_save_systemv`) and its *only* save today is
    the prologue push, so omit-FP must guarantee regalloc never allocates `rbp`
    (or save it via `gen_clobber_save`).
  - s390x/riscv64 already gate `setup_area_size`/prologue on
    `flags.preserve_frame_pointers()` (`isa/s390x/abi.rs:761,789,973`,
    `isa/riscv64/abi.rs:664`); x64 never implemented it. On x64 the extra piece
    is the SP-relative incoming-arg lowering above (RISC-V's is the template);
    the remaining gates are the `return_call` FP assert and the
    `get_frame_pointer` verifier.
- **Landed (safe, byte-identical):** the unroller's remaining per-function env
  reads (`RAKIT_UNROLL_DEBUG`, `RAKIT_LANE_DEBUG`, `RAKIT_NO_NARROW_FCVT`,
  `RAKIT_NO_LANES`) now go through cached `OnceLock` tunables, completing the
  "cache every knob in `tunables.rs`" discipline and removing global
  `std::env` map locks from the hottest per-function pass. **Neutral** on
  measurement (p2 knob A/B 164/228 vs 172/236 ms, noise; synthetic 800-function
  file 157/210 vs 157/213). The M-vs-U compile gap in the matrix (p2 compile M
  159/216 vs U 124/166) is the documented price of the extra passes, not this
  change.
- **Conclusion:** every remaining runtime gap (p1 codegen constants + branchy
  cast, p2 hash/sort codegen, p3 frame-pointer/call overhead, p4 vectorizer,
  p8 regalloc parallel-copy cycle, p9 libcall GOT reload) needs vendored
  backend work; safe pass-level micro-optimisation is exhausted.
- **Verification:** `./y.sh build` exit 0; 15/15 bench binaries md5-identical to
  the PR #14 build; `./y.sh test --sysroot rakit` -> 65 suites / 25,368 passed /
  0 failed; `bench/allaxes.sh` 45/45 rows `ok` (stdout matches LLVM).

### Full all-axes matrix (`bench/allaxes.sh`, CREPS=3 RREPS=7) — this build
L = LLVM, U = cg_clif upstream, M = modified Rakit. Compile = wall / cpu ms /
peak RSS MB / artifact bytes. Run = wall / cpu ms / peak RSS MB. All stdout
matched LLVM. Run wall is contention-sensitive; compare with run cpu.

| prog | bk | Cwall | Ccpu | Crss | size(B) | Rwall | Rcpu | Rrss |
|------|----|------:|-----:|-----:|--------:|------:|-----:|-----:|
| h_arith | L | 117 | 141 | 117.6 | 4617128 | 2 | 2 | 10.8 |
| h_arith | U | 88 | 115 | 120.0 | 5839120 | 2 | 2 | 11.1 |
| h_arith | M | 100 | 129 | 120.6 | 4078672 | 2 | 2 | 11.0 |
| h_branch | L | 118 | 143 | 118.1 | 4616832 | 2 | 2 | 11.0 |
| h_branch | U | 88 | 116 | 120.3 | 5837264 | 2 | 2 | 10.9 |
| h_branch | M | 103 | 135 | 120.4 | 4077008 | 2 | 2 | 11.0 |
| h_calls | L | 116 | 139 | 118.5 | 4616872 | 2 | 2 | 11.0 |
| h_calls | U | 88 | 115 | 120.7 | 5837792 | 2 | 2 | 11.0 |
| h_calls | M | 102 | 131 | 120.9 | 4077368 | 2 | 2 | 11.0 |
| h_float | L | 126 | 149 | 117.2 | 4637984 | 2 | 2 | 10.8 |
| h_float | U | 91 | 119 | 119.8 | 5861024 | 3 | 2 | 11.0 |
| h_float | M | 104 | 132 | 120.1 | 4118168 | 2 | 2 | 11.1 |
| h_mem | L | 260 | 419 | 137.8 | 4646848 | 3 | 3 | 10.8 |
| h_mem | U | 130 | 179 | 135.8 | 5959856 | 9 | 9 | 11.1 |
| h_mem | M | 170 | 245 | 137.1 | 4166120 | 5 | 4 | 10.8 |
| h_simd | L | 140 | 163 | 124.1 | 4617704 | 2 | 2 | 11.0 |
| h_simd | U | 98 | 127 | 125.9 | 5843736 | 4 | 3 | 11.0 |
| h_simd | M | 112 | 142 | 126.0 | 4081096 | 2 | 2 | 11.0 |
| p1_arith | L | 91 | 112 | 106.0 | 4635232 | 81 | 81 | 11.1 |
| p1_arith | U | 78 | 101 | 108.8 | 5877944 | 108 | 107 | 10.8 |
| p1_arith | M | 87 | 111 | 109.2 | 4146112 | 101 | 101 | 11.1 |
| p2_mem | L | 244 | 373 | 132.9 | 4639888 | 17 | 16 | 11.1 |
| p2_mem | U | 124 | 166 | 132.0 | 5936144 | 157 | 157 | 11.1 |
| p2_mem | M | 159 | 216 | 133.0 | 4143368 | 47 | 49 | 10.8 |
| p3_calls | L | 98 | 119 | 116.0 | 4612368 | 37 | 36 | 11.0 |
| p3_calls | U | 87 | 112 | 117.5 | 5820136 | 117 | 117 | 11.0 |
| p3_calls | M | 103 | 123 | 118.1 | 4067048 | 55 | 55 | 11.1 |
| p4_simd | L | 106 | 125 | 116.4 | 4636096 | 56 | 56 | 48.2 |
| p4_simd | U | 92 | 115 | 118.2 | 5892376 | 157 | 156 | 49.5 |
| p4_simd | M | 104 | 129 | 118.8 | 4152736 | 84 | 83 | 48.8 |
| p5_inline | L | 82 | 104 | 104.8 | 4609368 | 58 | 57 | 11.1 |
| p5_inline | U | 77 | 100 | 107.3 | 5813704 | 895 | 894 | 10.8 |
| p5_inline | M | 83 | 108 | 108.0 | 4059960 | 6 | 6 | 11.0 |
| p6_scalar | L | 81 | 99 | 101.3 | 4609368 | 58 | 57 | 11.1 |
| p6_scalar | U | 75 | 97 | 103.6 | 5813688 | 904 | 905 | 11.0 |
| p6_scalar | M | 82 | 104 | 104.1 | 4059960 | 6 | 6 | 11.0 |
| p7_branchy | L | 83 | 101 | 105.2 | 4609520 | 43 | 43 | 11.0 |
| p7_branchy | U | 77 | 100 | 107.6 | 5813800 | 100 | 100 | 10.7 |
| p7_branchy | M | 87 | 111 | 108.1 | 4060120 | 35 | 35 | 10.8 |
| p8_minmax | L | 84 | 101 | 105.1 | 4609544 | 18 | 17 | 11.0 |
| p8_minmax | U | 78 | 100 | 107.3 | 5813752 | 38 | 38 | 11.1 |
| p8_minmax | M | 85 | 107 | 107.9 | 4060088 | 20 | 20 | 10.8 |
| p9_u64f32 | L | 95 | 114 | 110.8 | 4635448 | 30 | 30 | 17.8 |
| p9_u64f32 | U | 86 | 109 | 113.7 | 5886872 | 76 | 76 | 18.8 |
| p9_u64f32 | M | 97 | 120 | 114.2 | 4149744 | 34 | 34 | 18.2 |

## Session (2026-11-24, branch `rakit-exp3`) — instrumentation + per-function guards (marginal), runtime walls re-confirmed

- **Branch:** from `replace-main-with-cgclif-experiments` after PR #13 merged
  (`f35f743`).
- **Runtime exploration (disassembly-backed, all architectural):**
  - `p8` (20 vs L 18): the lane transform already uses the efficient per-lane
    scheme (forcing advanced by a constant `slope`, 1 imul/iter). The residual
    gap is a **regalloc parallel-copy cycle** on the lane loop's 6 params: the
    back edge ends in `mov %rdx,%r13; mov %r12,%rdx; mov %r10,%r12; mov
    %r13,%r10; mov %rbx,%rcx` (a 3-cycle + 1), whereas LLVM's per-lane state
    needs no shuffles. Factor sweep F=2/4/6/8 = 21/23/22/23 ms (F=2 best). Not
    pass-fixable; it is register allocation.
  - `p9` (33 vs 30): 85% `sinf`. LLVM hoists the GOT entry to `%r15` once
    (`mov sinf@GOT,%r15`) then `call *%r15`; Rakit reloads `mov
    sinf@GOT,%rax` **every iteration**. The GOT load is emitted by ISel at the
    call site (post-CLIF), so Rakit's LICM cannot hoist it. Architectural.
  - `p1` (101 vs 82): `f += sqrt(i)*0.5` is a serial f64 add chain; LLVM
    pipelines the independent `sqrt`s by unrolling a **multi-block** body.
  - `p3` (57 vs 37): `fib` is 71%. M matches LLVM's loop shape (1 call/iter) but
    every call pays the **frame pointer** (`push rbp; mov rsp,rbp` + `mov
    rbp,rsp; pop rbp`) and spill-to-stack callee saves instead of `push`; LLVM
    omits the frame pointer (`push r14; push rbx; push rax`). This is the
    documented x64 `preserve_frame_pointers` trap (naive fixes segfault via
    `setup_area_size`).
- **Landed (safe, byte-identical — all 15 bench binaries md5-match the prior
  build; output unchanged):**
  - `passtime`: new `REC_LOOP`/`SINCOS` slots (ACC 7→9) so those passes are now
    timed and visible (`rec_loop=0.2ms sincos=0.4ms` on p2/h_mem cgu.1).
  - `RecLoopPass`/`SincosPass` adapters: check their disable switch first, run
    the pass inside the timer, and return the pass's **real** result.
    `rec_loop::run`/`sincos::run` now return whether they rewrote / fused
    anything. **No opcode pre-scan** for these two: `rec_loop::try_match` already
    rejects non-recursive functions at the entry terminator (O(1)) and
    `sincos::run` already does a single cheap walk, so a full `Call` scan in
    front would be slower for call-free straight-line functions.
  - `unroll::run`: skip `collapse_forwarders` and the loop `analyze` for
    **single-block** functions (no second block to forward to; a loop needs a
    back edge across blocks). `narrow_uint_fcvt` still runs. The
    `RAKIT_UNROLL_DEBUG` IR dump now runs **before** `narrow_uint_fcvt`, so
    single-block functions are still dumped and the conversion diagnostics
    `narrow_uint_fcvt` prints have the function header above them.
  - Review follow-ups: `rec_loop::try_match` now checks `is_self(fr1)` right
    after the first `call_sub`, before scanning the second arm (logically
    identical; `call_sub` reads `jump_target` first, so `return`-terminated
    shapes bail O(1) with **no** instruction scan). `sincos::run` site discovery
    iterates the layout directly instead of snapshotting blocks/insts into
    `Vec`s (no per-function allocation, call-free ones included).
- **Honest measurement:** these guards are **marginal**. A synthetic file with
  **800** single-block leaf functions compiles 144/197 ms with the guards vs
  145/198 ms without (~1 ms, within noise) — tiny functions are cheap per pass;
  the pass cost lives in the few large multi-block functions (store_forward
  ~9 ms is genuine work). The 15-program bench compile is unchanged.
- **Conclusion:** safe compile micro-optimisation is exhausted; the remaining
  runtime gaps need backend work (x64 omit-frame-pointer done correctly,
  regalloc coalescing, a loop vectorizer, GOT-load hoisting) — each a
  multi-session, correctness-critical change, not a pass-sized tweak.
- **Verification:** `./y.sh build` exit 0; `bench/allaxes.sh` 45/45 `ok`;
  `./y.sh test --sysroot rakit` → 65 suites / 25,368 passed / 0 failed.

### Full all-axes matrix (`bench/allaxes.sh`, CREPS=3 RREPS=7)
L = LLVM, U = cg_clif upstream, M = modified Rakit. Compile = wall / cpu ms /
peak RSS MB / artifact bytes. Run = wall / cpu ms / peak RSS MB. All stdout
matched LLVM. Run wall is contention-sensitive; compare with run cpu.

| prog | bk | Cwall | Ccpu | Crss | size(B) | Rwall | Rcpu | Rrss |
|------|----|------:|-----:|-----:|--------:|------:|-----:|-----:|
| h_arith | L | 119 | 145 | 117.7 | 4617128 | 2 | 2 | 11.0 |
| h_arith | U | 92 | 117 | 120.2 | 5839120 | 2 | 2 | 11.1 |
| h_arith | M | 103 | 134 | 120.8 | 4078672 | 2 | 2 | 10.8 |
| h_branch | L | 126 | 142 | 118.0 | 4616832 | 2 | 2 | 10.8 |
| h_branch | U | 92 | 119 | 120.2 | 5837264 | 2 | 2 | 10.8 |
| h_branch | M | 101 | 130 | 120.6 | 4077008 | 2 | 2 | 11.0 |
| h_calls | L | 119 | 140 | 118.5 | 4616872 | 2 | 2 | 11.1 |
| h_calls | U | 89 | 118 | 120.7 | 5837792 | 2 | 2 | 11.0 |
| h_calls | M | 101 | 127 | 120.9 | 4077368 | 3 | 2 | 11.1 |
| h_float | L | 127 | 150 | 116.9 | 4637984 | 2 | 2 | 11.1 |
| h_float | U | 90 | 126 | 119.9 | 5861024 | 3 | 2 | 11.1 |
| h_float | M | 106 | 139 | 119.9 | 4118168 | 2 | 2 | 11.1 |
| h_mem | L | 265 | 428 | 137.4 | 4646848 | 3 | 3 | 11.0 |
| h_mem | U | 140 | 189 | 136.2 | 5959856 | 9 | 9 | 11.1 |
| h_mem | M | 174 | 240 | 137.4 | 4166120 | 5 | 4 | 11.1 |
| h_simd | L | 142 | 167 | 124.0 | 4617704 | 2 | 2 | 11.1 |
| h_simd | U | 99 | 128 | 125.7 | 5843736 | 4 | 3 | 11.1 |
| h_simd | M | 111 | 141 | 126.4 | 4081096 | 3 | 2 | 11.0 |
| p1_arith | L | 86 | 104 | 105.7 | 4635232 | 82 | 81 | 11.1 |
| p1_arith | U | 79 | 101 | 108.8 | 5877944 | 108 | 108 | 11.0 |
| p1_arith | M | 90 | 112 | 109.5 | 4146112 | 101 | 100 | 11.1 |
| p2_mem | L | 244 | 378 | 133.9 | 4639888 | 17 | 16 | 11.0 |
| p2_mem | U | 131 | 173 | 132.2 | 5936144 | 155 | 156 | 10.8 |
| p2_mem | M | 160 | 220 | 133.5 | 4143368 | 48 | 48 | 10.8 |
| p3_calls | L | 103 | 129 | 116.2 | 4612368 | 37 | 36 | 11.0 |
| p3_calls | U | 86 | 114 | 117.6 | 5820136 | 116 | 115 | 10.8 |
| p3_calls | M | 95 | 120 | 118.1 | 4067048 | 56 | 56 | 10.8 |
| p4_simd | L | 108 | 129 | 116.6 | 4636096 | 56 | 56 | 48.2 |
| p4_simd | U | 95 | 120 | 118.2 | 5892376 | 156 | 156 | 49.5 |
| p4_simd | M | 106 | 129 | 118.7 | 4152736 | 84 | 83 | 48.8 |
| p5_inline | L | 82 | 103 | 104.7 | 4609368 | 58 | 57 | 11.0 |
| p5_inline | U | 79 | 102 | 107.5 | 5813704 | 892 | 891 | 11.1 |
| p5_inline | M | 84 | 107 | 108.0 | 4059960 | 7 | 6 | 11.1 |
| p6_scalar | L | 84 | 103 | 101.3 | 4609368 | 58 | 57 | 10.8 |
| p6_scalar | U | 77 | 98 | 103.7 | 5813688 | 902 | 902 | 11.1 |
| p6_scalar | M | 85 | 111 | 104.1 | 4059960 | 6 | 6 | 11.0 |
| p7_branchy | L | 85 | 105 | 104.9 | 4609520 | 43 | 42 | 10.8 |
| p7_branchy | U | 79 | 107 | 107.6 | 5813800 | 101 | 100 | 11.1 |
| p7_branchy | M | 86 | 111 | 108.4 | 4060120 | 36 | 36 | 11.1 |
| p8_minmax | L | 84 | 104 | 105.2 | 4609544 | 18 | 17 | 11.1 |
| p8_minmax | U | 79 | 101 | 107.2 | 5813752 | 39 | 40 | 10.8 |
| p8_minmax | M | 86 | 110 | 108.2 | 4060088 | 20 | 20 | 11.1 |
| p9_u64f32 | L | 95 | 114 | 110.7 | 4635448 | 30 | 30 | 17.6 |
| p9_u64f32 | U | 87 | 110 | 113.8 | 5886872 | 77 | 76 | 18.8 |
| p9_u64f32 | M | 96 | 120 | 114.2 | 4149744 | 34 | 33 | 18.2 |

## Session (2026-11-24, branch `rakit-exp2`) — opcode pre-scans for cheap passes

- **Branch:** created from `replace-main-with-cgclif-experiments` after PR #12
  merged (`cb273bf`).
- **Runtime exploration first (all architectural):**
  - `p8` lane-factor sweep on the current tree: F=2/4/6/8 → **21/23/22/23 ms**
    (L 18). F=2 still best; ≥4 spills. The residual gap is the forcing term being
    recomputed per super-iteration (`lea`/`add`) instead of advanced by a
    constant add as LLVM does. Closing it needs the deferred per-lane carried
    state rewrite (AGENTS "LCG lane transform — next step"), not a factor tweak.
  - `p9` disasm: loop is `xorps;cvtsi2ss;call sinf;` then reloads `a.ptr`/`a.len`
    from `0x8/0x10(%rsp)` every iteration (regalloc keeps loop-invariants on the
    stack) plus a `mov sinf@GOT` per iter. 85% is `sinf`; the reloads are L1 hits —
    the documented wall, no pass-sized win.
  - `p1` disasm: `f += sqrt(i)*0.5` is a serial f64 add chain; LLVM pipelines the
    independent `sqrt`s by unrolling a multi-block body. Needs a multi-block
    unroller, out of scope.
- **Landed: opcode pre-scans in the pass adapters** (`passes/mod.rs`). Each cheap
  pass did expensive setup (a use-count map, a CFG build, callee-name
  resolution) for *every* function before finding nothing. New `has_opcode`
  helper (opcode-only walk) gates them on a necessary condition:
  - `div_test`: skip unless the function has an `Opcode::Urem`.
  - `if_convert`: skip unless it has ≥3 blocks **and** an `Opcode::Brif`.
  - `sincos`: skip unless it has an `Opcode::Call`.
  All three are necessary conditions (no `urem`/`brif`/call ⇒ the pass cannot
  fire), so the IR is unchanged. The disable switches (`RAKIT_NO_DIVTEST`,
  `RAKIT_NO_IFCONV`) are checked **before** the scan, so a disabled pass costs
  nothing; and the pre-scan runs **inside** the `passtime::time` region so the
  reported per-pass time includes the scan. The adapters return the optimizer's
  own result: `div_test::run` and `if_convert::run` now return whether they
  actually rewrote (so a function with no `urem` candidate or no convertible
  diamond yields `false`), and a prefilter rejection also returns `false` — so
  `Pass::run`'s "did the function change" answer is truthful. `passtime::enabled()`
  memoises `RAKIT_PASS_TIME` in a `OnceLock`, so the extra `time()` calls for
  skipped functions do not re-read the environment in a normal build.
  - **Byte-identical:** all 15 bench programs md5-match the pre-change build.
  - **A/B compile (median, reps=15, same session):** `p2_mem` 158→**155** wall
    (cpu 217→217), `h_mem` 168→**165** (238→233), `p4_simd` 107→**105**
    (132→128). Scan-inclusive `div_test` per-pass on `p2_mem`/`h_mem` cgu.1:
    1.7/2.1 → **0.6** ms (the 0.4 quoted earlier excluded the scan).
- **Measurement caveat (wall vs cpu):** **runtime is unchanged on CPU time**;
  wall time is sensitive to machine contention and must be read with CPU time.
  One allaxes pass showed `p1_arith` M at **261 ms wall / 101 ms CPU** (a
  scheduling stall); an isolated 20-rep re-measure gives **101/100 ms** and the
  clean re-run matrix below shows 101/100. Judge runtime by CPU (and by a clean
  re-run), not a single wall sample.
- **Verification:** `./y.sh build` exit 0; `./y.sh test --sysroot rakit` → 65
  suites / 25,368 passed / 0 failed; `bench/allaxes.sh` (CREPS=3 RREPS=7) →
  45/45 rows `ok`. Runtime (CPU) and sizes unchanged (matrix below).

### Full all-axes matrix (`bench/allaxes.sh`, CREPS=3 RREPS=7)
L = LLVM, U = cg_clif upstream, M = modified Rakit. Compile = wall / cpu (user+sys)
ms / peak RSS MB / artifact bytes. Run = wall / cpu ms / peak RSS MB. All stdout
matched LLVM. (Run wall is contention-sensitive; compare with run cpu.)

| prog | bk | Cwall | Ccpu | Crss | size(B) | Rwall | Rcpu | Rrss |
|------|----|------:|-----:|-----:|--------:|------:|-----:|-----:|
| h_arith | L | 148 | 154 | 117.7 | 4617128 | 193 | 2 | 10.8 |
| h_arith | U | 92 | 119 | 120.0 | 5839120 | 2 | 2 | 11.0 |
| h_arith | M | 104 | 132 | 120.6 | 4078672 | 2 | 2 | 11.0 |
| h_branch | L | 122 | 149 | 118.1 | 4616832 | 2 | 2 | 11.1 |
| h_branch | U | 91 | 121 | 120.3 | 5837264 | 2 | 2 | 10.8 |
| h_branch | M | 103 | 131 | 120.5 | 4077008 | 2 | 2 | 11.0 |
| h_calls | L | 118 | 143 | 118.8 | 4616872 | 2 | 2 | 11.1 |
| h_calls | U | 92 | 122 | 120.8 | 5837792 | 3 | 2 | 10.8 |
| h_calls | M | 103 | 133 | 120.8 | 4077368 | 2 | 2 | 10.8 |
| h_float | L | 128 | 152 | 117.0 | 4637984 | 2 | 2 | 10.7 |
| h_float | U | 95 | 125 | 119.7 | 5861024 | 3 | 3 | 11.0 |
| h_float | M | 103 | 132 | 120.0 | 4118168 | 2 | 2 | 11.0 |
| h_mem | L | 276 | 441 | 138.3 | 4646848 | 3 | 3 | 11.0 |
| h_mem | U | 139 | 191 | 136.1 | 5959856 | 10 | 9 | 11.1 |
| h_mem | M | 172 | 245 | 137.8 | 4166120 | 5 | 4 | 10.7 |
| h_simd | L | 145 | 168 | 124.2 | 4617704 | 2 | 2 | 11.0 |
| h_simd | U | 102 | 128 | 125.7 | 5843736 | 4 | 3 | 11.0 |
| h_simd | M | 112 | 140 | 126.2 | 4081096 | 2 | 2 | 11.0 |
| p1_arith | L | 89 | 110 | 105.8 | 4635232 | 82 | 83 | 11.1 |
| p1_arith | U | 88 | 105 | 108.6 | 5877944 | 108 | 108 | 11.0 |
| p1_arith | M | 90 | 115 | 109.4 | 4146112 | 101 | 100 | 11.0 |
| p2_mem | L | 254 | 395 | 133.9 | 4639888 | 17 | 17 | 11.0 |
| p2_mem | U | 139 | 183 | 132.3 | 5936144 | 159 | 156 | 11.0 |
| p2_mem | M | 161 | 219 | 133.6 | 4143368 | 50 | 49 | 11.0 |
| p3_calls | L | 101 | 126 | 116.2 | 4612368 | 37 | 37 | 11.0 |
| p3_calls | U | 89 | 115 | 117.8 | 5820136 | 116 | 116 | 10.7 |
| p3_calls | M | 112 | 134 | 118.7 | 4067048 | 56 | 55 | 10.8 |
| p4_simd | L | 109 | 130 | 116.6 | 4636096 | 56 | 56 | 48.2 |
| p4_simd | U | 94 | 118 | 118.2 | 5892376 | 157 | 155 | 49.5 |
| p4_simd | M | 106 | 130 | 119.0 | 4152736 | 84 | 84 | 48.8 |
| p5_inline | L | 84 | 104 | 104.8 | 4609368 | 58 | 58 | 10.8 |
| p5_inline | U | 80 | 103 | 107.4 | 5813704 | 892 | 891 | 11.0 |
| p5_inline | M | 84 | 109 | 108.0 | 4059960 | 7 | 6 | 10.8 |
| p6_scalar | L | 83 | 102 | 101.2 | 4609368 | 58 | 57 | 11.1 |
| p6_scalar | U | 77 | 98 | 103.7 | 5813688 | 903 | 901 | 10.9 |
| p6_scalar | M | 83 | 105 | 104.2 | 4059960 | 7 | 6 | 10.8 |
| p7_branchy | L | 85 | 103 | 105.2 | 4609520 | 43 | 42 | 10.8 |
| p7_branchy | U | 79 | 102 | 107.6 | 5813800 | 102 | 102 | 11.0 |
| p7_branchy | M | 88 | 109 | 108.1 | 4060120 | 36 | 35 | 11.0 |
| p8_minmax | L | 87 | 106 | 105.2 | 4609544 | 18 | 18 | 10.7 |
| p8_minmax | U | 80 | 102 | 107.3 | 5813752 | 38 | 38 | 10.8 |
| p8_minmax | M | 87 | 110 | 107.9 | 4060088 | 21 | 20 | 11.1 |
| p9_u64f32 | L | 97 | 117 | 110.9 | 4635448 | 31 | 30 | 17.8 |
| p9_u64f32 | U | 108 | 116 | 113.6 | 5886872 | 77 | 76 | 18.8 |
| p9_u64f32 | M | 99 | 124 | 114.5 | 4149744 | 34 | 34 | 18.2 |

## Session (2026-11-24, branch `rakit-further-experiments`) — compile-axis wins

- **Goal shift:** the runtime gaps left (p2/h_mem hash+sort, p4 autovectorization,
  p1 saturating f64→u64) are the documented architectural wall, so this branch
  targets the **compile axis**, where M still loses to upstream U on `p2`/`h_mem`
  because the custom passes run on every function. Runtime and size are unchanged
  (full matrix below).
- **What the guards save (and don't):** only pass *execution*. The pipeline is
  `RecLoop, StoreForward, DivTest, IfConvert, Unroll, Rotate, Sincos`, and
  `DivTest`/`Unroll` always report a change, so `PassManager::run` has already
  cleared the analysis cache right before `IfConvert`/`Rotate`. A guard that skips
  those passes therefore preserves no cached CFG — `invalidate()` stays
  unconditional; the saving is not rebuilding the CFG / walking bodies inside the
  skipped pass.
- **Structural pass guards** (`passes/mod.rs`): `if_convert` skips functions with
  fewer than 3 blocks (a two-way diamond needs `then_block != else_block`, i.e.
  head + two distinct arms) and `rotate` skips single-block functions (no distinct
  latch). Both are **provably no-ops** on the skipped shapes.
  - **NOT guarded: `unroll`.** `unroll::run` also performs the loop-independent
    `narrow_uint_fcvt` rewrite (zero-extension / constant-mask proofs and the
    dominator bound proofs), which applies to single-block functions too, so an
    earlier single-block guard there was wrong and was removed.
  - **Zero IR change, proven empirically:** with vs without the remaining guards,
    the binaries for all 15 bench programs are **byte-identical** (md5).
- **Rotate reorder** (`rotate.rs`): the cheap structural rejection (header must
  have non-empty params and some arm must pass them through unchanged) now runs
  *before* the region reachability walk. Most `brif` blocks are plain if/else
  heads with no params, so this skips the walk. Rotate pass time on `p2_mem`/
  `h_mem` cgu.1 drops 2.2–3.0 ms → **0.3–0.4 ms**; accepted loops are identical,
  so the binary is byte-identical.
- **Compile measurements vs the pre-guard tree** (median, reps≥7, wall/cpu ms):
  `p2_mem` 161→159 / 223→219, `h_mem` 172→168 / 244→237, `p4_simd` 104→103 /
  130→127 (~1–3%). Full three-way matrix below.
- **Verification:** `./y.sh build` exit 0; `./y.sh test --sysroot rakit` → 65
  suites / 25,368 passed / 0 failed; `bench/allaxes.sh` (CREPS=3 RREPS=7) →
  45/45 rows `ok` (stdout validated vs LLVM). `fib` still rotated; p3 output
  byte-identical.

### Full all-axes matrix (`bench/allaxes.sh`, CREPS=3 RREPS=7)
L = stock LLVM nightly, U = cg_clif upstream, M = modified Rakit. Compile =
wall ms / cpu ms (user+sys) / peak RSS MB / artifact bytes. Run = wall ms /
cpu ms / peak RSS MB. All stdout matched LLVM.

| prog | bk | Cwall | Ccpu | Crss | size(B) | Rwall | Rcpu | Rrss |
|------|----|------:|-----:|-----:|--------:|------:|-----:|-----:|
| h_arith | L | 120 | 144 | 117.7 | 4617128 | 2 | 2 | 10.8 |
| h_arith | U | 94 | 127 | 120.2 | 5839120 | 2 | 2 | 11.0 |
| h_arith | M | 102 | 129 | 120.7 | 4078672 | 2 | 2 | 11.0 |
| h_branch | L | 119 | 145 | 118.3 | 4616832 | 2 | 2 | 11.1 |
| h_branch | U | 90 | 118 | 120.3 | 5837264 | 2 | 2 | 11.0 |
| h_branch | M | 101 | 129 | 120.9 | 4077008 | 2 | 2 | 11.0 |
| h_calls | L | 118 | 143 | 118.6 | 4616872 | 2 | 2 | 10.8 |
| h_calls | U | 92 | 123 | 120.8 | 5837792 | 2 | 2 | 11.0 |
| h_calls | M | 102 | 131 | 121.1 | 4077368 | 2 | 2 | 11.1 |
| h_float | L | 128 | 151 | 116.9 | 4637984 | 2 | 2 | 11.0 |
| h_float | U | 92 | 119 | 120.1 | 5861024 | 3 | 2 | 10.8 |
| h_float | M | 112 | 132 | 120.2 | 4118168 | 2 | 2 | 10.8 |
| h_mem | L | 273 | 442 | 138.2 | 4646848 | 3 | 3 | 11.0 |
| h_mem | U | 146 | 200 | 136.0 | 5959856 | 10 | 9 | 10.8 |
| h_mem | M | 170 | 241 | 137.4 | 4166120 | 5 | 4 | 11.1 |
| h_simd | L | 146 | 170 | 124.3 | 4617704 | 2 | 2 | 11.0 |
| h_simd | U | 101 | 130 | 125.4 | 5843736 | 4 | 4 | 11.1 |
| h_simd | M | 112 | 141 | 126.3 | 4081096 | 3 | 2 | 10.8 |
| p1_arith | L | 86 | 105 | 105.7 | 4635232 | 81 | 81 | 11.1 |
| p1_arith | U | 82 | 105 | 108.9 | 5877944 | 108 | 108 | 10.8 |
| p1_arith | M | 86 | 109 | 109.5 | 4146112 | 101 | 101 | 10.8 |
| p2_mem | L | 250 | 383 | 133.6 | 4639888 | 17 | 17 | 11.0 |
| p2_mem | U | 132 | 176 | 132.1 | 5936144 | 156 | 157 | 11.1 |
| p2_mem | M | 163 | 223 | 133.7 | 4143368 | 49 | 48 | 10.8 |
| p3_calls | L | 103 | 123 | 116.2 | 4612368 | 37 | 37 | 10.8 |
| p3_calls | U | 92 | 119 | 117.7 | 5820136 | 116 | 115 | 11.0 |
| p3_calls | M | 96 | 124 | 118.2 | 4067048 | 56 | 55 | 11.0 |
| p4_simd | L | 109 | 129 | 116.5 | 4636096 | 55 | 55 | 48.2 |
| p4_simd | U | 94 | 120 | 118.0 | 5892376 | 157 | 155 | 49.4 |
| p4_simd | M | 104 | 129 | 119.0 | 4152736 | 85 | 85 | 48.8 |
| p5_inline | L | 82 | 102 | 104.8 | 4609368 | 58 | 57 | 11.0 |
| p5_inline | U | 77 | 107 | 107.5 | 5813704 | 895 | 894 | 11.1 |
| p5_inline | M | 84 | 108 | 108.4 | 4059960 | 7 | 6 | 11.1 |
| p6_scalar | L | 82 | 99 | 101.2 | 4609368 | 58 | 57 | 11.1 |
| p6_scalar | U | 77 | 98 | 103.8 | 5813688 | 902 | 902 | 10.7 |
| p6_scalar | M | 83 | 104 | 104.3 | 4059960 | 6 | 6 | 11.0 |
| p7_branchy | L | 83 | 104 | 105.3 | 4609520 | 43 | 42 | 11.0 |
| p7_branchy | U | 78 | 101 | 107.4 | 5813800 | 100 | 100 | 10.8 |
| p7_branchy | M | 86 | 106 | 108.5 | 4060120 | 35 | 35 | 11.0 |
| p8_minmax | L | 84 | 103 | 104.9 | 4609544 | 18 | 18 | 11.0 |
| p8_minmax | U | 79 | 101 | 107.3 | 5813752 | 39 | 39 | 11.0 |
| p8_minmax | M | 84 | 108 | 108.0 | 4060088 | 20 | 20 | 11.1 |
| p9_u64f32 | L | 96 | 113 | 110.6 | 4635448 | 30 | 29 | 17.8 |
| p9_u64f32 | U | 86 | 109 | 113.8 | 5886872 | 76 | 76 | 18.9 |
| p9_u64f32 | M | 94 | 118 | 114.3 | 4149744 | 33 | 33 | 18.2 |

## Session (2026-11-24) — loop rotation pass (LANDED locally, p3 −5%)

- **New pass** `rakit-opt/src/passes/rotate.rs`, wired in `src/base.rs` after
  `unroll` and before `sincos`. Rakit lowers a guarded loop to a header that
  tests at the top plus a latch that `jump`s back, so every iteration costs a
  test *and* an unconditional back-jump. Rotation makes the body its own back
  edge: the header becomes a one-shot guard, and the latch branches
  `brif(cond2, body(next), exit(...))` with the test recomputed from the latch's
  next-state arguments. One branch per iteration, no header re-entry.
- **Where it fires:** only call-bound loops by default (`RAKIT_ROTATE_ALL=1`
  widens it; that regressed `p8_minmax` +15.7% by disturbing the unroller's lane
  loop, so it stays off). The win is the recursive-to-loop `fib` kernel:
  `p3_calls` 59 → 56 ms (−5%), the `h_calls` heldout small. All other runtime
  rows are within noise.
- **Soundness model (local, not loop-analysis):** the rewrite replaces
  `jump H(next)` with the expansion of H's `brif` at `params = next`. That is
  valid only if (a) H does nothing but a `brif` on one `icmp`, (b) the body arm
  passes the header params through unchanged, (c) `next` is positional to H's
  params, and (d) the recomputed test and exit edge depend only on H params
  (→ `next`), constants, or values defined outside the loop. Each is gated.
- **The bug that cost the round:** Rakit's frontend lets a block read a
  *dominating* block's values, so the loop body often reads the header's block
  parameters (`v1`) directly instead of the body block's own params. The first
  version bypassed the header and left `v1` pinned at the entry value →
  **infinite loop**. Caught by `coretests::ptr::align_offset_zst` (100% CPU,
  7+ min) after `./y.sh test`. Fix `header_values_escape`: reject the loop if
  either arm's subgraph reads a header parameter or non-constant header result
  (`dfg.inst_args` + branch args, aliases resolved). Constants are fine (same
  every iteration). Minimal repro: the `--test` build of that test at
  `-C opt-level=0` hangs; without rotation it passes.
- **Second guard `header_has_effects`:** skipping the header also skips any
  call/store/trapping op/volatile load in it, so only pure non-terminator header
  instructions are allowed (modelled on `if_convert`'s `is_speculatable`).
- **Compile cost removed:** the first version walked the successor subgraph for
  every `brif` block (O(V²) on branchy functions; +6–9 ms CPU on `p2_mem`).
  A loop header must be the target of some `jump`, so `run()` collects
  `jump_targets` once and only walks headers in that set. Compile cost is now 0
  (`p2_mem` 158/218 ms with and without rotation).
- **Knobs:** `RAKIT_NO_ROTATE` (off switch), `RAKIT_ROTATE_ALL` (widen, off).
- **Verification:** `./y.sh build` exit 0; `./y.sh test --sysroot rakit` →
  65 suites / 25,368 passed / 0 failed (needs `./y.sh prepare` for
  rand/regex/graviola first); `bench/allaxes.sh` 45/45 rows `ok` (output
  validated vs LLVM). `p3` output byte-identical; `fib` disasm has no back-`jmp`.
- **Negative result (do not re-try):** widening the gate to also rotate loops
  with a conditional body but no call (`RAKIT_NO_ROTATE_BRIF`-style) is neutral
  under interleaved A/B (p4 −0.7%, h_branch −2.1%, everything else ≤±0.4%, i.e.
  drift). The earlier apparent p4 −5.2% was measurement drift between runs, not
  the gate. Call-only stays the default; the `rotate_brif` knob was removed.

## Session (2026-11-23) — `rakit-ir` removed, `rakit-opt` kept
- **Decision:** delete the separate RIR crate and its CLIF↔RIR bridge. Keep
  `rakit-opt` as the production pass layer; it owns the passes that produce the
  real wins (rec_loop, store_forward, div_test, if_convert, unroll, sincos,
  inliner).
- **Evidence:** `RAKIT_NO_RIR=1` made `p3_calls` 93.8 → 56.8 ms. Isolating the
  RIR passes showed `simplify` was the trigger: `RAKIT_NO_RIR_SIMPLIFY=1` gave
  59.6 ms. p1/p2/p4–p9 were neutral. RIR changed `fib` so CLIF `rec_loop` no
  longer matched.
- **Removed:** `rakit-ir/` (IR, verifier, interpreter, tests, toy ISel/regalloc),
  `rakit-opt/src/rir.rs`, `RirPass`, RIR inliner bodies, RIR tunables, custom
  backend path, and the `rakit-ir` dependency.
- **Kept:** `rakit-opt` pass manager, shared analyses, `passtime`, and all
  first-party CLIF passes. `rakit-codegen` remains the generic optimizer/backend.
- **Docs:** README architecture section rewritten; RIR knobs removed.
- **Verification:** `./y.sh build` exit 0; `./y.sh test --sysroot rakit` 65
  suites / 25,368 passed / 0 failed; `rakit-codegen` 202, `rakit-opt` 2;
  regressions pass; heldout outputs match LLVM; allaxes p3 93.8 → 59 ms.

# AGENTS.md

## Session (2026-11-24) — all-axes output validation, strict held-out args
- **Bug fixed:** `allaxes.sh` timed discovered held-out programs without
  checking stdout. `bench/validate.sh` now holds the shared step: `--capture`
  runs L, `--compare` requires exit 0 and byte-identical stdout+stderr for U/M;
  validation uses the same args as the timed runs. `measure.py --expect REF`
  checks stdout+stderr on each warmup, every timed repetition, and one
  post-measurement verification run. Timed repetitions must be captured too:
  discarding them to `/dev/null` lets a zero-exit mismatch in a timed run pass
  unnoticed. The later verification run is excluded from runs, medians, and
  RSS. Only fully validated rows print runtime timings. Runtime failure rows
  carry no runtime numbers; compile failure rows keep compile wall/CPU/RSS
  (size `-`). Harnesses exit nonzero.
- **Strict inputs:** all `bench/progs/h_*.rs` use `arg<T>(index, fallback)`;
  a present-but-unparsable argument prints `invalid argument ...` and exits 2.
  `INPUTS=100:65536` now fails `h_simd` validation instead of silently using 7.
- **h_branch:** scales `threshold & 0xF` into the top nibble. Default inputs
  7/11/13 now alternate arms (~5k/21k/60k transitions) instead of one low hit.
- **measure.py:** CLI now `sys.exit(1)` when any warmup/rep fails or `--expect`
  output mismatches. With `--expect`, an invalid result prints only the stderr
  diagnostic, not a text timing summary; `--json` still carries `exit`,
  `output_ok`, and `valid`.
- **Verification:** build 0; regressions pass; `allaxes` h_branch/h_simd match
  LLVM; negative mismatch/exit-7 and invalid-seed cases all return rc 1 with no
  timing rows. Compiler suite unchanged (65/25,368/0).

## Session (2026-11-23) — rec_loop effect guard, analysis cache, held-out harness
- **Bug fixed:** `RecLoopPass` deleted base-case side effects. `try_match` only
  checked the base block's return value; `rewrite` removes five blocks, so a
  static store in the base block disappeared. `deleted_blocks_are_safe` now
  rejects any non-matched call/load/store/trapping op or escaping value in the
  removed blocks. Regression `bench/tests/t_rec_loop_effects.rs`: old guard
  prints `55 false false`, fixed prints `55 true true`.
- **Analysis cache is live:** `if_convert`, `sincos`, `unroll` (fcvt narrowing)
  and `store_forward` now read `PassCtx::analyses` instead of rebuilding CFG/dom.
  `if_convert` invalidates after each rewrite; `unroll` after forwarder collapse;
  the pass manager invalidates after every pass.
- **Held-out harness:** programs moved to `bench/progs/h_*.rs` (auto-discovered
  by `allaxes.sh`), with small argv fallbacks. Rakit column uses
  `env -u RAKIT_HOST_LLVM`; all backends get `--target <host>`. Mismatch/failed
  validation rows are `INVALID`, never `ok`; timed-run failures set exit 1.
  `measure.py` now reports failure if *any* rep/warmup fails. `h_branch` starts
  below threshold and compares before updating, so both arms run.
- **Verification:** build 0; 65 suites / 25,368 passed / 0 failed; regressions
  pass; heldout all inputs match LLVM; allaxes 15 programs compile/run.

> Historical notes below predate the RIR removal. Entries mentioning `rakit-ir`,
> `RIR`, `RirPass`, or `RAKIT_RIR_*` are archival and no longer describe the tree.


## Session (2026-11-23, Fase 2 step 19) — calls
- `Inst::Call(Option<Value>, u32, Vec<Value>)` plus `Function.externals`
  (`External { namespace, index, params, ret }`). Builder `call`.
- Bridge imports/exports CLIF `Call` for `ExternalName::User` with 0/1 returns;
  other external kinds or multi-returns are refused.
- DCE keeps calls; verifier checks target/arity/types; interpreter reports
  external calls as unsupported. Addresses passed to calls escape, so
  `mem_forward`/SROA leave those slots alone.
- Tests: 75 RIR (2 call) + 9 bridge; h23 (`f -> g`) prints 42 with RIR;
  h20-h22 unchanged; `./y.sh build` exit 0.

## Session (2026-11-23, Fase 2 step 18) — I128 memory
- `Type::I128` and `Val::I128(u128)` added. Interpreter reads/writes 16 bytes;
  `int_sem` rejects I128 arithmetic (no i64 representation).
- `mem_forward`/`sroa` use 16-byte size; bridge maps CLIF `I128`.
- Tests: 73 RIR (4 memory, 8 sroa) + 8 bridge; `./y.sh build` exit 0; h22
  (`u128` array) prints 42 with `sroa fired`; h20/h21 unchanged.

## Session (2026-11-23, Fase 2 step 17) — SROA
- New `rakit-ir/src/sroa.rs`: promote non-escaping scalar slots with constant
  load/store keys. Phi nodes are added as block params at dominance-frontier
  joins, then a dominator-tree rename replaces loads and drops stores.
- Skip dynamic addresses, load-before-store, overlapping access ranges, and
  phis at the entry block. Remove slots with no remaining `StackAddr` and
  renumber the rest.
- Wire into `RirPass` after `mem_forward`/`optimize`:
  `simplify -> optimize -> mem_forward -> optimize -> sroa -> optimize ->
  div_test -> if_convert -> gvn -> licm -> optimize`.
- Tests: 71 RIR (7 sroa) + 8 bridge; `./y.sh build` exit 0; h17-h21 outputs
  unchanged (`30/24/11/11/42`).

## Session (2026-11-23, Fase 2 step 16) — liveness DSE
- `mem_forward::dse` now uses byte-range liveness across the CFG:
  per-block `used`/`defined`, `live-in`/`live-out` fixed point, then a
  backward scan removes stores whose bytes are not live after them.
- Dynamic store/load covers the whole slot. Stores overwritten later in the
  same block or a successor are now removed; stores read in a successor stay.
- Tests: 64 RIR (11 mem_forward) + 8 bridge; `./y.sh build` exit 0; h20/h21
  unchanged (`11`/`42`).

## Session (2026-11-23, Fase 2 step 15) — dynamic addresses
- `mem_forward` now recognizes `Copy`, `Add` with constants, `Select`, and
  `StackAddr + dynamic index`. `Const` and `Dyn` keys are matched separately.
- A dynamic store kills all facts for its slot; a dynamic load disables DSE for
  that slot. Constant stores are removed only without a dynamic load; dynamic
  stores only when the slot has no loads.
- Tests: 61 RIR (8 mem_forward) + 8 bridge; `./y.sh build` exit 0; h21
  (`as_mut_ptr().add(i & 3)`) prints 42 and logs `mem_forward fired`.

## Session (2026-11-23, Fase 2 step 14) — store forwarding + DSE
- New `rakit-ir/src/mem_forward.rs`: track direct `StackAddr` values and
  `Copy` aliases, require non-escaping private slots and in-bounds accesses.
- Forward `store` -> later `load` within and across blocks using a
  must-analysis over predecessors; replace loads with `Copy`.
- Remove stores whose byte ranges are never loaded anywhere (conservative
  global DSE), then let `optimize` clean up.
- Wire into `RirPass` after `simplify`/`optimize`:
  `simplify -> optimize -> mem_forward -> optimize -> div_test -> if_convert
  -> gvn -> licm -> optimize`.
- Tests: 58 RIR (5 mem_forward) + 8 bridge; `./y.sh build` exit 0; h20/h19
  print 11 and log `mem_forward fired`.

## Session (2026-11-23, Fase 2 step 13) — memory bridge
- RIR: `StackSlot`/`StackSlotData`, `Function.stack_slots`, `Inst::StackAddr`,
  `Inst::Load`, `Inst::Store` (scalar, constant offset); builder helpers.
- Bridge `rakit-opt/src/rir.rs` imports fixed stack slots and
  `stack_addr`/`load`/`store`/`load_no_offset`/`store_no_offset`; export
  recreates slots and memory ops. Addresses are `I64`.
- Interpreter: stack-slot arena with bounds checks; `Val::P` addresses.
- Verifier checks slot range, address type, load/store types.
- Passes stay conservative: GVN/LICM skip memory ops; if-convert rejects arms
  with memory; DCE keeps loads/stores.
- Tests: 53 RIR (3 memory) + 8 bridge; `./y.sh build` exit 0; h19 (array via
  `get_unchecked`) imports, logs simplify/gvn, prints 11 like LLVM.

## Session (2026-11-23, Fase 2 step 12) — LICM
- New `rakit-ir/src/licm.rs`: finds natural loops from back edges + dominators,
  requires a unique outside predecessor ending in `jump` to the header, and
  hoists pure instructions whose operands are defined outside the loop and
  dominate the preheader.
- Skips div/rem/fdiv (speculation), loops without a preheader, and loops whose
  preheader does not dominate the whole body. Runs to a fixpoint so invariant
  chains hoist in successive rounds.
- Shared `predecessors/rpo/dominators/dominates` moved into `cfg_util`; `gvn`
  and `simplify` now use them.
- `RirPass` order:
  `simplify -> optimize -> div_test -> if_convert -> gvn -> licm -> optimize`.
- Tests: 50/50 (6 div_test + 7 gvn + 7 if_convert + 15 ir + 4 licm +
  11 simplify).
- End-to-end: `k` (x*y only inside a loop) logs `licm fired` and prints 24;
  `h` logs `gvn fired` + `licm fired` and prints 30, both identical.

## Session (2026-11-23, Fase 2 step 11) — block merging in simplify
- `simplify` now also merges a parameterless block with a single predecessor
  when the predecessor ends in `jump` with no args: the block body is appended
  to the predecessor and the block is removed (values stay defined).
- Does not merge across a `brif` edge, a block with params, or a block with
  multiple predecessors.
- Tests: 46/46 (6 div_test + 7 gvn + 7 if_convert + 15 ir + 11 simplify).
- End-to-end: `h` (loop), `pick` and `pick2` all log `simplify fired` with
  `RAKIT_RIR=1 RAKIT_RIR_DEBUG=1`; outputs 30, `11 12 1`, `8 18`, identical.

## Session (2026-11-23, Fase 2 step 10) — global GVN
- `gvn` is now global: computes predecessors, RPO and immediate dominators
  (Cooper-Harvey-Kennedy), inherits the expression table from the immediate
  dominator, and only merges an expression when the first result dominates it.
- Unreachable blocks fall back to local numbering.
- Tests: 43/43 (6 div_test + 7 gvn + 7 if_convert + 15 ir + 8 simplify). New:
  merge across dominating blocks, no merge across sibling branches, merge in a
  join dominated by the entry.
- End-to-end: a loop that computes `x*y` before the loop and again inside it
  logs `gvn fired` with `RAKIT_RIR=1 RAKIT_RIR_DEBUG=1` and prints 30,
  identical to normal.

## Session (2026-11-23, Fase 2 step 9) — CFG simplify
- New `rakit-ir/src/simplify.rs`: removes blocks unreachable from entry and
  collapses empty forwarding blocks (`nop*; jump target` with no params),
  redirecting both `jump` and `brif` edges; blocks/values are renumbered.
- Forwarders whose `nop` result is used outside are left alone. The entry block
  is never collapsed.
- Shared `cfg_util::{remove_blocks, remove_values}` moved out of `if_convert`.
- `RirPass` order:
  `simplify -> optimize -> div_test -> if_convert -> gvn -> optimize`.
- Tests: 41/41 (6 div_test + 5 gvn + 7 if_convert + 15 ir + 8 simplify).
- End-to-end: `pick` with `RAKIT_RIR=1 RAKIT_RIR_DEBUG=1` logs
  `simplify fired` (dead block removed) then `if_convert fired`; output
  `11 12 1`, identical to normal.

## Session (2026-11-23, Fase 2 step 8) — multi-value if_convert + local GVN
- `if_convert` now handles joins/returns carrying any number of values
  (including zero), not just one. Two identical `if`s compile to a two-param
  join; the pass emits one `select` per join param.
- New `rakit-ir/src/gvn.rs`: local value numbering keyed by opcode/type/operands;
  duplicate pure instructions become `Copy` of the first result. `Hash` derives
  added to `Type`, `BinOp`, `IntCc`, `FloatCc`.
- `RirPass` order: `optimize -> div_test -> if_convert -> gvn -> optimize`.
- Tests: 33/33 (6 div_test + 5 gvn + 7 if_convert + 15 ir). New: two-value
  join, duplicate add/constant/select, different types, cross-block non-merge.
- End-to-end: `pick2(c,x,y) = { let a = if c {x} else {y}; let b = ...; a+b }`
  with `RAKIT_RIR=1 RAKIT_RIR_DEBUG=1` logs `if_convert fired` then
  `gvn fired`, prints `8 18`, identical to normal.

## Session (2026-11-23, Fase 2 step 7) — `if_convert` migrated to RIR
- New `rakit-ir/src/if_convert.rs`: converts a two-way diamond (`brif` to two
  short arms that `jump` to a common join or both `return`) into a `select` in
  the head, then removes the arms and their now-dead values (block and value
  indices are renumbered).
- Conservative limits: arms have no params, one predecessor, <= 8 insts, no
  div/rem/fdiv, and no arm-defined value is used outside the arms.
- `RirPass` runs it after `div_test` and re-runs `optimize`.
- Tests: 27/27 (6 div_test + 6 if_convert + 15 ir; new: simple join diamond,
  head-value arms, return arms, skip division, skip params, skip large arm).
- End-to-end: `RAKIT_RIR=1 RAKIT_NO_IFCONV=1` on
  `if c { x + 1 } else { x + 2 }` logs `[rir] if_convert fired` and prints
  `11 12 1`, identical to the normal build.

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

## Session (2026-11-23) — Phase 3 backend + RIR gap closure (LANDED locally)
- **Phase 3 implemented** in `rakit/rakit-ir/src/backend/`: `mach.rs`
  (machine IR), `mod.rs` (`InstructionSelector`, `RegisterAllocator`,
  `BackendError`), `isel.rs` (`RirSelector`), `regalloc.rs` (`LinearScan` with
  liveness, graph coloring, spills), `exec.rs` (machine interpreter).
  Object-safe traits; `RAKIT_CUSTOM_BACKEND=1` runs ISel+regalloc as a
  validated alternate path (CLIF remains the production emitter).
- **Gap closure**: `I128` arithmetic/compare in `int_sem` + interpreter;
  direct/indirect calls, libcalls/known-symbol targets, multi-result calls in
  RIR/bridge; `run_with` / `run_with_indirect` interpreter callbacks; SROA of
  dynamic addresses into one-element slots (valid index is necessarily zero).
  Verifier, DCE, cfg remapping, and `External` now handle all call forms.
- **RIR default changed**: runs by default for **executable user crates**, but
  skips std crates (`core`, `std`, `alloc`, `compiler_builtins`, `unwind`,
  `rustc_*`) until the bridge is validated against the whole stdlib.
  `RAKIT_RIR=1` forces on; `RAKIT_NO_RIR=1` forces off. Enabling it for std
  still ICEs in `core`, so the skip is deliberate.
- **Bug found/fixed**: `RirPass` overwrote `new_func.params` with the old
  table after export; the new `ExternalName::User` indices then pointed at the
  wrong names. `h26` (function pointer) segfaulted until that assignment was
  removed. Multi-result support also needed `result_values()` in the verifier.
- **Validation**: `./y.sh build` exit 0 (13–14 s); `cargo test` in
  `rakit-ir` = 85 tests / 0 failed, `rakit-opt` = 11 / 0; `h20`–`h26` output
  identical for default-RIR, `RAKIT_NO_RIR=1`, and forced `RAKIT_RIR=1`
  (including `I128` array, direct call, libcall, and indirect-call programs);
  `RAKIT_CUSTOM_BACKEND=1 RAKIT_RIR_DEBUG=1` logs `custom backend: 8 vregs, 8
  regs, 0 spills` for `choose` in `h24`.
- **Docs**: README bridge/SROA/I128/calls and Phase 3 sections updated.
- **Push**: pending user-supplied PAT; branch `rakit-middle-end`.

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
