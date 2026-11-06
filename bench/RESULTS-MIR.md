# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-06 10:11 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 35.78 | 36.71 | 93484 | 502 | 0.25 | 0.12 | 24 |
| hello | ast | 1.67 | 1.55 | 3164 | 478 | 0.24 | 0.11 | 20 |
| hello | llvm | 2019.34 | 2022.75 | 187092 | 289416 | 0.83 | 0.71 | 2004 |
| hello | llvm/mir | 56.44 | 55.10 | 2.00 | 576.53 | 3.29 | 5.97 | 83.50 |
| fib | mir | 36.55 | 37.63 | 88216 | 640 | 6.52 | 6.35 | 24 |
| fib | ast | 1.44 | 1.30 | 3248 | 616 | 7.12 | 7.01 | 24 |
| fib | llvm | 2058.56 | 2063.12 | 189656 | 291696 | 7.11 | 6.97 | 2104 |
| fib | llvm/mir | 56.32 | 54.82 | 2.15 | 455.77 | 1.09 | 1.10 | 87.67 |
| mandel | mir | 40.36 | 41.48 | 90116 | 1110 | 90.92 | 90.60 | 20 |
| mandel | ast | 1.54 | 1.41 | 3264 | 941 | 92.33 | 92.14 | 20 |
| mandel | llvm | 2068.17 | 2072.31 | 191000 | 291776 | 105.12 | 104.92 | 2076 |
| mandel | llvm/mir | 51.24 | 49.96 | 2.12 | 262.86 | 1.16 | 1.16 | 103.80 |
| sieve | mir | 42.77 | 43.85 | 92444 | 867 | 1.69 | 1.57 | 1584 |
| sieve | ast | 1.44 | 1.32 | 3236 | 699 | 1.55 | 1.42 | 1584 |
| sieve | llvm | 2055.21 | 2059.76 | 188220 | 292032 | 2.49 | 2.31 | 3568 |
| sieve | llvm/mir | 48.05 | 46.98 | 2.04 | 336.83 | 1.48 | 1.47 | 2.25 |
| mix | mir | 39.04 | 40.08 | 90116 | 801 | 14.87 | 14.73 | 24 |
| mix | ast | 1.53 | 1.36 | 3284 | 869 | 14.65 | 14.53 | 24 |
| mix | llvm | 2052.12 | 2056.44 | 190980 | 291880 | 15.42 | 15.28 | 2104 |
| mix | llvm/mir | 52.56 | 51.30 | 2.12 | 364.39 | 1.04 | 1.04 | 87.67 |
| stats | mir | 40.40 | 41.51 | 90280 | 856 | 3.27 | 3.15 | 24 |
| stats | ast | 1.47 | 1.35 | 3184 | 770 | 3.24 | 3.13 | 20 |
| stats | llvm | 2058.43 | 2062.59 | 188084 | 292048 | 3.72 | 3.62 | 2036 |
| stats | llvm/mir | 50.95 | 49.69 | 2.08 | 341.18 | 1.14 | 1.15 | 84.83 |
| refs | mir | 40.32 | 41.45 | 91616 | 1432 | 1.73 | 1.62 | 24 |
| refs | ast | 1.60 | 1.47 | 3232 | 901 | 1.72 | 1.61 | 20 |
| refs | llvm | 2057.01 | 2060.78 | 189408 | 292256 | 3.85 | 3.71 | 2056 |
| refs | llvm/mir | 51.02 | 49.72 | 2.07 | 204.09 | 2.23 | 2.30 | 85.67 |
| heap | mir | 37.51 | 38.63 | 89540 | 989 | 3.31 | 3.19 | 4712 |
| heap | ast | 1.50 | 1.38 | 3176 | 954 | 3.17 | 3.07 | 4712 |
| heap | llvm | 2062.21 | 2066.74 | 188880 | 292400 | 3.49 | 3.35 | 6712 |
| heap | llvm/mir | 54.98 | 53.49 | 2.11 | 295.65 | 1.05 | 1.05 | 1.42 |
| callrec | mir | 36.99 | 38.13 | 87972 | 640 | 2.64 | 2.51 | 24 |
| callrec | ast | 1.44 | 1.32 | 3296 | 620 | 2.85 | 2.75 | 24 |
| callrec | llvm | 2048.40 | 2052.90 | 188040 | 291680 | 2.90 | 2.78 | 2032 |
| callrec | llvm/mir | 55.38 | 53.85 | 2.14 | 455.75 | 1.10 | 1.11 | 84.67 |
| forloops | mir | 40.46 | 41.61 | 89584 | 816 | 1.44 | 1.33 | 24 |
| forloops | ast | 1.47 | 1.35 | 3360 | 737 | 1.45 | 1.33 | 24 |
| forloops | llvm | 2058.14 | 2062.63 | 188204 | 292480 | 3.31 | 3.18 | 2100 |
| forloops | llvm/mir | 50.87 | 49.57 | 2.10 | 358.43 | 2.30 | 2.40 | 87.50 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 34.23 | 35.33 | 93484 | 1.55 | 1.38 | 2868 |
| fib | 34.82 | 36.02 | 88216 | 1.73 | 1.62 | 3008 |
| mandel | 37.81 | 39.09 | 90116 | 2.55 | 2.39 | 3008 |
| sieve | 39.65 | 40.86 | 92444 | 3.12 | 2.99 | 4644 |
| mix | 36.93 | 38.09 | 90116 | 2.11 | 1.99 | 2976 |
| stats | 38.31 | 39.55 | 90280 | 2.10 | 1.96 | 3024 |
| refs | 37.33 | 38.61 | 91616 | 2.99 | 2.84 | 2952 |
| heap | 35.58 | 36.82 | 89540 | 1.93 | 1.81 | 2964 |
| callrec | 35.23 | 36.48 | 87972 | 1.76 | 1.65 | 2928 |
| forloops | 38.46 | 39.71 | 89584 | 2.00 | 1.89 | 2944 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

