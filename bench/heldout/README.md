# Held-out benchmarks

These programs are not used for tuning. Each one reads its trip count and seed
from `argv`, so compile-time folding cannot hide a codegen gap.

Run the full 3-way matrix with:

```sh
CREPS=3 RREPS=5 bench/heldout.sh
```

Custom inputs use `n:seed` pairs:

```sh
INPUTS="10000:7 50000:11 200000:13" bench/heldout.sh
```

The harness compiles with LLVM (`L`), upstream cg_clif (`U`), and Rakit (`M`),
checks that every output matches LLVM, and prints compile wall/CPU/RSS/size
plus run wall/CPU/RSS.
