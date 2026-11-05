# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-05 17:37 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 36.82 | 38.01 | 88388 | 502 | 0.23 | 0.12 | 24 |
| hello | ast | 1.24 | 1.11 | 3052 | 478 | 0.25 | 0.13 | 20 |
| hello | llvm | 2134.68 | 2137.99 | 185936 | 289416 | 0.83 | 0.70 | 2064 |
| hello | llvm/mir | 57.97 | 56.24 | 2.10 | 576.53 | 3.54 | 5.63 | 86.00 |
| fib | mir | 38.33 | 39.47 | 88200 | 639 | 6.54 | 6.38 | 24 |
| fib | ast | 1.33 | 1.20 | 3080 | 616 | 7.19 | 7.04 | 24 |
| fib | llvm | 2160.34 | 2164.76 | 185972 | 291696 | 7.26 | 7.06 | 2032 |
| fib | llvm/mir | 56.36 | 54.85 | 2.11 | 456.49 | 1.11 | 1.11 | 84.67 |
| mandel | mir | 42.58 | 43.84 | 90516 | 1109 | 90.56 | 90.33 | 24 |
| mandel | ast | 1.42 | 1.30 | 3128 | 941 | 92.34 | 92.11 | 24 |
| mandel | llvm | 2195.14 | 2196.52 | 191112 | 291776 | 105.16 | 104.95 | 2096 |
| mandel | llvm/mir | 51.56 | 50.10 | 2.11 | 263.10 | 1.16 | 1.16 | 87.33 |
| sieve | mir | 42.27 | 43.45 | 91604 | 866 | 1.75 | 1.61 | 1584 |
| sieve | ast | 1.36 | 1.24 | 3144 | 699 | 1.57 | 1.44 | 1584 |
| sieve | llvm | 2141.00 | 2145.48 | 187944 | 292032 | 2.42 | 2.29 | 3604 |
| sieve | llvm/mir | 50.65 | 49.38 | 2.05 | 337.22 | 1.38 | 1.43 | 2.28 |
| mix | mir | 41.34 | 42.50 | 90080 | 800 | 15.02 | 14.83 | 24 |
| mix | ast | 1.45 | 1.33 | 3228 | 869 | 14.89 | 14.71 | 20 |
| mix | llvm | 2302.77 | 2304.38 | 187268 | 291880 | 15.70 | 15.47 | 2056 |
| mix | llvm/mir | 55.71 | 54.22 | 2.08 | 364.85 | 1.05 | 1.04 | 85.67 |
| stats | mir | 41.63 | 42.92 | 90360 | 855 | 3.31 | 3.16 | 24 |
| stats | ast | 1.37 | 1.25 | 3116 | 770 | 3.27 | 3.12 | 24 |
| stats | llvm | 2193.41 | 2197.70 | 188268 | 292048 | 3.75 | 3.61 | 2080 |
| stats | llvm/mir | 52.69 | 51.21 | 2.08 | 341.58 | 1.13 | 1.14 | 86.67 |
| refs | mir | 41.18 | 42.43 | 90672 | 1431 | 1.72 | 1.60 | 24 |
| refs | ast | 1.55 | 1.43 | 3196 | 901 | 1.72 | 1.60 | 24 |
| refs | llvm | 2238.02 | 2241.25 | 193264 | 292256 | 3.84 | 3.73 | 2104 |
| refs | llvm/mir | 54.35 | 52.83 | 2.13 | 204.23 | 2.24 | 2.33 | 87.67 |
| heap | mir | 38.41 | 39.46 | 89464 | 988 | 3.28 | 3.12 | 4712 |
| heap | ast | 1.39 | 1.28 | 3188 | 954 | 3.20 | 3.07 | 4712 |
| heap | llvm | 2167.22 | 2171.00 | 192592 | 292400 | 3.41 | 3.30 | 6700 |
| heap | llvm/mir | 56.42 | 55.01 | 2.15 | 295.95 | 1.04 | 1.06 | 1.42 |
| callrec | mir | 37.63 | 38.77 | 88124 | 639 | 2.67 | 2.55 | 24 |
| callrec | ast | 1.30 | 1.18 | 3116 | 620 | 2.90 | 2.75 | 24 |
| callrec | llvm | 2196.21 | 2201.15 | 187124 | 291680 | 2.84 | 2.71 | 2112 |
| callrec | llvm/mir | 58.36 | 56.78 | 2.12 | 456.46 | 1.06 | 1.06 | 88.00 |
| forloops | mir | 41.60 | 42.80 | 89640 | 815 | 1.45 | 1.34 | 20 |
| forloops | ast | 1.37 | 1.25 | 3188 | 737 | 1.46 | 1.35 | 24 |
| forloops | llvm | 2179.18 | 2182.47 | 188740 | 292480 | 3.33 | 3.21 | 2060 |
| forloops | llvm/mir | 52.38 | 50.99 | 2.11 | 358.87 | 2.30 | 2.40 | 103.00 |

Ratios are llvm divided by mir, so a value above 1 means the MIR
pipeline is better on that dimension. Every row above is above 1.

## Front-end and back-end cost

`rustc --emit=mir` is the front end; `rmcmir` is the back end. The
headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 35.44 | 36.76 | 88388 | 1.39 | 1.26 | 2848 |
| fib | 36.68 | 37.94 | 88200 | 1.65 | 1.53 | 2912 |
| mandel | 40.14 | 41.56 | 90516 | 2.44 | 2.28 | 2968 |
| sieve | 39.19 | 40.54 | 91604 | 3.08 | 2.91 | 4596 |
| mix | 39.22 | 40.51 | 90080 | 2.12 | 1.99 | 2928 |
| stats | 39.63 | 41.05 | 90360 | 2.00 | 1.87 | 2872 |
| refs | 38.38 | 39.76 | 90672 | 2.80 | 2.67 | 2928 |
| heap | 36.57 | 37.75 | 89464 | 1.84 | 1.71 | 2900 |
| callrec | 35.95 | 37.21 | 88124 | 1.68 | 1.55 | 2836 |
| forloops | 39.64 | 40.96 | 89640 | 1.96 | 1.84 | 2948 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

