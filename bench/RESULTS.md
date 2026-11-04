# Benchmark results: rmc vs rustc

Recorded 2026-11-04 12:43 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 6.30 | 6.21 | 2664 | 1384 |
| hello | rustc | 2051.85 | 2054.88 | 186564 | 289416 |
| hello | ratio (rustc/rmc) | 325.6 | 330.7 | 70.0 | 209.1 |
| fib | rmc | 6.43 | 6.34 | 2728 | 1432 |
| fib | rustc | 2115.72 | 2119.43 | 186368 | 291696 |
| fib | ratio (rustc/rmc) | 328.8 | 334.5 | 68.3 | 203.7 |
| mandel | rmc | 7.92 | 7.02 | 2772 | 1720 |
| mandel | rustc | 2151.20 | 2155.12 | 190408 | 291776 |
| mandel | ratio (rustc/rmc) | 271.5 | 306.8 | 68.7 | 169.6 |
| sieve | rmc | 8.60 | 8.52 | 4452 | 1472 |
| sieve | rustc | 2165.38 | 2163.74 | 191856 | 292016 |
| sieve | ratio (rustc/rmc) | 251.8 | 253.9 | 43.1 | 198.4 |
| mix | rmc | 6.55 | 6.44 | 2748 | 2048 |
| mix | rustc | 2117.75 | 2120.89 | 192148 | 291864 |
| mix | ratio (rustc/rmc) | 323.1 | 329.1 | 69.9 | 142.5 |
| stats | rmc | 6.62 | 6.52 | 2728 | 1560 |
| stats | rustc | 2118.13 | 2119.83 | 187572 | 292048 |
| stats | ratio (rustc/rmc) | 319.8 | 325.0 | 68.8 | 187.2 |
| refs | rmc | 6.60 | 6.47 | 2792 | 1704 |
| refs | rustc | 2131.45 | 2135.26 | 193120 | 292240 |
| refs | ratio (rustc/rmc) | 323.2 | 330.1 | 69.2 | 171.5 |
| heap | rmc | 6.69 | 6.55 | 2748 | 1848 |
| heap | rustc | 2255.10 | 2260.02 | 187868 | 292384 |
| heap | ratio (rustc/rmc) | 337.3 | 344.8 | 68.4 | 158.2 |
| callrec | rmc | 6.47 | 6.40 | 2672 | 1432 |
| callrec | rustc | 2150.80 | 2154.89 | 187740 | 291680 |
| callrec | ratio (rustc/rmc) | 332.2 | 336.5 | 70.3 | 203.7 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 1384 | 289416 | 4537096 | 1180416 |
| fib | 1432 | 291696 | 4541112 | 1183104 |
| mandel | 1720 | 291776 | 4541208 | 1183168 |
| sieve | 1472 | 292016 | 4541424 | 1183392 |
| mix | 2048 | 291864 | 4541416 | 1183328 |
| stats | 1560 | 292048 | 4541392 | 1183360 |
| refs | 1704 | 292240 | 4542008 | 1183648 |
| heap | 1848 | 292384 | 4542224 | 1183840 |
| callrec | 1432 | 291680 | 4541144 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.27 | 0.14 | 24 |
| hello | rustc | 0.87 | 0.74 | 2032 |
| hello | ratio (rustc/rmc) | 3.23 | 5.32 | 84.67 |
| fib | rmc | 7.21 | 7.06 | 24 |
| fib | rustc | 7.19 | 7.03 | 2000 |
| fib | ratio (rustc/rmc) | 1.00 | 1.00 | 83.33 |
| mandel | rmc | 100.12 | 99.89 | 20 |
| mandel | rustc | 106.98 | 106.74 | 2096 |
| mandel | ratio (rustc/rmc) | 1.07 | 1.07 | 104.80 |
| sieve | rmc | 1.71 | 1.54 | 1584 |
| sieve | rustc | 2.53 | 2.39 | 3592 |
| sieve | ratio (rustc/rmc) | 1.48 | 1.55 | 2.27 |
| mix | rmc | 14.75 | 14.60 | 24 |
| mix | rustc | 15.67 | 15.49 | 2036 |
| mix | ratio (rustc/rmc) | 1.06 | 1.06 | 84.83 |
| stats | rmc | 3.37 | 3.23 | 24 |
| stats | rustc | 4.04 | 3.82 | 2040 |
| stats | ratio (rustc/rmc) | 1.20 | 1.18 | 85.00 |
| refs | rmc | 1.79 | 1.65 | 24 |
| refs | rustc | 3.87 | 3.74 | 2068 |
| refs | ratio (rustc/rmc) | 2.17 | 2.27 | 86.17 |
| heap | rmc | 3.39 | 3.23 | 4712 |
| heap | rustc | 3.65 | 3.49 | 6688 |
| heap | ratio (rustc/rmc) | 1.08 | 1.08 | 1.42 |
| callrec | rmc | 2.98 | 2.85 | 24 |
| callrec | rustc | 2.92 | 2.78 | 2100 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.97 | 87.50 |

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
