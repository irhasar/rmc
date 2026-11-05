# Benchmark results: rmc vs rustc

Recorded 2026-11-05 01:55 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.19 | 1.07 | 2752 | 478 |
| hello | rustc | 2081.64 | 2084.53 | 188248 | 289416 |
| hello | ratio (rustc/rmc) | 1744.0 | 1946.3 | 68.4 | 605.5 |
| fib | rmc | 1.28 | 1.17 | 2772 | 618 |
| fib | rustc | 2156.54 | 2159.89 | 186928 | 291696 |
| fib | ratio (rustc/rmc) | 1682.1 | 1852.4 | 67.4 | 472.0 |
| mandel | rmc | 1.48 | 1.28 | 2792 | 941 |
| mandel | rustc | 2150.38 | 2152.83 | 189848 | 291776 |
| mandel | ratio (rustc/rmc) | 1456.9 | 1684.5 | 68.0 | 310.1 |
| sieve | rmc | 1.46 | 1.27 | 2816 | 699 |
| sieve | rustc | 2853.50 | 2841.94 | 191484 | 292016 |
| sieve | ratio (rustc/rmc) | 1961.0 | 2244.8 | 68.0 | 417.8 |
| mix | rmc | 1.74 | 1.59 | 2704 | 869 |
| mix | rustc | 2192.20 | 2184.45 | 192212 | 291864 |
| mix | ratio (rustc/rmc) | 1258.7 | 1369.6 | 71.1 | 335.9 |
| stats | rmc | 1.69 | 1.49 | 2804 | 770 |
| stats | rustc | 2204.77 | 2209.22 | 191380 | 292048 |
| stats | ratio (rustc/rmc) | 1305.0 | 1480.7 | 68.3 | 379.3 |
| refs | rmc | 1.95 | 1.82 | 2820 | 901 |
| refs | rustc | 2147.14 | 2147.93 | 192048 | 292240 |
| refs | ratio (rustc/rmc) | 1099.1 | 1178.9 | 68.1 | 324.4 |
| heap | rmc | 1.37 | 1.26 | 2716 | 954 |
| heap | rustc | 2191.40 | 2195.66 | 188912 | 292384 |
| heap | ratio (rustc/rmc) | 1603.7 | 1748.1 | 69.6 | 306.5 |
| callrec | rmc | 1.28 | 1.15 | 2792 | 622 |
| callrec | rustc | 2083.61 | 2088.28 | 187956 | 291680 |
| callrec | ratio (rustc/rmc) | 1629.6 | 1812.7 | 67.3 | 468.9 |
| forloops | rmc | 1.32 | 1.19 | 2760 | 737 |
| forloops | rustc | 2191.36 | 2195.30 | 187700 | 292480 |
| forloops | ratio (rustc/rmc) | 1664.3 | 1837.1 | 68.0 | 396.9 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 618 | 291696 | 4541112 | 1183104 |
| mandel | 941 | 291776 | 4541208 | 1183168 |
| sieve | 699 | 292016 | 4541424 | 1183392 |
| mix | 869 | 291864 | 4541416 | 1183328 |
| stats | 770 | 292048 | 4541392 | 1183360 |
| refs | 901 | 292240 | 4542008 | 1183648 |
| heap | 954 | 292384 | 4542224 | 1183840 |
| callrec | 622 | 291680 | 4541144 | 1183104 |
| forloops | 737 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.23 | 0.12 | 28 |
| hello | rustc | 0.84 | 0.73 | 2076 |
| hello | ratio (rustc/rmc) | 3.61 | 5.83 | 74.14 |
| fib | rmc | 11.75 | 11.58 | 28 |
| fib | rustc | 9.46 | 9.27 | 2016 |
| fib | ratio (rustc/rmc) | 0.80 | 0.80 | 72.00 |
| mandel | rmc | 126.05 | 125.62 | 28 |
| mandel | rustc | 107.39 | 107.15 | 2068 |
| mandel | ratio (rustc/rmc) | 0.85 | 0.85 | 73.86 |
| sieve | rmc | 2.25 | 2.12 | 1588 |
| sieve | rustc | 3.49 | 3.34 | 3648 |
| sieve | ratio (rustc/rmc) | 1.55 | 1.58 | 2.30 |
| mix | rmc | 16.08 | 15.75 | 28 |
| mix | rustc | 15.82 | 15.57 | 2044 |
| mix | ratio (rustc/rmc) | 0.98 | 0.99 | 73.00 |
| stats | rmc | 3.24 | 3.12 | 28 |
| stats | rustc | 3.78 | 3.65 | 2052 |
| stats | ratio (rustc/rmc) | 1.17 | 1.17 | 73.29 |
| refs | rmc | 1.76 | 1.61 | 28 |
| refs | rustc | 3.82 | 3.67 | 2028 |
| refs | ratio (rustc/rmc) | 2.17 | 2.28 | 72.43 |
| heap | rmc | 3.25 | 3.11 | 4716 |
| heap | rustc | 3.48 | 3.35 | 6668 |
| heap | ratio (rustc/rmc) | 1.07 | 1.08 | 1.41 |
| callrec | rmc | 2.93 | 2.79 | 28 |
| callrec | rustc | 2.88 | 2.75 | 2084 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.99 | 74.43 |
| forloops | rmc | 1.46 | 1.34 | 28 |
| forloops | rustc | 3.32 | 3.19 | 2068 |
| forloops | ratio (rustc/rmc) | 2.27 | 2.38 | 73.86 |

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
