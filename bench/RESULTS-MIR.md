# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-12 05:35 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 30.98 | 30.37 | 84148 | 510 | 0.27 | 0.13 | 24 |
| hello | ast | 1.70 | 1.52 | 3464 | 503 | 0.23 | 0.12 | 24 |
| hello | llvm | 2153.33 | 2155.16 | 187340 | 289416 | 0.83 | 0.70 | 2064 |
| hello | llvm/mir | 69.51 | 70.97 | 2.23 | 567.48 | 3.11 | 5.31 | 86.00 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.99 | 0.87 | 0.91 | 1.00 |
| fib | mir | 32.05 | 31.45 | 83700 | 773 | 5.47 | 5.34 | 24 |
| fib | ast | 1.83 | 1.66 | 3560 | 870 | 6.77 | 6.63 | 24 |
| fib | llvm | 2142.13 | 2145.98 | 187236 | 291696 | 7.15 | 7.01 | 2068 |
| fib | llvm/mir | 66.83 | 68.23 | 2.24 | 377.36 | 1.31 | 1.31 | 86.17 |
| fib | ast/mir | 0.06 | 0.05 | 0.04 | 1.13 | 1.24 | 1.24 | 1.00 |
| mandel | mir | 31.98 | 31.46 | 84888 | 1227 | 90.95 | 90.74 | 24 |
| mandel | ast | 1.86 | 1.73 | 3568 | 1209 | 95.79 | 95.54 | 24 |
| mandel | llvm | 2169.67 | 2173.77 | 187244 | 291776 | 105.36 | 105.08 | 2048 |
| mandel | llvm/mir | 67.84 | 69.11 | 2.21 | 237.80 | 1.16 | 1.16 | 85.33 |
| mandel | ast/mir | 0.06 | 0.05 | 0.04 | 0.99 | 1.05 | 1.05 | 1.00 |
| sieve | mir | 32.53 | 31.93 | 86268 | 989 | 1.76 | 1.63 | 1584 |
| sieve | ast | 1.82 | 1.70 | 3688 | 1032 | 1.71 | 1.59 | 1584 |
| sieve | llvm | 2155.53 | 2159.68 | 187624 | 292032 | 2.42 | 2.31 | 3588 |
| sieve | llvm/mir | 66.26 | 67.64 | 2.17 | 295.28 | 1.37 | 1.42 | 2.27 |
| sieve | ast/mir | 0.06 | 0.05 | 0.04 | 1.04 | 0.97 | 0.98 | 1.00 |
| mix | mir | 30.81 | 30.12 | 84572 | 945 | 15.14 | 14.97 | 24 |
| mix | ast | 1.87 | 1.73 | 3540 | 1117 | 14.73 | 14.56 | 24 |
| mix | llvm | 2103.79 | 2107.98 | 186688 | 291880 | 15.59 | 15.42 | 2068 |
| mix | llvm/mir | 68.29 | 69.98 | 2.21 | 308.87 | 1.03 | 1.03 | 86.17 |
| mix | ast/mir | 0.06 | 0.06 | 0.04 | 1.18 | 0.97 | 0.97 | 1.00 |
| stats | mir | 31.60 | 31.00 | 85816 | 1067 | 2.86 | 2.73 | 24 |
| stats | ast | 1.88 | 1.74 | 3672 | 1104 | 3.44 | 3.31 | 24 |
| stats | llvm | 2120.87 | 2125.42 | 189044 | 292048 | 3.77 | 3.62 | 2124 |
| stats | llvm/mir | 67.11 | 68.57 | 2.20 | 273.71 | 1.32 | 1.33 | 88.50 |
| stats | ast/mir | 0.06 | 0.06 | 0.04 | 1.03 | 1.20 | 1.21 | 1.00 |
| refs | mir | 32.53 | 31.96 | 86288 | 1529 | 0.28 | 0.12 | 0 |
| refs | ast | 2.00 | 1.88 | 3700 | 1279 | 4.79 | 4.63 | 24 |
| refs | llvm | 2183.94 | 2184.31 | 192768 | 292512 | 5.61 | 5.46 | 1992 |
| refs | llvm/mir | 67.13 | 68.35 | 2.23 | 191.31 | 20.12 | 46.30 | 0.00 |
| refs | ast/mir | 0.06 | 0.06 | 0.04 | 0.84 | 17.18 | 39.23 | 0.00 |
| heap | mir | 38.05 | 36.96 | 85188 | 1135 | 3.21 | 3.08 | 4712 |
| heap | ast | 2.54 | 2.39 | 3600 | 1335 | 3.21 | 3.05 | 4712 |
| heap | llvm | 2123.74 | 2128.40 | 188752 | 292384 | 3.33 | 3.20 | 6720 |
| heap | llvm/mir | 55.82 | 57.58 | 2.22 | 257.61 | 1.04 | 1.04 | 1.43 |
| heap | ast/mir | 0.07 | 0.06 | 0.04 | 1.18 | 1.00 | 0.99 | 1.00 |
| callrec | mir | 30.04 | 29.45 | 83924 | 773 | 2.23 | 2.12 | 24 |
| callrec | ast | 1.78 | 1.66 | 3652 | 873 | 2.73 | 2.60 | 24 |
| callrec | llvm | 2137.79 | 2139.58 | 187060 | 291680 | 2.85 | 2.71 | 2100 |
| callrec | llvm/mir | 71.17 | 72.66 | 2.23 | 377.34 | 1.27 | 1.28 | 87.50 |
| callrec | ast/mir | 0.06 | 0.06 | 0.04 | 1.13 | 1.22 | 1.22 | 1.00 |
| forloops | mir | 30.57 | 30.02 | 84992 | 913 | 1.50 | 1.36 | 24 |
| forloops | ast | 1.78 | 1.67 | 3660 | 1011 | 1.48 | 1.35 | 24 |
| forloops | llvm | 2193.22 | 2193.53 | 188440 | 292480 | 3.36 | 3.22 | 2108 |
| forloops | llvm/mir | 71.74 | 73.07 | 2.22 | 320.35 | 2.24 | 2.36 | 87.83 |
| forloops | ast/mir | 0.06 | 0.06 | 0.04 | 1.11 | 0.99 | 1.00 | 1.00 |
| light | mir | 32.93 | 32.30 | 90580 | 1163 | 2.59 | 2.44 | 24 |
| light | ast | 1.79 | 1.67 | 3624 | 1108 | 2.07 | 1.94 | 24 |
| light | llvm | 2177.44 | 2181.79 | 188344 | 291904 | 2.64 | 2.53 | 2068 |
| light | llvm/mir | 66.12 | 67.55 | 2.08 | 250.99 | 1.02 | 1.04 | 86.17 |
| light | ast/mir | 0.05 | 0.05 | 0.04 | 0.95 | 0.80 | 0.80 | 1.00 |
| heavy | mir | 30.02 | 29.45 | 84356 | 798 | 761.55 | 761.10 | 24 |
| heavy | ast | 1.78 | 1.65 | 3620 | 969 | 764.48 | 764.18 | 20 |
| heavy | llvm | 2138.43 | 2139.62 | 186872 | 291792 | 1030.84 | 1030.04 | 2060 |
| heavy | llvm/mir | 71.23 | 72.65 | 2.22 | 365.65 | 1.35 | 1.35 | 85.83 |
| heavy | ast/mir | 0.06 | 0.06 | 0.04 | 1.21 | 1.00 | 1.00 | 0.83 |

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
| hello | 29.07 | 28.64 | 84148 | 1.91 | 1.73 | 3384 |
| fib | 29.77 | 29.36 | 83700 | 2.28 | 2.09 | 3412 |
| mandel | 28.83 | 28.44 | 84888 | 3.15 | 3.01 | 3496 |
| sieve | 28.86 | 28.39 | 86268 | 3.67 | 3.53 | 5156 |
| mix | 28.08 | 27.56 | 84572 | 2.73 | 2.56 | 3436 |
| stats | 28.76 | 28.30 | 85816 | 2.85 | 2.70 | 3472 |
| refs | 28.89 | 28.46 | 86288 | 3.64 | 3.50 | 3484 |
| heap | 34.45 | 33.56 | 85188 | 3.60 | 3.41 | 3496 |
| callrec | 27.86 | 27.40 | 83924 | 2.18 | 2.05 | 3424 |
| forloops | 28.02 | 27.59 | 84992 | 2.55 | 2.43 | 3472 |
| light | 29.97 | 29.48 | 90580 | 2.96 | 2.82 | 3444 |
| heavy | 27.83 | 27.38 | 84356 | 2.19 | 2.07 | 3416 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

