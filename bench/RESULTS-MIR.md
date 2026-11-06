# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-06 10:25 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 35.21 | 36.19 | 88180 | 502 | 0.23 | 0.13 | 20 |
| hello | ast | 1.33 | 1.20 | 3224 | 478 | 0.25 | 0.13 | 24 |
| hello | llvm | 2095.30 | 2097.27 | 187512 | 289416 | 0.81 | 0.71 | 2084 |
| hello | llvm/mir | 59.51 | 57.96 | 2.13 | 576.53 | 3.46 | 5.56 | 104.20 |
| hello | ast/mir | 0.04 | 0.03 | 0.04 | 0.95 | 1.06 | 1.02 | 1.20 |
| fib | mir | 38.03 | 39.13 | 88156 | 640 | 6.48 | 6.34 | 24 |
| fib | ast | 1.43 | 1.31 | 3224 | 616 | 7.19 | 7.04 | 24 |
| fib | llvm | 2138.24 | 2142.77 | 189180 | 291696 | 7.10 | 6.97 | 2004 |
| fib | llvm/mir | 56.22 | 54.76 | 2.15 | 455.77 | 1.10 | 1.10 | 83.50 |
| fib | ast/mir | 0.04 | 0.03 | 0.04 | 0.96 | 1.11 | 1.11 | 1.00 |
| mandel | mir | 40.91 | 41.87 | 89496 | 1110 | 90.59 | 90.36 | 24 |
| mandel | ast | 1.52 | 1.40 | 3192 | 941 | 92.40 | 92.18 | 24 |
| mandel | llvm | 2088.96 | 2091.43 | 188068 | 291776 | 105.26 | 105.03 | 2096 |
| mandel | llvm/mir | 51.06 | 49.94 | 2.10 | 262.86 | 1.16 | 1.16 | 87.33 |
| mandel | ast/mir | 0.04 | 0.03 | 0.04 | 0.85 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 42.48 | 43.71 | 91628 | 867 | 2.54 | 2.40 | 1584 |
| sieve | ast | 1.47 | 1.34 | 3348 | 699 | 2.25 | 2.11 | 1584 |
| sieve | llvm | 2080.24 | 2084.49 | 189680 | 292032 | 2.59 | 2.44 | 3672 |
| sieve | llvm/mir | 48.97 | 47.69 | 2.07 | 336.83 | 1.02 | 1.01 | 2.32 |
| sieve | ast/mir | 0.03 | 0.03 | 0.04 | 0.81 | 0.88 | 0.88 | 1.00 |
| mix | mir | 39.11 | 40.36 | 90200 | 801 | 14.95 | 14.78 | 24 |
| mix | ast | 1.51 | 1.38 | 3192 | 869 | 14.74 | 14.60 | 24 |
| mix | llvm | 2074.57 | 2078.95 | 190832 | 291880 | 15.59 | 15.41 | 2008 |
| mix | llvm/mir | 53.04 | 51.50 | 2.12 | 364.39 | 1.04 | 1.04 | 83.67 |
| mix | ast/mir | 0.04 | 0.03 | 0.04 | 1.08 | 0.99 | 0.99 | 1.00 |
| stats | mir | 39.76 | 40.94 | 90328 | 856 | 3.28 | 3.14 | 24 |
| stats | ast | 1.52 | 1.37 | 3120 | 770 | 3.25 | 3.11 | 24 |
| stats | llvm | 2101.30 | 2104.87 | 188008 | 292048 | 3.74 | 3.61 | 2104 |
| stats | llvm/mir | 52.85 | 51.41 | 2.08 | 341.18 | 1.14 | 1.15 | 87.67 |
| stats | ast/mir | 0.04 | 0.03 | 0.03 | 0.90 | 0.99 | 0.99 | 1.00 |
| refs | mir | 41.20 | 42.31 | 90552 | 1432 | 1.75 | 1.63 | 24 |
| refs | ast | 1.62 | 1.50 | 3248 | 901 | 1.75 | 1.64 | 24 |
| refs | llvm | 2102.37 | 2106.65 | 189652 | 292256 | 3.84 | 3.72 | 2064 |
| refs | llvm/mir | 51.03 | 49.79 | 2.09 | 204.09 | 2.19 | 2.29 | 86.00 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.63 | 1.00 | 1.01 | 1.00 |
| heap | mir | 37.97 | 39.04 | 89580 | 989 | 3.38 | 3.23 | 4712 |
| heap | ast | 1.53 | 1.40 | 3232 | 954 | 3.27 | 3.14 | 4712 |
| heap | llvm | 2094.76 | 2098.86 | 188964 | 292400 | 3.53 | 3.39 | 6716 |
| heap | llvm/mir | 55.16 | 53.77 | 2.11 | 295.65 | 1.04 | 1.05 | 1.43 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.97 | 0.97 | 1.00 |
| callrec | mir | 37.73 | 38.81 | 88176 | 640 | 2.65 | 2.51 | 24 |
| callrec | ast | 1.47 | 1.31 | 3256 | 620 | 2.87 | 2.75 | 24 |
| callrec | llvm | 2098.25 | 2102.29 | 187032 | 291680 | 2.87 | 2.73 | 2064 |
| callrec | llvm/mir | 55.61 | 54.17 | 2.12 | 455.75 | 1.08 | 1.09 | 86.00 |
| callrec | ast/mir | 0.04 | 0.03 | 0.04 | 0.97 | 1.09 | 1.09 | 1.00 |
| forloops | mir | 41.56 | 42.87 | 89620 | 816 | 1.47 | 1.34 | 24 |
| forloops | ast | 1.53 | 1.38 | 3168 | 737 | 1.46 | 1.34 | 24 |
| forloops | llvm | 2085.03 | 2089.32 | 187748 | 292480 | 3.33 | 3.21 | 2064 |
| forloops | llvm/mir | 50.17 | 48.74 | 2.09 | 358.43 | 2.27 | 2.39 | 86.00 |
| forloops | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 0.99 | 1.00 | 1.00 |

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
| hello | 33.68 | 34.79 | 88180 | 1.53 | 1.40 | 2992 |
| fib | 36.26 | 37.47 | 88156 | 1.77 | 1.66 | 3016 |
| mandel | 38.43 | 39.52 | 89496 | 2.48 | 2.36 | 3100 |
| sieve | 39.31 | 40.68 | 91628 | 3.17 | 3.04 | 4744 |
| mix | 37.01 | 38.38 | 90200 | 2.11 | 1.99 | 3028 |
| stats | 37.62 | 38.95 | 90328 | 2.13 | 2.00 | 3012 |
| refs | 38.21 | 39.44 | 90552 | 2.99 | 2.86 | 3020 |
| heap | 36.00 | 37.19 | 89580 | 1.97 | 1.85 | 3084 |
| callrec | 35.96 | 37.15 | 88176 | 1.78 | 1.66 | 2952 |
| forloops | 39.50 | 40.93 | 89620 | 2.06 | 1.94 | 3012 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

