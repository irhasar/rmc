# Held-out benchmarks

The held-out programs live in `bench/progs/h_*.rs` (so `bench/allaxes.sh`
discovers them automatically) and are not used for tuning. Each one reads its trip count and seed
from `argv`, so compile-time folding cannot hide a codegen gap.

Run the full 3-way matrix with:

```sh
CREPS=3 RREPS=5 bench/heldout.sh
```

Custom inputs use `n:seed` pairs:

```sh
INPUTS="10000:7 50000:11 200000:13" bench/heldout.sh
```

A supplied argument that does not parse for that program is rejected with exit
2 and a diagnostic; the harness reports a failed validation instead of silently
timing a different workload.

The harness compiles with LLVM (`L`), upstream cg_clif (`U`), and Rakit (`M`),
checks that every output matches LLVM, and prints compile wall/CPU/RSS/size
plus run wall/CPU/RSS. `bench/allaxes.sh` uses the same `bench/validate.sh`
step, so discovered held-out programs are gated the same way. During timing,
`measure.py --expect` checks output on each warmup, on every timed repetition,
and on one post-measurement verification run. A zero-exit output change in any
timed repetition sets `valid=false`, so a workload whose output changes during
measurement produces no timing row.
