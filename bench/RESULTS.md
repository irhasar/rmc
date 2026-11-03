# Benchmark results: rmc vs rustc

Recorded 2026-11-03 09:18 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 7.47 | 7.02 | 2556 | 6648 |
| hello | rustc | 2206.71 | 2209.74 | 190528 | 289416 |
| hello | ratio (rustc/rmc) | 295.6 | 314.6 | 74.5 | 43.5 |
| fib | rmc | 6.50 | 6.45 | 2584 | 6744 |
| fib | rustc | 2187.91 | 2187.69 | 189196 | 291696 |
| fib | ratio (rustc/rmc) | 336.7 | 339.0 | 73.2 | 43.3 |
| mandel | rmc | 6.78 | 6.65 | 2692 | 7032 |
| mandel | rustc | 2111.38 | 2116.15 | 191216 | 291776 |
| mandel | ratio (rustc/rmc) | 311.6 | 318.4 | 71.0 | 41.5 |
| sieve | rmc | 8.70 | 8.65 | 4340 | 6792 |
| sieve | rustc | 2126.46 | 2130.93 | 189592 | 292016 |
| sieve | ratio (rustc/rmc) | 244.5 | 246.2 | 43.7 | 43.0 |
| mix | rmc | 6.67 | 6.64 | 2652 | 6808 |
| mix | rustc | 2157.05 | 2159.96 | 193088 | 291864 |
| mix | ratio (rustc/rmc) | 323.5 | 325.3 | 72.8 | 42.9 |
| stats | rmc | 6.57 | 6.48 | 2664 | 6880 |
| stats | rustc | 2127.51 | 2127.84 | 189492 | 292048 |
| stats | ratio (rustc/rmc) | 324.0 | 328.1 | 71.1 | 42.4 |
| refs | rmc | 6.85 | 6.79 | 2712 | 7376 |
| refs | rustc | 2124.85 | 2125.45 | 195228 | 292240 |
| refs | ratio (rustc/rmc) | 310.4 | 313.1 | 72.0 | 39.6 |
| heap | rmc | 6.44 | 6.39 | 2640 | 6808 |
| heap | rustc | 2134.91 | 2139.18 | 194452 | 292368 |
| heap | ratio (rustc/rmc) | 331.4 | 334.8 | 73.7 | 42.9 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.14 | 28 |
| hello | rustc | 0.89 | 0.77 | 2076 |
| hello | ratio (rustc/rmc) | 3.36 | 5.32 | 74.14 |
| fib | rmc | 10.07 | 9.93 | 24 |
| fib | rustc | 7.19 | 7.07 | 2036 |
| fib | ratio (rustc/rmc) | 0.71 | 0.71 | 84.83 |
| mandel | rmc | 95.67 | 95.47 | 28 |
| mandel | rustc | 106.05 | 105.73 | 2100 |
| mandel | ratio (rustc/rmc) | 1.11 | 1.11 | 75.00 |
| sieve | rmc | 1.67 | 1.55 | 1588 |
| sieve | rustc | 2.52 | 2.40 | 3668 |
| sieve | ratio (rustc/rmc) | 1.51 | 1.55 | 2.31 |
| mix | rmc | 16.86 | 16.72 | 24 |
| mix | rustc | 15.51 | 15.35 | 2040 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 85.00 |
| stats | rmc | 3.30 | 3.18 | 28 |
| stats | rustc | 3.75 | 3.63 | 2028 |
| stats | ratio (rustc/rmc) | 1.14 | 1.14 | 72.43 |
| refs | rmc | 5.34 | 4.75 | 28 |
| refs | rustc | 4.96 | 4.81 | 2000 |
| refs | ratio (rustc/rmc) | 0.93 | 1.01 | 71.43 |
| heap | rmc | 3.84 | 3.67 | 4716 |
| heap | rustc | 2.81 | 2.62 | 5132 |
| heap | ratio (rustc/rmc) | 0.73 | 0.71 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
