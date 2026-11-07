# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-07 03:22 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 35.56 | 36.64 | 88240 | 502 | 0.24 | 0.12 | 20 |
| hello | ast | 1.35 | 1.24 | 3164 | 478 | 0.22 | 0.11 | 24 |
| hello | llvm | 2038.33 | 2041.87 | 188904 | 289416 | 0.84 | 0.72 | 2096 |
| hello | llvm/mir | 57.31 | 55.73 | 2.14 | 576.53 | 3.53 | 6.19 | 104.80 |
| hello | ast/mir | 0.04 | 0.03 | 0.04 | 0.95 | 0.92 | 0.98 | 1.20 |
| fib | mir | 37.11 | 38.22 | 87912 | 640 | 6.48 | 6.33 | 24 |
| fib | ast | 1.47 | 1.33 | 3236 | 616 | 7.15 | 7.01 | 24 |
| fib | llvm | 2071.13 | 2075.83 | 190152 | 291696 | 7.11 | 6.97 | 2096 |
| fib | llvm/mir | 55.81 | 54.31 | 2.16 | 455.77 | 1.10 | 1.10 | 87.33 |
| fib | ast/mir | 0.04 | 0.03 | 0.04 | 0.96 | 1.10 | 1.11 | 1.00 |
| mandel | mir | 40.32 | 41.49 | 89356 | 1110 | 90.79 | 90.54 | 24 |
| mandel | ast | 1.54 | 1.42 | 3272 | 941 | 92.23 | 92.00 | 24 |
| mandel | llvm | 2078.90 | 2083.51 | 187400 | 291776 | 105.12 | 104.89 | 2084 |
| mandel | llvm/mir | 51.56 | 50.22 | 2.10 | 262.86 | 1.16 | 1.16 | 86.83 |
| mandel | ast/mir | 0.04 | 0.03 | 0.04 | 0.85 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 42.77 | 43.86 | 91736 | 867 | 1.66 | 1.55 | 1584 |
| sieve | ast | 1.49 | 1.37 | 3228 | 699 | 1.55 | 1.43 | 1584 |
| sieve | llvm | 2085.00 | 2088.75 | 192264 | 292032 | 2.36 | 2.25 | 3576 |
| sieve | llvm/mir | 48.75 | 47.63 | 2.10 | 336.83 | 1.42 | 1.45 | 2.26 |
| sieve | ast/mir | 0.03 | 0.03 | 0.04 | 0.81 | 0.93 | 0.92 | 1.00 |
| mix | mir | 39.16 | 40.37 | 90020 | 801 | 14.89 | 14.76 | 24 |
| mix | ast | 1.50 | 1.37 | 3248 | 869 | 14.70 | 14.55 | 24 |
| mix | llvm | 2076.86 | 2081.55 | 190956 | 291880 | 15.42 | 15.27 | 2036 |
| mix | llvm/mir | 53.03 | 51.57 | 2.12 | 364.39 | 1.04 | 1.04 | 84.83 |
| mix | ast/mir | 0.04 | 0.03 | 0.04 | 1.08 | 0.99 | 0.99 | 1.00 |
| stats | mir | 40.62 | 41.82 | 90308 | 856 | 3.31 | 3.17 | 24 |
| stats | ast | 1.48 | 1.36 | 3180 | 770 | 3.25 | 3.12 | 24 |
| stats | llvm | 2079.49 | 2084.07 | 188004 | 292048 | 3.74 | 3.60 | 2100 |
| stats | llvm/mir | 51.19 | 49.84 | 2.08 | 341.18 | 1.13 | 1.14 | 87.50 |
| stats | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 0.98 | 0.99 | 1.00 |
| refs | mir | 42.12 | 43.22 | 90704 | 1432 | 1.75 | 1.62 | 24 |
| refs | ast | 1.65 | 1.52 | 3328 | 901 | 1.72 | 1.60 | 24 |
| refs | llvm | 2074.53 | 2079.23 | 190180 | 292256 | 3.83 | 3.67 | 2112 |
| refs | llvm/mir | 49.26 | 48.10 | 2.10 | 204.09 | 2.19 | 2.27 | 88.00 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.63 | 0.98 | 0.99 | 1.00 |
| heap | mir | 38.42 | 39.69 | 89716 | 989 | 3.28 | 3.13 | 4708 |
| heap | ast | 1.52 | 1.41 | 3276 | 954 | 3.16 | 3.04 | 4712 |
| heap | llvm | 2069.69 | 2074.92 | 188576 | 292400 | 3.38 | 3.27 | 6692 |
| heap | llvm/mir | 53.86 | 52.27 | 2.10 | 295.65 | 1.03 | 1.04 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.96 | 0.97 | 1.00 |
| callrec | mir | 37.36 | 38.51 | 88068 | 640 | 2.64 | 2.53 | 24 |
| callrec | ast | 1.40 | 1.29 | 3228 | 620 | 2.89 | 2.76 | 24 |
| callrec | llvm | 2075.50 | 2078.85 | 190056 | 291680 | 2.84 | 2.71 | 2084 |
| callrec | llvm/mir | 55.55 | 53.99 | 2.16 | 455.75 | 1.08 | 1.07 | 86.83 |
| callrec | ast/mir | 0.04 | 0.03 | 0.04 | 0.97 | 1.09 | 1.09 | 1.00 |
| forloops | mir | 41.17 | 42.51 | 89760 | 816 | 1.45 | 1.34 | 24 |
| forloops | ast | 1.50 | 1.38 | 3232 | 737 | 1.46 | 1.34 | 24 |
| forloops | llvm | 2084.17 | 2088.97 | 188472 | 292480 | 3.33 | 3.21 | 2036 |
| forloops | llvm/mir | 50.63 | 49.14 | 2.10 | 358.43 | 2.30 | 2.40 | 84.83 |
| forloops | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 1.01 | 1.00 | 1.00 |

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
| hello | 34.04 | 35.24 | 88240 | 1.52 | 1.40 | 2916 |
| fib | 35.33 | 36.55 | 87912 | 1.78 | 1.67 | 2928 |
| mandel | 37.78 | 39.08 | 89356 | 2.54 | 2.41 | 2956 |
| sieve | 39.63 | 40.84 | 91736 | 3.15 | 3.02 | 4652 |
| mix | 36.99 | 38.33 | 90020 | 2.18 | 2.04 | 2948 |
| stats | 38.51 | 39.83 | 90308 | 2.11 | 1.99 | 3052 |
| refs | 39.17 | 40.41 | 90704 | 2.95 | 2.81 | 2968 |
| heap | 36.44 | 37.83 | 89716 | 1.98 | 1.86 | 2920 |
| callrec | 35.55 | 36.82 | 88068 | 1.82 | 1.69 | 2992 |
| forloops | 39.06 | 40.54 | 89760 | 2.11 | 1.97 | 3048 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

