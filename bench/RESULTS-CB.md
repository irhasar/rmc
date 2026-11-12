# Benchmark results: the rustc codegen-backend pipeline (`cb`)

Recorded 2026-11-12 on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.102.0-nightly (3ef0c22cd 2026-11-11)`.

Regenerate with `python3 bench/run_codegen.py`.

`cb` is the architecturally complete pipeline: rustc loads rmc as a
`-Zcodegen-backend` dylib (`backend/`, built by `backend/build.sh`), hands it
the *same* optimised MIR the LLVM backend would see, and rmc lowers it straight
to a freestanding x86-64 ELF. No `--emit=mir` text round-trip, no second
`rustc` process, no LLVM, no libc, no crates.

The harness runs `cb` and `llvm` the same way - one `rustc` process each - so
the comparison is apples to apples:

* `cb` - `rustc -Zcodegen-backend=librmc_codegen_backend.so -C opt-level=3
  -C panic=abort`.
* `llvm` - `rustc -C opt-level=3 -C lto=fat -C codegen-units=1 -C panic=abort`
  then `ld -s`.

Every axis is per program; `>1` means rmc is better, and `+/-0.02` counts as a
tie.

## Compile and run, all axes

| program | pipeline | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: | -----------: | -------: | -----------: | -----------: |
| hello | cb | 31.69 | 31.38 | 85764 | 510 | 0.21 | 0.12 | 20 |
| hello | llvm | 32.83 | 33.26 | 82992 | 17128 | 0.22 | 0.13 | 36 |
| fib | cb | 31.80 | 31.70 | 84804 | 773 | 5.44 | 5.31 | 24 |
| fib | llvm | 35.95 | 36.50 | 85460 | 17128 | 5.47 | 5.33 | 32 |
| mandel | cb | 33.73 | 33.55 | 85364 | 1248 | 97.28 | 97.06 | 24 |
| mandel | llvm | 38.11 | 38.48 | 86872 | 17128 | 96.17 | 95.83 | 32 |
| sieve | cb | 34.25 | 34.16 | 87776 | 989 | 1.74 | 1.63 | 1584 |
| sieve | llvm | 39.05 | 39.66 | 88280 | 17208 | 1.83 | 1.69 | 1596 |
| mix | cb | 32.63 | 32.54 | 85356 | 965 | 14.97 | 14.81 | 24 |
| mix | llvm | 36.84 | 37.51 | 87188 | 17208 | 14.87 | 14.71 | 32 |
| stats | cb | 33.64 | 33.57 | 85992 | 1067 | 2.81 | 2.70 | 24 |
| stats | llvm | 38.48 | 39.08 | 87472 | 17128 | 3.09 | 2.97 | 32 |
| refs | cb | 34.99 | 34.82 | 86452 | 1529 | 0.21 | 0.12 | 20 |
| refs | llvm | 37.99 | 38.69 | 89128 | 17128 | 0.22 | 0.13 | 28 |
| heap | cb | 33.28 | 33.18 | 85796 | 1135 | 3.15 | 3.03 | 4712 |
| heap | llvm | 36.94 | 37.71 | 87264 | 17128 | 3.11 | 2.98 | 4720 |
| callrec | cb | 32.43 | 32.34 | 85104 | 773 | 2.21 | 2.10 | 24 |
| callrec | llvm | 35.26 | 35.82 | 85688 | 17128 | 2.22 | 2.11 | 32 |
| forloops | cb | 32.74 | 32.64 | 85720 | 913 | 1.44 | 1.32 | 24 |
| forloops | llvm | 39.80 | 40.34 | 87760 | 17128 | 2.45 | 2.33 | 36 |
| light | cb | 34.56 | 34.45 | 89892 | 1134 | 1.81 | 1.69 | 24 |
| light | llvm | 39.39 | 40.19 | 89580 | 17128 | 2.02 | 1.91 | 32 |
| heavy | cb | 31.60 | 31.54 | 84792 | 798 | 761.30 | 761.03 | 24 |
| heavy | llvm | 36.01 | 36.60 | 86080 | 17128 | 1031.75 | 1031.40 | 32 |

## Speedups, per program (`cb` / `llvm`)

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| ------- | ---: | --: | --: | ---: | -------: | ------: | ------: |
| hello | 1.04 | 1.06 | 0.97 | 33.58 | 1.04 | 1.07 | 1.80 |
| fib | 1.13 | 1.15 | 1.01 | 22.16 | 1.01 | 1.00 | 1.33 |
| mandel | 1.13 | 1.15 | 1.02 | 13.72 | 0.99 | 0.99 | 1.33 |
| sieve | 1.14 | 1.16 | 1.01 | 17.40 | 1.05 | 1.04 | 1.01 |
| mix | 1.13 | 1.15 | 1.02 | 17.83 | 0.99 | 0.99 | 1.33 |
| stats | 1.14 | 1.16 | 1.02 | 16.05 | 1.10 | 1.10 | 1.33 |
| refs | 1.09 | 1.11 | 1.03 | 11.20 | 1.04 | 1.07 | 1.40 |
| heap | 1.11 | 1.14 | 1.02 | 15.09 | 0.99 | 0.98 | 1.00 |
| callrec | 1.09 | 1.11 | 1.01 | 22.16 | 1.01 | 1.01 | 1.33 |
| forloops | 1.22 | 1.24 | 1.02 | 18.76 | 1.70 | 1.76 | 1.50 |
| light | 1.14 | 1.17 | 1.00 | 15.10 | 1.12 | 1.13 | 1.33 |
| heavy | 1.14 | 1.16 | 1.02 | 21.46 | 1.36 | 1.36 | 1.33 |

Mean per axis:

| axis | mean | verdict |
| ---- | ---: | ------- |
| wall | 1.124 | win |
| cpu | 1.146 | win |
| rss | 1.011 | tie |
| size | 18.711 | win |
| run_wall | 1.115 | win |
| run_cpu | 1.125 | win |
| run_rss | 1.337 | win |

## The one remaining loss: `hello` compile RSS

Every axis is a win or a tie on the mean, and every per-program run-time axis
is a win or a tie. The single per-program loss is `hello` compile RSS at 0.97
(`cb` 85.8 MB vs `llvm` 83.0 MB).

The extra ~2.8 MB is rmc's codegen working set, not the backend dylib's pages:
loading the dylib alone is free (`--emit=metadata` with and without
`-Zcodegen-backend` both peak at ~71.8 MB), and `rmcmir` lowering `hello.mir`
on its own peaks at ~2.5 MB. rustc's fixed front-end cost (~71.8 MB) dominates
both sides, so on a program this small the difference is a larger fraction of
the total than it is anywhere else; by `heavy` the same absolute overhead is
1.4% of the run-time footprint. The mean RSS still lands at a tie (1.011)
because the larger programs give rmc's smaller allocations room to win.
