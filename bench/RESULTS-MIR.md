# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-08 18:59 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 37.14 | 38.15 | 88164 | 502 | 0.23 | 0.13 | 24 |
| hello | ast | 1.47 | 1.35 | 3124 | 478 | 0.24 | 0.12 | 28 |
| hello | llvm | 2053.08 | 2055.64 | 188148 | 289416 | 0.81 | 0.70 | 2104 |
| hello | llvm/mir | 55.28 | 53.88 | 2.13 | 576.53 | 3.51 | 5.53 | 87.67 |
| hello | ast/mir | 0.04 | 0.04 | 0.04 | 0.95 | 1.02 | 0.93 | 1.17 |
| fib | mir | 38.43 | 39.56 | 88156 | 640 | 6.50 | 6.36 | 28 |
| fib | ast | 1.52 | 1.40 | 3364 | 616 | 7.16 | 7.02 | 28 |
| fib | llvm | 2086.39 | 2090.71 | 190568 | 291696 | 7.13 | 6.99 | 2052 |
| fib | llvm/mir | 54.29 | 52.84 | 2.16 | 455.77 | 1.10 | 1.10 | 73.29 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.10 | 1.10 | 1.00 |
| mandel | mir | 39.73 | 40.91 | 89388 | 1110 | 90.68 | 90.46 | 28 |
| mandel | ast | 1.65 | 1.51 | 3376 | 941 | 92.29 | 92.08 | 28 |
| mandel | llvm | 2090.72 | 2095.06 | 188188 | 291776 | 105.24 | 104.99 | 2108 |
| mandel | llvm/mir | 52.62 | 51.21 | 2.11 | 262.86 | 1.16 | 1.16 | 75.29 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 40.86 | 42.12 | 92368 | 867 | 1.69 | 1.57 | 1588 |
| sieve | ast | 1.56 | 1.45 | 3332 | 699 | 1.54 | 1.42 | 1588 |
| sieve | llvm | 2094.00 | 2098.38 | 187992 | 292032 | 2.40 | 2.27 | 3632 |
| sieve | llvm/mir | 51.25 | 49.82 | 2.04 | 336.83 | 1.42 | 1.44 | 2.29 |
| sieve | ast/mir | 0.04 | 0.03 | 0.04 | 0.81 | 0.91 | 0.90 | 1.00 |
| mix | mir | 39.24 | 40.38 | 95264 | 801 | 14.96 | 14.82 | 24 |
| mix | ast | 1.59 | 1.47 | 3272 | 869 | 14.73 | 14.58 | 28 |
| mix | llvm | 2068.70 | 2073.06 | 187996 | 291880 | 15.53 | 15.37 | 2112 |
| mix | llvm/mir | 52.72 | 51.33 | 1.97 | 364.39 | 1.04 | 1.04 | 88.00 |
| mix | ast/mir | 0.04 | 0.04 | 0.03 | 1.08 | 0.99 | 0.98 | 1.17 |
| stats | mir | 39.95 | 41.16 | 97184 | 856 | 3.29 | 3.17 | 24 |
| stats | ast | 1.60 | 1.49 | 3356 | 770 | 3.23 | 3.09 | 24 |
| stats | llvm | 2086.35 | 2089.79 | 188388 | 292048 | 3.73 | 3.58 | 2068 |
| stats | llvm/mir | 52.23 | 50.77 | 1.94 | 341.18 | 1.13 | 1.13 | 86.17 |
| stats | ast/mir | 0.04 | 0.04 | 0.03 | 0.90 | 0.98 | 0.98 | 1.00 |
| refs | mir | 41.13 | 42.31 | 94864 | 1400 | 1.73 | 1.61 | 24 |
| refs | ast | 1.73 | 1.61 | 3324 | 901 | 1.73 | 1.61 | 28 |
| refs | llvm | 2100.16 | 2104.65 | 190064 | 292256 | 3.85 | 3.74 | 2020 |
| refs | llvm/mir | 51.07 | 49.74 | 2.00 | 208.75 | 2.23 | 2.31 | 84.17 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.64 | 1.00 | 1.00 | 1.17 |
| heap | mir | 37.68 | 38.84 | 89612 | 989 | 3.26 | 3.12 | 4712 |
| heap | ast | 1.58 | 1.45 | 3352 | 954 | 3.16 | 3.05 | 4716 |
| heap | llvm | 2082.88 | 2087.29 | 192188 | 292400 | 3.38 | 3.26 | 6720 |
| heap | llvm/mir | 55.28 | 53.74 | 2.14 | 295.65 | 1.04 | 1.04 | 1.43 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.97 | 0.98 | 1.00 |
| callrec | mir | 37.11 | 38.24 | 88332 | 640 | 2.65 | 2.52 | 28 |
| callrec | ast | 1.50 | 1.39 | 3272 | 620 | 2.95 | 2.81 | 28 |
| callrec | llvm | 2083.16 | 2087.75 | 186916 | 291680 | 2.86 | 2.73 | 2076 |
| callrec | llvm/mir | 56.13 | 54.60 | 2.12 | 455.75 | 1.08 | 1.08 | 74.14 |
| callrec | ast/mir | 0.04 | 0.04 | 0.04 | 0.97 | 1.11 | 1.11 | 1.00 |
| forloops | mir | 41.46 | 42.55 | 89848 | 816 | 1.45 | 1.34 | 28 |
| forloops | ast | 1.59 | 1.46 | 3352 | 737 | 1.46 | 1.35 | 28 |
| forloops | llvm | 2103.26 | 2107.16 | 188512 | 292480 | 3.31 | 3.19 | 2052 |
| forloops | llvm/mir | 50.73 | 49.52 | 2.10 | 358.43 | 2.28 | 2.38 | 73.29 |
| forloops | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 1.01 | 1.01 | 1.00 |

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
| hello | 35.53 | 36.67 | 88164 | 1.62 | 1.48 | 3036 |
| fib | 36.53 | 37.79 | 88156 | 1.90 | 1.77 | 2980 |
| mandel | 37.11 | 38.40 | 89388 | 2.62 | 2.50 | 3104 |
| sieve | 37.66 | 39.04 | 92368 | 3.20 | 3.08 | 4788 |
| mix | 37.01 | 38.26 | 95264 | 2.23 | 2.12 | 3132 |
| stats | 37.69 | 39.06 | 97184 | 2.26 | 2.10 | 3080 |
| refs | 38.01 | 39.31 | 94864 | 3.12 | 2.99 | 3088 |
| heap | 35.58 | 36.87 | 89612 | 2.10 | 1.96 | 3144 |
| callrec | 35.24 | 36.50 | 88332 | 1.87 | 1.74 | 3080 |
| forloops | 39.24 | 40.47 | 89848 | 2.22 | 2.09 | 3084 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

