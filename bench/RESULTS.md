# Benchmark results: rmc vs rustc

Recorded 2026-11-03 06:55 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 7.16 | 7.07 | 2584 | 6656 |
| hello | rustc | 2069.18 | 2067.92 | 185740 | 289416 |
| hello | ratio (rustc/rmc) | 289.0 | 292.6 | 71.9 | 43.5 |
| fib | rmc | 6.08 | 6.02 | 2596 | 6760 |
| fib | rustc | 2123.73 | 2128.18 | 190428 | 291696 |
| fib | ratio (rustc/rmc) | 349.5 | 353.4 | 73.4 | 43.2 |
| mandel | rmc | 6.27 | 6.23 | 2632 | 7048 |
| mandel | rustc | 2128.81 | 2132.86 | 190368 | 291776 |
| mandel | ratio (rustc/rmc) | 339.3 | 342.6 | 72.3 | 41.4 |
| sieve | rmc | 8.35 | 8.23 | 4324 | 6792 |
| sieve | rustc | 2111.69 | 2116.34 | 187896 | 292016 |
| sieve | ratio (rustc/rmc) | 253.0 | 257.1 | 43.5 | 43.0 |
| mix | rmc | 6.29 | 6.21 | 2600 | 6816 |
| mix | rustc | 2117.39 | 2120.55 | 188464 | 291864 |
| mix | ratio (rustc/rmc) | 336.6 | 341.7 | 72.5 | 42.8 |
| stats | rmc | 6.39 | 6.37 | 2648 | 6904 |
| stats | rustc | 2123.49 | 2126.23 | 188604 | 292048 |
| stats | ratio (rustc/rmc) | 332.1 | 334.1 | 71.2 | 42.3 |
| refs | rmc | 6.46 | 6.39 | 2696 | 7408 |
| refs | rustc | 2137.47 | 2139.24 | 192076 | 292144 |
| refs | ratio (rustc/rmc) | 330.8 | 334.6 | 71.2 | 39.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.23 | 0.12 | 20 |
| hello | rustc | 0.85 | 0.74 | 2088 |
| hello | ratio (rustc/rmc) | 3.65 | 6.07 | 104.40 |
| fib | rmc | 14.14 | 13.99 | 24 |
| fib | rustc | 7.19 | 7.03 | 2036 |
| fib | ratio (rustc/rmc) | 0.51 | 0.50 | 84.83 |
| mandel | rmc | 102.49 | 102.24 | 24 |
| mandel | rustc | 105.43 | 105.20 | 2088 |
| mandel | ratio (rustc/rmc) | 1.03 | 1.03 | 87.00 |
| sieve | rmc | 1.63 | 1.50 | 1584 |
| sieve | rustc | 2.49 | 2.34 | 3628 |
| sieve | ratio (rustc/rmc) | 1.53 | 1.57 | 2.29 |
| mix | rmc | 17.67 | 17.50 | 24 |
| mix | rustc | 15.54 | 15.37 | 2072 |
| mix | ratio (rustc/rmc) | 0.88 | 0.88 | 86.33 |
| stats | rmc | 3.90 | 3.77 | 24 |
| stats | rustc | 3.70 | 3.58 | 2100 |
| stats | ratio (rustc/rmc) | 0.95 | 0.95 | 87.50 |
| refs | rmc | 4.73 | 4.60 | 24 |
| refs | rustc | 0.83 | 0.73 | 2116 |
| refs | ratio (rustc/rmc) | 0.18 | 0.16 | 88.17 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
