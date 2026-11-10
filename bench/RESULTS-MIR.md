# Benchmark results: the rustc-MIR front end + rmc back end

Recorded 2026-11-10 14:46 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | mir | 28.18 | 27.61 | 84296 | 510 | 0.26 | 0.13 | 32 |
| hello | ast | 1.67 | 1.51 | 3396 | 503 | 0.25 | 0.13 | 32 |
| hello | llvm | 2043.13 | 2045.87 | 188380 | 289416 | 0.84 | 0.72 | 2024 |
| hello | llvm/mir | 72.51 | 74.09 | 2.23 | 567.48 | 3.20 | 5.45 | 63.25 |
| hello | ast/mir | 0.06 | 0.05 | 0.04 | 0.99 | 0.96 | 1.00 | 1.00 |
| fib | mir | 28.02 | 27.48 | 83932 | 638 | 5.46 | 5.31 | 32 |
| fib | ast | 1.70 | 1.58 | 3556 | 695 | 10.94 | 10.81 | 32 |
| fib | llvm | 2071.89 | 2074.60 | 191936 | 291696 | 7.15 | 7.03 | 2052 |
| fib | llvm/mir | 73.96 | 75.48 | 2.29 | 457.20 | 1.31 | 1.32 | 64.12 |
| fib | ast/mir | 0.06 | 0.06 | 0.04 | 1.09 | 2.01 | 2.04 | 1.00 |
| mandel | mir | 30.52 | 30.01 | 85112 | 1092 | 90.75 | 90.53 | 32 |
| mandel | ast | 1.84 | 1.71 | 3520 | 1035 | 92.19 | 91.99 | 32 |
| mandel | llvm | 2066.54 | 2070.17 | 191172 | 291776 | 105.43 | 105.20 | 2076 |
| mandel | llvm/mir | 67.71 | 68.98 | 2.25 | 267.19 | 1.16 | 1.16 | 64.88 |
| mandel | ast/mir | 0.06 | 0.06 | 0.04 | 0.95 | 1.02 | 1.02 | 1.00 |
| sieve | mir | 31.62 | 31.04 | 86480 | 850 | 1.77 | 1.64 | 1592 |
| sieve | ast | 1.76 | 1.62 | 3492 | 859 | 1.68 | 1.57 | 1592 |
| sieve | llvm | 2096.51 | 2099.44 | 188212 | 292032 | 2.41 | 2.27 | 3628 |
| sieve | llvm/mir | 66.30 | 67.63 | 2.18 | 343.57 | 1.36 | 1.38 | 2.28 |
| sieve | ast/mir | 0.06 | 0.05 | 0.04 | 1.01 | 0.95 | 0.96 | 1.00 |
| mix | mir | 28.82 | 28.25 | 84812 | 794 | 14.68 | 14.55 | 32 |
| mix | ast | 1.76 | 1.63 | 3468 | 949 | 14.77 | 14.62 | 32 |
| mix | llvm | 2093.58 | 2094.14 | 189356 | 291880 | 15.45 | 15.31 | 2076 |
| mix | llvm/mir | 72.65 | 74.14 | 2.23 | 367.61 | 1.05 | 1.05 | 64.88 |
| mix | ast/mir | 0.06 | 0.06 | 0.04 | 1.20 | 1.01 | 1.01 | 1.00 |
| stats | mir | 29.42 | 28.78 | 85968 | 846 | 3.27 | 3.15 | 32 |
| stats | ast | 1.82 | 1.69 | 3556 | 939 | 3.29 | 3.17 | 32 |
| stats | llvm | 2089.82 | 2093.65 | 188716 | 292048 | 3.70 | 3.57 | 2024 |
| stats | llvm/mir | 71.04 | 72.74 | 2.20 | 345.21 | 1.13 | 1.13 | 63.25 |
| stats | ast/mir | 0.06 | 0.06 | 0.04 | 1.11 | 1.01 | 1.01 | 1.00 |
| refs | mir | 31.46 | 30.91 | 86440 | 1394 | 0.22 | 0.12 | 28 |
| refs | ast | 1.93 | 1.82 | 3572 | 903 | 31.02 | 30.86 | 32 |
| refs | llvm | 2079.91 | 2083.91 | 193612 | 292256 | 3.87 | 3.74 | 2036 |
| refs | llvm/mir | 66.12 | 67.43 | 2.24 | 209.65 | 17.48 | 31.41 | 72.71 |
| refs | ast/mir | 0.06 | 0.06 | 0.04 | 0.65 | 140.21 | 259.33 | 1.14 |
| heap | mir | 29.07 | 28.42 | 85344 | 967 | 3.14 | 3.02 | 4716 |
| heap | ast | 1.83 | 1.70 | 3588 | 1164 | 20.60 | 20.40 | 37536 |
| heap | llvm | 2099.73 | 2103.65 | 189164 | 292400 | 3.46 | 3.32 | 6688 |
| heap | llvm/mir | 72.23 | 74.01 | 2.22 | 302.38 | 1.10 | 1.10 | 1.42 |
| heap | ast/mir | 0.06 | 0.06 | 0.04 | 1.20 | 6.57 | 6.75 | 7.96 |
| callrec | mir | 28.34 | 27.77 | 84204 | 638 | 2.25 | 2.13 | 32 |
| callrec | ast | 1.69 | 1.57 | 3568 | 698 | 5.14 | 5.00 | 32 |
| callrec | llvm | 2088.77 | 2091.16 | 187192 | 291680 | 2.88 | 2.73 | 2008 |
| callrec | llvm/mir | 73.72 | 75.29 | 2.22 | 457.18 | 1.28 | 1.28 | 62.75 |
| callrec | ast/mir | 0.06 | 0.06 | 0.04 | 1.09 | 2.28 | 2.34 | 1.00 |
| forloops | mir | 31.31 | 30.64 | 85156 | 771 | 1.48 | 1.36 | 32 |
| forloops | ast | 1.74 | 1.63 | 3536 | 839 | 2.07 | 1.96 | 28 |
| forloops | llvm | 2098.35 | 2102.60 | 188748 | 292480 | 3.30 | 3.19 | 2040 |
| forloops | llvm/mir | 67.03 | 68.62 | 2.22 | 379.35 | 2.23 | 2.34 | 63.75 |
| forloops | ast/mir | 0.06 | 0.05 | 0.04 | 1.09 | 1.39 | 1.44 | 0.88 |

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
| hello | 26.35 | 25.92 | 84296 | 1.83 | 1.69 | 3328 |
| fib | 25.88 | 25.49 | 83932 | 2.14 | 2.00 | 3420 |
| mandel | 27.46 | 27.07 | 85112 | 3.06 | 2.94 | 3404 |
| sieve | 28.03 | 27.59 | 86480 | 3.59 | 3.45 | 5112 |
| mix | 26.17 | 25.76 | 84812 | 2.65 | 2.48 | 3424 |
| stats | 26.91 | 26.39 | 85968 | 2.51 | 2.39 | 3380 |
| refs | 27.95 | 27.52 | 86440 | 3.51 | 3.38 | 3456 |
| heap | 26.71 | 26.19 | 85344 | 2.36 | 2.23 | 3376 |
| callrec | 26.20 | 25.76 | 84204 | 2.13 | 2.01 | 3388 |
| forloops | 28.85 | 28.31 | 85156 | 2.46 | 2.33 | 3360 |

The front end is the whole rustc driver (~35 ms, ~85 MB): the
parser, type checker and MIR lowering. The back end is rmc's
assembler and linker, an order of magnitude cheaper, and it is what
replaces LLVM's ~2 s. That split is where the compile-time win comes
from - LLVM codegen and optimisation are gone, not merely made faster.

