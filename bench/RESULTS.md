# Benchmark results: rmc vs rustc

Recorded 2026-11-03 11:39 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 7.03 | 6.98 | 2616 | 6616 |
| hello | rustc | 2081.23 | 2084.57 | 189012 | 289416 |
| hello | ratio (rustc/rmc) | 296.0 | 298.8 | 72.3 | 43.7 |
| fib | rmc | 6.86 | 6.76 | 2628 | 6704 |
| fib | rustc | 2132.60 | 2136.85 | 188348 | 291696 |
| fib | ratio (rustc/rmc) | 311.1 | 316.1 | 71.7 | 43.5 |
| mandel | rmc | 6.87 | 6.80 | 2664 | 6992 |
| mandel | rustc | 2178.24 | 2181.28 | 192752 | 291776 |
| mandel | ratio (rustc/rmc) | 317.2 | 320.8 | 72.4 | 41.7 |
| sieve | rmc | 8.86 | 8.82 | 4352 | 6744 |
| sieve | rustc | 2126.79 | 2131.29 | 190324 | 292016 |
| sieve | ratio (rustc/rmc) | 240.0 | 241.6 | 43.7 | 43.3 |
| mix | rmc | 6.89 | 6.80 | 2620 | 6792 |
| mix | rustc | 2151.98 | 2156.64 | 189064 | 291864 |
| mix | ratio (rustc/rmc) | 312.5 | 317.2 | 72.2 | 43.0 |
| stats | rmc | 9.00 | 8.83 | 2680 | 6832 |
| stats | rustc | 2141.48 | 2145.79 | 193252 | 292048 |
| stats | ratio (rustc/rmc) | 237.8 | 242.9 | 72.1 | 42.7 |
| refs | rmc | 7.26 | 7.20 | 2740 | 7344 |
| refs | rustc | 2147.45 | 2149.72 | 195156 | 292240 |
| refs | ratio (rustc/rmc) | 295.8 | 298.7 | 71.2 | 39.8 |
| heap | rmc | 7.02 | 6.96 | 2640 | 6736 |
| heap | rustc | 2124.96 | 2129.52 | 189672 | 292368 |
| heap | ratio (rustc/rmc) | 302.5 | 305.7 | 71.8 | 43.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.15 | 28 |
| hello | rustc | 0.89 | 0.75 | 2016 |
| hello | ratio (rustc/rmc) | 3.37 | 5.09 | 72.00 |
| fib | rmc | 7.20 | 7.07 | 28 |
| fib | rustc | 7.17 | 7.00 | 2112 |
| fib | ratio (rustc/rmc) | 1.00 | 0.99 | 75.43 |
| mandel | rmc | 96.39 | 96.21 | 28 |
| mandel | rustc | 105.36 | 105.12 | 2096 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 74.86 |
| sieve | rmc | 1.64 | 1.52 | 1588 |
| sieve | rustc | 2.63 | 2.49 | 3616 |
| sieve | ratio (rustc/rmc) | 1.61 | 1.64 | 2.28 |
| mix | rmc | 14.89 | 14.75 | 24 |
| mix | rustc | 16.22 | 15.81 | 2036 |
| mix | ratio (rustc/rmc) | 1.09 | 1.07 | 84.83 |
| stats | rmc | 5.46 | 5.33 | 24 |
| stats | rustc | 5.02 | 4.85 | 2096 |
| stats | ratio (rustc/rmc) | 0.92 | 0.91 | 87.33 |
| refs | rmc | 4.73 | 4.59 | 28 |
| refs | rustc | 3.96 | 3.82 | 2032 |
| refs | ratio (rustc/rmc) | 0.84 | 0.83 | 72.57 |
| heap | rmc | 3.18 | 3.04 | 4716 |
| heap | rustc | 2.78 | 2.65 | 5156 |
| heap | ratio (rustc/rmc) | 0.88 | 0.87 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
