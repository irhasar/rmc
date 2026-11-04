# Benchmark results: rmc vs rustc

Recorded 2026-11-04 12:54 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 6.62 | 6.55 | 2672 | 888 |
| hello | rustc | 2150.04 | 2150.08 | 190088 | 289416 |
| hello | ratio (rustc/rmc) | 324.8 | 328.3 | 71.1 | 325.9 |
| fib | rmc | 6.47 | 6.40 | 2708 | 912 |
| fib | rustc | 2163.76 | 2167.20 | 189900 | 291696 |
| fib | ratio (rustc/rmc) | 334.5 | 338.7 | 70.1 | 319.8 |
| mandel | rmc | 7.04 | 6.93 | 2664 | 1224 |
| mandel | rustc | 2223.36 | 2226.05 | 189968 | 291776 |
| mandel | ratio (rustc/rmc) | 315.8 | 321.1 | 71.3 | 238.4 |
| sieve | rmc | 8.74 | 8.67 | 4432 | 976 |
| sieve | rustc | 2189.60 | 2193.35 | 186940 | 292016 |
| sieve | ratio (rustc/rmc) | 250.5 | 252.9 | 42.2 | 299.2 |
| mix | rmc | 6.59 | 6.50 | 2740 | 1240 |
| mix | rustc | 2213.93 | 2218.41 | 192256 | 291864 |
| mix | ratio (rustc/rmc) | 335.9 | 341.1 | 70.2 | 235.4 |
| stats | rmc | 6.70 | 6.58 | 2724 | 1064 |
| stats | rustc | 2221.11 | 2225.25 | 191648 | 292048 |
| stats | ratio (rustc/rmc) | 331.6 | 338.4 | 70.4 | 274.5 |
| refs | rmc | 7.14 | 7.00 | 2740 | 1216 |
| refs | rustc | 2197.39 | 2202.07 | 192852 | 292240 |
| refs | ratio (rustc/rmc) | 307.7 | 314.6 | 70.4 | 240.3 |
| heap | rmc | 6.88 | 6.79 | 2744 | 1216 |
| heap | rustc | 2227.65 | 2229.43 | 192564 | 292384 |
| heap | ratio (rustc/rmc) | 323.9 | 328.5 | 70.2 | 240.4 |
| callrec | rmc | 6.73 | 6.61 | 2756 | 912 |
| callrec | rustc | 2218.68 | 2222.33 | 187656 | 291680 |
| callrec | ratio (rustc/rmc) | 329.5 | 336.3 | 68.1 | 319.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 888 | 289416 | 4537096 | 1180416 |
| fib | 912 | 291696 | 4541112 | 1183104 |
| mandel | 1224 | 291776 | 4541208 | 1183168 |
| sieve | 976 | 292016 | 4541424 | 1183392 |
| mix | 1240 | 291864 | 4541416 | 1183328 |
| stats | 1064 | 292048 | 4541392 | 1183360 |
| refs | 1216 | 292240 | 4542008 | 1183648 |
| heap | 1216 | 292384 | 4542224 | 1183840 |
| callrec | 912 | 291680 | 4541144 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.27 | 0.14 | 24 |
| hello | rustc | 0.88 | 0.74 | 2056 |
| hello | ratio (rustc/rmc) | 3.23 | 5.14 | 85.67 |
| fib | rmc | 7.32 | 7.16 | 24 |
| fib | rustc | 7.22 | 7.05 | 2016 |
| fib | ratio (rustc/rmc) | 0.99 | 0.98 | 84.00 |
| mandel | rmc | 96.45 | 96.19 | 24 |
| mandel | rustc | 105.84 | 105.60 | 2068 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 86.17 |
| sieve | rmc | 1.61 | 1.49 | 1584 |
| sieve | rustc | 2.56 | 2.42 | 3608 |
| sieve | ratio (rustc/rmc) | 1.59 | 1.62 | 2.28 |
| mix | rmc | 14.78 | 14.60 | 24 |
| mix | rustc | 15.58 | 15.39 | 2080 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 86.67 |
| stats | rmc | 3.43 | 3.29 | 24 |
| stats | rustc | 3.82 | 3.69 | 2080 |
| stats | ratio (rustc/rmc) | 1.11 | 1.12 | 86.67 |
| refs | rmc | 1.79 | 1.66 | 24 |
| refs | rustc | 3.95 | 3.81 | 2056 |
| refs | ratio (rustc/rmc) | 2.21 | 2.29 | 85.67 |
| heap | rmc | 3.42 | 3.27 | 4712 |
| heap | rustc | 3.87 | 3.69 | 6708 |
| heap | ratio (rustc/rmc) | 1.13 | 1.13 | 1.42 |
| callrec | rmc | 2.93 | 2.78 | 24 |
| callrec | rustc | 2.89 | 2.76 | 2028 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.99 | 84.50 |

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
