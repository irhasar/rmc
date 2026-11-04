# Benchmark results: rmc vs rustc

Recorded 2026-11-04 11:56 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 6.18 | 6.09 | 2700 | 1384 |
| hello | rustc | 2085.20 | 2088.42 | 187660 | 289416 |
| hello | ratio (rustc/rmc) | 337.2 | 342.7 | 69.5 | 209.1 |
| fib | rmc | 6.29 | 6.19 | 2644 | 1432 |
| fib | rustc | 2134.45 | 2136.42 | 186660 | 291696 |
| fib | ratio (rustc/rmc) | 339.1 | 345.0 | 70.6 | 203.7 |
| mandel | rmc | 6.56 | 6.46 | 2704 | 1720 |
| mandel | rustc | 2162.97 | 2167.12 | 190712 | 291776 |
| mandel | ratio (rustc/rmc) | 329.6 | 335.6 | 70.5 | 169.6 |
| sieve | rmc | 8.57 | 8.47 | 4384 | 1472 |
| sieve | rustc | 2183.81 | 2187.69 | 190648 | 292016 |
| sieve | ratio (rustc/rmc) | 254.8 | 258.3 | 43.5 | 198.4 |
| mix | rmc | 6.40 | 6.32 | 2708 | 2048 |
| mix | rustc | 2103.12 | 2104.43 | 190136 | 291864 |
| mix | ratio (rustc/rmc) | 328.5 | 332.8 | 70.2 | 142.5 |
| stats | rmc | 6.28 | 6.22 | 2724 | 1560 |
| stats | rustc | 2124.61 | 2128.48 | 187600 | 292048 |
| stats | ratio (rustc/rmc) | 338.1 | 342.0 | 68.9 | 187.2 |
| refs | rmc | 6.76 | 6.69 | 2812 | 2056 |
| refs | rustc | 2171.04 | 2173.98 | 192456 | 292240 |
| refs | ratio (rustc/rmc) | 321.2 | 324.9 | 68.4 | 142.1 |
| heap | rmc | 6.69 | 6.58 | 2736 | 1848 |
| heap | rustc | 2145.80 | 2149.91 | 192836 | 292384 |
| heap | ratio (rustc/rmc) | 320.7 | 327.0 | 70.5 | 158.2 |
| callrec | rmc | 6.35 | 6.24 | 2724 | 1432 |
| callrec | rustc | 2162.88 | 2165.55 | 189544 | 291680 |
| callrec | ratio (rustc/rmc) | 340.8 | 346.9 | 69.6 | 203.7 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 1384 | 289416 | 4537096 | 1180416 |
| fib | 1432 | 291696 | 4541112 | 1183104 |
| mandel | 1720 | 291776 | 4541208 | 1183168 |
| sieve | 1472 | 292016 | 4541424 | 1183392 |
| mix | 2048 | 291864 | 4541416 | 1183328 |
| stats | 1560 | 292048 | 4541392 | 1183360 |
| refs | 2056 | 292240 | 4542008 | 1183648 |
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
| hello | rmc | 0.23 | 0.12 | 20 |
| hello | rustc | 0.85 | 0.73 | 2008 |
| hello | ratio (rustc/rmc) | 3.75 | 5.95 | 100.40 |
| fib | rmc | 7.27 | 7.13 | 24 |
| fib | rustc | 7.31 | 7.15 | 2020 |
| fib | ratio (rustc/rmc) | 1.01 | 1.00 | 84.17 |
| mandel | rmc | 96.92 | 96.68 | 24 |
| mandel | rustc | 105.95 | 105.72 | 2080 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 86.67 |
| sieve | rmc | 1.61 | 1.47 | 1584 |
| sieve | rustc | 2.49 | 2.38 | 3564 |
| sieve | ratio (rustc/rmc) | 1.55 | 1.61 | 2.25 |
| mix | rmc | 14.71 | 14.57 | 24 |
| mix | rustc | 15.46 | 15.29 | 2068 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 86.17 |
| stats | rmc | 3.33 | 3.19 | 20 |
| stats | rustc | 3.75 | 3.62 | 2068 |
| stats | ratio (rustc/rmc) | 1.13 | 1.13 | 103.40 |
| refs | rmc | 1.81 | 1.66 | 24 |
| refs | rustc | 3.92 | 3.78 | 2092 |
| refs | ratio (rustc/rmc) | 2.17 | 2.27 | 87.17 |
| heap | rmc | 3.36 | 3.22 | 4712 |
| heap | rustc | 3.60 | 3.46 | 6696 |
| heap | ratio (rustc/rmc) | 1.07 | 1.07 | 1.42 |
| callrec | rmc | 2.96 | 2.84 | 24 |
| callrec | rustc | 2.90 | 2.76 | 2072 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.97 | 86.33 |

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
