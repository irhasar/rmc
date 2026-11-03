# Benchmark results: rmc vs rustc

Recorded 2026-11-03 09:55 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.40 | 6.33 | 2632 | 6648 |
| hello | rustc | 2123.38 | 2125.51 | 190188 | 289416 |
| hello | ratio (rustc/rmc) | 332.0 | 335.7 | 72.3 | 43.5 |
| fib | rmc | 7.11 | 7.03 | 2644 | 6744 |
| fib | rustc | 2143.01 | 2144.60 | 189952 | 291696 |
| fib | ratio (rustc/rmc) | 301.3 | 305.2 | 71.8 | 43.3 |
| mandel | rmc | 7.30 | 7.01 | 2624 | 7032 |
| mandel | rustc | 2148.02 | 2152.23 | 192304 | 291776 |
| mandel | ratio (rustc/rmc) | 294.2 | 306.8 | 73.3 | 41.5 |
| sieve | rmc | 8.64 | 8.54 | 4372 | 6792 |
| sieve | rustc | 2130.52 | 2133.10 | 189784 | 292016 |
| sieve | ratio (rustc/rmc) | 246.5 | 249.8 | 43.4 | 43.0 |
| mix | rmc | 6.93 | 6.86 | 2612 | 6800 |
| mix | rustc | 2140.44 | 2144.72 | 194096 | 291864 |
| mix | ratio (rustc/rmc) | 309.0 | 312.9 | 74.3 | 42.9 |
| stats | rmc | 6.62 | 6.58 | 2688 | 6880 |
| stats | rustc | 2138.06 | 2142.58 | 189876 | 292048 |
| stats | ratio (rustc/rmc) | 322.9 | 325.5 | 70.6 | 42.4 |
| refs | rmc | 6.83 | 6.78 | 2728 | 7376 |
| refs | rustc | 2150.11 | 2153.82 | 194456 | 292240 |
| refs | ratio (rustc/rmc) | 314.9 | 317.9 | 71.3 | 39.6 |
| heap | rmc | 6.69 | 6.61 | 2644 | 6800 |
| heap | rustc | 2192.38 | 2188.34 | 190956 | 292368 |
| heap | ratio (rustc/rmc) | 327.8 | 331.1 | 72.2 | 43.0 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.27 | 0.14 | 28 |
| hello | rustc | 0.86 | 0.75 | 2064 |
| hello | ratio (rustc/rmc) | 3.18 | 5.29 | 73.71 |
| fib | rmc | 10.20 | 10.05 | 28 |
| fib | rustc | 7.25 | 7.10 | 2048 |
| fib | ratio (rustc/rmc) | 0.71 | 0.71 | 73.14 |
| mandel | rmc | 96.62 | 96.41 | 28 |
| mandel | rustc | 105.47 | 105.24 | 2096 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 74.86 |
| sieve | rmc | 1.66 | 1.52 | 1588 |
| sieve | rustc | 2.52 | 2.40 | 3632 |
| sieve | ratio (rustc/rmc) | 1.52 | 1.59 | 2.29 |
| mix | rmc | 16.86 | 16.71 | 28 |
| mix | rustc | 15.49 | 15.35 | 2036 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 72.71 |
| stats | rmc | 3.30 | 3.18 | 28 |
| stats | rustc | 3.77 | 3.65 | 2112 |
| stats | ratio (rustc/rmc) | 1.14 | 1.15 | 75.43 |
| refs | rmc | 4.72 | 4.59 | 24 |
| refs | rustc | 3.88 | 3.74 | 2024 |
| refs | ratio (rustc/rmc) | 0.82 | 0.81 | 84.33 |
| heap | rmc | 3.60 | 3.46 | 4716 |
| heap | rustc | 2.89 | 2.75 | 5136 |
| heap | ratio (rustc/rmc) | 0.80 | 0.80 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
