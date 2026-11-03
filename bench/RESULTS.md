# Benchmark results: rmc vs rustc

Recorded 2026-11-03 09:30 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.55 | 6.48 | 2612 | 6648 |
| hello | rustc | 2138.91 | 2139.95 | 189328 | 289416 |
| hello | ratio (rustc/rmc) | 326.5 | 330.2 | 72.5 | 43.5 |
| fib | rmc | 6.71 | 6.63 | 2648 | 6744 |
| fib | rustc | 2129.69 | 2131.73 | 189052 | 291696 |
| fib | ratio (rustc/rmc) | 317.6 | 321.6 | 71.4 | 43.3 |
| mandel | rmc | 6.64 | 6.59 | 2636 | 7032 |
| mandel | rustc | 2130.06 | 2132.37 | 192508 | 291776 |
| mandel | ratio (rustc/rmc) | 320.6 | 323.5 | 73.0 | 41.5 |
| sieve | rmc | 8.60 | 8.52 | 4340 | 6792 |
| sieve | rustc | 2108.36 | 2113.15 | 189032 | 292016 |
| sieve | ratio (rustc/rmc) | 245.0 | 248.0 | 43.6 | 43.0 |
| mix | rmc | 6.50 | 6.43 | 2596 | 6808 |
| mix | rustc | 2162.07 | 2166.46 | 193844 | 291864 |
| mix | ratio (rustc/rmc) | 332.6 | 336.9 | 74.7 | 42.9 |
| stats | rmc | 6.60 | 6.50 | 2696 | 6880 |
| stats | rustc | 2144.71 | 2149.35 | 189244 | 292048 |
| stats | ratio (rustc/rmc) | 325.1 | 330.5 | 70.2 | 42.4 |
| refs | rmc | 6.77 | 6.73 | 2708 | 7376 |
| refs | rustc | 2121.44 | 2126.30 | 195052 | 292240 |
| refs | ratio (rustc/rmc) | 313.5 | 315.9 | 72.0 | 39.6 |
| heap | rmc | 6.53 | 6.47 | 2656 | 6808 |
| heap | rustc | 2133.44 | 2136.84 | 195032 | 292368 |
| heap | ratio (rustc/rmc) | 326.6 | 330.1 | 73.4 | 42.9 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.28 | 0.15 | 28 |
| hello | rustc | 0.86 | 0.74 | 2120 |
| hello | ratio (rustc/rmc) | 3.11 | 4.90 | 75.71 |
| fib | rmc | 10.10 | 9.94 | 28 |
| fib | rustc | 7.13 | 6.98 | 2064 |
| fib | ratio (rustc/rmc) | 0.71 | 0.70 | 73.71 |
| mandel | rmc | 96.47 | 96.25 | 28 |
| mandel | rustc | 105.39 | 105.21 | 2080 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 74.29 |
| sieve | rmc | 1.69 | 1.57 | 1588 |
| sieve | rustc | 2.50 | 2.36 | 3552 |
| sieve | ratio (rustc/rmc) | 1.48 | 1.50 | 2.24 |
| mix | rmc | 16.94 | 16.76 | 28 |
| mix | rustc | 15.62 | 15.44 | 2084 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 74.43 |
| stats | rmc | 3.76 | 3.61 | 24 |
| stats | rustc | 3.78 | 3.65 | 2104 |
| stats | ratio (rustc/rmc) | 1.00 | 1.01 | 87.67 |
| refs | rmc | 4.72 | 4.59 | 28 |
| refs | rustc | 3.86 | 3.72 | 2112 |
| refs | ratio (rustc/rmc) | 0.82 | 0.81 | 75.43 |
| heap | rmc | 3.61 | 3.47 | 4712 |
| heap | rustc | 2.76 | 2.63 | 5144 |
| heap | ratio (rustc/rmc) | 0.76 | 0.76 | 1.09 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians and is the
one dimension where rustc wins on call-heavy code.
