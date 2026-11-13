# Four-way benchmark on one source: rmc, rustc+rmc, Cranelift, LLVM

Recorded 2026-11-13 06:05 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: unknown.
`rustc 1.102.0-nightly (3ef0c22cd 2026-11-11)`.

Regenerate with `python3 bench/run_four.py`.

All four pipelines compile the **same** file, `bench/uni/*.rs` (generated
by `bench/gen_uni.py`). The runtime declarations and the panic handler sit
in a `mod` that rmc drops and rustc compiles, and the names are re-exported
with `use`, which rmc drops and rustc honours, so both front ends see the
same calls. This is what makes the run-time columns comparable: a difference
is the compiler, not the source.

* `rmc (AST)` - rmc's own front end and back end, no rustc at all.
* `rustc+rmc (cb)` - rustc's MIR front end, rmc's back end.
* `rustc Cranelift (cl)` - rustc's Cranelift back end.
* `rustc LLVM` - rustc's LLVM back end (`-O -C lto=fat`), the reference.

Every axis is per program; `>1` means the first of the pair is better, and
`+/- 0.02` counts as a tie. A run axis below the
sub-millisecond process-startup resolution is left unmeasured (`-`).

## Compile and run, all axes

| program | backend | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | rmc (AST) | 2.24 | 2.05 | 3800 | 387 | 0.22 | 0.12 | 20 |
| hello | rustc+rmc (`cb`) | 26.10 | 26.03 | 72864 | 380 | 0.22 | 0.12 | 0 |
| hello | rustc LLVM | 32.66 | 33.03 | 79248 | 17128 | 0.23 | 0.13 | 28 |
| hello | rustc Cranelift (`cl`) | 30.32 | 30.70 | 77400 | 17008 | 0.22 | 0.13 | 24 |
| fib | rmc (AST) | 2.44 | 2.29 | 3892 | 702 | 7.20 | 7.05 | 24 |
| fib | rustc+rmc (`cb`) | 28.21 | 28.15 | 75836 | 691 | 5.47 | 5.34 | 24 |
| fib | rustc LLVM | 37.16 | 37.69 | 85280 | 17128 | 5.50 | 5.36 | 32 |
| fib | rustc Cranelift (`cl`) | 32.74 | 33.31 | 80548 | 17008 | 16.46 | 16.31 | 28 |
| mandel | rmc (AST) | 2.49 | 2.36 | 3844 | 1031 | 96.57 | 96.32 | 24 |
| mandel | rustc+rmc (`cb`) | 28.99 | 28.96 | 76672 | 1107 | 97.40 | 97.12 | 24 |
| mandel | rustc LLVM | 38.58 | 39.18 | 85296 | 17128 | 96.11 | 95.81 | 32 |
| mandel | rustc Cranelift (`cl`) | 32.17 | 32.64 | 81324 | 17008 | 109.44 | 109.19 | 28 |
| sieve | rmc (AST) | 2.49 | 2.34 | 3936 | 913 | 1.58 | 1.47 | 1584 |
| sieve | rustc+rmc (`cb`) | 29.29 | 29.27 | 78920 | 888 | 1.75 | 1.62 | 1584 |
| sieve | rustc LLVM | 39.55 | 40.12 | 86844 | 17208 | 1.83 | 1.70 | 1596 |
| sieve | rustc Cranelift (`cl`) | 32.20 | 32.89 | 80108 | 17088 | 2.17 | 2.05 | 1592 |
| mix | rmc (AST) | 2.40 | 2.27 | 3916 | 941 | 14.81 | 14.62 | 24 |
| mix | rustc+rmc (`cb`) | 28.02 | 28.06 | 76456 | 834 | 15.09 | 14.92 | 24 |
| mix | rustc LLVM | 38.13 | 38.71 | 85760 | 17208 | 15.03 | 14.86 | 32 |
| mix | rustc Cranelift (`cl`) | 32.42 | 33.02 | 78968 | 17088 | 17.03 | 16.86 | 28 |
| stats | rmc (AST) | 2.42 | 2.27 | 3840 | 837 | 3.43 | 3.30 | 24 |
| stats | rustc+rmc (`cb`) | 28.63 | 28.54 | 77460 | 899 | 2.64 | 2.52 | 24 |
| stats | rustc LLVM | 38.86 | 39.36 | 87292 | 17128 | 3.11 | 2.98 | 32 |
| stats | rustc Cranelift (`cl`) | 32.53 | 33.17 | 81836 | 17008 | 4.39 | 4.28 | 28 |
| refs | rmc (AST) | 2.57 | 2.43 | 3980 | 991 | 1.85 | 1.68 | 24 |
| refs | rustc+rmc (`cb`) | 29.72 | 29.62 | 78112 | 1448 | 0.27 | 0.14 | 24 |
| refs | rustc LLVM | 37.73 | 38.27 | 85932 | 17128 | 0.24 | 0.14 | 32 |
| refs | rustc Cranelift (`cl`) | 32.70 | 33.24 | 79572 | 17008 | 40.63 | 40.47 | 28 |
| heap | rmc (AST) | 2.46 | 2.32 | 3816 | 1017 | 3.46 | 3.33 | 4712 |
| heap | rustc+rmc (`cb`) | 28.97 | 28.75 | 77116 | 950 | 3.08 | 2.95 | 4712 |
| heap | rustc LLVM | 37.91 | 38.44 | 85756 | 17128 | 3.19 | 3.05 | 4720 |
| heap | rustc Cranelift (`cl`) | 32.40 | 32.98 | 79016 | 17008 | 3.09 | 2.96 | 4716 |
| callrec | rmc (AST) | 2.34 | 2.20 | 3836 | 706 | 2.93 | 2.78 | 24 |
| callrec | rustc+rmc (`cb`) | 27.16 | 27.10 | 76044 | 691 | 2.25 | 2.14 | 24 |
| callrec | rustc LLVM | 36.19 | 36.74 | 84496 | 17128 | 2.27 | 2.15 | 32 |
| callrec | rustc Cranelift (`cl`) | 30.85 | 31.41 | 77868 | 17008 | 6.53 | 6.38 | 28 |
| forloops | rmc (AST) | 2.39 | 2.25 | 3932 | 779 | 2.03 | 1.92 | 24 |
| forloops | rustc+rmc (`cb`) | 28.20 | 28.11 | 77352 | 771 | 2.03 | 1.92 | 24 |
| forloops | rustc LLVM | 39.70 | 40.32 | 86288 | 17128 | 2.48 | 2.36 | 32 |
| forloops | rustc Cranelift (`cl`) | 31.82 | 32.41 | 79056 | 17008 | 2.64 | 2.51 | 28 |
| light | rmc (AST) | 2.43 | 2.29 | 3900 | 1033 | 2.07 | 1.95 | 24 |
| light | rustc+rmc (`cb`) | 29.40 | 29.39 | 81212 | 978 | 1.82 | 1.70 | 24 |
| light | rustc LLVM | 40.15 | 40.78 | 88140 | 17128 | 2.03 | 1.91 | 32 |
| light | rustc Cranelift (`cl`) | 33.04 | 33.51 | 85880 | 17088 | 2.54 | 2.43 | 28 |
| heavy | rmc (AST) | 2.31 | 2.19 | 3900 | 719 | 761.07 | 760.77 | 24 |
| heavy | rustc+rmc (`cb`) | 27.07 | 26.87 | 76284 | 702 | 759.18 | 758.90 | 24 |
| heavy | rustc LLVM | 36.73 | 37.29 | 84560 | 17128 | 1030.63 | 1030.20 | 32 |
| heavy | rustc Cranelift (`cl`) | 31.03 | 31.52 | 78168 | 17008 | 1058.60 | 1058.33 | 28 |

### rmc (AST) vs rustc+rmc (`cb`) - front end only

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 11.66 | 12.68 | 19.17 | 0.98 | - | - | - |
| fib | 11.55 | 12.31 | 19.49 | 0.98 | 0.76 | 0.76 | 1.00 |
| mandel | 11.63 | 12.28 | 19.95 | 1.07 | 1.01 | 1.01 | 1.00 |
| sieve | 11.78 | 12.52 | 20.05 | 0.97 | 1.11 | 1.11 | 1.00 |
| mix | 11.66 | 12.38 | 19.52 | 0.89 | 1.02 | 1.02 | 1.00 |
| stats | 11.82 | 12.56 | 20.17 | 1.07 | 0.77 | 0.76 | 1.00 |
| refs | 11.57 | 12.17 | 19.63 | 1.46 | - | - | 1.00 |
| heap | 11.79 | 12.38 | 20.21 | 0.93 | 0.89 | 0.89 | 1.00 |
| callrec | 11.61 | 12.33 | 19.82 | 0.98 | 0.77 | 0.77 | 1.00 |
| forloops | 11.79 | 12.52 | 19.67 | 0.99 | 1.00 | 1.00 | 1.00 |
| light | 12.09 | 12.82 | 20.82 | 0.95 | 0.88 | 0.87 | 1.00 |
| heavy | 11.70 | 12.29 | 19.56 | 0.98 | 1.00 | 1.00 | 1.00 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 11.720 | win |
| cpu | 12.436 | win |
| rss | 19.839 | win |
| size | 1.022 | win |
| run_wall | 0.920 | loss |
| run_cpu | 0.918 | loss |
| run_rss | 1.000 | tie |

### rustc+rmc (`cb`) vs LLVM

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 1.25 | 1.27 | 1.09 | 45.07 | - | - | - |
| fib | 1.32 | 1.34 | 1.12 | 24.79 | 1.00 | 1.00 | 1.33 |
| mandel | 1.33 | 1.35 | 1.11 | 15.47 | 0.99 | 0.99 | 1.33 |
| sieve | 1.35 | 1.37 | 1.10 | 19.38 | 1.04 | 1.05 | 1.01 |
| mix | 1.36 | 1.38 | 1.12 | 20.63 | 1.00 | 1.00 | 1.33 |
| stats | 1.36 | 1.38 | 1.13 | 19.05 | 1.18 | 1.18 | 1.33 |
| refs | 1.27 | 1.29 | 1.10 | 11.83 | - | - | 1.33 |
| heap | 1.31 | 1.34 | 1.11 | 18.03 | 1.04 | 1.03 | 1.00 |
| callrec | 1.33 | 1.36 | 1.11 | 24.79 | 1.01 | 1.01 | 1.33 |
| forloops | 1.41 | 1.43 | 1.12 | 22.22 | 1.22 | 1.23 | 1.33 |
| light | 1.37 | 1.39 | 1.09 | 17.51 | 1.12 | 1.13 | 1.33 |
| heavy | 1.36 | 1.39 | 1.11 | 24.40 | 1.36 | 1.36 | 1.33 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 1.334 | win |
| cpu | 1.357 | win |
| rss | 1.109 | win |
| size | 21.931 | win |
| run_wall | 1.094 | win |
| run_cpu | 1.096 | win |
| run_rss | 1.274 | win |

### rustc+rmc (`cb`) vs rustc Cranelift (`cl`)

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 1.16 | 1.18 | 1.06 | 44.76 | - | - | - |
| fib | 1.16 | 1.18 | 1.06 | 24.61 | 3.01 | 3.05 | 1.17 |
| mandel | 1.11 | 1.13 | 1.06 | 15.36 | 1.12 | 1.12 | 1.17 |
| sieve | 1.10 | 1.12 | 1.02 | 19.24 | 1.24 | 1.26 | 1.01 |
| mix | 1.16 | 1.18 | 1.03 | 20.49 | 1.13 | 1.13 | 1.17 |
| stats | 1.14 | 1.16 | 1.06 | 18.92 | 1.66 | 1.70 | 1.17 |
| refs | 1.10 | 1.12 | 1.02 | 11.75 | - | - | 1.17 |
| heap | 1.12 | 1.15 | 1.02 | 17.90 | 1.01 | 1.00 | 1.00 |
| callrec | 1.14 | 1.16 | 1.02 | 24.61 | 2.90 | 2.99 | 1.17 |
| forloops | 1.13 | 1.15 | 1.02 | 22.06 | 1.30 | 1.31 | 1.17 |
| light | 1.12 | 1.14 | 1.06 | 17.47 | 1.40 | 1.43 | 1.17 |
| heavy | 1.15 | 1.17 | 1.02 | 24.23 | 1.39 | 1.39 | 1.17 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 1.131 | win |
| cpu | 1.154 | win |
| rss | 1.038 | win |
| size | 21.784 | win |
| run_wall | 1.616 | win |
| run_cpu | 1.639 | win |
| run_rss | 1.137 | win |

### rmc (AST) vs LLVM

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 14.59 | 16.09 | 20.85 | 44.26 | - | - | 1.40 |
| fib | 15.21 | 16.48 | 21.91 | 24.40 | 0.76 | 0.76 | 1.33 |
| mandel | 15.48 | 16.61 | 22.19 | 16.61 | 1.00 | 0.99 | 1.33 |
| sieve | 15.90 | 17.16 | 22.06 | 18.85 | 1.16 | 1.16 | 1.01 |
| mix | 15.87 | 17.08 | 21.90 | 18.29 | 1.01 | 1.02 | 1.33 |
| stats | 16.05 | 17.32 | 22.73 | 20.46 | 0.90 | 0.90 | 1.33 |
| refs | 14.68 | 15.72 | 21.59 | 17.28 | - | - | 1.33 |
| heap | 15.43 | 16.56 | 22.47 | 16.84 | 0.92 | 0.92 | 1.00 |
| callrec | 15.46 | 16.71 | 22.03 | 24.26 | 0.78 | 0.77 | 1.33 |
| forloops | 16.60 | 17.95 | 21.95 | 21.99 | 1.22 | 1.23 | 1.33 |
| light | 16.51 | 17.79 | 22.60 | 16.58 | 0.98 | 0.98 | 1.33 |
| heavy | 15.87 | 17.06 | 21.68 | 23.82 | 1.35 | 1.35 | 1.33 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 15.638 | win |
| cpu | 16.878 | win |
| rss | 21.997 | win |
| size | 21.970 | win |
| run_wall | 1.009 | tie |
| run_cpu | 1.009 | tie |
| run_rss | 1.284 | win |

### rmc (AST) vs rustc Cranelift (`cl`)

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 13.54 | 14.95 | 20.37 | 43.95 | - | - | 1.20 |
| fib | 13.40 | 14.56 | 20.70 | 24.23 | 2.28 | 2.31 | 1.17 |
| mandel | 12.91 | 13.84 | 21.16 | 16.50 | 1.13 | 1.13 | 1.17 |
| sieve | 12.95 | 14.07 | 20.35 | 18.72 | 1.37 | 1.40 | 1.01 |
| mix | 13.50 | 14.57 | 20.17 | 18.16 | 1.15 | 1.15 | 1.17 |
| stats | 13.43 | 14.60 | 21.31 | 20.32 | 1.28 | 1.30 | 1.17 |
| refs | 12.73 | 13.66 | 19.99 | 17.16 | 21.96 | 24.09 | 1.17 |
| heap | 13.19 | 14.21 | 20.71 | 16.72 | 0.90 | 0.89 | 1.00 |
| callrec | 13.18 | 14.29 | 20.30 | 24.09 | 2.23 | 2.30 | 1.17 |
| forloops | 13.31 | 14.43 | 20.11 | 21.83 | 1.30 | 1.31 | 1.17 |
| light | 13.58 | 14.61 | 22.02 | 16.54 | 1.23 | 1.24 | 1.17 |
| heavy | 13.41 | 14.42 | 20.04 | 23.66 | 1.39 | 1.39 | 1.17 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 13.260 | win |
| cpu | 14.351 | win |
| rss | 20.601 | win |
| size | 21.823 | win |
| run_wall | 3.293 | win |
| run_cpu | 3.501 | win |
| run_rss | 1.142 | win |

## Verdict

* `rmc vs cb`: some axes lost
* `cb vs llvm`: all axes win
* `cb vs cl`: all axes win
* `rmc vs llvm`: some axes lost
* `rmc vs cl`: some axes lost
