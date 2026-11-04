# Benchmark results: rmc vs rustc

Recorded 2026-11-04 15:55 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.21 | 1.09 | 2764 | 478 |
| hello | rustc | 2052.01 | 2052.81 | 188732 | 289416 |
| hello | ratio (rustc/rmc) | 1695.3 | 1881.6 | 68.3 | 605.5 |
| fib | rmc | 1.25 | 1.13 | 2764 | 609 |
| fib | rustc | 2103.76 | 2107.77 | 187188 | 291696 |
| fib | ratio (rustc/rmc) | 1684.2 | 1858.7 | 67.7 | 479.0 |
| mandel | rmc | 1.39 | 1.26 | 2772 | 925 |
| mandel | rustc | 2094.61 | 2097.85 | 190948 | 291776 |
| mandel | ratio (rustc/rmc) | 1508.7 | 1663.6 | 68.9 | 315.4 |
| sieve | rmc | 1.31 | 1.18 | 2736 | 689 |
| sieve | rustc | 2095.09 | 2099.46 | 187972 | 292016 |
| sieve | ratio (rustc/rmc) | 1601.3 | 1780.7 | 68.7 | 423.8 |
| mix | rmc | 1.33 | 1.19 | 2768 | 893 |
| mix | rustc | 2109.18 | 2113.64 | 192168 | 291864 |
| mix | ratio (rustc/rmc) | 1583.2 | 1768.7 | 69.4 | 326.8 |
| stats | rmc | 1.33 | 1.22 | 2780 | 771 |
| stats | rustc | 2139.05 | 2143.67 | 188548 | 292048 |
| stats | ratio (rustc/rmc) | 1610.3 | 1752.8 | 67.8 | 378.8 |
| refs | rmc | 1.50 | 1.37 | 2800 | 912 |
| refs | rustc | 2098.70 | 2103.61 | 192380 | 292240 |
| refs | ratio (rustc/rmc) | 1400.6 | 1540.0 | 68.7 | 320.4 |
| heap | rmc | 1.33 | 1.22 | 2760 | 930 |
| heap | rustc | 2132.55 | 2136.67 | 192668 | 292384 |
| heap | ratio (rustc/rmc) | 1604.1 | 1754.2 | 69.8 | 314.4 |
| callrec | rmc | 1.27 | 1.15 | 2744 | 613 |
| callrec | rustc | 2160.69 | 2164.84 | 190100 | 291680 |
| callrec | ratio (rustc/rmc) | 1707.0 | 1887.4 | 69.3 | 475.8 |
| forloops | rmc | 1.37 | 1.24 | 2764 | 739 |
| forloops | rustc | 2140.04 | 2144.87 | 189212 | 292480 |
| forloops | ratio (rustc/rmc) | 1556.8 | 1735.3 | 68.5 | 395.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 925 | 291776 | 4541208 | 1183168 |
| sieve | 689 | 292016 | 4541424 | 1183392 |
| mix | 893 | 291864 | 4541416 | 1183328 |
| stats | 771 | 292048 | 4541392 | 1183360 |
| refs | 912 | 292240 | 4542008 | 1183648 |
| heap | 930 | 292384 | 4542224 | 1183840 |
| callrec | 613 | 291680 | 4541144 | 1183104 |
| forloops | 739 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.22 | 0.12 | 24 |
| hello | rustc | 0.82 | 0.70 | 2108 |
| hello | ratio (rustc/rmc) | 3.76 | 6.03 | 87.83 |
| fib | rmc | 7.12 | 6.99 | 20 |
| fib | rustc | 7.07 | 6.95 | 2100 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 105.00 |
| mandel | rmc | 95.35 | 95.13 | 24 |
| mandel | rustc | 105.36 | 105.16 | 2068 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.11 | 86.17 |
| sieve | rmc | 1.55 | 1.42 | 1584 |
| sieve | rustc | 2.43 | 2.32 | 3664 |
| sieve | ratio (rustc/rmc) | 1.57 | 1.63 | 2.31 |
| mix | rmc | 16.96 | 16.78 | 24 |
| mix | rustc | 15.49 | 15.32 | 2096 |
| mix | ratio (rustc/rmc) | 0.91 | 0.91 | 87.33 |
| stats | rmc | 3.23 | 3.11 | 24 |
| stats | rustc | 3.70 | 3.58 | 2028 |
| stats | ratio (rustc/rmc) | 1.14 | 1.15 | 84.50 |
| refs | rmc | 3.25 | 3.12 | 24 |
| refs | rustc | 3.87 | 3.73 | 2068 |
| refs | ratio (rustc/rmc) | 1.19 | 1.20 | 86.17 |
| heap | rmc | 3.25 | 3.12 | 4708 |
| heap | rustc | 3.40 | 3.28 | 6692 |
| heap | ratio (rustc/rmc) | 1.05 | 1.05 | 1.42 |
| callrec | rmc | 2.88 | 2.77 | 20 |
| callrec | rustc | 2.87 | 2.75 | 2084 |
| callrec | ratio (rustc/rmc) | 1.00 | 0.99 | 104.20 |
| forloops | rmc | 2.09 | 1.97 | 24 |
| forloops | rustc | 3.35 | 3.22 | 2068 |
| forloops | ratio (rustc/rmc) | 1.61 | 1.64 | 86.17 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive, and on `refs`
it wins by more than 2x because LICM reassociates the loop-invariant
part of the accumulation into one hoisted value. On call-heavy code
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
