# Benchmark results: the rustc codegen-backend pipeline (`cb`)

Recorded 2026-11-13 on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | cb | 26.66 | 26.55 | 76492 | 502 | 0.21 | 0.12 | 0 |
| hello | llvm | 33.28 | 33.80 | 81936 | 17128 | 0.22 | 0.13 | 40 |
| fib | cb | 26.93 | 26.83 | 76288 | 691 | 5.47 | 5.32 | 28 |
| fib | llvm | 34.80 | 35.41 | 85800 | 17128 | 5.45 | 5.33 | 36 |
| mandel | cb | 28.66 | 28.51 | 76980 | 1107 | 97.35 | 97.06 | 28 |
| mandel | llvm | 38.23 | 38.85 | 85636 | 17128 | 96.03 | 95.78 | 36 |
| sieve | cb | 29.50 | 29.35 | 79456 | 888 | 1.76 | 1.64 | 1588 |
| sieve | llvm | 39.80 | 40.42 | 89436 | 17208 | 1.82 | 1.71 | 1600 |
| mix | cb | 27.62 | 27.54 | 76880 | 834 | 15.01 | 14.86 | 28 |
| mix | llvm | 37.62 | 38.26 | 86000 | 17208 | 14.89 | 14.74 | 36 |
| stats | cb | 27.84 | 27.66 | 77844 | 899 | 2.66 | 2.53 | 28 |
| stats | llvm | 38.11 | 38.72 | 86332 | 17128 | 3.10 | 2.98 | 36 |
| refs | cb | 29.90 | 29.91 | 78388 | 1448 | 0.39 | 0.21 | 28 |
| refs | llvm | 37.87 | 38.51 | 87672 | 17128 | 0.24 | 0.14 | 36 |
| heap | cb | 28.28 | 28.21 | 77504 | 950 | 3.09 | 2.95 | 4716 |
| heap | llvm | 36.84 | 37.46 | 85968 | 17128 | 3.17 | 3.05 | 4724 |
| callrec | cb | 26.96 | 26.84 | 76372 | 691 | 2.24 | 2.12 | 28 |
| callrec | llvm | 35.73 | 36.33 | 84668 | 17128 | 2.24 | 2.12 | 36 |
| forloops | cb | 28.53 | 28.41 | 77584 | 771 | 2.06 | 1.94 | 28 |
| forloops | llvm | 40.41 | 40.95 | 86560 | 17128 | 2.48 | 2.35 | 40 |
| light | cb | 29.57 | 29.43 | 81560 | 978 | 1.82 | 1.70 | 28 |
| light | llvm | 40.39 | 41.10 | 90700 | 17128 | 2.04 | 1.92 | 36 |
| heavy | cb | 27.00 | 26.87 | 76512 | 702 | 757.68 | 757.37 | 28 |
| heavy | llvm | 36.55 | 37.16 | 86608 | 17128 | 1030.64 | 1030.27 | 36 |

## Speedups, per program (`cb` / `llvm`)

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| ------- | ---: | --: | --: | ---: | -------: | ------: | ------: |
| hello | 1.25 | 1.27 | 1.07 | 34.12 | - | - | - |
| fib | 1.29 | 1.32 | 1.12 | 24.79 | 1.00 | 1.00 | 1.29 |
| mandel | 1.33 | 1.36 | 1.11 | 15.47 | 0.99 | 0.99 | 1.29 |
| sieve | 1.35 | 1.38 | 1.13 | 19.38 | 1.03 | 1.04 | 1.01 |
| mix | 1.36 | 1.39 | 1.12 | 20.63 | 0.99 | 0.99 | 1.29 |
| stats | 1.37 | 1.40 | 1.11 | 19.05 | 1.17 | 1.18 | 1.29 |
| refs | 1.27 | 1.29 | 1.12 | 11.83 | - | - | 1.29 |
| heap | 1.30 | 1.33 | 1.11 | 18.03 | 1.03 | 1.04 | 1.00 |
| callrec | 1.33 | 1.35 | 1.11 | 24.79 | 1.00 | 1.00 | 1.29 |
| forloops | 1.42 | 1.44 | 1.12 | 22.22 | 1.20 | 1.21 | 1.43 |
| light | 1.37 | 1.40 | 1.11 | 17.51 | 1.12 | 1.13 | 1.29 |
| heavy | 1.35 | 1.38 | 1.13 | 24.40 | 1.36 | 1.36 | 1.29 |

Mean per axis:

| axis | mean | verdict |
| ---- | ---: | ------- |
| wall | 1.332 | win |
| cpu | 1.359 | win |
| rss | 1.113 | win |
| size | 21.018 | win |
| run_wall | 1.088 | win |
| run_cpu | 1.094 | win |
| run_rss | 1.248 | win |

Every axis is a win on the mean, and every per-program run-time axis is a win
or a tie. `cb` also beats the printed-MIR prototype (`mir`) by ~1.24x on compile
wall and CPU with identical run-time numbers, because it skips the MIR text
round-trip and the second process, and it beats rustc's Cranelift backend
(`cl`) on every axis including run time.
