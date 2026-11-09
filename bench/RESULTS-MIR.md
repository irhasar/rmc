# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-09 00:18 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 37.19 | 38.11 | 94000 | 502 | 0.26 | 0.13 | 24 |
| hello | ast | 1.91 | 1.77 | 3224 | 478 | 0.24 | 0.12 | 24 |
| hello | llvm | 2014.63 | 2017.02 | 189968 | 289416 | 0.85 | 0.73 | 2096 |
| hello | llvm/mir | 54.17 | 52.93 | 2.02 | 576.53 | 3.34 | 5.50 | 87.33 |
| hello | ast/mir | 0.05 | 0.05 | 0.03 | 0.95 | 0.95 | 0.92 | 1.00 |
| fib | mir | 37.28 | 38.35 | 90560 | 640 | 6.52 | 6.38 | 24 |
| fib | ast | 1.54 | 1.43 | 3404 | 616 | 7.16 | 7.02 | 24 |
| fib | llvm | 2087.73 | 2091.53 | 189036 | 291696 | 7.10 | 6.95 | 2084 |
| fib | llvm/mir | 56.00 | 54.54 | 2.09 | 455.77 | 1.09 | 1.09 | 86.83 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.10 | 1.10 | 1.00 |
| mandel | mir | 39.19 | 40.25 | 89488 | 1110 | 90.68 | 90.49 | 20 |
| mandel | ast | 1.64 | 1.51 | 3316 | 941 | 92.79 | 92.56 | 24 |
| mandel | llvm | 2068.51 | 2072.96 | 188704 | 291776 | 105.30 | 105.10 | 2064 |
| mandel | llvm/mir | 52.78 | 51.50 | 2.11 | 262.86 | 1.16 | 1.16 | 103.20 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 1.20 |
| sieve | mir | 41.13 | 42.27 | 91764 | 867 | 1.68 | 1.57 | 1584 |
| sieve | ast | 1.56 | 1.45 | 3284 | 699 | 1.55 | 1.43 | 1584 |
| sieve | llvm | 2050.75 | 2054.94 | 191720 | 292032 | 2.47 | 2.34 | 3620 |
| sieve | llvm/mir | 49.86 | 48.61 | 2.09 | 336.83 | 1.47 | 1.49 | 2.29 |
| sieve | ast/mir | 0.04 | 0.03 | 0.04 | 0.81 | 0.93 | 0.91 | 1.00 |
| mix | mir | 39.40 | 40.40 | 90608 | 801 | 14.91 | 14.77 | 20 |
| mix | ast | 1.62 | 1.50 | 3348 | 869 | 14.71 | 14.57 | 24 |
| mix | llvm | 2090.25 | 2094.59 | 191908 | 291880 | 15.44 | 15.30 | 2100 |
| mix | llvm/mir | 53.05 | 51.85 | 2.12 | 364.39 | 1.04 | 1.04 | 105.00 |
| mix | ast/mir | 0.04 | 0.04 | 0.04 | 1.08 | 0.99 | 0.99 | 1.20 |
| stats | mir | 39.60 | 40.28 | 91360 | 856 | 3.30 | 3.18 | 20 |
| stats | ast | 1.63 | 1.50 | 3316 | 770 | 3.21 | 3.09 | 24 |
| stats | llvm | 2051.73 | 2056.21 | 188228 | 292048 | 3.69 | 3.57 | 2064 |
| stats | llvm/mir | 51.81 | 51.05 | 2.06 | 341.18 | 1.12 | 1.12 | 103.20 |
| stats | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.97 | 0.97 | 1.20 |
| refs | mir | 40.13 | 41.29 | 92380 | 1400 | 1.81 | 1.63 | 20 |
| refs | ast | 1.79 | 1.63 | 3392 | 901 | 1.92 | 1.75 | 24 |
| refs | llvm | 2067.75 | 2072.37 | 188008 | 292256 | 5.09 | 4.56 | 2104 |
| refs | llvm/mir | 51.52 | 50.19 | 2.04 | 208.75 | 2.81 | 2.80 | 105.20 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.64 | 1.06 | 1.07 | 1.20 |
| heap | mir | 38.23 | 39.42 | 97836 | 989 | 3.23 | 3.11 | 4712 |
| heap | ast | 1.61 | 1.48 | 3276 | 954 | 3.15 | 3.03 | 4712 |
| heap | llvm | 2073.60 | 2078.36 | 190216 | 292400 | 3.42 | 3.29 | 6712 |
| heap | llvm/mir | 54.24 | 52.72 | 1.94 | 295.65 | 1.06 | 1.06 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.03 | 0.96 | 0.97 | 0.98 | 1.00 |
| callrec | mir | 37.01 | 38.08 | 88032 | 640 | 2.63 | 2.51 | 24 |
| callrec | ast | 1.52 | 1.41 | 3372 | 620 | 2.89 | 2.75 | 24 |
| callrec | llvm | 2056.10 | 2060.57 | 190752 | 291680 | 2.88 | 2.72 | 2056 |
| callrec | llvm/mir | 55.56 | 54.11 | 2.17 | 455.75 | 1.09 | 1.08 | 85.67 |
| callrec | ast/mir | 0.04 | 0.04 | 0.04 | 0.97 | 1.10 | 1.09 | 1.00 |
| forloops | mir | 40.49 | 41.58 | 89696 | 816 | 1.48 | 1.36 | 24 |
| forloops | ast | 1.56 | 1.44 | 3288 | 737 | 1.45 | 1.33 | 24 |
| forloops | llvm | 2060.11 | 2063.21 | 187668 | 292480 | 3.32 | 3.21 | 2012 |
| forloops | llvm/mir | 50.88 | 49.62 | 2.09 | 358.43 | 2.24 | 2.35 | 83.83 |
| forloops | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 0.97 | 0.98 | 1.00 |

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
| hello | 35.10 | 36.18 | 94000 | 2.09 | 1.93 | 2972 |
| fib | 35.37 | 36.56 | 90560 | 1.91 | 1.79 | 3028 |
| mandel | 36.62 | 37.80 | 89488 | 2.57 | 2.45 | 3024 |
| sieve | 37.91 | 39.18 | 91764 | 3.23 | 3.10 | 4760 |
| mix | 37.18 | 38.29 | 90608 | 2.22 | 2.11 | 3088 |
| stats | 37.40 | 38.19 | 91360 | 2.20 | 2.09 | 3112 |
| refs | 37.07 | 38.34 | 92380 | 3.07 | 2.95 | 3164 |
| heap | 36.18 | 37.49 | 97836 | 2.05 | 1.93 | 3120 |
| callrec | 35.12 | 36.32 | 88032 | 1.89 | 1.76 | 3012 |
| forloops | 38.32 | 39.54 | 89696 | 2.17 | 2.04 | 3024 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

