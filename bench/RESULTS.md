# Benchmark results: rmc vs rustc

Recorded 2026-11-03 08:44 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.24 | 6.19 | 2588 | 6648 |
| hello | rustc | 2148.85 | 2147.44 | 188676 | 289416 |
| hello | ratio (rustc/rmc) | 344.5 | 346.9 | 72.9 | 43.5 |
| fib | rmc | 6.28 | 6.25 | 2604 | 6744 |
| fib | rustc | 2266.74 | 2269.53 | 187620 | 291696 |
| fib | ratio (rustc/rmc) | 360.8 | 363.2 | 72.1 | 43.3 |
| mandel | rmc | 7.12 | 7.08 | 2616 | 7032 |
| mandel | rustc | 2286.16 | 2279.98 | 193160 | 291776 |
| mandel | ratio (rustc/rmc) | 320.9 | 322.1 | 73.8 | 41.5 |
| sieve | rmc | 8.66 | 8.56 | 4328 | 6792 |
| sieve | rustc | 2225.03 | 2227.98 | 189260 | 292016 |
| sieve | ratio (rustc/rmc) | 257.0 | 260.1 | 43.7 | 43.0 |
| mix | rmc | 7.07 | 7.00 | 2576 | 6808 |
| mix | rustc | 2297.26 | 2301.33 | 190412 | 291864 |
| mix | ratio (rustc/rmc) | 324.8 | 328.9 | 73.9 | 42.9 |
| stats | rmc | 6.47 | 6.37 | 2664 | 6888 |
| stats | rustc | 2190.82 | 2194.53 | 190160 | 292048 |
| stats | ratio (rustc/rmc) | 338.5 | 344.6 | 71.4 | 42.4 |
| refs | rmc | 6.80 | 6.73 | 2772 | 7376 |
| refs | rustc | 2192.56 | 2195.96 | 194840 | 292240 |
| refs | ratio (rustc/rmc) | 322.4 | 326.3 | 70.3 | 39.6 |
| heap | rmc | 9.25 | 8.49 | 2640 | 6808 |
| heap | rustc | 2271.23 | 2276.13 | 194824 | 292368 |
| heap | ratio (rustc/rmc) | 245.6 | 268.1 | 73.8 | 42.9 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.28 | 0.15 | 24 |
| hello | rustc | 0.87 | 0.75 | 2068 |
| hello | ratio (rustc/rmc) | 3.08 | 5.13 | 86.17 |
| fib | rmc | 10.90 | 10.69 | 24 |
| fib | rustc | 7.47 | 7.32 | 2032 |
| fib | ratio (rustc/rmc) | 0.69 | 0.68 | 84.67 |
| mandel | rmc | 96.65 | 96.38 | 24 |
| mandel | rustc | 106.49 | 106.27 | 2052 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 85.50 |
| sieve | rmc | 1.74 | 1.61 | 1584 |
| sieve | rustc | 2.67 | 2.53 | 3608 |
| sieve | ratio (rustc/rmc) | 1.53 | 1.57 | 2.28 |
| mix | rmc | 17.19 | 17.01 | 24 |
| mix | rustc | 15.55 | 15.40 | 2056 |
| mix | ratio (rustc/rmc) | 0.90 | 0.91 | 85.67 |
| stats | rmc | 3.29 | 3.17 | 24 |
| stats | rustc | 3.77 | 3.64 | 2060 |
| stats | ratio (rustc/rmc) | 1.14 | 1.15 | 85.83 |
| refs | rmc | 5.29 | 4.79 | 24 |
| refs | rustc | 5.10 | 4.84 | 2016 |
| refs | ratio (rustc/rmc) | 0.96 | 1.01 | 84.00 |
| heap | rmc | 3.79 | 3.66 | 4712 |
| heap | rustc | 2.79 | 2.64 | 5124 |
| heap | ratio (rustc/rmc) | 0.73 | 0.72 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
