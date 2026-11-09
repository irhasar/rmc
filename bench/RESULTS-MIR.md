# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-09 00:36 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 37.02 | 38.14 | 88168 | 502 | 0.28 | 0.13 | 24 |
| hello | ast | 1.93 | 1.75 | 3208 | 478 | 0.26 | 0.12 | 24 |
| hello | llvm | 2056.43 | 2059.67 | 188328 | 289416 | 0.80 | 0.70 | 2060 |
| hello | llvm/mir | 55.55 | 54.01 | 2.14 | 576.53 | 2.88 | 5.24 | 85.83 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.95 | 0.92 | 0.93 | 1.00 |
| fib | mir | 37.23 | 38.25 | 88148 | 640 | 6.53 | 6.41 | 24 |
| fib | ast | 1.53 | 1.41 | 3288 | 616 | 7.21 | 7.07 | 24 |
| fib | llvm | 2145.09 | 2147.70 | 189500 | 291696 | 7.07 | 6.95 | 2064 |
| fib | llvm/mir | 57.61 | 56.15 | 2.15 | 455.77 | 1.08 | 1.08 | 86.00 |
| fib | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 1.10 | 1.10 | 1.00 |
| mandel | mir | 41.21 | 42.44 | 89280 | 1110 | 90.68 | 90.47 | 24 |
| mandel | ast | 1.65 | 1.50 | 3272 | 941 | 92.30 | 92.06 | 24 |
| mandel | llvm | 2084.07 | 2087.96 | 188180 | 291776 | 105.28 | 105.07 | 2064 |
| mandel | llvm/mir | 50.58 | 49.20 | 2.11 | 262.86 | 1.16 | 1.16 | 86.00 |
| mandel | ast/mir | 0.04 | 0.04 | 0.04 | 0.85 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 44.39 | 45.51 | 100208 | 867 | 1.67 | 1.53 | 1584 |
| sieve | ast | 1.95 | 1.75 | 3320 | 699 | 1.53 | 1.43 | 1584 |
| sieve | llvm | 2104.04 | 2108.68 | 187876 | 292032 | 2.40 | 2.28 | 3624 |
| sieve | llvm/mir | 47.40 | 46.34 | 1.87 | 336.83 | 1.44 | 1.49 | 2.29 |
| sieve | ast/mir | 0.04 | 0.04 | 0.03 | 0.81 | 0.92 | 0.93 | 1.00 |
| mix | mir | 41.81 | 42.99 | 90216 | 801 | 14.93 | 14.78 | 24 |
| mix | ast | 1.64 | 1.51 | 3324 | 869 | 14.76 | 14.62 | 24 |
| mix | llvm | 2099.36 | 2104.17 | 190664 | 291880 | 15.48 | 15.33 | 2016 |
| mix | llvm/mir | 50.21 | 48.94 | 2.11 | 364.39 | 1.04 | 1.04 | 84.00 |
| mix | ast/mir | 0.04 | 0.04 | 0.04 | 1.08 | 0.99 | 0.99 | 1.00 |
| stats | mir | 41.24 | 42.41 | 90444 | 856 | 3.28 | 3.15 | 24 |
| stats | ast | 1.68 | 1.55 | 3260 | 770 | 3.23 | 3.10 | 24 |
| stats | llvm | 2098.31 | 2102.10 | 188792 | 292048 | 3.71 | 3.58 | 2040 |
| stats | llvm/mir | 50.88 | 49.56 | 2.09 | 341.18 | 1.13 | 1.14 | 85.00 |
| stats | ast/mir | 0.04 | 0.04 | 0.04 | 0.90 | 0.98 | 0.98 | 1.00 |
| refs | mir | 39.59 | 40.76 | 92068 | 1400 | 1.84 | 1.68 | 24 |
| refs | ast | 1.75 | 1.62 | 3312 | 901 | 1.73 | 1.62 | 24 |
| refs | llvm | 2085.83 | 2087.84 | 192464 | 292256 | 3.82 | 3.67 | 2068 |
| refs | llvm/mir | 52.69 | 51.22 | 2.09 | 208.75 | 2.07 | 2.18 | 86.17 |
| refs | ast/mir | 0.04 | 0.04 | 0.04 | 0.64 | 0.94 | 0.96 | 1.00 |
| heap | mir | 38.62 | 39.82 | 89768 | 989 | 3.28 | 3.14 | 4712 |
| heap | ast | 1.61 | 1.49 | 3284 | 954 | 3.13 | 2.98 | 4712 |
| heap | llvm | 2088.66 | 2090.33 | 189296 | 292400 | 3.40 | 3.26 | 6708 |
| heap | llvm/mir | 54.08 | 52.50 | 2.11 | 295.65 | 1.04 | 1.04 | 1.42 |
| heap | ast/mir | 0.04 | 0.04 | 0.04 | 0.96 | 0.95 | 0.95 | 1.00 |
| callrec | mir | 37.75 | 38.97 | 88292 | 640 | 2.63 | 2.51 | 24 |
| callrec | ast | 1.51 | 1.39 | 3236 | 620 | 2.87 | 2.75 | 24 |
| callrec | llvm | 2074.25 | 2078.64 | 188608 | 291680 | 2.80 | 2.69 | 2000 |
| callrec | llvm/mir | 54.94 | 53.34 | 2.14 | 455.75 | 1.06 | 1.07 | 83.33 |
| callrec | ast/mir | 0.04 | 0.04 | 0.04 | 0.97 | 1.09 | 1.10 | 1.00 |
| forloops | mir | 41.05 | 42.26 | 89892 | 816 | 1.45 | 1.34 | 20 |
| forloops | ast | 1.58 | 1.47 | 3384 | 737 | 1.46 | 1.34 | 24 |
| forloops | llvm | 2118.36 | 2123.34 | 188584 | 292480 | 3.31 | 3.20 | 2064 |
| forloops | llvm/mir | 51.61 | 50.25 | 2.10 | 358.43 | 2.28 | 2.39 | 103.20 |
| forloops | ast/mir | 0.04 | 0.03 | 0.04 | 0.90 | 1.01 | 1.00 | 1.20 |

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
| hello | 34.92 | 36.18 | 88168 | 2.10 | 1.96 | 2968 |
| fib | 35.35 | 36.51 | 88148 | 1.88 | 1.74 | 3056 |
| mandel | 38.57 | 39.93 | 89280 | 2.64 | 2.51 | 3060 |
| sieve | 39.77 | 41.11 | 100208 | 4.62 | 4.39 | 4764 |
| mix | 39.43 | 40.75 | 90216 | 2.38 | 2.24 | 3080 |
| stats | 38.25 | 39.56 | 90444 | 2.99 | 2.85 | 3028 |
| refs | 36.55 | 37.85 | 92068 | 3.04 | 2.91 | 3036 |
| heap | 36.56 | 37.86 | 89768 | 2.07 | 1.95 | 3080 |
| callrec | 35.80 | 37.14 | 88292 | 1.95 | 1.82 | 3048 |
| forloops | 38.86 | 40.21 | 89892 | 2.18 | 2.05 | 2988 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

