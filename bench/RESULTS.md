# Benchmark results: rmc vs rustc

Recorded 2026-11-03 12:22 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 7.22 | 7.12 | 2656 | 6616 |
| hello | rustc | 2180.34 | 2183.02 | 189332 | 289416 |
| hello | ratio (rustc/rmc) | 302.0 | 306.6 | 71.3 | 43.7 |
| fib | rmc | 6.90 | 6.83 | 2660 | 6704 |
| fib | rustc | 2200.16 | 2203.71 | 191140 | 291696 |
| fib | ratio (rustc/rmc) | 318.9 | 322.6 | 71.9 | 43.5 |
| mandel | rmc | 7.03 | 6.96 | 2656 | 6992 |
| mandel | rustc | 2206.36 | 2211.07 | 192712 | 291776 |
| mandel | ratio (rustc/rmc) | 313.8 | 317.8 | 72.6 | 41.7 |
| sieve | rmc | 9.12 | 9.05 | 4328 | 6744 |
| sieve | rustc | 2186.92 | 2191.54 | 189880 | 292016 |
| sieve | ratio (rustc/rmc) | 239.7 | 242.2 | 43.9 | 43.3 |
| mix | rmc | 6.98 | 6.92 | 2640 | 6792 |
| mix | rustc | 2161.44 | 2165.73 | 189900 | 291864 |
| mix | ratio (rustc/rmc) | 309.5 | 312.7 | 71.9 | 43.0 |
| stats | rmc | 7.15 | 7.12 | 2696 | 6832 |
| stats | rustc | 2184.16 | 2188.85 | 193468 | 292048 |
| stats | ratio (rustc/rmc) | 305.6 | 307.5 | 71.8 | 42.7 |
| refs | rmc | 7.33 | 7.24 | 2752 | 7344 |
| refs | rustc | 2224.64 | 2229.26 | 190020 | 292240 |
| refs | ratio (rustc/rmc) | 303.6 | 307.8 | 69.0 | 39.8 |
| heap | rmc | 10.96 | 10.02 | 2664 | 6736 |
| heap | rustc | 2157.77 | 2162.69 | 195104 | 292368 |
| heap | ratio (rustc/rmc) | 196.9 | 215.8 | 73.2 | 43.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.14 | 24 |
| hello | rustc | 0.91 | 0.77 | 2056 |
| hello | ratio (rustc/rmc) | 3.50 | 5.33 | 85.67 |
| fib | rmc | 7.27 | 7.13 | 28 |
| fib | rustc | 7.21 | 7.09 | 2064 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 73.71 |
| mandel | rmc | 96.26 | 96.06 | 28 |
| mandel | rustc | 105.80 | 105.33 | 2124 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 75.86 |
| sieve | rmc | 1.75 | 1.63 | 1588 |
| sieve | rustc | 2.61 | 2.46 | 3580 |
| sieve | ratio (rustc/rmc) | 1.49 | 1.51 | 2.25 |
| mix | rmc | 14.85 | 14.69 | 28 |
| mix | rustc | 15.48 | 15.32 | 2076 |
| mix | ratio (rustc/rmc) | 1.04 | 1.04 | 74.14 |
| stats | rmc | 3.37 | 3.23 | 24 |
| stats | rustc | 3.79 | 3.65 | 2052 |
| stats | ratio (rustc/rmc) | 1.12 | 1.13 | 85.50 |
| refs | rmc | 4.81 | 4.64 | 28 |
| refs | rustc | 5.22 | 4.94 | 2060 |
| refs | ratio (rustc/rmc) | 1.09 | 1.06 | 73.57 |
| heap | rmc | 3.40 | 3.25 | 4712 |
| heap | rustc | 3.11 | 2.96 | 5148 |
| heap | ratio (rustc/rmc) | 0.91 | 0.91 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
