# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-05 14:00 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 38.74 | 39.58 | 89068 | 502 | 0.27 | 0.14 | 28 |
| hello | ast | 1.32 | 1.18 | 3048 | 478 | 0.26 | 0.14 | 24 |
| hello | llvm | 2129.82 | 2130.64 | 188260 | 289416 | 0.86 | 0.73 | 2068 |
| hello | llvm/mir | 54.98 | 53.83 | 2.11 | 576.53 | 3.18 | 5.17 | 73.86 |
| fib | mir | 38.78 | 40.08 | 88004 | 639 | 6.52 | 6.37 | 28 |
| fib | ast | 1.73 | 1.57 | 3184 | 616 | 7.20 | 7.06 | 28 |
| fib | llvm | 2182.25 | 2187.29 | 188124 | 291696 | 7.14 | 7.00 | 2036 |
| fib | llvm/mir | 56.28 | 54.58 | 2.14 | 456.49 | 1.09 | 1.10 | 72.71 |
| mandel | mir | 41.68 | 43.05 | 89648 | 1109 | 90.80 | 90.53 | 24 |
| mandel | ast | 1.42 | 1.30 | 3036 | 941 | 92.16 | 91.92 | 28 |
| mandel | llvm | 2165.41 | 2169.60 | 190976 | 291776 | 105.23 | 105.00 | 2096 |
| mandel | llvm/mir | 51.96 | 50.40 | 2.13 | 263.10 | 1.16 | 1.16 | 87.33 |
| sieve | mir | 44.46 | 44.79 | 99940 | 836 | 1.72 | 1.59 | 1588 |
| sieve | ast | 1.32 | 1.21 | 3144 | 699 | 1.53 | 1.42 | 1588 |
| sieve | llvm | 2181.19 | 2185.53 | 191020 | 292032 | 2.40 | 2.28 | 3584 |
| sieve | llvm/mir | 49.06 | 48.79 | 1.91 | 349.32 | 1.40 | 1.43 | 2.26 |
| mix | mir | 42.00 | 43.24 | 90032 | 794 | 14.97 | 14.79 | 28 |
| mix | ast | 1.47 | 1.33 | 3116 | 869 | 14.78 | 14.61 | 28 |
| mix | llvm | 2160.99 | 2166.15 | 186944 | 291880 | 15.73 | 15.51 | 1992 |
| mix | llvm/mir | 51.45 | 50.10 | 2.08 | 367.61 | 1.05 | 1.05 | 71.14 |
| stats | mir | 40.57 | 41.76 | 91268 | 855 | 3.29 | 3.19 | 28 |
| stats | ast | 1.37 | 1.26 | 3108 | 770 | 3.29 | 3.16 | 28 |
| stats | llvm | 2128.03 | 2132.89 | 187500 | 292048 | 3.72 | 3.58 | 2012 |
| stats | llvm/mir | 52.45 | 51.07 | 2.05 | 341.58 | 1.13 | 1.12 | 71.86 |
| refs | mir | 41.23 | 42.27 | 90392 | 1489 | 3.18 | 3.03 | 28 |
| refs | ast | 1.51 | 1.39 | 3152 | 901 | 3.13 | 2.83 | 24 |
| refs | llvm | 2135.12 | 2135.70 | 192788 | 292256 | 3.88 | 3.75 | 2100 |
| refs | llvm/mir | 51.78 | 50.52 | 2.13 | 196.28 | 1.22 | 1.24 | 75.00 |
| heap | mir | 39.25 | 40.43 | 89524 | 988 | 3.28 | 3.15 | 4716 |
| heap | ast | 1.47 | 1.29 | 3152 | 954 | 3.17 | 3.04 | 4716 |
| heap | llvm | 2175.84 | 2180.25 | 189220 | 292400 | 3.46 | 3.35 | 6700 |
| heap | llvm/mir | 55.43 | 53.93 | 2.11 | 295.95 | 1.06 | 1.06 | 1.42 |
| callrec | mir | 40.03 | 41.32 | 97236 | 639 | 2.66 | 2.53 | 28 |
| callrec | ast | 1.29 | 1.18 | 3132 | 620 | 2.90 | 2.78 | 28 |
| callrec | llvm | 2174.93 | 2179.41 | 188112 | 291680 | 2.86 | 2.70 | 2112 |
| callrec | llvm/mir | 54.33 | 52.74 | 1.93 | 456.46 | 1.07 | 1.07 | 75.43 |
| forloops | mir | 43.55 | 44.96 | 92652 | 815 | 1.56 | 1.35 | 24 |
| forloops | ast | 1.38 | 1.26 | 3196 | 737 | 1.72 | 1.54 | 28 |
| forloops | llvm | 2226.07 | 2228.88 | 188756 | 292480 | 3.40 | 3.26 | 2096 |
| forloops | llvm/mir | 51.12 | 49.57 | 2.04 | 358.87 | 2.18 | 2.41 | 87.33 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 37.35 | 38.30 | 89068 | 1.39 | 1.28 | 2824 |
| fib | 37.13 | 38.55 | 88004 | 1.65 | 1.53 | 2840 |
| mandel | 39.29 | 40.80 | 89648 | 2.39 | 2.25 | 2844 |
| sieve | 41.48 | 41.96 | 99940 | 2.98 | 2.84 | 4580 |
| mix | 39.89 | 41.28 | 90032 | 2.11 | 1.96 | 2952 |
| stats | 38.62 | 39.92 | 91268 | 1.95 | 1.85 | 2872 |
| refs | 38.28 | 39.45 | 90392 | 2.95 | 2.82 | 2920 |
| heap | 37.43 | 38.76 | 89524 | 1.82 | 1.67 | 2860 |
| callrec | 38.30 | 39.74 | 97236 | 1.73 | 1.58 | 2884 |
| forloops | 41.56 | 43.11 | 92652 | 1.99 | 1.86 | 2908 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

