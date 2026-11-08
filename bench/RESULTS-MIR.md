# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-08 19:50 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 35.67 | 36.77 | 94016 | 502 | 0.24 | 0.12 | 24 |
| hello | ast | 1.58 | 1.43 | 3096 | 478 | 0.23 | 0.12 | 28 |
| hello | llvm | 2037.68 | 2041.14 | 188500 | 289416 | 0.83 | 0.70 | 2032 |
| hello | llvm/mir | 57.12 | 55.51 | 2.00 | 576.53 | 3.44 | 5.68 | 84.67 |
| hello | ast/mir | 0.04 | 0.04 | 0.03 | 0.95 | 0.96 | 0.96 | 1.17 |
| fib | mir | 36.69 | 37.74 | 88176 | 640 | 6.53 | 6.37 | 32 |
| fib | ast | 1.55 | 1.41 | 3324 | 616 | 7.16 | 7.02 | 32 |
| fib | llvm | 2075.65 | 2079.11 | 189776 | 291696 | 7.14 | 6.98 | 2076 |
| fib | llvm/mir | 56.57 | 55.10 | 2.15 | 455.77 | 1.09 | 1.10 | 64.88 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.10 | 1.10 | 1.00 |
| mandel | mir | 40.21 | 41.40 | 89484 | 1110 | 90.60 | 90.29 | 32 |
| mandel | ast | 1.65 | 1.51 | 3276 | 941 | 92.22 | 92.03 | 28 |
| mandel | llvm | 2070.82 | 2075.38 | 187924 | 291776 | 105.16 | 104.94 | 2124 |
| mandel | llvm/mir | 51.50 | 50.13 | 2.10 | 262.86 | 1.16 | 1.16 | 66.38 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 0.88 |
| sieve | mir | 41.94 | 43.06 | 91804 | 867 | 1.69 | 1.56 | 1592 |
| sieve | ast | 1.60 | 1.47 | 3360 | 699 | 1.52 | 1.41 | 1592 |
| sieve | llvm | 2086.94 | 2091.24 | 188248 | 292032 | 2.39 | 2.26 | 3628 |
| sieve | llvm/mir | 49.76 | 48.57 | 2.05 | 336.83 | 1.41 | 1.45 | 2.28 |
| sieve | ast/mir | 0.04 | 0.03 | 0.04 | 0.81 | 0.90 | 0.90 | 1.00 |
| mix | mir | 38.84 | 39.95 | 90012 | 801 | 14.90 | 14.76 | 32 |
| mix | ast | 1.62 | 1.50 | 3272 | 869 | 14.69 | 14.55 | 32 |
| mix | llvm | 2071.79 | 2076.19 | 190132 | 291880 | 15.47 | 15.31 | 2064 |
| mix | llvm/mir | 53.34 | 51.97 | 2.11 | 364.39 | 1.04 | 1.04 | 64.50 |
| mix | ast/mir | 0.04 | 0.04 | 0.04 | 1.08 | 0.99 | 0.99 | 1.00 |
| stats | mir | 39.29 | 40.46 | 90360 | 856 | 3.27 | 3.15 | 32 |
| stats | ast | 1.61 | 1.48 | 3368 | 770 | 3.20 | 3.09 | 32 |
| stats | llvm | 2092.68 | 2096.80 | 187308 | 292048 | 3.74 | 3.60 | 2068 |
| stats | llvm/mir | 53.26 | 51.83 | 2.07 | 341.18 | 1.14 | 1.14 | 64.62 |
| stats | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.98 | 0.98 | 1.00 |
| refs | mir | 40.57 | 41.62 | 91720 | 1400 | 1.74 | 1.61 | 32 |
| refs | ast | 1.81 | 1.68 | 3368 | 901 | 1.75 | 1.62 | 32 |
| refs | llvm | 2089.83 | 2094.28 | 189696 | 292256 | 3.81 | 3.67 | 2096 |
| refs | llvm/mir | 51.51 | 50.32 | 2.07 | 208.75 | 2.18 | 2.27 | 65.50 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.64 | 1.01 | 1.00 | 1.00 |
| heap | mir | 38.91 | 39.97 | 89676 | 989 | 3.27 | 3.13 | 4720 |
| heap | ast | 1.64 | 1.51 | 3304 | 954 | 3.18 | 3.04 | 4720 |
| heap | llvm | 2099.64 | 2103.03 | 192912 | 292400 | 3.39 | 3.27 | 6704 |
| heap | llvm/mir | 53.96 | 52.62 | 2.15 | 295.65 | 1.04 | 1.04 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.97 | 0.97 | 1.00 |
| callrec | mir | 37.48 | 38.55 | 88076 | 640 | 2.64 | 2.51 | 32 |
| callrec | ast | 1.70 | 1.56 | 3388 | 620 | 2.91 | 2.75 | 32 |
| callrec | llvm | 2070.82 | 2074.84 | 187276 | 291680 | 2.82 | 2.69 | 2048 |
| callrec | llvm/mir | 55.25 | 53.82 | 2.13 | 455.75 | 1.07 | 1.07 | 64.00 |
| callrec | ast/mir | 0.05 | 0.04 | 0.04 | 0.97 | 1.10 | 1.09 | 1.00 |
| forloops | mir | 40.49 | 41.64 | 89648 | 816 | 1.50 | 1.38 | 32 |
| forloops | ast | 1.62 | 1.49 | 3396 | 737 | 1.49 | 1.36 | 32 |
| forloops | llvm | 2076.17 | 2081.09 | 188448 | 292480 | 3.32 | 3.21 | 2088 |
| forloops | llvm/mir | 51.28 | 49.98 | 2.10 | 358.43 | 2.21 | 2.33 | 65.25 |
| forloops | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.99 | 0.99 | 1.00 |

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
| hello | 34.02 | 35.25 | 94016 | 1.66 | 1.52 | 3024 |
| fib | 34.80 | 35.98 | 88176 | 1.89 | 1.76 | 3072 |
| mandel | 37.61 | 38.93 | 89484 | 2.60 | 2.47 | 3084 |
| sieve | 38.73 | 39.98 | 91804 | 3.21 | 3.08 | 4752 |
| mix | 36.57 | 37.81 | 90012 | 2.28 | 2.14 | 3124 |
| stats | 37.07 | 38.38 | 90360 | 2.22 | 2.08 | 3000 |
| refs | 37.45 | 38.65 | 91720 | 3.12 | 2.97 | 3132 |
| heap | 36.81 | 38.00 | 89676 | 2.10 | 1.97 | 3092 |
| callrec | 35.54 | 36.74 | 88076 | 1.94 | 1.81 | 3024 |
| forloops | 38.33 | 39.59 | 89648 | 2.16 | 2.04 | 3060 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

