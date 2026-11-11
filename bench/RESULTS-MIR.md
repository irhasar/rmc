# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-11 01:51 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run_mir.py --markdown bench/RESULTS-MIR.md`.

Three pipelines are compared on every axis:

* `mir` - `rustc --emit=mir -O -Zno-codegen` then `rmcmir` (rmc's
  in-process assembler/linker; no LLVM, no libc, no crates). The
  front-end and back-end costs are split out below.
* `ast` - `rmc` compiling `bench/rust/{name}.rs` with its own AST back end
  pipeline).
* `llvm` - `rustc -O` with fat LTO, one codegen unit, `panic=abort`
  and stripped symbols.

## Compile and run, all axes

| program | pipeline | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: | -----------: | -------: | -----------: | -----------: |
| hello | mir | 31.37 | 30.82 | 84168 | 510 | 0.24 | 0.12 | 20 |
| hello | ast | 1.70 | 1.58 | 3372 | 503 | 0.23 | 0.12 | 20 |
| hello | llvm | 2122.13 | 2124.87 | 188576 | 289416 | 0.86 | 0.73 | 2036 |
| hello | llvm/mir | 67.64 | 68.94 | 2.24 | 567.48 | 3.58 | 6.09 | 101.80 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.99 | 0.96 | 0.98 | 1.00 |
| fib | mir | 30.29 | 29.76 | 83708 | 773 | 5.45 | 5.32 | 24 |
| fib | ast | 1.73 | 1.61 | 3536 | 858 | 10.91 | 10.76 | 24 |
| fib | llvm | 2152.02 | 2156.09 | 190920 | 291696 | 7.12 | 7.00 | 2052 |
| fib | llvm/mir | 71.04 | 72.45 | 2.28 | 377.36 | 1.31 | 1.32 | 85.50 |
| fib | ast/mir | 0.06 | 0.05 | 0.04 | 1.11 | 2.00 | 2.02 | 1.00 |
| mandel | mir | 35.75 | 35.01 | 84868 | 1227 | 90.66 | 90.42 | 24 |
| mandel | ast | 1.97 | 1.84 | 3596 | 1200 | 92.12 | 91.90 | 20 |
| mandel | llvm | 2249.72 | 2254.21 | 186940 | 291776 | 105.43 | 105.23 | 2072 |
| mandel | llvm/mir | 62.94 | 64.39 | 2.20 | 237.80 | 1.16 | 1.16 | 86.33 |
| mandel | ast/mir | 0.06 | 0.05 | 0.04 | 0.98 | 1.02 | 1.02 | 0.83 |
| sieve | mir | 34.18 | 33.57 | 86256 | 985 | 2.09 | 1.97 | 1584 |
| sieve | ast | 1.78 | 1.66 | 3520 | 1024 | 1.99 | 1.86 | 1584 |
| sieve | llvm | 2175.12 | 2177.94 | 188032 | 292032 | 2.74 | 2.60 | 3624 |
| sieve | llvm/mir | 63.64 | 64.88 | 2.18 | 296.48 | 1.31 | 1.32 | 2.29 |
| sieve | ast/mir | 0.05 | 0.05 | 0.04 | 1.04 | 0.95 | 0.95 | 1.00 |
| mix | mir | 35.75 | 35.16 | 84532 | 929 | 14.77 | 14.57 | 20 |
| mix | ast | 1.96 | 1.82 | 3596 | 1117 | 14.87 | 14.69 | 24 |
| mix | llvm | 2217.43 | 2221.01 | 187476 | 291880 | 15.68 | 15.50 | 2096 |
| mix | llvm/mir | 62.02 | 63.16 | 2.22 | 314.19 | 1.06 | 1.06 | 104.80 |
| mix | ast/mir | 0.05 | 0.05 | 0.04 | 1.20 | 1.01 | 1.01 | 1.20 |
| stats | mir | 33.02 | 32.44 | 85704 | 981 | 3.27 | 3.14 | 20 |
| stats | ast | 1.87 | 1.72 | 3596 | 1104 | 3.29 | 3.16 | 24 |
| stats | llvm | 2131.18 | 2135.63 | 188224 | 292048 | 3.72 | 3.59 | 2008 |
| stats | llvm/mir | 64.55 | 65.83 | 2.20 | 297.70 | 1.14 | 1.14 | 100.40 |
| stats | ast/mir | 0.06 | 0.05 | 0.04 | 1.13 | 1.01 | 1.01 | 1.20 |
| refs | mir | 32.96 | 32.40 | 86260 | 1529 | 0.24 | 0.12 | 20 |
| refs | ast | 2.13 | 1.95 | 3524 | 1068 | 30.93 | 30.77 | 24 |
| refs | llvm | 2165.22 | 2169.33 | 193160 | 292256 | 4.03 | 3.88 | 2024 |
| refs | llvm/mir | 65.68 | 66.96 | 2.24 | 191.14 | 16.86 | 32.61 | 101.20 |
| refs | ast/mir | 0.06 | 0.06 | 0.04 | 0.70 | 129.38 | 258.55 | 1.20 |
| heap | mir | 31.49 | 30.89 | 85188 | 1102 | 3.39 | 3.25 | 4712 |
| heap | ast | 1.88 | 1.76 | 3560 | 1332 | 25.02 | 24.81 | 37528 |
| heap | llvm | 2171.82 | 2176.60 | 190392 | 292400 | 4.22 | 4.01 | 6716 |
| heap | llvm/mir | 68.96 | 70.46 | 2.23 | 265.34 | 1.24 | 1.24 | 1.43 |
| heap | ast/mir | 0.06 | 0.06 | 0.04 | 1.21 | 7.38 | 7.64 | 7.96 |
| callrec | mir | 37.12 | 36.45 | 83924 | 773 | 2.33 | 2.18 | 24 |
| callrec | ast | 1.86 | 1.73 | 3584 | 861 | 5.20 | 5.06 | 24 |
| callrec | llvm | 2138.03 | 2142.04 | 190220 | 291680 | 2.97 | 2.84 | 2112 |
| callrec | llvm/mir | 57.59 | 58.77 | 2.27 | 377.34 | 1.27 | 1.30 | 88.00 |
| callrec | ast/mir | 0.05 | 0.05 | 0.04 | 1.11 | 2.23 | 2.32 | 1.00 |
| forloops | mir | 32.89 | 32.27 | 84920 | 906 | 1.46 | 1.34 | 24 |
| forloops | ast | 1.86 | 1.69 | 3508 | 1004 | 1.46 | 1.34 | 24 |
| forloops | llvm | 2231.34 | 2233.54 | 190120 | 292480 | 3.33 | 3.20 | 2088 |
| forloops | llvm/mir | 67.83 | 69.21 | 2.24 | 322.83 | 2.28 | 2.38 | 87.00 |
| forloops | ast/mir | 0.06 | 0.05 | 0.04 | 1.11 | 1.00 | 1.00 | 1.00 |

Ratios are the other pipeline divided by mir, so a value above 1
means the MIR pipeline is better on that dimension. Every
`llvm/mir` row above is above 1: the MIR pipeline beats LLVM on
every axis. The `ast/mir` rows are reported for honesty - the
AST path skips rustc's front end, so it is cheaper to compile,
but the MIR path is the one that replaces LLVM.

## Front-end and back-end cost

`rustc --emit=mir -Zno-codegen` is the front end; `rmcmir` is the back
end. The headline `mir` number is their sum.

| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |
| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |
| hello | 29.43 | 29.04 | 84168 | 1.94 | 1.78 | 3328 |
| fib | 28.10 | 27.70 | 83708 | 2.20 | 2.06 | 3348 |
| mandel | 32.41 | 31.80 | 84868 | 3.34 | 3.21 | 3364 |
| sieve | 30.57 | 30.13 | 86256 | 3.61 | 3.44 | 5088 |
| mix | 32.89 | 32.42 | 84532 | 2.86 | 2.74 | 3420 |
| stats | 30.42 | 29.97 | 85704 | 2.60 | 2.48 | 3392 |
| refs | 29.37 | 28.91 | 86260 | 3.60 | 3.48 | 3452 |
| heap | 29.09 | 28.62 | 85188 | 2.40 | 2.28 | 3400 |
| callrec | 34.45 | 33.94 | 83924 | 2.67 | 2.51 | 3396 |
| forloops | 30.35 | 29.86 | 84920 | 2.54 | 2.42 | 3412 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

