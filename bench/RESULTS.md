# Benchmark results: rmc vs rustc

Recorded 2026-11-03 07:28 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.84 | 6.72 | 2548 | 6656 |
| hello | rustc | 2047.66 | 2050.94 | 188684 | 289416 |
| hello | ratio (rustc/rmc) | 299.6 | 305.3 | 74.1 | 43.5 |
| fib | rmc | 6.18 | 6.15 | 2632 | 6760 |
| fib | rustc | 2132.33 | 2132.48 | 188308 | 291696 |
| fib | ratio (rustc/rmc) | 345.2 | 346.7 | 71.5 | 43.2 |
| mandel | rmc | 6.66 | 6.59 | 2724 | 7032 |
| mandel | rustc | 2157.52 | 2161.99 | 190392 | 291776 |
| mandel | ratio (rustc/rmc) | 323.8 | 328.1 | 69.9 | 41.5 |
| sieve | rmc | 10.09 | 9.61 | 4260 | 6792 |
| sieve | rustc | 2104.23 | 2108.76 | 190556 | 292016 |
| sieve | ratio (rustc/rmc) | 208.5 | 219.4 | 44.7 | 43.0 |
| mix | rmc | 6.18 | 6.12 | 2628 | 6816 |
| mix | rustc | 2133.50 | 2138.17 | 190976 | 291864 |
| mix | ratio (rustc/rmc) | 345.0 | 349.4 | 72.7 | 42.8 |
| stats | rmc | 6.16 | 6.11 | 2728 | 6896 |
| stats | rustc | 2098.44 | 2102.82 | 187676 | 292048 |
| stats | ratio (rustc/rmc) | 340.4 | 343.9 | 68.8 | 42.4 |
| refs | rmc | 6.50 | 6.47 | 2692 | 7408 |
| refs | rustc | 2121.05 | 2124.71 | 193468 | 292240 |
| refs | ratio (rustc/rmc) | 326.3 | 328.6 | 71.9 | 39.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.25 | 0.13 | 20 |
| hello | rustc | 0.86 | 0.74 | 2096 |
| hello | ratio (rustc/rmc) | 3.38 | 5.76 | 104.80 |
| fib | rmc | 14.13 | 13.97 | 24 |
| fib | rustc | 7.46 | 7.30 | 2072 |
| fib | ratio (rustc/rmc) | 0.53 | 0.52 | 86.33 |
| mandel | rmc | 96.05 | 95.88 | 24 |
| mandel | rustc | 105.51 | 105.30 | 2044 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 85.17 |
| sieve | rmc | 1.75 | 1.59 | 1584 |
| sieve | rustc | 2.47 | 2.35 | 3672 |
| sieve | ratio (rustc/rmc) | 1.41 | 1.48 | 2.32 |
| mix | rmc | 16.96 | 16.81 | 24 |
| mix | rustc | 15.58 | 15.43 | 2100 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 87.50 |
| stats | rmc | 4.08 | 3.91 | 24 |
| stats | rustc | 3.95 | 3.77 | 2056 |
| stats | ratio (rustc/rmc) | 0.97 | 0.96 | 85.67 |
| refs | rmc | 4.78 | 4.66 | 24 |
| refs | rustc | 3.91 | 3.76 | 2100 |
| refs | ratio (rustc/rmc) | 0.82 | 0.81 | 87.50 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
