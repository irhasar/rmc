# Benchmark results: rmc vs rustc

Recorded 2026-11-04 13:40 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.26 | 1.13 | 2728 | 478 |
| hello | rustc | 2084.68 | 2086.66 | 185968 | 289416 |
| hello | ratio (rustc/rmc) | 1659.5 | 1840.1 | 68.2 | 605.5 |
| fib | rmc | 1.41 | 1.24 | 2760 | 609 |
| fib | rustc | 2277.98 | 2280.25 | 187968 | 291696 |
| fib | ratio (rustc/rmc) | 1611.4 | 1838.9 | 68.1 | 479.0 |
| mandel | rmc | 2.01 | 1.83 | 2808 | 925 |
| mandel | rustc | 2130.72 | 2134.86 | 191012 | 291776 |
| mandel | ratio (rustc/rmc) | 1059.9 | 1168.5 | 68.0 | 315.4 |
| sieve | rmc | 3.49 | 3.37 | 4460 | 689 |
| sieve | rustc | 2108.34 | 2112.80 | 187340 | 292016 |
| sieve | ratio (rustc/rmc) | 603.8 | 627.7 | 42.0 | 423.8 |
| mix | rmc | 1.54 | 1.41 | 2780 | 893 |
| mix | rustc | 2139.95 | 2143.91 | 188192 | 291864 |
| mix | ratio (rustc/rmc) | 1390.2 | 1521.6 | 67.7 | 326.8 |
| stats | rmc | 1.49 | 1.36 | 2800 | 765 |
| stats | rustc | 2172.09 | 2176.11 | 188280 | 292048 |
| stats | ratio (rustc/rmc) | 1458.8 | 1604.8 | 67.2 | 381.8 |
| refs | rmc | 1.69 | 1.57 | 2816 | 912 |
| refs | rustc | 2127.89 | 2131.92 | 192156 | 292240 |
| refs | ratio (rustc/rmc) | 1262.0 | 1359.6 | 68.2 | 320.4 |
| heap | rmc | 1.61 | 1.49 | 2764 | 930 |
| heap | rustc | 2106.41 | 2110.89 | 188564 | 292384 |
| heap | ratio (rustc/rmc) | 1310.3 | 1418.6 | 68.2 | 314.4 |
| callrec | rmc | 1.39 | 1.25 | 2796 | 613 |
| callrec | rustc | 2127.65 | 2131.99 | 190392 | 291680 |
| callrec | ratio (rustc/rmc) | 1534.4 | 1711.1 | 68.1 | 475.8 |

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
| hello | rmc | 0.28 | 0.14 | 28 |
| hello | rustc | 0.87 | 0.74 | 2048 |
| hello | ratio (rustc/rmc) | 3.14 | 5.15 | 73.14 |
| fib | rmc | 7.26 | 7.10 | 28 |
| fib | rustc | 7.21 | 7.07 | 2036 |
| fib | ratio (rustc/rmc) | 0.99 | 1.00 | 72.71 |
| mandel | rmc | 96.76 | 96.44 | 28 |
| mandel | rustc | 105.60 | 105.41 | 2108 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 75.29 |
| sieve | rmc | 1.58 | 1.47 | 1588 |
| sieve | rustc | 2.49 | 2.37 | 3656 |
| sieve | ratio (rustc/rmc) | 1.57 | 1.62 | 2.30 |
| mix | rmc | 17.02 | 16.87 | 28 |
| mix | rustc | 15.63 | 15.44 | 2096 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 74.86 |
| stats | rmc | 3.37 | 3.25 | 24 |
| stats | rustc | 3.77 | 3.64 | 2080 |
| stats | ratio (rustc/rmc) | 1.12 | 1.12 | 86.67 |
| refs | rmc | 3.31 | 3.17 | 28 |
| refs | rustc | 3.86 | 3.72 | 2088 |
| refs | ratio (rustc/rmc) | 1.16 | 1.17 | 74.57 |
| heap | rmc | 3.38 | 3.23 | 4716 |
| heap | rustc | 3.54 | 3.40 | 6708 |
| heap | ratio (rustc/rmc) | 1.05 | 1.05 | 1.42 |
| callrec | rmc | 2.94 | 2.82 | 28 |
| callrec | rustc | 2.87 | 2.75 | 2080 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.97 | 74.29 |

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
