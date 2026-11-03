# Benchmark results: rmc vs rustc

Recorded 2026-11-03 12:44 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 8.04 | 7.67 | 2616 | 6616 |
| hello | rustc | 2170.15 | 2173.48 | 190128 | 289416 |
| hello | ratio (rustc/rmc) | 269.8 | 283.3 | 72.7 | 43.7 |
| fib | rmc | 7.10 | 7.00 | 2636 | 6704 |
| fib | rustc | 2174.36 | 2174.82 | 190160 | 291696 |
| fib | ratio (rustc/rmc) | 306.3 | 310.6 | 72.1 | 43.5 |
| mandel | rmc | 7.21 | 7.15 | 2664 | 6992 |
| mandel | rustc | 2233.26 | 2237.51 | 192584 | 291776 |
| mandel | ratio (rustc/rmc) | 309.7 | 312.8 | 72.3 | 41.7 |
| sieve | rmc | 9.44 | 9.37 | 4348 | 6744 |
| sieve | rustc | 2181.07 | 2185.75 | 193000 | 292016 |
| sieve | ratio (rustc/rmc) | 231.1 | 233.2 | 44.4 | 43.3 |
| mix | rmc | 7.06 | 6.99 | 2688 | 6792 |
| mix | rustc | 2156.97 | 2158.81 | 189208 | 291864 |
| mix | ratio (rustc/rmc) | 305.5 | 309.0 | 70.4 | 43.0 |
| stats | rmc | 7.53 | 7.49 | 2644 | 6832 |
| stats | rustc | 2219.12 | 2219.29 | 190016 | 292048 |
| stats | ratio (rustc/rmc) | 294.8 | 296.5 | 71.9 | 42.7 |
| refs | rmc | 7.26 | 7.21 | 2724 | 7344 |
| refs | rustc | 2164.88 | 2169.39 | 195260 | 292240 |
| refs | ratio (rustc/rmc) | 298.3 | 300.7 | 71.7 | 39.8 |
| heap | rmc | 7.03 | 6.96 | 2744 | 6736 |
| heap | rustc | 2185.82 | 2178.86 | 191428 | 292384 |
| heap | ratio (rustc/rmc) | 310.8 | 312.9 | 69.8 | 43.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.25 | 0.13 | 28 |
| hello | rustc | 0.86 | 0.74 | 2064 |
| hello | ratio (rustc/rmc) | 3.39 | 5.71 | 73.71 |
| fib | rmc | 7.21 | 7.08 | 28 |
| fib | rustc | 7.24 | 7.10 | 2100 |
| fib | ratio (rustc/rmc) | 1.00 | 1.00 | 75.00 |
| mandel | rmc | 96.18 | 95.95 | 28 |
| mandel | rustc | 105.45 | 105.22 | 2120 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 75.71 |
| sieve | rmc | 1.72 | 1.60 | 1588 |
| sieve | rustc | 2.73 | 2.58 | 3640 |
| sieve | ratio (rustc/rmc) | 1.59 | 1.61 | 2.29 |
| mix | rmc | 14.88 | 14.67 | 24 |
| mix | rustc | 15.48 | 15.32 | 2060 |
| mix | ratio (rustc/rmc) | 1.04 | 1.04 | 85.83 |
| stats | rmc | 3.35 | 3.21 | 28 |
| stats | rustc | 3.75 | 3.62 | 2092 |
| stats | ratio (rustc/rmc) | 1.12 | 1.13 | 74.71 |
| refs | rmc | 4.73 | 4.60 | 28 |
| refs | rustc | 3.88 | 3.73 | 2040 |
| refs | ratio (rustc/rmc) | 0.82 | 0.81 | 72.86 |
| heap | rmc | 3.18 | 3.03 | 4716 |
| heap | rustc | 3.47 | 3.33 | 6696 |
| heap | ratio (rustc/rmc) | 1.09 | 1.10 | 1.42 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
