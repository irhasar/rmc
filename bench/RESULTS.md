# Benchmark results: rmc vs rustc

Recorded 2026-11-03 06:20 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.18 | 6.11 | 2608 | 6656 |
| hello | rustc | 2106.35 | 2108.64 | 187476 | 289416 |
| hello | ratio (rustc/rmc) | 340.9 | 345.2 | 71.9 | 43.5 |
| fib | rmc | 6.14 | 6.08 | 2624 | 6760 |
| fib | rustc | 2115.89 | 2120.37 | 186972 | 291696 |
| fib | ratio (rustc/rmc) | 344.4 | 348.7 | 71.3 | 43.2 |
| mandel | rmc | 6.12 | 6.07 | 2604 | 7048 |
| mandel | rustc | 2144.51 | 2147.31 | 189756 | 291776 |
| mandel | ratio (rustc/rmc) | 350.6 | 353.8 | 72.9 | 41.4 |
| sieve | rmc | 8.14 | 8.04 | 4316 | 6792 |
| sieve | rustc | 2133.38 | 2137.47 | 187148 | 292016 |
| sieve | ratio (rustc/rmc) | 262.0 | 265.9 | 43.4 | 43.0 |
| mix | rmc | 6.24 | 6.18 | 2604 | 6816 |
| mix | rustc | 2165.36 | 2169.67 | 187424 | 291864 |
| mix | ratio (rustc/rmc) | 347.2 | 351.0 | 72.0 | 42.8 |
| stats | rmc | 6.22 | 6.13 | 2700 | 6904 |
| stats | rustc | 2128.05 | 2132.62 | 188588 | 292048 |
| stats | ratio (rustc/rmc) | 342.2 | 348.0 | 69.8 | 42.3 |
| refs | rmc | 6.34 | 6.29 | 2708 | 7352 |
| refs | rustc | 2118.62 | 2122.41 | 192800 | 292144 |
| refs | ratio (rustc/rmc) | 334.3 | 337.3 | 71.2 | 39.7 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.13 | 20 |
| hello | rustc | 0.84 | 0.72 | 2004 |
| hello | ratio (rustc/rmc) | 3.16 | 5.44 | 100.20 |
| fib | rmc | 14.21 | 14.05 | 24 |
| fib | rustc | 7.29 | 7.16 | 2036 |
| fib | ratio (rustc/rmc) | 0.51 | 0.51 | 84.83 |
| mandel | rmc | 101.88 | 101.69 | 24 |
| mandel | rustc | 105.97 | 105.76 | 2108 |
| mandel | ratio (rustc/rmc) | 1.04 | 1.04 | 87.83 |
| sieve | rmc | 1.67 | 1.55 | 1584 |
| sieve | rustc | 2.56 | 2.40 | 3604 |
| sieve | ratio (rustc/rmc) | 1.53 | 1.56 | 2.28 |
| mix | rmc | 16.87 | 16.73 | 20 |
| mix | rustc | 15.56 | 15.40 | 2016 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 100.80 |
| stats | rmc | 3.89 | 3.76 | 20 |
| stats | rustc | 3.74 | 3.62 | 2076 |
| stats | ratio (rustc/rmc) | 0.96 | 0.97 | 103.80 |
| refs | rmc | 6.31 | 6.14 | 20 |
| refs | rustc | 0.88 | 0.76 | 2100 |
| refs | ratio (rustc/rmc) | 0.14 | 0.12 | 105.00 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
