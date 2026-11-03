# Benchmark results: rmc vs rustc

Recorded 2026-11-03 11:12 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 7.39 | 7.30 | 2624 | 6616 |
| hello | rustc | 2133.03 | 2135.61 | 190460 | 289416 |
| hello | ratio (rustc/rmc) | 288.6 | 292.3 | 72.6 | 43.7 |
| fib | rmc | 6.86 | 6.82 | 2668 | 6704 |
| fib | rustc | 2161.12 | 2162.88 | 191628 | 291696 |
| fib | ratio (rustc/rmc) | 315.1 | 317.0 | 71.8 | 43.5 |
| mandel | rmc | 8.99 | 8.92 | 2584 | 6992 |
| mandel | rustc | 2137.15 | 2140.84 | 192228 | 291776 |
| mandel | ratio (rustc/rmc) | 237.6 | 240.0 | 74.4 | 41.7 |
| sieve | rmc | 9.11 | 8.99 | 4212 | 6744 |
| sieve | rustc | 2140.84 | 2143.56 | 191700 | 292016 |
| sieve | ratio (rustc/rmc) | 234.9 | 238.4 | 45.5 | 43.3 |
| mix | rmc | 6.80 | 6.71 | 2676 | 6792 |
| mix | rustc | 2205.05 | 2208.43 | 190880 | 291864 |
| mix | ratio (rustc/rmc) | 324.4 | 329.1 | 71.3 | 43.0 |
| stats | rmc | 6.89 | 6.79 | 2744 | 6832 |
| stats | rustc | 2133.77 | 2136.69 | 190020 | 292048 |
| stats | ratio (rustc/rmc) | 309.7 | 314.7 | 69.2 | 42.7 |
| refs | rmc | 7.05 | 6.99 | 2728 | 7344 |
| refs | rustc | 2155.21 | 2159.53 | 194616 | 292240 |
| refs | ratio (rustc/rmc) | 305.7 | 309.0 | 71.3 | 39.8 |
| heap | rmc | 6.87 | 6.81 | 2664 | 6736 |
| heap | rustc | 2132.56 | 2135.77 | 194880 | 292368 |
| heap | ratio (rustc/rmc) | 310.5 | 313.6 | 73.2 | 43.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.14 | 28 |
| hello | rustc | 0.99 | 0.83 | 2036 |
| hello | ratio (rustc/rmc) | 3.78 | 6.01 | 72.71 |
| fib | rmc | 10.26 | 10.09 | 28 |
| fib | rustc | 7.23 | 7.09 | 2032 |
| fib | ratio (rustc/rmc) | 0.70 | 0.70 | 72.57 |
| mandel | rmc | 97.26 | 97.06 | 24 |
| mandel | rustc | 105.67 | 105.43 | 2076 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 86.50 |
| sieve | rmc | 1.63 | 1.51 | 1588 |
| sieve | rustc | 2.49 | 2.34 | 3620 |
| sieve | ratio (rustc/rmc) | 1.53 | 1.55 | 2.28 |
| mix | rmc | 14.85 | 14.67 | 28 |
| mix | rustc | 15.67 | 15.47 | 2116 |
| mix | ratio (rustc/rmc) | 1.06 | 1.05 | 75.57 |
| stats | rmc | 3.32 | 3.18 | 24 |
| stats | rustc | 3.75 | 3.62 | 2056 |
| stats | ratio (rustc/rmc) | 1.13 | 1.14 | 85.67 |
| refs | rmc | 4.75 | 4.60 | 28 |
| refs | rustc | 3.92 | 3.79 | 2068 |
| refs | ratio (rustc/rmc) | 0.83 | 0.82 | 73.86 |
| heap | rmc | 3.20 | 3.08 | 4716 |
| heap | rustc | 2.73 | 2.60 | 5096 |
| heap | ratio (rustc/rmc) | 0.85 | 0.84 | 1.08 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
