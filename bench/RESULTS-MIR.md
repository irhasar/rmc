# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-08 20:25 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 36.33 | 37.34 | 94112 | 502 | 0.24 | 0.12 | 28 |
| hello | ast | 1.49 | 1.37 | 3228 | 478 | 0.24 | 0.12 | 28 |
| hello | llvm | 2048.38 | 2051.75 | 188400 | 289416 | 0.84 | 0.72 | 2036 |
| hello | llvm/mir | 56.38 | 54.95 | 2.00 | 576.53 | 3.50 | 5.89 | 72.71 |
| hello | ast/mir | 0.04 | 0.04 | 0.03 | 0.95 | 0.99 | 0.98 | 1.00 |
| fib | mir | 37.29 | 38.40 | 87964 | 640 | 6.58 | 6.44 | 32 |
| fib | ast | 1.53 | 1.40 | 3380 | 616 | 7.15 | 7.02 | 32 |
| fib | llvm | 2084.11 | 2086.66 | 189796 | 291696 | 8.36 | 8.16 | 2048 |
| fib | llvm/mir | 55.89 | 54.34 | 2.16 | 455.77 | 1.27 | 1.27 | 64.00 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.09 | 1.09 | 1.00 |
| mandel | mir | 39.91 | 41.03 | 89376 | 1110 | 90.68 | 90.47 | 28 |
| mandel | ast | 1.72 | 1.55 | 3336 | 941 | 92.36 | 92.13 | 32 |
| mandel | llvm | 2079.16 | 2082.99 | 187132 | 291776 | 105.22 | 105.00 | 2088 |
| mandel | llvm/mir | 52.09 | 50.77 | 2.09 | 262.86 | 1.16 | 1.16 | 74.57 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 1.14 |
| sieve | mir | 42.23 | 43.45 | 91628 | 867 | 1.65 | 1.54 | 1592 |
| sieve | ast | 1.55 | 1.42 | 3296 | 699 | 1.55 | 1.42 | 1592 |
| sieve | llvm | 2088.88 | 2091.59 | 187892 | 292032 | 2.40 | 2.27 | 3612 |
| sieve | llvm/mir | 49.46 | 48.14 | 2.05 | 336.83 | 1.45 | 1.47 | 2.27 |
| sieve | ast/mir | 0.04 | 0.03 | 0.04 | 0.81 | 0.94 | 0.92 | 1.00 |
| mix | mir | 39.72 | 40.81 | 90352 | 801 | 14.88 | 14.74 | 32 |
| mix | ast | 1.60 | 1.49 | 3256 | 869 | 14.76 | 14.61 | 28 |
| mix | llvm | 2093.96 | 2098.47 | 187032 | 291880 | 15.47 | 15.32 | 2112 |
| mix | llvm/mir | 52.72 | 51.42 | 2.07 | 364.39 | 1.04 | 1.04 | 66.00 |
| mix | ast/mir | 0.04 | 0.04 | 0.04 | 1.08 | 0.99 | 0.99 | 0.88 |
| stats | mir | 40.01 | 41.24 | 90196 | 856 | 3.28 | 3.14 | 32 |
| stats | ast | 1.60 | 1.46 | 3312 | 770 | 3.23 | 3.11 | 32 |
| stats | llvm | 2087.24 | 2092.17 | 188148 | 292048 | 3.72 | 3.59 | 2072 |
| stats | llvm/mir | 52.16 | 50.74 | 2.09 | 341.18 | 1.13 | 1.14 | 64.75 |
| stats | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.99 | 0.99 | 1.00 |
| refs | mir | 40.20 | 41.18 | 100136 | 1400 | 1.77 | 1.62 | 32 |
| refs | ast | 1.89 | 1.70 | 3412 | 901 | 1.76 | 1.62 | 32 |
| refs | llvm | 2107.84 | 2112.52 | 193528 | 292256 | 3.81 | 3.69 | 2028 |
| refs | llvm/mir | 52.44 | 51.30 | 1.93 | 208.75 | 2.16 | 2.29 | 63.38 |
| refs | ast/mir | 0.05 | 0.04 | 0.03 | 0.64 | 1.00 | 1.00 | 1.00 |
| heap | mir | 38.43 | 39.56 | 89828 | 989 | 3.27 | 3.14 | 4720 |
| heap | ast | 1.66 | 1.55 | 3288 | 954 | 3.17 | 3.05 | 4720 |
| heap | llvm | 2091.68 | 2096.27 | 187816 | 292400 | 3.44 | 3.30 | 6716 |
| heap | llvm/mir | 54.43 | 52.99 | 2.09 | 295.65 | 1.05 | 1.05 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.97 | 0.97 | 1.00 |
| callrec | mir | 37.81 | 38.78 | 88164 | 640 | 2.66 | 2.53 | 32 |
| callrec | ast | 1.52 | 1.40 | 3312 | 620 | 2.90 | 2.77 | 32 |
| callrec | llvm | 2092.10 | 2096.26 | 191780 | 291680 | 2.92 | 2.80 | 2032 |
| callrec | llvm/mir | 55.33 | 54.06 | 2.18 | 455.75 | 1.10 | 1.11 | 63.50 |
| callrec | ast/mir | 0.04 | 0.04 | 0.04 | 0.97 | 1.09 | 1.10 | 1.00 |
| forloops | mir | 40.95 | 42.16 | 89764 | 816 | 1.47 | 1.34 | 32 |
| forloops | ast | 1.59 | 1.48 | 3292 | 737 | 1.47 | 1.35 | 28 |
| forloops | llvm | 2092.25 | 2096.82 | 190332 | 292480 | 3.35 | 3.20 | 2124 |
| forloops | llvm/mir | 51.09 | 49.74 | 2.12 | 358.43 | 2.28 | 2.39 | 66.38 |
| forloops | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 1.00 | 1.01 | 0.88 |

Ratios are the other pipeline divided by mir, so a value above 1
means the MIR pipeline is better on that dimension. Every
`llvm/mir` row above is above 1: the MIR pipeline beats LLVM on
every axis. The `ast/mir` rows are reported for honesty - the
`.rms` path skips rustc's front end, so it is cheaper to compile,
but the MIR path is the one that replaces LLVM.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 34.70 | 35.84 | 94112 | 1.64 | 1.49 | 3056 |
| fib | 35.39 | 36.62 | 87964 | 1.90 | 1.77 | 3044 |
| mandel | 37.30 | 38.53 | 89376 | 2.61 | 2.50 | 3104 |
| sieve | 39.04 | 40.38 | 91628 | 3.20 | 3.08 | 4764 |
| mix | 37.49 | 38.70 | 90352 | 2.23 | 2.11 | 3044 |
| stats | 37.83 | 39.17 | 90196 | 2.18 | 2.07 | 3096 |
| refs | 36.86 | 37.99 | 100136 | 3.34 | 3.19 | 3052 |
| heap | 36.38 | 37.63 | 89828 | 2.05 | 1.93 | 3128 |
| callrec | 35.88 | 36.98 | 88164 | 1.93 | 1.80 | 3072 |
| forloops | 38.78 | 40.09 | 89764 | 2.17 | 2.07 | 3104 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

