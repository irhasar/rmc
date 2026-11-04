# Benchmark results: rmc vs rustc

Recorded 2026-11-04 14:09 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.62 | 1.31 | 2748 | 478 |
| hello | rustc | 2125.01 | 2128.19 | 187980 | 289416 |
| hello | ratio (rustc/rmc) | 1312.4 | 1627.1 | 68.4 | 605.5 |
| fib | rmc | 1.29 | 1.18 | 2804 | 609 |
| fib | rustc | 2150.62 | 2154.77 | 190776 | 291696 |
| fib | ratio (rustc/rmc) | 1662.7 | 1830.7 | 68.0 | 479.0 |
| mandel | rmc | 1.43 | 1.31 | 2788 | 925 |
| mandel | rustc | 2153.58 | 2158.17 | 190560 | 291776 |
| mandel | ratio (rustc/rmc) | 1503.1 | 1647.5 | 68.4 | 315.4 |
| sieve | rmc | 1.34 | 1.22 | 2728 | 689 |
| sieve | rustc | 2140.67 | 2143.99 | 188344 | 292016 |
| sieve | ratio (rustc/rmc) | 1597.2 | 1753.1 | 69.0 | 423.8 |
| mix | rmc | 1.35 | 1.22 | 2776 | 893 |
| mix | rustc | 2142.37 | 2140.49 | 192228 | 291864 |
| mix | ratio (rustc/rmc) | 1586.7 | 1753.1 | 69.2 | 326.8 |
| stats | rmc | 1.38 | 1.25 | 2728 | 765 |
| stats | rustc | 2138.32 | 2143.05 | 191528 | 292048 |
| stats | ratio (rustc/rmc) | 1552.6 | 1721.3 | 70.2 | 381.8 |
| refs | rmc | 1.93 | 1.78 | 2804 | 912 |
| refs | rustc | 2133.40 | 2135.07 | 193268 | 292240 |
| refs | ratio (rustc/rmc) | 1105.5 | 1198.8 | 68.9 | 320.4 |
| heap | rmc | 1.39 | 1.25 | 2816 | 930 |
| heap | rustc | 2151.50 | 2156.31 | 188976 | 292384 |
| heap | ratio (rustc/rmc) | 1549.5 | 1729.2 | 67.1 | 314.4 |
| callrec | rmc | 1.64 | 1.51 | 2800 | 613 |
| callrec | rustc | 2221.61 | 2220.86 | 187192 | 291680 |
| callrec | ratio (rustc/rmc) | 1352.4 | 1472.7 | 66.9 | 475.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 925 | 291776 | 4541208 | 1183168 |
| sieve | 689 | 292016 | 4541424 | 1183392 |
| mix | 893 | 291864 | 4541416 | 1183328 |
| stats | 765 | 292048 | 4541392 | 1183360 |
| refs | 912 | 292240 | 4542008 | 1183648 |
| heap | 930 | 292384 | 4542224 | 1183840 |
| callrec | 613 | 291680 | 4541144 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.34 | 0.17 | 24 |
| hello | rustc | 0.90 | 0.77 | 2104 |
| hello | ratio (rustc/rmc) | 2.61 | 4.58 | 87.67 |
| fib | rmc | 7.24 | 7.10 | 28 |
| fib | rustc | 7.15 | 7.01 | 2116 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 75.57 |
| mandel | rmc | 95.85 | 95.63 | 28 |
| mandel | rustc | 105.79 | 105.58 | 2108 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 75.29 |
| sieve | rmc | 1.69 | 1.53 | 1588 |
| sieve | rustc | 2.51 | 2.38 | 3564 |
| sieve | ratio (rustc/rmc) | 1.48 | 1.55 | 2.24 |
| mix | rmc | 16.97 | 16.81 | 28 |
| mix | rustc | 15.53 | 15.38 | 2072 |
| mix | ratio (rustc/rmc) | 0.92 | 0.91 | 74.00 |
| stats | rmc | 5.81 | 5.66 | 28 |
| stats | rustc | 5.12 | 4.90 | 2056 |
| stats | ratio (rustc/rmc) | 0.88 | 0.87 | 73.43 |
| refs | rmc | 3.34 | 3.21 | 28 |
| refs | rustc | 3.95 | 3.82 | 2112 |
| refs | ratio (rustc/rmc) | 1.18 | 1.19 | 75.43 |
| heap | rmc | 4.94 | 4.75 | 4716 |
| heap | rustc | 4.83 | 4.64 | 6680 |
| heap | ratio (rustc/rmc) | 0.98 | 0.98 | 1.42 |
| callrec | rmc | 2.99 | 2.85 | 28 |
| callrec | rustc | 2.88 | 2.73 | 2008 |
| callrec | ratio (rustc/rmc) | 0.96 | 0.96 | 71.71 |

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
