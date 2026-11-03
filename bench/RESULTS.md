# Benchmark results: rmc vs rustc

Recorded 2026-11-03 07:47 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.43 | 6.38 | 2576 | 6648 |
| hello | rustc | 2054.57 | 2056.94 | 187412 | 289416 |
| hello | ratio (rustc/rmc) | 319.3 | 322.2 | 72.8 | 43.5 |
| fib | rmc | 6.88 | 6.65 | 2568 | 6760 |
| fib | rustc | 2131.42 | 2132.47 | 189180 | 291696 |
| fib | ratio (rustc/rmc) | 309.6 | 320.6 | 73.7 | 43.2 |
| mandel | rmc | 6.36 | 6.29 | 2608 | 7032 |
| mandel | rustc | 2115.48 | 2119.58 | 192296 | 291776 |
| mandel | ratio (rustc/rmc) | 332.4 | 336.8 | 73.7 | 41.5 |
| sieve | rmc | 8.52 | 8.38 | 4276 | 6792 |
| sieve | rustc | 2102.05 | 2102.15 | 188028 | 292016 |
| sieve | ratio (rustc/rmc) | 246.6 | 250.7 | 44.0 | 43.0 |
| mix | rmc | 6.28 | 6.20 | 2616 | 6816 |
| mix | rustc | 2128.49 | 2128.18 | 189948 | 291864 |
| mix | ratio (rustc/rmc) | 339.0 | 343.3 | 72.6 | 42.8 |
| stats | rmc | 6.56 | 6.49 | 2668 | 6888 |
| stats | rustc | 2133.55 | 2136.32 | 190588 | 292048 |
| stats | ratio (rustc/rmc) | 325.4 | 329.0 | 71.4 | 42.4 |
| refs | rmc | 6.53 | 6.49 | 2728 | 7400 |
| refs | rustc | 2130.70 | 2135.59 | 195036 | 292240 |
| refs | ratio (rustc/rmc) | 326.1 | 329.2 | 71.5 | 39.5 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.25 | 0.13 | 20 |
| hello | rustc | 1.06 | 0.89 | 2024 |
| hello | ratio (rustc/rmc) | 4.27 | 7.03 | 101.20 |
| fib | rmc | 13.12 | 12.98 | 24 |
| fib | rustc | 7.38 | 7.25 | 2108 |
| fib | ratio (rustc/rmc) | 0.56 | 0.56 | 87.83 |
| mandel | rmc | 96.15 | 95.92 | 24 |
| mandel | rustc | 105.66 | 105.45 | 2064 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 86.00 |
| sieve | rmc | 1.66 | 1.53 | 1584 |
| sieve | rustc | 2.55 | 2.41 | 3640 |
| sieve | ratio (rustc/rmc) | 1.53 | 1.57 | 2.30 |
| mix | rmc | 16.87 | 16.70 | 24 |
| mix | rustc | 15.54 | 15.39 | 2060 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 85.83 |
| stats | rmc | 3.58 | 3.46 | 24 |
| stats | rustc | 3.82 | 3.70 | 2056 |
| stats | ratio (rustc/rmc) | 1.07 | 1.07 | 85.67 |
| refs | rmc | 4.75 | 4.63 | 24 |
| refs | rustc | 3.92 | 3.77 | 2028 |
| refs | ratio (rustc/rmc) | 0.83 | 0.81 | 84.50 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
