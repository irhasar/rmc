# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-05 13:34 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 36.95 | 38.09 | 88396 | 502 | 0.36 | 0.17 | 28 |
| hello | ast | 1.31 | 1.11 | 3024 | 478 | 0.36 | 0.20 | 28 |
| hello | llvm | 2141.12 | 2141.36 | 188008 | 289416 | 1.17 | 1.02 | 2040 |
| hello | llvm/mir | 57.94 | 56.22 | 2.13 | 576.53 | 3.20 | 5.85 | 72.86 |
| fib | mir | 40.09 | 41.31 | 88020 | 639 | 7.13 | 6.97 | 28 |
| fib | ast | 1.36 | 1.19 | 3136 | 618 | 7.39 | 7.25 | 28 |
| fib | llvm | 2178.68 | 2182.99 | 187276 | 291696 | 8.06 | 7.89 | 2052 |
| fib | llvm/mir | 54.34 | 52.84 | 2.13 | 456.49 | 1.13 | 1.13 | 73.29 |
| mandel | mir | 46.10 | 47.40 | 90540 | 1109 | 90.62 | 90.41 | 28 |
| mandel | ast | 1.43 | 1.30 | 3088 | 941 | 92.47 | 92.26 | 28 |
| mandel | llvm | 2203.81 | 2208.48 | 190448 | 291776 | 105.85 | 105.66 | 2016 |
| mandel | llvm/mir | 47.80 | 46.59 | 2.10 | 263.10 | 1.17 | 1.17 | 72.00 |
| sieve | mir | 55.10 | 56.31 | 91872 | 836 | 1.71 | 1.59 | 1588 |
| sieve | ast | 1.31 | 1.19 | 3084 | 699 | 1.52 | 1.42 | 1588 |
| sieve | llvm | 2152.06 | 2147.16 | 187624 | 292032 | 2.40 | 2.28 | 3588 |
| sieve | llvm/mir | 39.06 | 38.13 | 2.04 | 349.32 | 1.41 | 1.44 | 2.26 |
| mix | mir | 41.47 | 42.61 | 90156 | 794 | 14.90 | 14.76 | 24 |
| mix | ast | 1.40 | 1.28 | 3172 | 869 | 14.70 | 14.55 | 28 |
| mix | llvm | 2155.28 | 2160.11 | 192424 | 291880 | 15.41 | 15.26 | 2116 |
| mix | llvm/mir | 51.97 | 50.70 | 2.13 | 367.61 | 1.03 | 1.03 | 88.17 |
| stats | mir | 42.62 | 43.79 | 91008 | 855 | 3.31 | 3.18 | 28 |
| stats | ast | 1.42 | 1.29 | 3100 | 770 | 3.24 | 3.11 | 24 |
| stats | llvm | 2154.84 | 2155.81 | 188828 | 292048 | 3.68 | 3.57 | 2108 |
| stats | llvm/mir | 50.56 | 49.23 | 2.07 | 341.58 | 1.11 | 1.12 | 75.29 |
| refs | mir | 42.16 | 43.42 | 90636 | 1489 | 3.18 | 3.01 | 24 |
| refs | ast | 1.58 | 1.45 | 3220 | 901 | 2.05 | 1.82 | 28 |
| refs | llvm | 2128.77 | 2131.82 | 189436 | 292256 | 4.20 | 4.03 | 2072 |
| refs | llvm/mir | 50.49 | 49.10 | 2.09 | 196.28 | 1.32 | 1.34 | 86.33 |
| heap | mir | 40.99 | 41.81 | 92828 | 988 | 3.26 | 3.14 | 4716 |
| heap | ast | 1.49 | 1.37 | 3120 | 954 | 3.18 | 3.05 | 4716 |
| heap | llvm | 2108.43 | 2113.63 | 188692 | 292400 | 3.43 | 3.30 | 6712 |
| heap | llvm/mir | 51.44 | 50.55 | 2.03 | 295.95 | 1.05 | 1.05 | 1.42 |
| callrec | mir | 39.42 | 40.51 | 88636 | 639 | 2.64 | 2.53 | 28 |
| callrec | ast | 1.39 | 1.27 | 3104 | 622 | 2.88 | 2.76 | 28 |
| callrec | llvm | 2143.11 | 2147.83 | 186664 | 291680 | 2.82 | 2.71 | 2064 |
| callrec | llvm/mir | 54.36 | 53.01 | 2.11 | 456.46 | 1.07 | 1.07 | 73.71 |
| forloops | mir | 41.73 | 42.95 | 89552 | 815 | 1.58 | 1.42 | 28 |
| forloops | ast | 1.37 | 1.24 | 3132 | 737 | 1.58 | 1.44 | 28 |
| forloops | llvm | 2181.57 | 2184.09 | 187704 | 292480 | 3.33 | 3.21 | 2076 |
| forloops | llvm/mir | 52.28 | 50.85 | 2.10 | 358.87 | 2.11 | 2.25 | 74.14 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 35.45 | 36.79 | 88396 | 1.50 | 1.30 | 2796 |
| fib | 38.41 | 39.78 | 88020 | 1.69 | 1.53 | 2908 |
| mandel | 43.68 | 45.14 | 90540 | 2.42 | 2.27 | 2756 |
| sieve | 39.59 | 40.97 | 91872 | 15.51 | 15.34 | 6812 |
| mix | 39.39 | 40.69 | 90156 | 2.08 | 1.91 | 2948 |
| stats | 40.55 | 41.86 | 91008 | 2.07 | 1.93 | 2872 |
| refs | 39.22 | 40.61 | 90636 | 2.94 | 2.81 | 2936 |
| heap | 38.99 | 39.95 | 92828 | 2.00 | 1.86 | 2844 |
| callrec | 37.69 | 38.91 | 88636 | 1.73 | 1.60 | 2912 |
| forloops | 39.81 | 41.16 | 89552 | 1.92 | 1.79 | 2920 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

