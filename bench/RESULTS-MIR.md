# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-05 16:17 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run_mir.py --markdown bench/RESULTS-MIR.md`.

Three pipelines are compared on every axis:

* `mir` - `rustc --emit=mir -O` then `rmcmir` (rmc's in-process
  assembler/linker; no LLVM, no libc, no crates). The front-end and
  back-end costs are split out below.
* `ast` - `rmc` compiling the `.rms` twin (the original all-in-one
  pipeline).
* `llvm` - `rustc -O` with fat LTO, one codegen unit, `panic=abort`
  and stripped symbols.

## Compile and run, all axes

| program | pipeline | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: | -----------: | -------: | -----------: | -----------: |
| hello | mir | 38.80 | 39.95 | 93956 | 502 | 0.23 | 0.13 | 24 |
| hello | ast | 1.25 | 1.12 | 3076 | 478 | 0.23 | 0.12 | 28 |
| hello | llvm | 2105.57 | 2108.95 | 187988 | 289416 | 0.87 | 0.75 | 2108 |
| hello | llvm/mir | 54.27 | 52.79 | 2.00 | 576.53 | 3.70 | 5.88 | 87.83 |
| fib | mir | 40.25 | 41.49 | 88040 | 639 | 6.52 | 6.40 | 28 |
| fib | ast | 1.30 | 1.18 | 3188 | 616 | 7.18 | 7.03 | 28 |
| fib | llvm | 2134.09 | 2138.67 | 189020 | 291696 | 7.15 | 7.01 | 2088 |
| fib | llvm/mir | 53.01 | 51.55 | 2.15 | 456.49 | 1.10 | 1.10 | 74.57 |
| mandel | mir | 42.96 | 44.17 | 89592 | 1109 | 90.67 | 90.45 | 28 |
| mandel | ast | 1.45 | 1.32 | 3136 | 941 | 92.33 | 92.10 | 24 |
| mandel | llvm | 2136.90 | 2141.45 | 190432 | 291776 | 105.15 | 104.92 | 2144 |
| mandel | llvm/mir | 49.74 | 48.48 | 2.13 | 263.10 | 1.16 | 1.16 | 76.57 |
| sieve | mir | 42.75 | 43.86 | 95884 | 866 | 1.78 | 1.64 | 1588 |
| sieve | ast | 1.35 | 1.23 | 3212 | 699 | 1.55 | 1.44 | 1588 |
| sieve | llvm | 2097.70 | 2102.10 | 187952 | 292032 | 2.41 | 2.28 | 3592 |
| sieve | llvm/mir | 49.07 | 47.92 | 1.96 | 337.22 | 1.36 | 1.39 | 2.26 |
| mix | mir | 39.92 | 41.06 | 89912 | 800 | 15.02 | 14.82 | 28 |
| mix | ast | 1.39 | 1.27 | 3188 | 869 | 14.86 | 14.67 | 28 |
| mix | llvm | 2152.02 | 2150.73 | 190032 | 291880 | 15.53 | 15.35 | 2048 |
| mix | llvm/mir | 53.91 | 52.38 | 2.11 | 364.85 | 1.03 | 1.04 | 73.14 |
| stats | mir | 41.97 | 43.24 | 90512 | 855 | 3.32 | 3.19 | 28 |
| stats | ast | 1.40 | 1.28 | 3188 | 770 | 3.21 | 3.09 | 28 |
| stats | llvm | 2128.42 | 2132.95 | 188400 | 292048 | 3.70 | 3.58 | 2108 |
| stats | llvm/mir | 50.71 | 49.33 | 2.08 | 341.58 | 1.11 | 1.12 | 75.29 |
| refs | mir | 41.25 | 42.32 | 90764 | 1431 | 1.74 | 1.62 | 28 |
| refs | ast | 1.55 | 1.41 | 3184 | 901 | 1.72 | 1.60 | 24 |
| refs | llvm | 2115.66 | 2120.63 | 192976 | 292256 | 3.83 | 3.69 | 2024 |
| refs | llvm/mir | 51.29 | 50.11 | 2.13 | 204.23 | 2.20 | 2.27 | 72.29 |
| heap | mir | 39.00 | 40.14 | 89500 | 988 | 3.42 | 3.25 | 4716 |
| heap | ast | 1.39 | 1.27 | 3140 | 954 | 3.23 | 3.08 | 4716 |
| heap | llvm | 2128.53 | 2132.60 | 192772 | 292400 | 3.54 | 3.38 | 6732 |
| heap | llvm/mir | 54.57 | 53.13 | 2.15 | 295.95 | 1.03 | 1.04 | 1.43 |
| callrec | mir | 39.01 | 40.20 | 88232 | 639 | 2.62 | 2.51 | 28 |
| callrec | ast | 1.34 | 1.21 | 3152 | 620 | 2.88 | 2.76 | 28 |
| callrec | llvm | 2139.55 | 2143.54 | 187904 | 291680 | 2.87 | 2.76 | 2064 |
| callrec | llvm/mir | 54.84 | 53.32 | 2.13 | 456.46 | 1.10 | 1.10 | 73.71 |
| forloops | mir | 42.80 | 44.14 | 90932 | 815 | 1.47 | 1.35 | 28 |
| forloops | ast | 1.36 | 1.25 | 3168 | 737 | 1.46 | 1.34 | 28 |
| forloops | llvm | 2171.47 | 2172.83 | 188320 | 292480 | 3.35 | 3.20 | 2056 |
| forloops | llvm/mir | 50.73 | 49.22 | 2.07 | 358.87 | 2.27 | 2.38 | 73.43 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 37.38 | 38.68 | 93956 | 1.41 | 1.26 | 2844 |
| fib | 38.59 | 39.95 | 88040 | 1.66 | 1.54 | 2868 |
| mandel | 40.57 | 41.92 | 89592 | 2.39 | 2.25 | 2972 |
| sieve | 39.73 | 40.98 | 95884 | 3.02 | 2.88 | 4616 |
| mix | 37.91 | 39.18 | 89912 | 2.01 | 1.88 | 2884 |
| stats | 39.92 | 41.33 | 90512 | 2.05 | 1.90 | 2980 |
| refs | 38.23 | 39.47 | 90764 | 3.02 | 2.86 | 2868 |
| heap | 37.13 | 38.39 | 89500 | 1.87 | 1.76 | 2868 |
| callrec | 37.32 | 38.65 | 88232 | 1.70 | 1.55 | 2872 |
| forloops | 40.89 | 42.36 | 90932 | 1.91 | 1.78 | 2876 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

