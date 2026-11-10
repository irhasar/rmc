# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-10 06:14 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run_mir.py --markdown bench/RESULTS-MIR.md`.

Three pipelines are compared on every axis:

* `mir` - `rustc --emit=mir -O -Zno-codegen` then `rmcmir` (rmc's
  in-process assembler/linker; no LLVM, no libc, no crates). The
  front-end and back-end costs are split out below.
* `ast` - `rmc` compiling the `.rms` twin (the original all-in-one
  pipeline).
* `llvm` - `rustc -O` with fat LTO, one codegen unit, `panic=abort`
  and stripped symbols.

## Compile and run, all axes

| program | pipeline | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: | -----------: | -------: | -----------: | -----------: |
| hello | mir | 30.56 | 29.73 | 84288 | 510 | 0.23 | 0.12 | 28 |
| hello | ast | 1.91 | 1.77 | 3288 | 494 | 0.22 | 0.12 | 28 |
| hello | llvm | 2143.31 | 2144.93 | 187592 | 289416 | 0.81 | 0.71 | 2080 |
| hello | llvm/mir | 70.13 | 72.16 | 2.23 | 567.48 | 3.49 | 6.00 | 74.29 |
| hello | ast/mir | 0.06 | 0.06 | 0.04 | 0.97 | 0.94 | 1.01 | 1.00 |
| fib | mir | 30.01 | 29.13 | 83844 | 638 | 5.46 | 5.33 | 28 |
| fib | ast | 2.05 | 1.88 | 3300 | 624 | 6.71 | 6.58 | 28 |
| fib | llvm | 2166.79 | 2160.87 | 187960 | 291696 | 7.10 | 6.97 | 2060 |
| fib | llvm/mir | 72.20 | 74.19 | 2.24 | 457.20 | 1.30 | 1.31 | 73.57 |
| fib | ast/mir | 0.07 | 0.06 | 0.04 | 0.98 | 1.23 | 1.24 | 1.00 |
| mandel | mir | 29.65 | 29.02 | 84956 | 1092 | 90.56 | 90.28 | 28 |
| mandel | ast | 1.70 | 1.53 | 3328 | 953 | 91.89 | 91.68 | 24 |
| mandel | llvm | 2140.26 | 2144.19 | 187888 | 291776 | 105.10 | 104.89 | 2104 |
| mandel | llvm/mir | 72.19 | 73.90 | 2.21 | 267.19 | 1.16 | 1.16 | 75.14 |
| mandel | ast/mir | 0.06 | 0.05 | 0.04 | 0.87 | 1.01 | 1.02 | 0.86 |
| sieve | mir | 32.15 | 31.46 | 86420 | 850 | 1.76 | 1.64 | 1588 |
| sieve | ast | 1.66 | 1.48 | 3280 | 703 | 1.55 | 1.42 | 1588 |
| sieve | llvm | 2120.43 | 2125.04 | 188024 | 292032 | 2.38 | 2.26 | 3600 |
| sieve | llvm/mir | 65.96 | 67.55 | 2.18 | 343.57 | 1.35 | 1.38 | 2.27 |
| sieve | ast/mir | 0.05 | 0.05 | 0.04 | 0.83 | 0.88 | 0.87 | 1.00 |
| mix | mir | 30.09 | 29.49 | 84644 | 794 | 14.73 | 14.58 | 28 |
| mix | ast | 1.66 | 1.55 | 3304 | 877 | 14.76 | 14.57 | 28 |
| mix | llvm | 2143.38 | 2144.68 | 186932 | 291880 | 15.56 | 15.38 | 2068 |
| mix | llvm/mir | 71.23 | 72.73 | 2.21 | 367.61 | 1.06 | 1.06 | 73.86 |
| mix | ast/mir | 0.06 | 0.05 | 0.04 | 1.10 | 1.00 | 1.00 | 1.00 |
| stats | mir | 30.63 | 29.87 | 85932 | 846 | 3.31 | 3.18 | 28 |
| stats | ast | 1.65 | 1.52 | 3320 | 778 | 3.31 | 3.19 | 28 |
| stats | llvm | 2135.17 | 2139.89 | 187904 | 292048 | 3.74 | 3.59 | 2064 |
| stats | llvm/mir | 69.71 | 71.64 | 2.19 | 345.21 | 1.13 | 1.13 | 73.71 |
| stats | ast/mir | 0.05 | 0.05 | 0.04 | 0.92 | 1.00 | 1.00 | 1.00 |
| refs | mir | 32.36 | 31.73 | 86388 | 1414 | 1.86 | 1.72 | 28 |
| refs | ast | 1.77 | 1.66 | 3312 | 909 | 1.76 | 1.63 | 24 |
| refs | llvm | 2128.85 | 2132.80 | 193468 | 292256 | 3.86 | 3.72 | 2100 |
| refs | llvm/mir | 65.78 | 67.21 | 2.24 | 206.69 | 2.07 | 2.16 | 75.00 |
| refs | ast/mir | 0.05 | 0.05 | 0.04 | 0.64 | 0.94 | 0.95 | 0.86 |
| heap | mir | 29.73 | 29.23 | 85280 | 976 | 3.33 | 3.20 | 4716 |
| heap | ast | 1.64 | 1.51 | 3344 | 946 | 3.26 | 3.14 | 4716 |
| heap | llvm | 2114.75 | 2119.15 | 192504 | 292400 | 3.54 | 3.39 | 6712 |
| heap | llvm/mir | 71.12 | 72.50 | 2.26 | 299.59 | 1.06 | 1.06 | 1.42 |
| heap | ast/mir | 0.06 | 0.05 | 0.04 | 0.97 | 0.98 | 0.98 | 1.00 |
| callrec | mir | 29.67 | 29.05 | 84056 | 638 | 2.24 | 2.13 | 28 |
| callrec | ast | 1.54 | 1.42 | 3324 | 628 | 2.73 | 2.62 | 28 |
| callrec | llvm | 2108.09 | 2112.68 | 190060 | 291680 | 2.94 | 2.79 | 2088 |
| callrec | llvm/mir | 71.05 | 72.73 | 2.26 | 457.18 | 1.31 | 1.31 | 74.57 |
| callrec | ast/mir | 0.05 | 0.05 | 0.04 | 0.98 | 1.22 | 1.23 | 1.00 |
| forloops | mir | 30.19 | 29.58 | 85116 | 771 | 1.47 | 1.35 | 28 |
| forloops | ast | 1.62 | 1.49 | 3308 | 725 | 2.09 | 1.98 | 24 |
| forloops | llvm | 2119.26 | 2123.48 | 187204 | 292480 | 3.35 | 3.21 | 2060 |
| forloops | llvm/mir | 70.21 | 71.78 | 2.20 | 379.35 | 2.29 | 2.37 | 73.57 |
| forloops | ast/mir | 0.05 | 0.05 | 0.04 | 0.94 | 1.42 | 1.46 | 0.86 |

Ratios are the other pipeline divided by mir, so a value above 1
means the MIR pipeline is better on that dimension. Every
`llvm/mir` row above is above 1: the MIR pipeline beats LLVM on
every axis. The `ast/mir` rows are reported for honesty - the
`.rms` path skips rustc's front end, so it is cheaper to compile,
but the MIR path is the one that replaces LLVM.

## Front-end and back-end cost

`rustc --emit=mir -Zno-codegen` is the front end; `rmcmir` is the back
end. The headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 28.21 | 27.53 | 84288 | 2.36 | 2.19 | 3220 |
| fib | 27.16 | 26.43 | 83844 | 2.85 | 2.69 | 3260 |
| mandel | 26.57 | 26.10 | 84956 | 3.08 | 2.91 | 3280 |
| sieve | 28.60 | 28.09 | 86420 | 3.55 | 3.37 | 4944 |
| mix | 27.58 | 27.12 | 84644 | 2.51 | 2.37 | 3252 |
| stats | 28.15 | 27.53 | 85932 | 2.48 | 2.33 | 3284 |
| refs | 28.81 | 28.32 | 86388 | 3.55 | 3.41 | 3344 |
| heap | 27.43 | 27.05 | 85280 | 2.30 | 2.19 | 3284 |
| callrec | 27.59 | 27.10 | 84056 | 2.08 | 1.94 | 3284 |
| forloops | 27.80 | 27.33 | 85116 | 2.39 | 2.25 | 3272 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

