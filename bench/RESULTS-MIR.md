# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-07 09:14 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 40.92 | 41.41 | 88464 | 502 | 0.22 | 0.12 | 20 |
| hello | ast | 1.49 | 1.36 | 3172 | 478 | 0.22 | 0.12 | 24 |
| hello | llvm | 2075.83 | 2079.15 | 189388 | 289416 | 0.81 | 0.69 | 2028 |
| hello | llvm/mir | 50.73 | 50.21 | 2.14 | 576.53 | 3.70 | 5.91 | 101.40 |
| hello | ast/mir | 0.04 | 0.03 | 0.04 | 0.95 | 1.00 | 1.01 | 1.20 |
| fib | mir | 38.52 | 39.56 | 97172 | 640 | 6.52 | 6.38 | 24 |
| fib | ast | 1.52 | 1.40 | 3284 | 616 | 7.17 | 7.05 | 24 |
| fib | llvm | 2114.86 | 2116.39 | 186696 | 291696 | 7.15 | 7.00 | 2052 |
| fib | llvm/mir | 54.90 | 53.49 | 1.92 | 455.77 | 1.10 | 1.10 | 85.50 |
| fib | ast/mir | 0.04 | 0.04 | 0.03 | 0.96 | 1.10 | 1.11 | 1.00 |
| mandel | mir | 40.87 | 41.76 | 89468 | 1110 | 90.91 | 90.72 | 24 |
| mandel | ast | 1.64 | 1.51 | 3292 | 941 | 92.68 | 92.47 | 20 |
| mandel | llvm | 2103.52 | 2107.02 | 188276 | 291776 | 105.23 | 105.03 | 2084 |
| mandel | llvm/mir | 51.46 | 50.45 | 2.10 | 262.86 | 1.16 | 1.16 | 86.83 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 0.83 |
| sieve | mir | 41.85 | 43.01 | 91500 | 867 | 1.68 | 1.57 | 1584 |
| sieve | ast | 1.56 | 1.44 | 3308 | 699 | 1.54 | 1.43 | 1584 |
| sieve | llvm | 2111.10 | 2115.19 | 188760 | 292032 | 2.43 | 2.30 | 3640 |
| sieve | llvm/mir | 50.45 | 49.18 | 2.06 | 336.83 | 1.44 | 1.47 | 2.30 |
| sieve | ast/mir | 0.04 | 0.03 | 0.04 | 0.81 | 0.92 | 0.91 | 1.00 |
| mix | mir | 39.47 | 40.57 | 90196 | 801 | 14.87 | 14.72 | 24 |
| mix | ast | 1.62 | 1.49 | 3276 | 869 | 14.70 | 14.54 | 24 |
| mix | llvm | 2131.19 | 2131.45 | 186652 | 291880 | 15.51 | 15.36 | 2072 |
| mix | llvm/mir | 54.00 | 52.54 | 2.07 | 364.39 | 1.04 | 1.04 | 86.33 |
| mix | ast/mir | 0.04 | 0.04 | 0.04 | 1.08 | 0.99 | 0.99 | 1.00 |
| stats | mir | 39.73 | 40.79 | 90344 | 856 | 3.28 | 3.16 | 20 |
| stats | ast | 1.58 | 1.45 | 3268 | 770 | 3.25 | 3.13 | 24 |
| stats | llvm | 2133.79 | 2136.84 | 188212 | 292048 | 3.75 | 3.61 | 2000 |
| stats | llvm/mir | 53.70 | 52.38 | 2.08 | 341.18 | 1.14 | 1.14 | 100.00 |
| stats | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.99 | 0.99 | 1.20 |
| refs | mir | 40.68 | 42.03 | 90680 | 1400 | 1.73 | 1.61 | 24 |
| refs | ast | 2.65 | 2.20 | 3332 | 901 | 1.73 | 1.61 | 24 |
| refs | llvm | 2120.05 | 2124.72 | 189628 | 292256 | 3.83 | 3.71 | 2028 |
| refs | llvm/mir | 52.11 | 50.55 | 2.09 | 208.75 | 2.22 | 2.29 | 84.50 |
| refs | ast/mir | 0.07 | 0.05 | 0.04 | 0.64 | 1.00 | 1.00 | 1.00 |
| heap | mir | 38.96 | 40.26 | 93900 | 989 | 3.33 | 3.20 | 4712 |
| heap | ast | 1.62 | 1.50 | 3320 | 954 | 3.25 | 3.12 | 4708 |
| heap | llvm | 2118.43 | 2122.95 | 192980 | 292400 | 3.53 | 3.39 | 6728 |
| heap | llvm/mir | 54.37 | 52.73 | 2.06 | 295.65 | 1.06 | 1.06 | 1.43 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.97 | 0.97 | 1.00 |
| callrec | mir | 38.22 | 39.41 | 89532 | 640 | 2.63 | 2.51 | 24 |
| callrec | ast | 1.50 | 1.38 | 3308 | 620 | 2.89 | 2.77 | 24 |
| callrec | llvm | 2094.20 | 2098.70 | 187260 | 291680 | 2.83 | 2.71 | 2052 |
| callrec | llvm/mir | 54.79 | 53.26 | 2.09 | 455.75 | 1.08 | 1.08 | 85.50 |
| callrec | ast/mir | 0.04 | 0.03 | 0.04 | 0.97 | 1.10 | 1.11 | 1.00 |
| forloops | mir | 41.27 | 42.53 | 90696 | 816 | 1.46 | 1.35 | 24 |
| forloops | ast | 1.75 | 1.59 | 3296 | 737 | 1.43 | 1.32 | 24 |
| forloops | llvm | 2122.93 | 2125.02 | 190620 | 292480 | 3.33 | 3.19 | 2068 |
| forloops | llvm/mir | 51.44 | 49.97 | 2.10 | 358.43 | 2.28 | 2.37 | 86.17 |
| forloops | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.98 | 0.98 | 1.00 |

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
| hello | 39.30 | 39.92 | 88464 | 1.62 | 1.48 | 2972 |
| fib | 36.68 | 37.84 | 97172 | 1.85 | 1.72 | 3012 |
| mandel | 38.21 | 39.22 | 89468 | 2.66 | 2.54 | 3100 |
| sieve | 38.60 | 39.89 | 91500 | 3.24 | 3.11 | 4784 |
| mix | 37.16 | 38.40 | 90196 | 2.31 | 2.17 | 3104 |
| stats | 37.39 | 38.61 | 90344 | 2.34 | 2.18 | 3096 |
| refs | 37.48 | 38.98 | 90680 | 3.20 | 3.06 | 3128 |
| heap | 36.88 | 38.30 | 93900 | 2.08 | 1.96 | 3080 |
| callrec | 36.36 | 37.67 | 89532 | 1.86 | 1.74 | 3104 |
| forloops | 39.01 | 40.40 | 90696 | 2.26 | 2.13 | 3040 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

