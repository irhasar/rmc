# Benchmark results: rmc vs rustc

Recorded 2026-11-03 08:59 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.31 | 6.27 | 2620 | 6648 |
| hello | rustc | 2090.90 | 2091.46 | 190472 | 289416 |
| hello | ratio (rustc/rmc) | 331.4 | 333.7 | 72.7 | 43.5 |
| fib | rmc | 6.45 | 6.40 | 2640 | 6744 |
| fib | rustc | 2110.88 | 2114.95 | 190688 | 291696 |
| fib | ratio (rustc/rmc) | 327.5 | 330.7 | 72.2 | 43.3 |
| mandel | rmc | 7.37 | 6.91 | 2628 | 7032 |
| mandel | rustc | 2141.67 | 2142.49 | 193796 | 291776 |
| mandel | ratio (rustc/rmc) | 290.5 | 310.2 | 73.7 | 41.5 |
| sieve | rmc | 8.55 | 8.47 | 4328 | 6792 |
| sieve | rustc | 2107.70 | 2111.99 | 189340 | 292016 |
| sieve | ratio (rustc/rmc) | 246.4 | 249.3 | 43.7 | 43.0 |
| mix | rmc | 6.42 | 6.38 | 2632 | 6808 |
| mix | rustc | 2100.23 | 2104.52 | 190164 | 291864 |
| mix | ratio (rustc/rmc) | 327.2 | 329.9 | 72.3 | 42.9 |
| stats | rmc | 6.39 | 6.34 | 2672 | 6888 |
| stats | rustc | 2109.00 | 2108.07 | 190040 | 292048 |
| stats | ratio (rustc/rmc) | 330.1 | 332.5 | 71.1 | 42.4 |
| refs | rmc | 6.71 | 6.65 | 2740 | 7376 |
| refs | rustc | 2122.76 | 2127.45 | 195344 | 292240 |
| refs | ratio (rustc/rmc) | 316.4 | 319.9 | 71.3 | 39.6 |
| heap | rmc | 6.54 | 6.45 | 2672 | 6808 |
| heap | rustc | 2154.70 | 2159.06 | 189728 | 292368 |
| heap | ratio (rustc/rmc) | 329.5 | 334.9 | 71.0 | 42.9 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.28 | 0.14 | 28 |
| hello | rustc | 0.89 | 0.76 | 2124 |
| hello | ratio (rustc/rmc) | 3.16 | 5.51 | 75.86 |
| fib | rmc | 9.97 | 9.84 | 28 |
| fib | rustc | 7.23 | 7.09 | 2088 |
| fib | ratio (rustc/rmc) | 0.73 | 0.72 | 74.57 |
| mandel | rmc | 96.49 | 96.25 | 28 |
| mandel | rustc | 105.79 | 105.53 | 2096 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 74.86 |
| sieve | rmc | 1.65 | 1.51 | 1588 |
| sieve | rustc | 2.51 | 2.39 | 3628 |
| sieve | ratio (rustc/rmc) | 1.52 | 1.58 | 2.28 |
| mix | rmc | 16.94 | 16.76 | 28 |
| mix | rustc | 15.55 | 15.41 | 2056 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 73.43 |
| stats | rmc | 3.27 | 3.14 | 28 |
| stats | rustc | 3.76 | 3.62 | 2052 |
| stats | ratio (rustc/rmc) | 1.15 | 1.15 | 73.29 |
| refs | rmc | 4.88 | 4.72 | 24 |
| refs | rustc | 5.00 | 4.81 | 2048 |
| refs | ratio (rustc/rmc) | 1.02 | 1.02 | 85.33 |
| heap | rmc | 3.63 | 3.49 | 4716 |
| heap | rustc | 2.75 | 2.61 | 5124 |
| heap | ratio (rustc/rmc) | 0.76 | 0.75 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
