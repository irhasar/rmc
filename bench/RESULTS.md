# Benchmark results: rmc vs rustc

Recorded 2026-11-03 10:51 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 7.12 | 7.06 | 2648 | 6616 |
| hello | rustc | 2083.20 | 2086.46 | 190192 | 289416 |
| hello | ratio (rustc/rmc) | 292.7 | 295.4 | 71.8 | 43.7 |
| fib | rmc | 6.78 | 6.70 | 2592 | 6704 |
| fib | rustc | 2166.56 | 2169.62 | 189864 | 291696 |
| fib | ratio (rustc/rmc) | 319.4 | 323.7 | 73.2 | 43.5 |
| mandel | rmc | 6.88 | 6.81 | 2604 | 6992 |
| mandel | rustc | 2138.01 | 2142.62 | 192744 | 291776 |
| mandel | ratio (rustc/rmc) | 310.7 | 314.5 | 74.0 | 41.7 |
| sieve | rmc | 9.16 | 9.07 | 4316 | 6744 |
| sieve | rustc | 2126.10 | 2131.05 | 194040 | 292016 |
| sieve | ratio (rustc/rmc) | 232.1 | 235.0 | 45.0 | 43.3 |
| mix | rmc | 6.87 | 6.80 | 2672 | 6760 |
| mix | rustc | 2135.36 | 2139.68 | 192744 | 291864 |
| mix | ratio (rustc/rmc) | 311.0 | 314.8 | 72.1 | 43.2 |
| stats | rmc | 7.11 | 7.07 | 2684 | 6832 |
| stats | rustc | 2154.27 | 2154.25 | 189844 | 292048 |
| stats | ratio (rustc/rmc) | 302.8 | 304.6 | 70.7 | 42.7 |
| refs | rmc | 7.04 | 6.95 | 2720 | 7344 |
| refs | rustc | 2145.37 | 2149.37 | 193484 | 292240 |
| refs | ratio (rustc/rmc) | 304.8 | 309.4 | 71.1 | 39.8 |
| heap | rmc | 8.02 | 7.97 | 2672 | 6736 |
| heap | rustc | 2157.17 | 2161.75 | 189628 | 292368 |
| heap | ratio (rustc/rmc) | 269.0 | 271.3 | 71.0 | 43.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.14 | 24 |
| hello | rustc | 0.85 | 0.73 | 2116 |
| hello | ratio (rustc/rmc) | 3.24 | 5.04 | 88.17 |
| fib | rmc | 10.21 | 10.04 | 28 |
| fib | rustc | 7.42 | 7.28 | 2048 |
| fib | ratio (rustc/rmc) | 0.73 | 0.73 | 73.14 |
| mandel | rmc | 96.17 | 95.96 | 28 |
| mandel | rustc | 105.56 | 105.30 | 2120 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 75.71 |
| sieve | rmc | 1.65 | 1.52 | 1588 |
| sieve | rustc | 2.51 | 2.39 | 3668 |
| sieve | ratio (rustc/rmc) | 1.52 | 1.58 | 2.31 |
| mix | rmc | 16.85 | 16.70 | 24 |
| mix | rustc | 15.50 | 15.35 | 2108 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 87.83 |
| stats | rmc | 3.78 | 3.56 | 28 |
| stats | rustc | 3.74 | 3.61 | 2112 |
| stats | ratio (rustc/rmc) | 0.99 | 1.01 | 75.43 |
| refs | rmc | 4.99 | 4.77 | 28 |
| refs | rustc | 4.99 | 4.83 | 2116 |
| refs | ratio (rustc/rmc) | 1.00 | 1.01 | 75.57 |
| heap | rmc | 3.21 | 3.07 | 4712 |
| heap | rustc | 3.17 | 3.05 | 5124 |
| heap | ratio (rustc/rmc) | 0.99 | 0.99 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
