# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-05 17:18 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 34.94 | 35.96 | 88308 | 502 | 0.27 | 0.14 | 24 |
| hello | ast | 1.22 | 1.12 | 2988 | 478 | 0.24 | 0.14 | 24 |
| hello | llvm | 2067.62 | 2071.00 | 188912 | 289416 | 0.96 | 0.81 | 2068 |
| hello | llvm/mir | 59.18 | 57.59 | 2.14 | 576.53 | 3.57 | 5.95 | 86.17 |
| fib | mir | 39.77 | 40.93 | 88100 | 639 | 6.54 | 6.42 | 24 |
| fib | ast | 1.37 | 1.24 | 3096 | 616 | 7.26 | 7.14 | 24 |
| fib | llvm | 2111.62 | 2115.54 | 187400 | 291696 | 7.22 | 7.09 | 2072 |
| fib | llvm/mir | 53.09 | 51.68 | 2.13 | 456.49 | 1.10 | 1.11 | 86.33 |
| mandel | mir | 40.52 | 41.62 | 89252 | 1109 | 91.08 | 90.86 | 24 |
| mandel | ast | 1.42 | 1.30 | 3076 | 941 | 92.12 | 91.92 | 24 |
| mandel | llvm | 2137.53 | 2141.92 | 191116 | 291776 | 105.25 | 105.01 | 2108 |
| mandel | llvm/mir | 52.75 | 51.46 | 2.14 | 263.10 | 1.16 | 1.16 | 87.83 |
| sieve | mir | 42.17 | 43.30 | 91604 | 866 | 1.67 | 1.56 | 1584 |
| sieve | ast | 1.36 | 1.24 | 3180 | 699 | 1.54 | 1.42 | 1584 |
| sieve | llvm | 2061.55 | 2066.16 | 188648 | 292032 | 2.57 | 2.34 | 3584 |
| sieve | llvm/mir | 48.89 | 47.71 | 2.06 | 337.22 | 1.54 | 1.51 | 2.26 |
| mix | mir | 39.73 | 40.86 | 90132 | 800 | 15.09 | 14.93 | 24 |
| mix | ast | 1.37 | 1.25 | 3156 | 869 | 14.88 | 14.73 | 24 |
| mix | llvm | 2130.85 | 2135.56 | 189920 | 291880 | 15.64 | 15.49 | 2052 |
| mix | llvm/mir | 53.64 | 52.27 | 2.11 | 364.85 | 1.04 | 1.04 | 85.50 |
| stats | mir | 39.33 | 40.54 | 90364 | 855 | 3.29 | 3.16 | 24 |
| stats | ast | 1.44 | 1.31 | 3080 | 770 | 6.51 | 6.36 | 20 |
| stats | llvm | 2078.72 | 2080.95 | 187556 | 292048 | 3.75 | 3.62 | 2080 |
| stats | llvm/mir | 52.85 | 51.34 | 2.08 | 341.58 | 1.14 | 1.14 | 86.67 |
| refs | mir | 40.13 | 41.45 | 90344 | 1431 | 1.71 | 1.60 | 20 |
| refs | ast | 1.54 | 1.42 | 3184 | 901 | 1.71 | 1.60 | 24 |
| refs | llvm | 2106.89 | 2109.64 | 187860 | 292256 | 3.84 | 3.70 | 1984 |
| refs | llvm/mir | 52.50 | 50.90 | 2.08 | 204.23 | 2.25 | 2.31 | 99.20 |
| heap | mir | 38.44 | 39.51 | 96084 | 988 | 3.29 | 3.17 | 4712 |
| heap | ast | 1.39 | 1.27 | 3096 | 954 | 3.19 | 3.07 | 4708 |
| heap | llvm | 2121.32 | 2122.46 | 188832 | 292400 | 3.44 | 3.31 | 6704 |
| heap | llvm/mir | 55.19 | 53.72 | 1.97 | 295.95 | 1.04 | 1.04 | 1.42 |
| callrec | mir | 37.27 | 38.51 | 88300 | 639 | 2.64 | 2.52 | 24 |
| callrec | ast | 1.31 | 1.19 | 3092 | 620 | 2.87 | 2.75 | 24 |
| callrec | llvm | 2097.99 | 2102.25 | 189760 | 291680 | 2.86 | 2.73 | 2060 |
| callrec | llvm/mir | 56.29 | 54.59 | 2.15 | 456.46 | 1.08 | 1.08 | 85.83 |
| forloops | mir | 40.88 | 42.06 | 89716 | 815 | 1.45 | 1.34 | 20 |
| forloops | ast | 1.34 | 1.22 | 3148 | 737 | 1.44 | 1.33 | 24 |
| forloops | llvm | 2124.10 | 2128.28 | 188208 | 292480 | 3.32 | 3.21 | 2088 |
| forloops | llvm/mir | 51.96 | 50.61 | 2.10 | 358.87 | 2.29 | 2.39 | 104.40 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 33.52 | 34.68 | 88308 | 1.42 | 1.28 | 2804 |
| fib | 38.08 | 39.37 | 88100 | 1.69 | 1.56 | 2932 |
| mandel | 38.13 | 39.37 | 89252 | 2.39 | 2.25 | 2904 |
| sieve | 39.20 | 40.45 | 91604 | 2.97 | 2.85 | 4600 |
| mix | 37.69 | 38.96 | 90132 | 2.03 | 1.90 | 2916 |
| stats | 37.37 | 38.68 | 90364 | 1.97 | 1.86 | 2944 |
| refs | 37.32 | 38.77 | 90344 | 2.81 | 2.68 | 2996 |
| heap | 36.53 | 37.74 | 96084 | 1.91 | 1.77 | 2964 |
| callrec | 35.59 | 36.95 | 88300 | 1.68 | 1.56 | 2868 |
| forloops | 38.92 | 40.22 | 89716 | 1.96 | 1.84 | 2868 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

