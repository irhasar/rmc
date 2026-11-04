# Benchmark results: rmc vs rustc

Recorded 2026-11-04 16:18 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.21 | 1.07 | 2780 | 478 |
| hello | rustc | 2076.55 | 2078.62 | 187356 | 289416 |
| hello | ratio (rustc/rmc) | 1719.5 | 1939.0 | 67.4 | 605.5 |
| fib | rmc | 1.27 | 1.15 | 2788 | 609 |
| fib | rustc | 2082.52 | 2083.99 | 186952 | 291696 |
| fib | ratio (rustc/rmc) | 1644.3 | 1807.4 | 67.1 | 479.0 |
| mandel | rmc | 1.39 | 1.28 | 2776 | 925 |
| mandel | rustc | 2076.13 | 2079.88 | 190312 | 291776 |
| mandel | ratio (rustc/rmc) | 1488.7 | 1619.8 | 68.6 | 315.4 |
| sieve | rmc | 1.30 | 1.19 | 2832 | 689 |
| sieve | rustc | 2110.98 | 2114.46 | 187520 | 292016 |
| sieve | ratio (rustc/rmc) | 1625.5 | 1770.9 | 66.2 | 423.8 |
| mix | rmc | 1.33 | 1.20 | 2788 | 861 |
| mix | rustc | 2080.79 | 2083.29 | 187008 | 291864 |
| mix | ratio (rustc/rmc) | 1569.5 | 1733.2 | 67.1 | 339.0 |
| stats | rmc | 1.77 | 1.61 | 2832 | 771 |
| stats | rustc | 2096.96 | 2097.96 | 191260 | 292048 |
| stats | ratio (rustc/rmc) | 1185.5 | 1303.9 | 67.5 | 378.8 |
| refs | rmc | 1.48 | 1.37 | 2840 | 912 |
| refs | rustc | 2107.52 | 2112.34 | 193388 | 292240 |
| refs | ratio (rustc/rmc) | 1421.8 | 1541.9 | 68.1 | 320.4 |
| heap | rmc | 1.35 | 1.22 | 2804 | 930 |
| heap | rustc | 2122.30 | 2126.80 | 190276 | 292384 |
| heap | ratio (rustc/rmc) | 1575.5 | 1749.0 | 67.9 | 314.4 |
| callrec | rmc | 1.28 | 1.17 | 2772 | 613 |
| callrec | rustc | 2078.89 | 2083.10 | 187428 | 291680 |
| callrec | ratio (rustc/rmc) | 1624.9 | 1777.4 | 67.6 | 475.8 |
| forloops | rmc | 1.33 | 1.21 | 2808 | 739 |
| forloops | rustc | 2068.09 | 2072.77 | 187836 | 292480 |
| forloops | ratio (rustc/rmc) | 1552.3 | 1707.4 | 66.9 | 395.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 925 | 291776 | 4541208 | 1183168 |
| sieve | 689 | 292016 | 4541424 | 1183392 |
| mix | 861 | 291864 | 4541416 | 1183328 |
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
| hello | rmc | 0.29 | 0.17 | 20 |
| hello | rustc | 0.85 | 0.72 | 2048 |
| hello | ratio (rustc/rmc) | 2.91 | 4.29 | 102.40 |
| fib | rmc | 7.15 | 6.99 | 24 |
| fib | rustc | 7.07 | 6.96 | 2108 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 87.83 |
| mandel | rmc | 95.59 | 95.36 | 24 |
| mandel | rustc | 105.63 | 105.35 | 2096 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 87.33 |
| sieve | rmc | 1.54 | 1.42 | 1584 |
| sieve | rustc | 2.46 | 2.34 | 3616 |
| sieve | ratio (rustc/rmc) | 1.60 | 1.64 | 2.28 |
| mix | rmc | 16.81 | 16.66 | 24 |
| mix | rustc | 15.47 | 15.31 | 2060 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 85.83 |
| stats | rmc | 3.26 | 3.12 | 24 |
| stats | rustc | 3.74 | 3.62 | 2036 |
| stats | ratio (rustc/rmc) | 1.15 | 1.16 | 84.83 |
| refs | rmc | 3.24 | 3.12 | 24 |
| refs | rustc | 3.86 | 3.71 | 2072 |
| refs | ratio (rustc/rmc) | 1.19 | 1.19 | 86.33 |
| heap | rmc | 3.22 | 3.10 | 4712 |
| heap | rustc | 3.42 | 3.29 | 6708 |
| heap | ratio (rustc/rmc) | 1.06 | 1.06 | 1.42 |
| callrec | rmc | 2.89 | 2.77 | 24 |
| callrec | rustc | 2.87 | 2.72 | 2036 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.98 | 84.83 |
| forloops | rmc | 2.06 | 1.94 | 24 |
| forloops | rustc | 3.33 | 3.20 | 2064 |
| forloops | ratio (rustc/rmc) | 1.61 | 1.65 | 86.00 |

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
