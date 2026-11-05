# Benchmark results: rmc vs rustc

Recorded 2026-11-05 00:21 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

The rustc column is an aggressive configuration (`-O` with fat LTO,
`codegen-units=1`, `panic=abort`, stripped). That is the right target
for *compile cost* - it is the slowest rustc build - but it is the
*smallest* rustc artifact, so the size ratio here understates the gap
against a default build. See the size table below for both.

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 1.20 | 1.09 | 2772 | 478 |
| hello | rustc | 2109.66 | 2112.26 | 188588 | 289416 |
| hello | ratio (rustc/rmc) | 1764.1 | 1941.4 | 68.0 | 605.5 |
| fib | rmc | 1.44 | 1.26 | 2784 | 611 |
| fib | rustc | 2070.83 | 2074.79 | 189400 | 291696 |
| fib | ratio (rustc/rmc) | 1441.6 | 1650.6 | 68.0 | 477.4 |
| mandel | rmc | 1.39 | 1.27 | 2828 | 918 |
| mandel | rustc | 2114.50 | 2118.65 | 189772 | 291776 |
| mandel | ratio (rustc/rmc) | 1521.3 | 1670.9 | 67.1 | 317.8 |
| sieve | rmc | 1.31 | 1.19 | 2792 | 692 |
| sieve | rustc | 2137.30 | 2141.19 | 186944 | 292016 |
| sieve | ratio (rustc/rmc) | 1631.0 | 1803.9 | 67.0 | 422.0 |
| mix | rmc | 1.35 | 1.24 | 2788 | 853 |
| mix | rustc | 2057.63 | 2061.97 | 190940 | 291864 |
| mix | ratio (rustc/rmc) | 1520.7 | 1658.9 | 68.5 | 342.2 |
| stats | rmc | 1.36 | 1.25 | 2796 | 767 |
| stats | rustc | 2112.84 | 2109.91 | 187476 | 292048 |
| stats | ratio (rustc/rmc) | 1554.3 | 1694.7 | 67.1 | 380.8 |
| refs | rmc | 1.54 | 1.38 | 2812 | 890 |
| refs | rustc | 2144.25 | 2147.20 | 192592 | 292240 |
| refs | ratio (rustc/rmc) | 1388.9 | 1551.4 | 68.5 | 328.4 |
| heap | rmc | 1.35 | 1.23 | 2764 | 932 |
| heap | rustc | 2151.64 | 2156.11 | 187148 | 292384 |
| heap | ratio (rustc/rmc) | 1592.3 | 1750.1 | 67.7 | 313.7 |
| callrec | rmc | 1.30 | 1.19 | 2816 | 615 |
| callrec | rustc | 2125.38 | 2129.64 | 187624 | 291680 |
| callrec | ratio (rustc/rmc) | 1635.4 | 1795.6 | 66.6 | 474.3 |
| forloops | rmc | 1.34 | 1.21 | 2808 | 726 |
| forloops | rustc | 2167.46 | 2171.71 | 188316 | 292480 |
| forloops | ratio (rustc/rmc) | 1616.3 | 1794.8 | 67.1 | 402.9 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 611 | 291696 | 4541112 | 1183104 |
| mandel | 918 | 291776 | 4541208 | 1183168 |
| sieve | 692 | 292016 | 4541424 | 1183392 |
| mix | 853 | 291864 | 4541416 | 1183328 |
| stats | 767 | 292048 | 4541392 | 1183360 |
| refs | 890 | 292240 | 4542008 | 1183648 |
| heap | 932 | 292384 | 4542224 | 1183840 |
| callrec | 615 | 291680 | 4541144 | 1183104 |
| forloops | 726 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.21 | 0.12 | 24 |
| hello | rustc | 0.83 | 0.70 | 2012 |
| hello | ratio (rustc/rmc) | 3.89 | 6.07 | 83.83 |
| fib | rmc | 7.12 | 7.00 | 24 |
| fib | rustc | 7.23 | 7.06 | 2084 |
| fib | ratio (rustc/rmc) | 1.01 | 1.01 | 86.83 |
| mandel | rmc | 92.78 | 92.56 | 24 |
| mandel | rustc | 105.46 | 105.25 | 2028 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 84.50 |
| sieve | rmc | 1.57 | 1.47 | 1584 |
| sieve | rustc | 2.47 | 2.35 | 3612 |
| sieve | ratio (rustc/rmc) | 1.57 | 1.61 | 2.28 |
| mix | rmc | 16.79 | 16.67 | 24 |
| mix | rustc | 15.43 | 15.29 | 2032 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 84.67 |
| stats | rmc | 3.30 | 3.16 | 20 |
| stats | rustc | 3.73 | 3.61 | 2104 |
| stats | ratio (rustc/rmc) | 1.13 | 1.14 | 105.20 |
| refs | rmc | 1.76 | 1.64 | 24 |
| refs | rustc | 3.85 | 3.72 | 2052 |
| refs | ratio (rustc/rmc) | 2.18 | 2.27 | 85.50 |
| heap | rmc | 3.33 | 3.19 | 4712 |
| heap | rustc | 3.50 | 3.37 | 6692 |
| heap | ratio (rustc/rmc) | 1.05 | 1.06 | 1.42 |
| callrec | rmc | 2.87 | 2.75 | 24 |
| callrec | rustc | 2.84 | 2.70 | 2108 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.98 | 87.83 |
| forloops | rmc | 1.46 | 1.34 | 24 |
| forloops | rustc | 3.34 | 3.21 | 2028 |
| forloops | ratio (rustc/rmc) | 2.28 | 2.40 | 84.50 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive: it wins on
`forloops` because the rotated `for` compares the counter against the
bound directly, and on `refs` because LICM reassociates the
loop-invariant part of the accumulation into one hoisted value. On call-heavy code
(`callrec`) rmc now keeps up because the self-recursion pass folds the
right spine into a loop, leaving one call per level - the same
transform rustc applies. That pass is narrow (one `i64` parameter, an
associative `+`/`*`, and a pure left operand), so a call-heavy shape it
does not match would still lose: a stack-based generator with no
register allocator pays the argument setup and register save/restore on
every call that LLVM avoids. The per-program ratio above is the honest
answer; there is no single speedup factor.

Run-time ratios are also noisy and machine-dependent. Programs near
parity (notably `fib` and `callrec`) flip between a small rmc win and a
small rmc loss from run to run and from host to host - a single recorded
ratio for such a program should not be read as a stable property. Treat
the run column as indicative, and re-run on the target machine before
drawing a conclusion about a specific workload.

Peak RSS for a compiled rmc binary is a few kB because the binary
links no libc and is mapped by the kernel directly. A dynamically
linked rustc binary pulls in ld.so and libc, so its RSS floors at a
couple of MB regardless of what the program does. That gap is real but
is mostly the C library's footprint, not the generated code's.
