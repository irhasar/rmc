# Benchmark results: rmc vs rustc

Recorded 2026-11-04 10:07 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 6.58 | 6.48 | 2700 | 6616 |
| hello | rustc | 2066.61 | 2069.58 | 188708 | 289416 |
| hello | ratio (rustc/rmc) | 313.9 | 319.6 | 69.9 | 43.7 |
| fib | rmc | 5.89 | 5.85 | 2660 | 6704 |
| fib | rustc | 2076.70 | 2080.06 | 188452 | 291696 |
| fib | ratio (rustc/rmc) | 352.5 | 355.8 | 70.8 | 43.5 |
| mandel | rmc | 7.33 | 7.13 | 2696 | 6992 |
| mandel | rustc | 2093.16 | 2096.84 | 190664 | 291776 |
| mandel | ratio (rustc/rmc) | 285.4 | 293.9 | 70.7 | 41.7 |
| sieve | rmc | 8.06 | 7.97 | 4400 | 6744 |
| sieve | rustc | 2095.56 | 2099.48 | 189448 | 292016 |
| sieve | ratio (rustc/rmc) | 260.0 | 263.5 | 43.1 | 43.3 |
| mix | rmc | 6.01 | 5.96 | 2728 | 6792 |
| mix | rustc | 2088.19 | 2092.34 | 188208 | 291864 |
| mix | ratio (rustc/rmc) | 347.6 | 351.1 | 69.0 | 43.0 |
| stats | rmc | 7.04 | 6.92 | 2748 | 6832 |
| stats | rustc | 2109.45 | 2113.47 | 188000 | 292048 |
| stats | ratio (rustc/rmc) | 299.8 | 305.4 | 68.4 | 42.7 |
| refs | rmc | 6.30 | 6.23 | 2776 | 7336 |
| refs | rustc | 2078.77 | 2083.42 | 192296 | 292240 |
| refs | ratio (rustc/rmc) | 330.1 | 334.3 | 69.3 | 39.8 |
| heap | rmc | 6.22 | 6.15 | 2748 | 6736 |
| heap | rustc | 2085.55 | 2088.51 | 188232 | 292384 |
| heap | ratio (rustc/rmc) | 335.3 | 339.6 | 68.5 | 43.4 |

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.26 | 0.13 | 24 |
| hello | rustc | 0.87 | 0.75 | 2040 |
| hello | ratio (rustc/rmc) | 3.34 | 5.59 | 85.00 |
| fib | rmc | 7.22 | 7.11 | 20 |
| fib | rustc | 7.13 | 6.99 | 2028 |
| fib | ratio (rustc/rmc) | 0.99 | 0.98 | 101.40 |
| mandel | rmc | 95.62 | 95.41 | 20 |
| mandel | rustc | 105.17 | 104.96 | 2044 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 102.20 |
| sieve | rmc | 1.65 | 1.53 | 1584 |
| sieve | rustc | 2.51 | 2.39 | 3576 |
| sieve | ratio (rustc/rmc) | 1.52 | 1.56 | 2.26 |
| mix | rmc | 14.84 | 14.68 | 20 |
| mix | rustc | 15.50 | 15.33 | 1972 |
| mix | ratio (rustc/rmc) | 1.04 | 1.04 | 98.60 |
| stats | rmc | 3.33 | 3.19 | 24 |
| stats | rustc | 3.76 | 3.64 | 2088 |
| stats | ratio (rustc/rmc) | 1.13 | 1.14 | 87.00 |
| refs | rmc | 1.81 | 1.67 | 24 |
| refs | rustc | 3.91 | 3.77 | 2080 |
| refs | ratio (rustc/rmc) | 2.17 | 2.26 | 86.67 |
| heap | rmc | 3.27 | 3.14 | 4708 |
| heap | rustc | 3.53 | 3.37 | 6720 |
| heap | ratio (rustc/rmc) | 1.08 | 1.07 | 1.43 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension. Run time is a ratio of medians: rmc
matches or beats rustc on every program, and on `refs` it wins by
more than 2x because LICM reassociates the loop-invariant part of
the accumulation into one hoisted value, so the loop does a single
add per iteration instead of one per invariant term.
