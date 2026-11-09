# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-09 01:24 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 35.88 | 36.85 | 94136 | 502 | 0.23 | 0.12 | 24 |
| hello | ast | 1.45 | 1.34 | 3228 | 478 | 0.25 | 0.12 | 24 |
| hello | llvm | 2034.61 | 2038.09 | 187300 | 289416 | 0.85 | 0.72 | 2052 |
| hello | llvm/mir | 56.71 | 55.30 | 1.99 | 576.53 | 3.72 | 5.98 | 85.50 |
| hello | ast/mir | 0.04 | 0.04 | 0.03 | 0.95 | 1.08 | 1.02 | 1.00 |
| fib | mir | 37.17 | 38.25 | 92180 | 640 | 6.54 | 6.41 | 24 |
| fib | ast | 1.64 | 1.49 | 3280 | 616 | 7.09 | 6.98 | 24 |
| fib | llvm | 2088.13 | 2091.81 | 189640 | 291696 | 7.11 | 6.97 | 2088 |
| fib | llvm/mir | 56.17 | 54.69 | 2.06 | 455.77 | 1.09 | 1.09 | 87.00 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.08 | 1.09 | 1.00 |
| mandel | mir | 41.50 | 42.83 | 90448 | 1110 | 90.82 | 90.60 | 24 |
| mandel | ast | 1.63 | 1.51 | 3364 | 941 | 92.19 | 91.93 | 24 |
| mandel | llvm | 2090.38 | 2094.62 | 188920 | 291776 | 105.44 | 105.23 | 2084 |
| mandel | llvm/mir | 50.37 | 48.90 | 2.09 | 262.86 | 1.16 | 1.16 | 86.83 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.01 | 1.00 |
| sieve | mir | 41.29 | 42.58 | 91868 | 884 | 1.71 | 1.57 | 1584 |
| sieve | ast | 1.58 | 1.45 | 3292 | 699 | 1.53 | 1.41 | 1584 |
| sieve | llvm | 2087.17 | 2091.71 | 189048 | 292032 | 2.42 | 2.30 | 3660 |
| sieve | llvm/mir | 50.55 | 49.12 | 2.06 | 330.35 | 1.42 | 1.46 | 2.31 |
| sieve | ast/mir | 0.04 | 0.03 | 0.04 | 0.79 | 0.90 | 0.90 | 1.00 |
| mix | mir | 39.62 | 40.81 | 90132 | 801 | 14.95 | 14.81 | 24 |
| mix | ast | 1.80 | 1.64 | 3276 | 869 | 14.76 | 14.62 | 24 |
| mix | llvm | 2109.07 | 2113.59 | 188028 | 291880 | 15.52 | 15.37 | 2048 |
| mix | llvm/mir | 53.24 | 51.79 | 2.09 | 364.39 | 1.04 | 1.04 | 85.33 |
| mix | ast/mir | 0.05 | 0.04 | 0.04 | 1.08 | 0.99 | 0.99 | 1.00 |
| stats | mir | 40.46 | 41.67 | 90316 | 856 | 3.30 | 3.17 | 24 |
| stats | ast | 1.60 | 1.48 | 3340 | 770 | 3.27 | 3.14 | 24 |
| stats | llvm | 2080.75 | 2083.08 | 187844 | 292048 | 3.72 | 3.60 | 2060 |
| stats | llvm/mir | 51.42 | 49.99 | 2.08 | 341.18 | 1.13 | 1.13 | 85.83 |
| stats | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.99 | 0.99 | 1.00 |
| refs | mir | 41.46 | 42.64 | 90732 | 1416 | 1.72 | 1.60 | 24 |
| refs | ast | 1.74 | 1.63 | 3312 | 901 | 1.71 | 1.60 | 24 |
| refs | llvm | 2087.29 | 2090.12 | 189672 | 292256 | 3.83 | 3.69 | 2092 |
| refs | llvm/mir | 50.34 | 49.02 | 2.09 | 206.40 | 2.23 | 2.31 | 87.17 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.64 | 1.00 | 1.00 | 1.00 |
| heap | mir | 38.48 | 39.63 | 89664 | 989 | 3.26 | 3.15 | 4712 |
| heap | ast | 1.66 | 1.53 | 3272 | 954 | 3.20 | 3.07 | 4712 |
| heap | llvm | 2130.88 | 2133.42 | 193108 | 292400 | 3.44 | 3.32 | 6708 |
| heap | llvm/mir | 55.38 | 53.83 | 2.15 | 295.65 | 1.05 | 1.05 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.98 | 0.98 | 1.00 |
| callrec | mir | 38.49 | 39.67 | 88236 | 640 | 2.63 | 2.51 | 24 |
| callrec | ast | 1.59 | 1.45 | 3292 | 620 | 2.88 | 2.76 | 24 |
| callrec | llvm | 2077.38 | 2081.97 | 190532 | 291680 | 2.84 | 2.72 | 2096 |
| callrec | llvm/mir | 53.97 | 52.48 | 2.16 | 455.75 | 1.08 | 1.08 | 87.33 |
| callrec | ast/mir | 0.04 | 0.04 | 0.04 | 0.97 | 1.09 | 1.10 | 1.00 |
| forloops | mir | 40.51 | 41.68 | 89700 | 816 | 1.46 | 1.34 | 24 |
| forloops | ast | 1.60 | 1.47 | 3328 | 737 | 1.44 | 1.33 | 24 |
| forloops | llvm | 2099.91 | 2104.79 | 187684 | 292480 | 3.34 | 3.21 | 2064 |
| forloops | llvm/mir | 51.84 | 50.50 | 2.09 | 358.43 | 2.28 | 2.40 | 86.00 |
| forloops | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.98 | 0.99 | 1.00 |

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
| hello | 34.25 | 35.35 | 94136 | 1.62 | 1.50 | 3048 |
| fib | 35.25 | 36.46 | 92180 | 1.92 | 1.79 | 3080 |
| mandel | 38.87 | 40.31 | 90448 | 2.63 | 2.52 | 3072 |
| sieve | 37.98 | 39.40 | 91868 | 3.31 | 3.18 | 4780 |
| mix | 37.36 | 38.68 | 90132 | 2.26 | 2.13 | 3160 |
| stats | 38.25 | 39.56 | 90316 | 2.22 | 2.11 | 3148 |
| refs | 38.29 | 39.60 | 90732 | 3.17 | 3.04 | 3172 |
| heap | 36.37 | 37.64 | 89664 | 2.11 | 1.99 | 3124 |
| callrec | 36.53 | 37.84 | 88236 | 1.97 | 1.83 | 3140 |
| forloops | 38.21 | 39.55 | 89700 | 2.30 | 2.13 | 3148 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

