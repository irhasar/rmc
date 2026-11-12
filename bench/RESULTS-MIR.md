# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-12 10:47 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 34.27 | 34.02 | 84368 | 510 | 0.27 | 0.14 | 24 |
| hello | ast | 1.65 | 1.53 | 3504 | 503 | 0.25 | 0.13 | 24 |
| hello | llvm | 2105.14 | 2108.82 | 185400 | 289416 | 0.79 | 0.69 | 1980 |
| hello | llvm/mir | 61.44 | 61.99 | 2.20 | 567.48 | 2.98 | 5.14 | 82.50 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.99 | 0.93 | 0.93 | 1.00 |
| fib | mir | 34.51 | 34.20 | 84060 | 773 | 5.47 | 5.33 | 24 |
| fib | ast | 1.76 | 1.65 | 3652 | 870 | 6.79 | 6.65 | 24 |
| fib | llvm | 2111.86 | 2116.56 | 192004 | 291696 | 7.14 | 7.02 | 2032 |
| fib | llvm/mir | 61.19 | 61.89 | 2.28 | 377.36 | 1.31 | 1.32 | 84.67 |
| fib | ast/mir | 0.05 | 0.05 | 0.04 | 1.13 | 1.24 | 1.25 | 1.00 |
| mandel | mir | 37.44 | 37.15 | 85160 | 1227 | 90.73 | 90.50 | 24 |
| mandel | ast | 1.90 | 1.78 | 3504 | 1209 | 95.62 | 95.45 | 24 |
| mandel | llvm | 2120.57 | 2124.84 | 190968 | 291776 | 105.31 | 105.09 | 2068 |
| mandel | llvm/mir | 56.63 | 57.20 | 2.24 | 237.80 | 1.16 | 1.16 | 86.17 |
| mandel | ast/mir | 0.05 | 0.05 | 0.04 | 0.99 | 1.05 | 1.05 | 1.00 |
| sieve | mir | 37.21 | 36.92 | 86568 | 989 | 1.77 | 1.66 | 1584 |
| sieve | ast | 1.84 | 1.71 | 3572 | 1032 | 1.68 | 1.57 | 1584 |
| sieve | llvm | 2102.08 | 2106.85 | 187080 | 292032 | 2.42 | 2.28 | 3612 |
| sieve | llvm/mir | 56.50 | 57.06 | 2.16 | 295.28 | 1.37 | 1.38 | 2.28 |
| sieve | ast/mir | 0.05 | 0.05 | 0.04 | 1.04 | 0.95 | 0.95 | 1.00 |
| mix | mir | 35.84 | 35.55 | 84888 | 945 | 15.05 | 14.89 | 24 |
| mix | ast | 1.88 | 1.76 | 3564 | 1117 | 14.71 | 14.57 | 24 |
| mix | llvm | 2104.48 | 2108.64 | 187196 | 291880 | 15.52 | 15.38 | 2088 |
| mix | llvm/mir | 58.72 | 59.32 | 2.21 | 308.87 | 1.03 | 1.03 | 87.00 |
| mix | ast/mir | 0.05 | 0.05 | 0.04 | 1.18 | 0.98 | 0.98 | 1.00 |
| stats | mir | 38.97 | 38.37 | 86072 | 1067 | 2.86 | 2.75 | 20 |
| stats | ast | 2.05 | 1.85 | 3600 | 1104 | 3.45 | 3.33 | 20 |
| stats | llvm | 2128.25 | 2133.14 | 187920 | 292048 | 3.71 | 3.58 | 1976 |
| stats | llvm/mir | 54.61 | 55.59 | 2.18 | 273.71 | 1.30 | 1.30 | 98.80 |
| stats | ast/mir | 0.05 | 0.05 | 0.04 | 1.03 | 1.20 | 1.21 | 1.00 |
| refs | mir | 38.06 | 37.66 | 86464 | 1529 | 0.24 | 0.12 | 24 |
| refs | ast | 2.05 | 1.93 | 3648 | 1279 | 4.76 | 4.64 | 24 |
| refs | llvm | 2119.20 | 2123.82 | 192748 | 292512 | 5.58 | 5.45 | 2052 |
| refs | llvm/mir | 55.67 | 56.39 | 2.23 | 191.31 | 23.66 | 45.79 | 85.50 |
| refs | ast/mir | 0.05 | 0.05 | 0.04 | 0.84 | 20.18 | 38.99 | 1.00 |
| heap | mir | 36.45 | 36.10 | 85504 | 1135 | 3.17 | 3.03 | 4708 |
| heap | ast | 1.97 | 1.84 | 3656 | 1335 | 3.19 | 3.05 | 4712 |
| heap | llvm | 2124.95 | 2128.57 | 187356 | 292384 | 3.35 | 3.22 | 6628 |
| heap | llvm/mir | 58.30 | 58.96 | 2.19 | 257.61 | 1.06 | 1.06 | 1.41 |
| heap | ast/mir | 0.05 | 0.05 | 0.04 | 1.18 | 1.01 | 1.01 | 1.00 |
| callrec | mir | 34.60 | 34.31 | 84232 | 773 | 2.23 | 2.12 | 24 |
| callrec | ast | 1.78 | 1.67 | 3584 | 873 | 2.73 | 2.62 | 24 |
| callrec | llvm | 2155.56 | 2159.98 | 190028 | 291680 | 2.84 | 2.72 | 2056 |
| callrec | llvm/mir | 62.30 | 62.95 | 2.26 | 377.34 | 1.27 | 1.28 | 85.67 |
| callrec | ast/mir | 0.05 | 0.05 | 0.04 | 1.13 | 1.22 | 1.24 | 1.00 |
| forloops | mir | 34.53 | 34.27 | 85236 | 913 | 1.46 | 1.34 | 24 |
| forloops | ast | 1.85 | 1.72 | 3628 | 1011 | 1.46 | 1.34 | 20 |
| forloops | llvm | 2112.77 | 2117.15 | 187712 | 292480 | 3.33 | 3.21 | 2040 |
| forloops | llvm/mir | 61.18 | 61.78 | 2.20 | 320.35 | 2.28 | 2.40 | 85.00 |
| forloops | ast/mir | 0.05 | 0.05 | 0.04 | 1.11 | 1.00 | 1.00 | 0.83 |
| light | mir | 37.50 | 37.20 | 90892 | 1134 | 1.95 | 1.83 | 24 |
| light | ast | 1.88 | 1.75 | 3624 | 1108 | 2.13 | 2.00 | 24 |
| light | llvm | 2136.75 | 2139.92 | 187328 | 291904 | 2.61 | 2.50 | 2036 |
| light | llvm/mir | 56.97 | 57.52 | 2.06 | 257.41 | 1.34 | 1.36 | 84.83 |
| light | ast/mir | 0.05 | 0.05 | 0.04 | 0.98 | 1.09 | 1.09 | 1.00 |
| heavy | mir | 33.99 | 33.76 | 84592 | 798 | 768.25 | 767.93 | 20 |
| heavy | ast | 1.78 | 1.65 | 3612 | 969 | 770.52 | 770.18 | 24 |
| heavy | llvm | 2144.45 | 2149.34 | 187404 | 291792 | 1032.54 | 1032.17 | 2084 |
| heavy | llvm/mir | 63.09 | 63.67 | 2.22 | 365.65 | 1.34 | 1.34 | 104.20 |
| heavy | ast/mir | 0.05 | 0.05 | 0.04 | 1.21 | 1.00 | 1.00 | 1.20 |

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
| hello | 32.38 | 32.27 | 84368 | 1.88 | 1.75 | 3264 |
| fib | 32.32 | 32.15 | 84060 | 2.19 | 2.05 | 3332 |
| mandel | 34.29 | 34.14 | 85160 | 3.16 | 3.01 | 3392 |
| sieve | 33.52 | 33.39 | 86568 | 3.68 | 3.53 | 5084 |
| mix | 33.11 | 32.95 | 84888 | 2.72 | 2.59 | 3368 |
| stats | 35.00 | 34.62 | 86072 | 3.97 | 3.75 | 3372 |
| refs | 34.49 | 34.20 | 86464 | 3.57 | 3.46 | 3356 |
| heap | 33.91 | 33.70 | 85504 | 2.54 | 2.40 | 3356 |
| callrec | 32.30 | 32.14 | 84232 | 2.30 | 2.17 | 3400 |
| forloops | 32.01 | 31.86 | 85236 | 2.53 | 2.40 | 3360 |
| light | 34.54 | 34.37 | 90892 | 2.96 | 2.83 | 3360 |
| heavy | 31.79 | 31.72 | 84592 | 2.19 | 2.04 | 3336 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

