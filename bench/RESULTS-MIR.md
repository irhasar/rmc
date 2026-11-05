# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-05 13:51 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 38.66 | 39.83 | 88324 | 502 | 0.23 | 0.12 | 28 |
| hello | ast | 1.20 | 1.10 | 3012 | 478 | 0.24 | 0.13 | 28 |
| hello | llvm | 2138.38 | 2137.09 | 185748 | 289416 | 0.82 | 0.71 | 2048 |
| hello | llvm/mir | 55.32 | 53.65 | 2.10 | 576.53 | 3.52 | 5.92 | 73.14 |
| fib | mir | 39.58 | 41.00 | 88120 | 639 | 7.22 | 7.03 | 28 |
| fib | ast | 1.29 | 1.18 | 3172 | 618 | 7.96 | 7.80 | 28 |
| fib | llvm | 2149.50 | 2150.75 | 188292 | 291696 | 7.72 | 7.57 | 2108 |
| fib | llvm/mir | 54.31 | 52.45 | 2.14 | 456.49 | 1.07 | 1.08 | 75.29 |
| mandel | mir | 44.68 | 46.01 | 89576 | 1109 | 90.82 | 90.61 | 28 |
| mandel | ast | 1.99 | 1.77 | 3128 | 941 | 93.20 | 92.99 | 28 |
| mandel | llvm | 2178.83 | 2183.85 | 192608 | 291776 | 106.38 | 106.02 | 2108 |
| mandel | llvm/mir | 48.77 | 47.47 | 2.15 | 263.10 | 1.17 | 1.17 | 75.29 |
| sieve | mir | 43.14 | 44.42 | 91648 | 836 | 1.80 | 1.61 | 1588 |
| sieve | ast | 1.34 | 1.23 | 3100 | 699 | 2.25 | 2.11 | 1588 |
| sieve | llvm | 2127.03 | 2131.28 | 187312 | 292032 | 3.28 | 3.03 | 3664 |
| sieve | llvm/mir | 49.30 | 47.98 | 2.04 | 349.32 | 1.82 | 1.89 | 2.31 |
| mix | mir | 39.30 | 40.37 | 94040 | 794 | 15.26 | 15.09 | 28 |
| mix | ast | 1.37 | 1.27 | 3180 | 869 | 14.79 | 14.61 | 28 |
| mix | llvm | 2116.58 | 2120.27 | 190056 | 291880 | 15.93 | 15.75 | 2064 |
| mix | llvm/mir | 53.85 | 52.52 | 2.02 | 367.61 | 1.04 | 1.04 | 73.71 |
| stats | mir | 42.78 | 44.09 | 90332 | 855 | 3.45 | 3.31 | 28 |
| stats | ast | 1.38 | 1.26 | 3092 | 770 | 3.45 | 3.32 | 24 |
| stats | llvm | 2173.28 | 2178.13 | 187692 | 292048 | 3.87 | 3.76 | 2092 |
| stats | llvm/mir | 50.80 | 49.41 | 2.08 | 341.58 | 1.12 | 1.13 | 74.71 |
| refs | mir | 41.91 | 43.13 | 90708 | 1489 | 1.83 | 1.70 | 28 |
| refs | ast | 1.67 | 1.52 | 3192 | 901 | 1.79 | 1.68 | 24 |
| refs | llvm | 2117.94 | 2122.72 | 193688 | 292256 | 4.02 | 3.88 | 2028 |
| refs | llvm/mir | 50.54 | 49.22 | 2.14 | 196.28 | 2.20 | 2.28 | 72.43 |
| heap | mir | 40.17 | 41.37 | 89464 | 988 | 3.27 | 3.15 | 4716 |
| heap | ast | 1.38 | 1.27 | 3180 | 954 | 3.19 | 3.08 | 4716 |
| heap | llvm | 2166.30 | 2171.98 | 188388 | 292400 | 3.45 | 3.33 | 6724 |
| heap | llvm/mir | 53.92 | 52.50 | 2.11 | 295.95 | 1.06 | 1.06 | 1.43 |
| callrec | mir | 39.18 | 40.47 | 87976 | 639 | 2.64 | 2.53 | 28 |
| callrec | ast | 1.30 | 1.18 | 3160 | 622 | 2.89 | 2.76 | 28 |
| callrec | llvm | 2124.72 | 2129.53 | 187724 | 291680 | 2.85 | 2.73 | 2088 |
| callrec | llvm/mir | 54.22 | 52.62 | 2.13 | 456.46 | 1.08 | 1.08 | 74.57 |
| forloops | mir | 42.89 | 44.20 | 89880 | 815 | 1.47 | 1.34 | 28 |
| forloops | ast | 1.39 | 1.26 | 3116 | 737 | 1.47 | 1.35 | 28 |
| forloops | llvm | 2123.51 | 2128.18 | 187860 | 292480 | 3.29 | 3.18 | 2040 |
| forloops | llvm/mir | 49.51 | 48.15 | 2.09 | 358.87 | 2.23 | 2.37 | 72.86 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 37.27 | 38.55 | 88324 | 1.39 | 1.28 | 2752 |
| fib | 37.98 | 39.52 | 88120 | 1.60 | 1.48 | 2832 |
| mandel | 42.21 | 43.68 | 89576 | 2.47 | 2.33 | 2824 |
| sieve | 40.10 | 41.52 | 91648 | 3.04 | 2.89 | 4520 |
| mix | 37.35 | 38.53 | 94040 | 1.95 | 1.84 | 2832 |
| stats | 40.77 | 42.21 | 90332 | 2.01 | 1.88 | 2856 |
| refs | 38.84 | 40.19 | 90708 | 3.07 | 2.94 | 2860 |
| heap | 38.30 | 39.63 | 89464 | 1.88 | 1.74 | 2804 |
| callrec | 37.49 | 38.90 | 87976 | 1.69 | 1.56 | 2816 |
| forloops | 40.95 | 42.39 | 89880 | 1.94 | 1.81 | 2808 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

