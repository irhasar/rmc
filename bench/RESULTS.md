# Benchmark results: rmc vs rustc

Recorded 2026-11-03 07:12 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.53 | 6.31 | 2600 | 6656 |
| hello | rustc | 2091.63 | 2094.79 | 187544 | 289416 |
| hello | ratio (rustc/rmc) | 320.2 | 332.2 | 72.1 | 43.5 |
| fib | rmc | 6.15 | 6.08 | 2592 | 6760 |
| fib | rustc | 2158.17 | 2155.31 | 187756 | 291696 |
| fib | ratio (rustc/rmc) | 350.9 | 354.7 | 72.4 | 43.2 |
| mandel | rmc | 6.25 | 6.22 | 2640 | 7048 |
| mandel | rustc | 2142.39 | 2145.91 | 190664 | 291776 |
| mandel | ratio (rustc/rmc) | 342.8 | 344.8 | 72.2 | 41.4 |
| sieve | rmc | 8.64 | 8.44 | 4336 | 6792 |
| sieve | rustc | 2129.05 | 2131.88 | 187136 | 292016 |
| sieve | ratio (rustc/rmc) | 246.3 | 252.7 | 43.2 | 43.0 |
| mix | rmc | 6.45 | 6.35 | 2628 | 6816 |
| mix | rustc | 2103.20 | 2106.98 | 190896 | 291864 |
| mix | ratio (rustc/rmc) | 326.2 | 331.6 | 72.6 | 42.8 |
| stats | rmc | 6.20 | 6.17 | 2632 | 6904 |
| stats | rustc | 2122.96 | 2125.15 | 187252 | 292048 |
| stats | ratio (rustc/rmc) | 342.6 | 344.6 | 71.1 | 42.3 |
| refs | rmc | 6.48 | 6.41 | 2684 | 7408 |
| refs | rustc | 2122.03 | 2120.40 | 192884 | 292240 |
| refs | ratio (rustc/rmc) | 327.5 | 330.7 | 71.9 | 39.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.27 | 0.14 | 20 |
| hello | rustc | 0.86 | 0.74 | 2072 |
| hello | ratio (rustc/rmc) | 3.14 | 5.46 | 103.60 |
| fib | rmc | 14.33 | 14.17 | 24 |
| fib | rustc | 7.17 | 7.02 | 2052 |
| fib | ratio (rustc/rmc) | 0.50 | 0.50 | 85.50 |
| mandel | rmc | 102.23 | 102.00 | 24 |
| mandel | rustc | 105.75 | 105.47 | 2068 |
| mandel | ratio (rustc/rmc) | 1.03 | 1.03 | 86.17 |
| sieve | rmc | 1.68 | 1.56 | 1584 |
| sieve | rustc | 2.54 | 2.40 | 3584 |
| sieve | ratio (rustc/rmc) | 1.51 | 1.54 | 2.26 |
| mix | rmc | 16.90 | 16.75 | 24 |
| mix | rustc | 15.52 | 15.36 | 2020 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 84.17 |
| stats | rmc | 3.88 | 3.74 | 24 |
| stats | rustc | 3.76 | 3.65 | 2004 |
| stats | ratio (rustc/rmc) | 0.97 | 0.97 | 83.50 |
| refs | rmc | 4.72 | 4.59 | 24 |
| refs | rustc | 3.86 | 3.74 | 2036 |
| refs | ratio (rustc/rmc) | 0.82 | 0.81 | 84.83 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
