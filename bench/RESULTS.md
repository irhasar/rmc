# Benchmark results: rmc vs rustc

Recorded 2026-11-03 08:14 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.54 | 6.46 | 2592 | 6648 |
| hello | rustc | 2209.56 | 2213.20 | 187964 | 289416 |
| hello | ratio (rustc/rmc) | 337.6 | 342.9 | 72.5 | 43.5 |
| fib | rmc | 6.63 | 6.57 | 2640 | 6752 |
| fib | rustc | 2238.60 | 2241.10 | 189364 | 291696 |
| fib | ratio (rustc/rmc) | 337.4 | 341.2 | 71.7 | 43.2 |
| mandel | rmc | 6.65 | 6.55 | 2608 | 7032 |
| mandel | rustc | 2267.25 | 2262.30 | 192620 | 291776 |
| mandel | ratio (rustc/rmc) | 340.9 | 345.4 | 73.9 | 41.5 |
| sieve | rmc | 8.46 | 8.39 | 4268 | 6792 |
| sieve | rustc | 2212.00 | 2213.89 | 192972 | 292016 |
| sieve | ratio (rustc/rmc) | 261.4 | 263.8 | 45.2 | 43.0 |
| mix | rmc | 6.74 | 6.71 | 2636 | 6816 |
| mix | rustc | 2261.15 | 2265.23 | 189884 | 291864 |
| mix | ratio (rustc/rmc) | 335.6 | 337.3 | 72.0 | 42.8 |
| stats | rmc | 6.57 | 6.47 | 2624 | 6888 |
| stats | rustc | 2275.17 | 2279.90 | 190164 | 292048 |
| stats | ratio (rustc/rmc) | 346.3 | 352.1 | 72.5 | 42.4 |
| refs | rmc | 6.73 | 6.64 | 2700 | 7392 |
| refs | rustc | 2216.99 | 2219.15 | 195128 | 292240 |
| refs | ratio (rustc/rmc) | 329.2 | 334.4 | 72.3 | 39.5 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.25 | 0.13 | 24 |
| hello | rustc | 0.89 | 0.78 | 2084 |
| hello | ratio (rustc/rmc) | 3.56 | 5.96 | 86.83 |
| fib | rmc | 10.39 | 10.24 | 24 |
| fib | rustc | 7.41 | 7.26 | 2028 |
| fib | ratio (rustc/rmc) | 0.71 | 0.71 | 84.50 |
| mandel | rmc | 98.11 | 97.91 | 24 |
| mandel | rustc | 108.14 | 107.90 | 2104 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 87.67 |
| sieve | rmc | 1.74 | 1.61 | 1584 |
| sieve | rustc | 2.60 | 2.48 | 3664 |
| sieve | ratio (rustc/rmc) | 1.49 | 1.54 | 2.31 |
| mix | rmc | 18.18 | 17.99 | 24 |
| mix | rustc | 16.76 | 16.54 | 2064 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 86.00 |
| stats | rmc | 3.64 | 3.51 | 24 |
| stats | rustc | 4.14 | 3.99 | 2020 |
| stats | ratio (rustc/rmc) | 1.14 | 1.14 | 84.17 |
| refs | rmc | 5.55 | 5.38 | 24 |
| refs | rustc | 4.54 | 4.37 | 2108 |
| refs | ratio (rustc/rmc) | 0.82 | 0.81 | 87.83 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
