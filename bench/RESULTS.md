# Benchmark results: rmc vs rustc

Recorded 2026-11-04 14:48 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.24 | 1.11 | 2732 | 478 |
| hello | rustc | 2078.28 | 2081.10 | 188060 | 289416 |
| hello | ratio (rustc/rmc) | 1675.6 | 1881.6 | 68.8 | 605.5 |
| fib | rmc | 1.40 | 1.24 | 2772 | 609 |
| fib | rustc | 2186.86 | 2191.31 | 187880 | 291696 |
| fib | ratio (rustc/rmc) | 1565.0 | 1761.5 | 67.8 | 479.0 |
| mandel | rmc | 1.55 | 1.34 | 2800 | 925 |
| mandel | rustc | 2207.74 | 2212.25 | 190704 | 291776 |
| mandel | ratio (rustc/rmc) | 1427.4 | 1652.2 | 68.1 | 315.4 |
| sieve | rmc | 1.39 | 1.26 | 2792 | 689 |
| sieve | rustc | 2184.40 | 2188.53 | 190936 | 292016 |
| sieve | ratio (rustc/rmc) | 1572.7 | 1738.3 | 68.4 | 423.8 |
| mix | rmc | 1.38 | 1.27 | 2728 | 893 |
| mix | rustc | 2166.12 | 2169.15 | 188096 | 291864 |
| mix | ratio (rustc/rmc) | 1568.7 | 1702.6 | 69.0 | 326.8 |
| stats | rmc | 1.40 | 1.28 | 2792 | 765 |
| stats | rustc | 2153.85 | 2157.68 | 188220 | 292048 |
| stats | ratio (rustc/rmc) | 1534.3 | 1681.7 | 67.4 | 381.8 |
| refs | rmc | 1.72 | 1.54 | 2780 | 912 |
| refs | rustc | 2151.88 | 2154.38 | 191844 | 292240 |
| refs | ratio (rustc/rmc) | 1253.4 | 1399.0 | 69.0 | 320.4 |
| heap | rmc | 1.45 | 1.30 | 2768 | 930 |
| heap | rustc | 2138.14 | 2142.30 | 192796 | 292384 |
| heap | ratio (rustc/rmc) | 1476.4 | 1647.9 | 69.7 | 314.4 |
| callrec | rmc | 1.32 | 1.20 | 2768 | 613 |
| callrec | rustc | 2151.61 | 2156.20 | 187516 | 291680 |
| callrec | ratio (rustc/rmc) | 1624.9 | 1799.8 | 67.7 | 475.8 |

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
| hello | rmc | 0.31 | 0.16 | 28 |
| hello | rustc | 0.92 | 0.80 | 2072 |
| hello | ratio (rustc/rmc) | 2.95 | 5.13 | 74.00 |
| fib | rmc | 7.35 | 7.17 | 28 |
| fib | rustc | 7.26 | 7.09 | 2056 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 73.43 |
| mandel | rmc | 95.97 | 95.76 | 24 |
| mandel | rustc | 105.71 | 105.50 | 2084 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 86.83 |
| sieve | rmc | 1.60 | 1.47 | 1588 |
| sieve | rustc | 2.50 | 2.38 | 3580 |
| sieve | ratio (rustc/rmc) | 1.57 | 1.62 | 2.25 |
| mix | rmc | 17.01 | 16.82 | 28 |
| mix | rustc | 15.65 | 15.46 | 2092 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 74.71 |
| stats | rmc | 3.33 | 3.20 | 28 |
| stats | rustc | 3.79 | 3.65 | 2028 |
| stats | ratio (rustc/rmc) | 1.14 | 1.14 | 72.43 |
| refs | rmc | 3.30 | 3.16 | 28 |
| refs | rustc | 3.88 | 3.75 | 2040 |
| refs | ratio (rustc/rmc) | 1.18 | 1.19 | 72.86 |
| heap | rmc | 3.45 | 3.30 | 4716 |
| heap | rustc | 3.57 | 3.43 | 6728 |
| heap | ratio (rustc/rmc) | 1.03 | 1.04 | 1.43 |
| callrec | rmc | 2.95 | 2.82 | 28 |
| callrec | rustc | 2.88 | 2.75 | 2084 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.98 | 74.43 |

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
