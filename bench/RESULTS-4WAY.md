# Four-way benchmark on one source: rmc, rustc+rmc, Cranelift, LLVM

Recorded 2026-11-13 12:18 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
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
| hello | rmc (AST) | 2.22 | 2.08 | 3828 | 387 | 0.26 | 0.12 | 0 |
| hello | rustc+rmc (`cb`) | 26.03 | 25.92 | 73784 | 380 | 0.26 | 0.12 | 0 |
| hello | rustc LLVM | 33.00 | 33.37 | 79748 | 17128 | 0.27 | 0.13 | 28 |
| hello | rustc Cranelift (`cl`) | 30.22 | 30.70 | 75276 | 17008 | 0.27 | 0.13 | 24 |
| fib | rmc (AST) | 2.36 | 2.22 | 3900 | 702 | 7.17 | 7.01 | 24 |
| fib | rustc+rmc (`cb`) | 27.18 | 27.16 | 76840 | 691 | 5.44 | 5.31 | 24 |
| fib | rustc LLVM | 35.66 | 36.23 | 85260 | 17128 | 5.46 | 5.31 | 32 |
| fib | rustc Cranelift (`cl`) | 31.72 | 32.34 | 78776 | 17008 | 16.41 | 16.26 | 28 |
| mandel | rmc (AST) | 2.53 | 2.39 | 3940 | 1031 | 96.29 | 96.04 | 24 |
| mandel | rustc+rmc (`cb`) | 29.43 | 29.30 | 77680 | 1107 | 97.18 | 96.93 | 24 |
| mandel | rustc LLVM | 39.45 | 40.07 | 86548 | 17128 | 95.91 | 95.64 | 32 |
| mandel | rustc Cranelift (`cl`) | 32.68 | 33.33 | 79668 | 17008 | 109.25 | 108.99 | 28 |
| sieve | rmc (AST) | 2.51 | 2.38 | 3912 | 913 | 1.59 | 1.46 | 1584 |
| sieve | rustc+rmc (`cb`) | 29.41 | 29.40 | 79908 | 888 | 1.73 | 1.61 | 1584 |
| sieve | rustc LLVM | 40.66 | 41.25 | 88028 | 17208 | 1.81 | 1.70 | 1596 |
| sieve | rustc Cranelift (`cl`) | 32.16 | 32.81 | 81116 | 17088 | 2.19 | 2.08 | 1592 |
| mix | rmc (AST) | 2.43 | 2.29 | 3924 | 941 | 14.71 | 14.55 | 24 |
| mix | rustc+rmc (`cb`) | 28.43 | 28.34 | 77476 | 834 | 15.03 | 14.84 | 24 |
| mix | rustc LLVM | 38.38 | 39.00 | 87020 | 17208 | 14.94 | 14.75 | 32 |
| mix | rustc Cranelift (`cl`) | 32.05 | 32.68 | 79860 | 17088 | 16.97 | 16.80 | 28 |
| stats | rmc (AST) | 2.42 | 2.27 | 3884 | 837 | 3.43 | 3.29 | 24 |
| stats | rustc+rmc (`cb`) | 28.91 | 28.82 | 78556 | 899 | 2.67 | 2.53 | 24 |
| stats | rustc LLVM | 38.99 | 39.54 | 89028 | 17128 | 3.09 | 2.98 | 32 |
| stats | rustc Cranelift (`cl`) | 32.00 | 32.72 | 80520 | 17008 | 4.41 | 4.29 | 28 |
| refs | rmc (AST) | 2.58 | 2.44 | 3960 | 991 | 1.90 | 1.70 | 24 |
| refs | rustc+rmc (`cb`) | 30.25 | 30.14 | 79000 | 1448 | 0.27 | 0.14 | 0 |
| refs | rustc LLVM | 38.73 | 39.42 | 89284 | 17128 | 0.25 | 0.13 | 32 |
| refs | rustc Cranelift (`cl`) | 33.06 | 33.66 | 83468 | 17008 | 40.21 | 40.04 | 28 |
| heap | rmc (AST) | 2.49 | 2.34 | 3876 | 1017 | 3.40 | 3.28 | 4712 |
| heap | rustc+rmc (`cb`) | 28.41 | 28.31 | 78116 | 950 | 3.11 | 2.92 | 4712 |
| heap | rustc LLVM | 37.82 | 38.41 | 87020 | 17128 | 3.18 | 3.05 | 4720 |
| heap | rustc Cranelift (`cl`) | 32.49 | 33.12 | 80044 | 17008 | 3.08 | 2.93 | 4716 |
| callrec | rmc (AST) | 2.33 | 2.19 | 3840 | 706 | 2.90 | 2.76 | 24 |
| callrec | rustc+rmc (`cb`) | 26.75 | 26.66 | 77024 | 691 | 2.23 | 2.11 | 24 |
| callrec | rustc LLVM | 35.42 | 35.98 | 85584 | 17128 | 2.23 | 2.11 | 32 |
| callrec | rustc Cranelift (`cl`) | 31.05 | 31.65 | 78984 | 17008 | 6.46 | 6.33 | 28 |
| forloops | rmc (AST) | 2.43 | 2.28 | 3928 | 779 | 2.05 | 1.92 | 24 |
| forloops | rustc+rmc (`cb`) | 28.74 | 28.63 | 78352 | 771 | 2.03 | 1.91 | 24 |
| forloops | rustc LLVM | 40.16 | 40.72 | 87508 | 17128 | 2.45 | 2.34 | 36 |
| forloops | rustc Cranelift (`cl`) | 32.52 | 33.22 | 80180 | 17008 | 2.63 | 2.51 | 28 |
| light | rmc (AST) | 2.49 | 2.33 | 3872 | 1033 | 2.09 | 1.96 | 24 |
| light | rustc+rmc (`cb`) | 30.42 | 30.34 | 82228 | 978 | 1.81 | 1.70 | 24 |
| light | rustc LLVM | 41.01 | 41.80 | 89360 | 17128 | 2.03 | 1.91 | 32 |
| light | rustc Cranelift (`cl`) | 34.04 | 34.76 | 84560 | 17088 | 2.54 | 2.41 | 28 |
| heavy | rmc (AST) | 2.37 | 2.24 | 3804 | 719 | 758.69 | 758.39 | 24 |
| heavy | rustc+rmc (`cb`) | 27.61 | 27.63 | 77200 | 702 | 758.41 | 758.06 | 24 |
| heavy | rustc LLVM | 36.92 | 37.52 | 85720 | 17128 | 1028.95 | 1028.66 | 32 |
| heavy | rustc Cranelift (`cl`) | 31.92 | 32.47 | 79164 | 17008 | 1054.13 | 1053.80 | 28 |

### rmc (AST) vs rustc+rmc (`cb`) - front end only

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 11.75 | 12.45 | 19.27 | 0.98 | - | - | - |
| fib | 11.50 | 12.22 | 19.70 | 0.98 | 0.76 | 0.76 | 1.00 |
| mandel | 11.64 | 12.28 | 19.72 | 1.07 | 1.01 | 1.01 | 1.00 |
| sieve | 11.69 | 12.37 | 20.43 | 0.97 | 1.09 | 1.10 | 1.00 |
| mix | 11.70 | 12.36 | 19.74 | 0.89 | 1.02 | 1.02 | 1.00 |
| stats | 11.95 | 12.67 | 20.23 | 1.07 | 0.78 | 0.77 | 1.00 |
| refs | 11.70 | 12.33 | 19.95 | 1.46 | - | - | - |
| heap | 11.42 | 12.09 | 20.15 | 0.93 | 0.91 | 0.89 | 1.00 |
| callrec | 11.47 | 12.16 | 20.06 | 0.98 | 0.77 | 0.76 | 1.00 |
| forloops | 11.83 | 12.54 | 19.95 | 0.99 | 0.99 | 1.00 | 1.00 |
| light | 12.20 | 13.04 | 21.24 | 0.95 | 0.87 | 0.87 | 1.00 |
| heavy | 11.63 | 12.33 | 20.29 | 0.98 | 1.00 | 1.00 | 1.00 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 11.707 | win |
| cpu | 12.404 | win |
| rss | 20.061 | win |
| size | 1.022 | win |
| run_wall | 0.920 | loss |
| run_cpu | 0.918 | loss |
| run_rss | 1.000 | tie |

### rustc+rmc (`cb`) vs LLVM

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 1.27 | 1.29 | 1.08 | 45.07 | - | - | - |
| fib | 1.31 | 1.33 | 1.11 | 24.79 | 1.00 | 1.00 | 1.33 |
| mandel | 1.34 | 1.37 | 1.11 | 15.47 | 0.99 | 0.99 | 1.33 |
| sieve | 1.38 | 1.40 | 1.10 | 19.38 | 1.05 | 1.06 | 1.01 |
| mix | 1.35 | 1.38 | 1.12 | 20.63 | 0.99 | 0.99 | 1.33 |
| stats | 1.35 | 1.37 | 1.13 | 19.05 | 1.16 | 1.18 | 1.33 |
| refs | 1.28 | 1.31 | 1.13 | 11.83 | - | - | - |
| heap | 1.33 | 1.36 | 1.11 | 18.03 | 1.02 | 1.04 | 1.00 |
| callrec | 1.32 | 1.35 | 1.11 | 24.79 | 1.00 | 1.00 | 1.33 |
| forloops | 1.40 | 1.42 | 1.12 | 22.22 | 1.21 | 1.22 | 1.50 |
| light | 1.35 | 1.38 | 1.09 | 17.51 | 1.12 | 1.13 | 1.33 |
| heavy | 1.34 | 1.36 | 1.11 | 24.40 | 1.36 | 1.36 | 1.33 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 1.335 | win |
| cpu | 1.359 | win |
| rss | 1.111 | win |
| size | 21.931 | win |
| run_wall | 1.090 | win |
| run_cpu | 1.096 | win |
| run_rss | 1.284 | win |

### rustc+rmc (`cb`) vs rustc Cranelift (`cl`)

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 1.16 | 1.18 | 1.02 | 44.76 | - | - | - |
| fib | 1.17 | 1.19 | 1.03 | 24.61 | 3.02 | 3.06 | 1.17 |
| mandel | 1.11 | 1.14 | 1.03 | 15.36 | 1.12 | 1.12 | 1.17 |
| sieve | 1.09 | 1.12 | 1.02 | 19.24 | 1.27 | 1.29 | 1.01 |
| mix | 1.13 | 1.15 | 1.03 | 20.49 | 1.13 | 1.13 | 1.17 |
| stats | 1.11 | 1.14 | 1.03 | 18.92 | 1.65 | 1.69 | 1.17 |
| refs | 1.09 | 1.12 | 1.06 | 11.75 | - | - | - |
| heap | 1.14 | 1.17 | 1.02 | 17.90 | 0.99 | 1.00 | 1.00 |
| callrec | 1.16 | 1.19 | 1.03 | 24.61 | 2.90 | 3.00 | 1.17 |
| forloops | 1.13 | 1.16 | 1.02 | 22.06 | 1.29 | 1.31 | 1.17 |
| light | 1.12 | 1.15 | 1.03 | 17.47 | 1.40 | 1.42 | 1.17 |
| heavy | 1.16 | 1.18 | 1.03 | 24.23 | 1.39 | 1.39 | 1.17 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 1.131 | win |
| cpu | 1.156 | win |
| rss | 1.027 | win |
| size | 21.784 | win |
| run_wall | 1.616 | win |
| run_cpu | 1.643 | win |
| run_rss | 1.134 | win |

### rmc (AST) vs LLVM

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 14.89 | 16.03 | 20.83 | 44.26 | - | - | - |
| fib | 15.09 | 16.30 | 21.86 | 24.40 | 0.76 | 0.76 | 1.33 |
| mandel | 15.60 | 16.79 | 21.97 | 16.61 | 1.00 | 1.00 | 1.33 |
| sieve | 16.17 | 17.36 | 22.50 | 18.85 | 1.14 | 1.16 | 1.01 |
| mix | 15.80 | 17.02 | 22.18 | 18.29 | 1.02 | 1.01 | 1.33 |
| stats | 16.12 | 17.39 | 22.92 | 20.46 | 0.90 | 0.90 | 1.33 |
| refs | 14.98 | 16.13 | 22.55 | 17.28 | - | - | 1.33 |
| heap | 15.20 | 16.40 | 22.45 | 16.84 | 0.94 | 0.93 | 1.00 |
| callrec | 15.18 | 16.42 | 22.29 | 24.26 | 0.77 | 0.76 | 1.33 |
| forloops | 16.53 | 17.84 | 22.28 | 21.99 | 1.20 | 1.22 | 1.50 |
| light | 16.44 | 17.97 | 23.08 | 16.58 | 0.97 | 0.98 | 1.33 |
| heavy | 15.56 | 16.75 | 22.53 | 23.82 | 1.36 | 1.36 | 1.33 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 15.630 | win |
| cpu | 16.866 | win |
| rss | 22.286 | win |
| size | 21.970 | win |
| run_wall | 1.005 | tie |
| run_cpu | 1.008 | tie |
| run_rss | 1.289 | win |

### rmc (AST) vs rustc Cranelift (`cl`)

| program | wall | cpu | rss | size | run_wall | run_cpu | run_rss |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| hello | 13.64 | 14.75 | 19.66 | 43.95 | - | - | - |
| fib | 13.43 | 14.55 | 20.20 | 24.23 | 2.29 | 2.32 | 1.17 |
| mandel | 12.93 | 13.96 | 20.22 | 16.50 | 1.13 | 1.13 | 1.17 |
| sieve | 12.79 | 13.80 | 20.74 | 18.72 | 1.38 | 1.42 | 1.01 |
| mix | 13.19 | 14.26 | 20.35 | 18.16 | 1.15 | 1.15 | 1.17 |
| stats | 13.23 | 14.39 | 20.73 | 20.32 | 1.29 | 1.30 | 1.17 |
| refs | 12.79 | 13.77 | 21.08 | 17.16 | 21.12 | 23.57 | 1.17 |
| heap | 13.06 | 14.14 | 20.65 | 16.72 | 0.91 | 0.90 | 1.00 |
| callrec | 13.31 | 14.44 | 20.57 | 24.09 | 2.22 | 2.29 | 1.17 |
| forloops | 13.38 | 14.56 | 20.41 | 21.83 | 1.28 | 1.31 | 1.17 |
| light | 13.65 | 14.95 | 21.84 | 16.54 | 1.22 | 1.23 | 1.17 |
| heavy | 13.45 | 14.49 | 20.81 | 23.66 | 1.39 | 1.39 | 1.17 |

| axis | mean | verdict |
| --- | ---: | --- |
| wall | 13.237 | win |
| cpu | 14.338 | win |
| rss | 20.605 | win |
| size | 21.823 | win |
| run_wall | 3.217 | win |
| run_cpu | 3.456 | win |
| run_rss | 1.137 | win |

## Verdict

* `rmc vs cb`: some axes lost
* `cb vs llvm`: all axes win
* `cb vs cl`: all axes win
* `rmc vs llvm`: some axes lost
* `rmc vs cl`: some axes lost
