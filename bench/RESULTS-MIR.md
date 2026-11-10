# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-10 22:49 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 31.13 | 30.57 | 84204 | 510 | 0.23 | 0.13 | 28 |
| hello | ast | 1.56 | 1.45 | 3476 | 503 | 0.27 | 0.14 | 28 |
| hello | llvm | 2073.95 | 2076.80 | 188168 | 289416 | 0.84 | 0.71 | 2116 |
| hello | llvm/mir | 66.61 | 67.93 | 2.23 | 567.48 | 3.62 | 5.52 | 75.57 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.99 | 1.15 | 1.05 | 1.00 |
| fib | mir | 28.58 | 28.10 | 83908 | 773 | 5.47 | 5.35 | 28 |
| fib | ast | 1.72 | 1.61 | 3556 | 834 | 10.88 | 10.75 | 28 |
| fib | llvm | 2088.76 | 2091.96 | 191384 | 291696 | 7.15 | 7.02 | 2116 |
| fib | llvm/mir | 73.07 | 74.44 | 2.28 | 377.36 | 1.31 | 1.31 | 75.57 |
| fib | ast/mir | 0.06 | 0.06 | 0.04 | 1.08 | 1.99 | 2.01 | 1.00 |
| mandel | mir | 31.64 | 31.04 | 85032 | 1227 | 91.01 | 90.80 | 28 |
| mandel | ast | 1.83 | 1.72 | 3580 | 1183 | 92.04 | 91.82 | 24 |
| mandel | llvm | 2158.58 | 2159.88 | 187864 | 291776 | 105.41 | 105.18 | 2092 |
| mandel | llvm/mir | 68.22 | 69.58 | 2.21 | 237.80 | 1.16 | 1.16 | 74.71 |
| mandel | ast/mir | 0.06 | 0.06 | 0.04 | 0.96 | 1.01 | 1.01 | 0.86 |
| sieve | mir | 30.58 | 30.01 | 86416 | 985 | 1.73 | 1.62 | 1588 |
| sieve | ast | 1.79 | 1.66 | 3528 | 991 | 1.67 | 1.55 | 1588 |
| sieve | llvm | 2088.93 | 2093.18 | 189064 | 292032 | 2.40 | 2.26 | 3644 |
| sieve | llvm/mir | 68.31 | 69.75 | 2.19 | 296.48 | 1.38 | 1.39 | 2.29 |
| sieve | ast/mir | 0.06 | 0.06 | 0.04 | 1.01 | 0.97 | 0.96 | 1.00 |
| mix | mir | 29.64 | 29.02 | 84732 | 929 | 14.70 | 14.55 | 28 |
| mix | ast | 1.81 | 1.68 | 3552 | 1101 | 14.75 | 14.60 | 28 |
| mix | llvm | 2131.56 | 2125.88 | 188312 | 291880 | 15.50 | 15.36 | 2064 |
| mix | llvm/mir | 71.91 | 73.25 | 2.22 | 314.19 | 1.05 | 1.06 | 73.71 |
| mix | ast/mir | 0.06 | 0.06 | 0.04 | 1.19 | 1.00 | 1.00 | 1.00 |
| stats | mir | 30.92 | 30.33 | 85904 | 981 | 3.29 | 3.18 | 28 |
| stats | ast | 1.98 | 1.83 | 3564 | 1071 | 3.31 | 3.19 | 28 |
| stats | llvm | 2129.94 | 2134.49 | 187792 | 292048 | 3.75 | 3.62 | 2076 |
| stats | llvm/mir | 68.88 | 70.38 | 2.19 | 297.70 | 1.14 | 1.14 | 74.14 |
| stats | ast/mir | 0.06 | 0.06 | 0.04 | 1.09 | 1.00 | 1.00 | 1.00 |
| refs | mir | 33.22 | 32.58 | 86336 | 1529 | 0.24 | 0.12 | 28 |
| refs | ast | 1.94 | 1.82 | 3604 | 1051 | 30.88 | 30.74 | 28 |
| refs | llvm | 2102.16 | 2105.43 | 192416 | 292256 | 3.95 | 3.80 | 2032 |
| refs | llvm/mir | 63.27 | 64.62 | 2.23 | 191.14 | 16.68 | 30.65 | 72.57 |
| refs | ast/mir | 0.06 | 0.06 | 0.04 | 0.69 | 130.30 | 247.88 | 1.00 |
| heap | mir | 30.81 | 30.16 | 85292 | 1102 | 3.34 | 3.19 | 4716 |
| heap | ast | 1.89 | 1.76 | 3584 | 1315 | 21.82 | 21.68 | 37528 |
| heap | llvm | 2108.98 | 2112.50 | 188144 | 292400 | 3.72 | 3.54 | 6700 |
| heap | llvm/mir | 68.46 | 70.05 | 2.21 | 265.34 | 1.12 | 1.11 | 1.42 |
| heap | ast/mir | 0.06 | 0.06 | 0.04 | 1.19 | 6.54 | 6.79 | 7.96 |
| callrec | mir | 28.91 | 28.29 | 84132 | 773 | 2.27 | 2.14 | 24 |
| callrec | ast | 1.74 | 1.57 | 3540 | 837 | 5.10 | 4.97 | 28 |
| callrec | llvm | 2116.98 | 2121.17 | 189776 | 291680 | 2.81 | 2.69 | 2044 |
| callrec | llvm/mir | 73.23 | 74.98 | 2.26 | 377.34 | 1.24 | 1.26 | 85.17 |
| callrec | ast/mir | 0.06 | 0.06 | 0.04 | 1.08 | 2.25 | 2.33 | 1.17 |
| forloops | mir | 30.05 | 29.45 | 85116 | 906 | 1.46 | 1.35 | 28 |
| forloops | ast | 1.82 | 1.67 | 3476 | 987 | 1.47 | 1.34 | 24 |
| forloops | llvm | 2092.51 | 2096.82 | 188444 | 292480 | 3.32 | 3.18 | 2068 |
| forloops | llvm/mir | 69.62 | 71.20 | 2.21 | 322.83 | 2.28 | 2.36 | 73.86 |
| forloops | ast/mir | 0.06 | 0.06 | 0.04 | 1.09 | 1.01 | 1.00 | 0.86 |

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
| hello | 29.36 | 28.93 | 84204 | 1.77 | 1.64 | 3308 |
| fib | 26.46 | 26.09 | 83908 | 2.13 | 2.01 | 3288 |
| mandel | 28.62 | 28.15 | 85032 | 3.03 | 2.89 | 3292 |
| sieve | 26.99 | 26.54 | 86416 | 3.59 | 3.47 | 5036 |
| mix | 27.07 | 26.58 | 84732 | 2.57 | 2.45 | 3308 |
| stats | 28.40 | 27.92 | 85904 | 2.53 | 2.41 | 3348 |
| refs | 29.69 | 29.21 | 86336 | 3.53 | 3.37 | 3332 |
| heap | 28.41 | 27.89 | 85292 | 2.39 | 2.27 | 3276 |
| callrec | 26.76 | 26.28 | 84132 | 2.15 | 2.01 | 3312 |
| forloops | 27.60 | 27.15 | 85116 | 2.45 | 2.30 | 3312 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

