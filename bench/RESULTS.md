# Benchmark results: rmc vs rustc

Recorded 2026-11-04 17:02 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.23 | 1.10 | 2756 | 478 |
| hello | rustc | 2046.66 | 2049.55 | 187756 | 289416 |
| hello | ratio (rustc/rmc) | 1667.7 | 1868.3 | 68.1 | 605.5 |
| fib | rmc | 1.35 | 1.17 | 2800 | 609 |
| fib | rustc | 2058.64 | 2060.90 | 188620 | 291696 |
| fib | ratio (rustc/rmc) | 1528.7 | 1755.5 | 67.4 | 479.0 |
| mandel | rmc | 1.38 | 1.27 | 2784 | 922 |
| mandel | rustc | 2097.08 | 2101.71 | 190756 | 291776 |
| mandel | ratio (rustc/rmc) | 1524.2 | 1657.5 | 68.5 | 316.5 |
| sieve | rmc | 1.29 | 1.17 | 2780 | 689 |
| sieve | rustc | 2076.34 | 2081.05 | 188480 | 292016 |
| sieve | ratio (rustc/rmc) | 1614.6 | 1774.1 | 67.8 | 423.8 |
| mix | rmc | 1.33 | 1.21 | 2772 | 845 |
| mix | rustc | 2068.68 | 2072.93 | 188372 | 291864 |
| mix | ratio (rustc/rmc) | 1555.0 | 1713.2 | 68.0 | 345.4 |
| stats | rmc | 1.35 | 1.22 | 2788 | 771 |
| stats | rustc | 2066.64 | 2071.12 | 191288 | 292048 |
| stats | ratio (rustc/rmc) | 1530.2 | 1692.1 | 68.6 | 378.8 |
| refs | rmc | 1.56 | 1.39 | 2832 | 912 |
| refs | rustc | 2095.74 | 2098.82 | 193248 | 292240 |
| refs | ratio (rustc/rmc) | 1344.6 | 1514.3 | 68.2 | 320.4 |
| heap | rmc | 1.36 | 1.25 | 2768 | 930 |
| heap | rustc | 2117.16 | 2121.70 | 192492 | 292384 |
| heap | ratio (rustc/rmc) | 1559.3 | 1704.2 | 69.5 | 314.4 |
| callrec | rmc | 1.28 | 1.16 | 2736 | 613 |
| callrec | rustc | 2079.19 | 2084.30 | 187488 | 291680 |
| callrec | ratio (rustc/rmc) | 1628.7 | 1803.0 | 68.5 | 475.8 |
| forloops | rmc | 1.34 | 1.21 | 2816 | 739 |
| forloops | rustc | 2093.35 | 2093.61 | 188124 | 292480 |
| forloops | ratio (rustc/rmc) | 1562.6 | 1733.1 | 66.8 | 395.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 922 | 291776 | 4541208 | 1183168 |
| sieve | 689 | 292016 | 4541424 | 1183392 |
| mix | 845 | 291864 | 4541416 | 1183328 |
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
| hello | rmc | 0.24 | 0.11 | 24 |
| hello | rustc | 0.82 | 0.71 | 2072 |
| hello | ratio (rustc/rmc) | 3.43 | 6.27 | 86.33 |
| fib | rmc | 7.17 | 7.03 | 24 |
| fib | rustc | 7.10 | 6.96 | 2028 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 84.50 |
| mandel | rmc | 92.28 | 92.09 | 24 |
| mandel | rustc | 105.24 | 105.03 | 2096 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 87.33 |
| sieve | rmc | 1.56 | 1.43 | 1584 |
| sieve | rustc | 2.44 | 2.32 | 3636 |
| sieve | ratio (rustc/rmc) | 1.57 | 1.62 | 2.30 |
| mix | rmc | 16.83 | 16.68 | 24 |
| mix | rustc | 15.43 | 15.27 | 2100 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 87.50 |
| stats | rmc | 3.24 | 3.11 | 24 |
| stats | rustc | 3.75 | 3.62 | 2120 |
| stats | ratio (rustc/rmc) | 1.16 | 1.16 | 88.33 |
| refs | rmc | 3.34 | 3.16 | 24 |
| refs | rustc | 3.83 | 3.71 | 2036 |
| refs | ratio (rustc/rmc) | 1.15 | 1.17 | 84.83 |
| heap | rmc | 3.25 | 3.11 | 4712 |
| heap | rustc | 3.40 | 3.27 | 6704 |
| heap | ratio (rustc/rmc) | 1.05 | 1.05 | 1.42 |
| callrec | rmc | 2.89 | 2.78 | 24 |
| callrec | rustc | 2.81 | 2.70 | 2100 |
| callrec | ratio (rustc/rmc) | 0.97 | 0.97 | 87.50 |
| forloops | rmc | 2.07 | 1.96 | 24 |
| forloops | rustc | 3.32 | 3.20 | 2068 |
| forloops | ratio (rustc/rmc) | 1.60 | 1.63 | 86.17 |

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
