# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-09 08:03 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 28.45 | 27.90 | 84352 | 510 | 0.27 | 0.14 | 36 |
| hello | ast | 1.47 | 1.34 | 3188 | 494 | 0.25 | 0.13 | 36 |
| hello | llvm | 2017.98 | 2020.70 | 188728 | 289416 | 0.83 | 0.71 | 2120 |
| hello | llvm/mir | 70.93 | 72.43 | 2.24 | 567.48 | 3.09 | 5.27 | 58.89 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.97 | 0.93 | 0.99 | 1.00 |
| fib | mir | 28.42 | 27.84 | 84004 | 638 | 5.46 | 5.33 | 36 |
| fib | ast | 1.63 | 1.48 | 3336 | 624 | 6.76 | 6.63 | 36 |
| fib | llvm | 2094.91 | 2098.72 | 188120 | 291696 | 7.15 | 6.99 | 2116 |
| fib | llvm/mir | 73.72 | 75.37 | 2.24 | 457.20 | 1.31 | 1.31 | 58.78 |
| fib | ast/mir | 0.06 | 0.05 | 0.04 | 0.98 | 1.24 | 1.24 | 1.00 |
| mandel | mir | 30.17 | 29.60 | 85180 | 1092 | 90.56 | 90.36 | 36 |
| mandel | ast | 1.75 | 1.58 | 3312 | 953 | 92.12 | 91.91 | 36 |
| mandel | llvm | 2062.04 | 2066.05 | 190748 | 291776 | 105.31 | 105.09 | 2032 |
| mandel | llvm/mir | 68.36 | 69.80 | 2.24 | 267.19 | 1.16 | 1.16 | 56.44 |
| mandel | ast/mir | 0.06 | 0.05 | 0.04 | 0.87 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 30.54 | 29.95 | 86488 | 850 | 1.78 | 1.66 | 1596 |
| sieve | ast | 1.62 | 1.49 | 3276 | 703 | 1.53 | 1.41 | 1596 |
| sieve | llvm | 2067.28 | 2071.21 | 191144 | 292032 | 2.41 | 2.28 | 3588 |
| sieve | llvm/mir | 67.70 | 69.15 | 2.21 | 343.57 | 1.35 | 1.38 | 2.25 |
| sieve | ast/mir | 0.05 | 0.05 | 0.04 | 0.83 | 0.86 | 0.85 | 1.00 |
| mix | mir | 28.44 | 27.92 | 84800 | 794 | 14.70 | 14.53 | 32 |
| mix | ast | 1.63 | 1.50 | 3312 | 877 | 14.75 | 14.61 | 36 |
| mix | llvm | 2060.45 | 2064.56 | 190460 | 291880 | 15.52 | 15.34 | 2068 |
| mix | llvm/mir | 72.45 | 73.96 | 2.25 | 367.61 | 1.06 | 1.06 | 64.62 |
| mix | ast/mir | 0.06 | 0.05 | 0.04 | 1.10 | 1.00 | 1.01 | 1.12 |
| stats | mir | 30.60 | 30.04 | 86060 | 846 | 3.33 | 3.19 | 36 |
| stats | ast | 1.65 | 1.51 | 3340 | 778 | 3.30 | 3.18 | 36 |
| stats | llvm | 2136.02 | 2140.04 | 187008 | 292048 | 3.78 | 3.63 | 2104 |
| stats | llvm/mir | 69.80 | 71.24 | 2.17 | 345.21 | 1.14 | 1.14 | 58.44 |
| stats | ast/mir | 0.05 | 0.05 | 0.04 | 0.92 | 0.99 | 1.00 | 1.00 |
| refs | mir | 31.94 | 31.32 | 86452 | 1414 | 1.73 | 1.62 | 36 |
| refs | ast | 1.79 | 1.63 | 3440 | 909 | 1.75 | 1.64 | 36 |
| refs | llvm | 2073.98 | 2076.72 | 193436 | 292256 | 3.83 | 3.68 | 2116 |
| refs | llvm/mir | 64.93 | 66.30 | 2.24 | 206.69 | 2.22 | 2.27 | 58.78 |
| refs | ast/mir | 0.06 | 0.05 | 0.04 | 0.64 | 1.02 | 1.01 | 1.00 |
| heap | mir | 28.18 | 27.65 | 85352 | 976 | 3.32 | 3.21 | 4724 |
| heap | ast | 1.62 | 1.50 | 3352 | 946 | 3.23 | 3.11 | 4724 |
| heap | llvm | 2089.98 | 2092.31 | 189516 | 292400 | 3.40 | 3.27 | 6712 |
| heap | llvm/mir | 74.16 | 75.68 | 2.22 | 299.59 | 1.02 | 1.02 | 1.42 |
| heap | ast/mir | 0.06 | 0.05 | 0.04 | 0.97 | 0.97 | 0.97 | 1.00 |
| callrec | mir | 28.01 | 27.50 | 84156 | 638 | 2.24 | 2.12 | 32 |
| callrec | ast | 1.53 | 1.41 | 3152 | 628 | 2.74 | 2.62 | 32 |
| callrec | llvm | 2073.23 | 2077.48 | 190204 | 291680 | 2.85 | 2.71 | 2012 |
| callrec | llvm/mir | 74.01 | 75.55 | 2.26 | 457.18 | 1.27 | 1.28 | 62.88 |
| callrec | ast/mir | 0.05 | 0.05 | 0.04 | 0.98 | 1.22 | 1.24 | 1.00 |
| forloops | mir | 30.49 | 29.90 | 85196 | 771 | 1.49 | 1.37 | 32 |
| forloops | ast | 1.62 | 1.50 | 3372 | 725 | 2.05 | 1.94 | 36 |
| forloops | llvm | 2054.75 | 2059.11 | 188592 | 292480 | 3.31 | 3.19 | 2084 |
| forloops | llvm/mir | 67.40 | 68.86 | 2.21 | 379.35 | 2.22 | 2.33 | 65.12 |
| forloops | ast/mir | 0.05 | 0.05 | 0.04 | 0.94 | 1.38 | 1.42 | 1.12 |

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
| hello | 26.72 | 26.31 | 84352 | 1.73 | 1.59 | 3140 |
| fib | 26.38 | 25.97 | 84004 | 2.04 | 1.87 | 3152 |
| mandel | 27.32 | 26.86 | 85180 | 2.85 | 2.73 | 3228 |
| sieve | 27.18 | 26.75 | 86488 | 3.35 | 3.21 | 4800 |
| mix | 26.04 | 25.66 | 84800 | 2.40 | 2.26 | 3232 |
| stats | 28.25 | 27.81 | 86060 | 2.35 | 2.23 | 3236 |
| refs | 28.57 | 28.11 | 86452 | 3.37 | 3.22 | 3224 |
| heap | 26.00 | 25.59 | 85352 | 2.18 | 2.06 | 3192 |
| callrec | 26.07 | 25.67 | 84156 | 1.94 | 1.83 | 3192 |
| forloops | 28.18 | 27.73 | 85196 | 2.31 | 2.17 | 3228 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

