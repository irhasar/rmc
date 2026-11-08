# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-08 15:26 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 38.39 | 39.29 | 90264 | 502 | 0.23 | 0.12 | 28 |
| hello | ast | 1.51 | 1.38 | 3232 | 478 | 0.24 | 0.12 | 28 |
| hello | llvm | 2117.95 | 2121.22 | 189300 | 289416 | 0.83 | 0.71 | 2056 |
| hello | llvm/mir | 55.17 | 53.98 | 2.10 | 576.53 | 3.66 | 5.88 | 73.43 |
| hello | ast/mir | 0.04 | 0.04 | 0.04 | 0.95 | 1.08 | 0.98 | 1.00 |
| fib | mir | 38.81 | 39.81 | 88096 | 640 | 6.54 | 6.38 | 28 |
| fib | ast | 1.52 | 1.41 | 3212 | 616 | 11.51 | 11.34 | 28 |
| fib | llvm | 2143.63 | 2143.88 | 189844 | 291696 | 7.13 | 7.00 | 2104 |
| fib | llvm/mir | 55.24 | 53.85 | 2.15 | 455.77 | 1.09 | 1.10 | 75.14 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.76 | 1.78 | 1.00 |
| mandel | mir | 42.19 | 43.35 | 89572 | 1110 | 90.80 | 90.60 | 28 |
| mandel | ast | 1.66 | 1.53 | 3336 | 941 | 92.41 | 92.21 | 28 |
| mandel | llvm | 2143.03 | 2145.09 | 188024 | 291776 | 105.28 | 105.05 | 2092 |
| mandel | llvm/mir | 50.79 | 49.48 | 2.10 | 262.86 | 1.16 | 1.16 | 74.71 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 44.52 | 45.67 | 91648 | 867 | 1.68 | 1.57 | 1588 |
| sieve | ast | 1.56 | 1.44 | 3304 | 699 | 1.54 | 1.44 | 1588 |
| sieve | llvm | 2146.75 | 2148.99 | 187996 | 292032 | 2.42 | 2.29 | 3640 |
| sieve | llvm/mir | 48.22 | 47.06 | 2.05 | 336.83 | 1.44 | 1.46 | 2.29 |
| sieve | ast/mir | 0.03 | 0.03 | 0.04 | 0.81 | 0.92 | 0.92 | 1.00 |
| mix | mir | 41.64 | 42.98 | 97636 | 801 | 14.90 | 14.75 | 28 |
| mix | ast | 1.59 | 1.47 | 3300 | 869 | 14.77 | 14.59 | 24 |
| mix | llvm | 2144.78 | 2149.44 | 192576 | 291880 | 15.49 | 15.33 | 2092 |
| mix | llvm/mir | 51.50 | 50.01 | 1.97 | 364.39 | 1.04 | 1.04 | 74.71 |
| mix | ast/mir | 0.04 | 0.03 | 0.03 | 1.08 | 0.99 | 0.99 | 0.86 |
| stats | mir | 41.87 | 43.26 | 90392 | 856 | 3.53 | 3.34 | 28 |
| stats | ast | 1.57 | 1.46 | 3332 | 770 | 3.24 | 3.12 | 28 |
| stats | llvm | 2164.34 | 2169.12 | 188324 | 292048 | 3.69 | 3.57 | 2080 |
| stats | llvm/mir | 51.69 | 50.14 | 2.08 | 341.18 | 1.04 | 1.07 | 74.29 |
| stats | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 0.92 | 0.94 | 1.00 |
| refs | mir | 41.50 | 42.75 | 90500 | 1400 | 1.74 | 1.62 | 28 |
| refs | ast | 1.73 | 1.60 | 3368 | 901 | 1.76 | 1.63 | 28 |
| refs | llvm | 2130.58 | 2135.56 | 189784 | 292256 | 3.88 | 3.74 | 2088 |
| refs | llvm/mir | 51.34 | 49.96 | 2.10 | 208.75 | 2.23 | 2.31 | 74.57 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.64 | 1.01 | 1.01 | 1.00 |
| heap | mir | 40.63 | 41.78 | 89808 | 989 | 3.28 | 3.15 | 4716 |
| heap | ast | 1.61 | 1.50 | 3340 | 954 | 3.17 | 3.05 | 4716 |
| heap | llvm | 2148.66 | 2153.38 | 191992 | 292400 | 3.43 | 3.31 | 6704 |
| heap | llvm/mir | 52.89 | 51.54 | 2.14 | 295.65 | 1.05 | 1.05 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.97 | 0.97 | 1.00 |
| callrec | mir | 38.85 | 40.03 | 88492 | 640 | 2.65 | 2.50 | 28 |
| callrec | ast | 1.53 | 1.41 | 3296 | 620 | 2.90 | 2.77 | 28 |
| callrec | llvm | 2161.75 | 2163.65 | 190180 | 291680 | 2.86 | 2.74 | 2032 |
| callrec | llvm/mir | 55.64 | 54.05 | 2.15 | 455.75 | 1.08 | 1.09 | 72.57 |
| callrec | ast/mir | 0.04 | 0.04 | 0.04 | 0.97 | 1.09 | 1.11 | 1.00 |
| forloops | mir | 43.34 | 44.56 | 89760 | 816 | 1.44 | 1.32 | 28 |
| forloops | ast | 1.57 | 1.44 | 3284 | 737 | 1.48 | 1.36 | 28 |
| forloops | llvm | 2132.45 | 2137.04 | 187804 | 292480 | 3.34 | 3.21 | 2088 |
| forloops | llvm/mir | 49.20 | 47.95 | 2.09 | 358.43 | 2.32 | 2.43 | 74.57 |
| forloops | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 1.02 | 1.03 | 1.00 |

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
| hello | 36.76 | 37.78 | 90264 | 1.63 | 1.51 | 3024 |
| fib | 36.94 | 38.07 | 88096 | 1.87 | 1.75 | 3104 |
| mandel | 39.56 | 40.86 | 89572 | 2.63 | 2.49 | 3088 |
| sieve | 41.25 | 42.53 | 91648 | 3.27 | 3.13 | 4792 |
| mix | 39.42 | 40.87 | 97636 | 2.23 | 2.11 | 3144 |
| stats | 39.69 | 41.19 | 90392 | 2.19 | 2.07 | 3036 |
| refs | 38.45 | 39.84 | 90500 | 3.06 | 2.91 | 3168 |
| heap | 38.57 | 39.86 | 89808 | 2.06 | 1.92 | 3140 |
| callrec | 36.97 | 38.28 | 88492 | 1.89 | 1.76 | 3076 |
| forloops | 41.15 | 42.51 | 89760 | 2.19 | 2.06 | 3056 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

