# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-11 01:02 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 31.19 | 30.58 | 84304 | 510 | 0.26 | 0.13 | 32 |
| hello | ast | 1.64 | 1.51 | 3392 | 503 | 0.25 | 0.13 | 32 |
| hello | llvm | 2074.80 | 2077.21 | 187468 | 289416 | 0.84 | 0.72 | 2072 |
| hello | llvm/mir | 66.52 | 67.93 | 2.22 | 567.48 | 3.30 | 5.54 | 64.75 |
| hello | ast/mir | 0.05 | 0.05 | 0.04 | 0.99 | 0.96 | 1.03 | 1.00 |
| fib | mir | 30.11 | 29.50 | 83920 | 773 | 5.48 | 5.34 | 32 |
| fib | ast | 1.74 | 1.60 | 3496 | 858 | 10.92 | 10.79 | 28 |
| fib | llvm | 2130.39 | 2134.26 | 189132 | 291696 | 7.13 | 6.99 | 2080 |
| fib | llvm/mir | 70.75 | 72.34 | 2.25 | 377.36 | 1.30 | 1.31 | 65.00 |
| fib | ast/mir | 0.06 | 0.05 | 0.04 | 1.11 | 1.99 | 2.02 | 0.88 |
| mandel | mir | 32.20 | 31.61 | 85080 | 1227 | 91.02 | 90.80 | 32 |
| mandel | ast | 1.87 | 1.73 | 3504 | 1200 | 92.18 | 91.96 | 32 |
| mandel | llvm | 2088.02 | 2091.83 | 189976 | 291776 | 105.43 | 105.19 | 2120 |
| mandel | llvm/mir | 64.85 | 66.17 | 2.23 | 237.80 | 1.16 | 1.16 | 66.25 |
| mandel | ast/mir | 0.06 | 0.05 | 0.04 | 0.98 | 1.01 | 1.01 | 1.00 |
| sieve | mir | 30.93 | 30.34 | 86372 | 985 | 1.77 | 1.66 | 1592 |
| sieve | ast | 1.79 | 1.67 | 3612 | 1024 | 1.69 | 1.58 | 1592 |
| sieve | llvm | 2096.48 | 2100.88 | 186344 | 292032 | 2.45 | 2.33 | 3660 |
| sieve | llvm/mir | 67.78 | 69.24 | 2.16 | 296.48 | 1.39 | 1.41 | 2.30 |
| sieve | ast/mir | 0.06 | 0.05 | 0.04 | 1.04 | 0.95 | 0.95 | 1.00 |
| mix | mir | 31.05 | 30.40 | 84680 | 929 | 14.68 | 14.55 | 32 |
| mix | ast | 1.84 | 1.71 | 3584 | 1117 | 14.78 | 14.65 | 32 |
| mix | llvm | 2107.82 | 2112.05 | 187780 | 291880 | 15.58 | 15.42 | 2056 |
| mix | llvm/mir | 67.89 | 69.48 | 2.22 | 314.19 | 1.06 | 1.06 | 64.25 |
| mix | ast/mir | 0.06 | 0.06 | 0.04 | 1.20 | 1.01 | 1.01 | 1.00 |
| stats | mir | 31.26 | 30.61 | 85928 | 981 | 3.31 | 3.19 | 28 |
| stats | ast | 1.85 | 1.73 | 3476 | 1104 | 3.33 | 3.19 | 32 |
| stats | llvm | 2129.23 | 2131.97 | 187636 | 292048 | 3.76 | 3.62 | 2052 |
| stats | llvm/mir | 68.10 | 69.66 | 2.18 | 297.70 | 1.13 | 1.14 | 73.29 |
| stats | ast/mir | 0.06 | 0.06 | 0.04 | 1.13 | 1.01 | 1.00 | 1.14 |
| refs | mir | 31.41 | 30.82 | 86400 | 1529 | 0.24 | 0.12 | 28 |
| refs | ast | 1.95 | 1.81 | 3512 | 1068 | 30.80 | 30.66 | 32 |
| refs | llvm | 2134.17 | 2138.16 | 191884 | 292256 | 3.95 | 3.80 | 2112 |
| refs | llvm/mir | 67.94 | 69.37 | 2.22 | 191.14 | 16.23 | 32.52 | 75.43 |
| refs | ast/mir | 0.06 | 0.06 | 0.04 | 0.70 | 126.43 | 262.04 | 1.14 |
| heap | mir | 31.76 | 31.16 | 85376 | 1102 | 3.37 | 3.23 | 4720 |
| heap | ast | 1.93 | 1.80 | 3560 | 1332 | 21.23 | 21.06 | 37536 |
| heap | llvm | 2139.40 | 2143.91 | 189664 | 292400 | 3.84 | 3.66 | 6684 |
| heap | llvm/mir | 67.36 | 68.81 | 2.22 | 265.34 | 1.14 | 1.13 | 1.42 |
| heap | ast/mir | 0.06 | 0.06 | 0.04 | 1.21 | 6.30 | 6.51 | 7.95 |
| callrec | mir | 29.92 | 29.26 | 84096 | 773 | 2.25 | 2.13 | 32 |
| callrec | ast | 1.73 | 1.60 | 3512 | 861 | 5.16 | 5.01 | 32 |
| callrec | llvm | 2127.05 | 2131.37 | 189940 | 291680 | 2.87 | 2.74 | 2016 |
| callrec | llvm/mir | 71.09 | 72.83 | 2.26 | 377.34 | 1.28 | 1.29 | 63.00 |
| callrec | ast/mir | 0.06 | 0.05 | 0.04 | 1.11 | 2.29 | 2.35 | 1.00 |
| forloops | mir | 30.89 | 30.31 | 85124 | 906 | 1.48 | 1.35 | 28 |
| forloops | ast | 1.78 | 1.66 | 3556 | 1004 | 1.48 | 1.35 | 32 |
| forloops | llvm | 2139.17 | 2142.98 | 187840 | 292480 | 3.32 | 3.21 | 2116 |
| forloops | llvm/mir | 69.25 | 70.71 | 2.21 | 322.83 | 2.25 | 2.37 | 75.57 |
| forloops | ast/mir | 0.06 | 0.05 | 0.04 | 1.11 | 1.00 | 1.00 | 1.14 |

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
| hello | 29.32 | 28.85 | 84304 | 1.87 | 1.73 | 3392 |
| fib | 27.95 | 27.47 | 83920 | 2.16 | 2.04 | 3408 |
| mandel | 29.09 | 28.63 | 85080 | 3.11 | 2.98 | 3432 |
| sieve | 27.29 | 26.85 | 86372 | 3.64 | 3.49 | 5104 |
| mix | 28.43 | 27.92 | 84680 | 2.62 | 2.48 | 3388 |
| stats | 28.65 | 28.13 | 85928 | 2.61 | 2.48 | 3448 |
| refs | 27.85 | 27.40 | 86400 | 3.56 | 3.43 | 3464 |
| heap | 29.33 | 28.88 | 85376 | 2.43 | 2.28 | 3404 |
| callrec | 27.73 | 27.21 | 84096 | 2.19 | 2.05 | 3360 |
| forloops | 28.39 | 27.95 | 85124 | 2.50 | 2.36 | 3436 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

