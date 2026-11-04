# Benchmark results: rmc vs rustc

Recorded 2026-11-04 16:37 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.20 | 1.07 | 2796 | 478 |
| hello | rustc | 2060.86 | 2063.22 | 189648 | 289416 |
| hello | ratio (rustc/rmc) | 1721.2 | 1926.4 | 67.8 | 605.5 |
| fib | rmc | 1.27 | 1.15 | 2748 | 609 |
| fib | rustc | 2057.14 | 2061.58 | 187292 | 291696 |
| fib | ratio (rustc/rmc) | 1617.6 | 1786.5 | 68.2 | 479.0 |
| mandel | rmc | 1.41 | 1.29 | 2820 | 922 |
| mandel | rustc | 2063.72 | 2068.58 | 190568 | 291776 |
| mandel | ratio (rustc/rmc) | 1466.5 | 1598.6 | 67.6 | 316.5 |
| sieve | rmc | 1.32 | 1.19 | 2800 | 689 |
| sieve | rustc | 2084.12 | 2087.12 | 186520 | 292016 |
| sieve | ratio (rustc/rmc) | 1581.4 | 1756.8 | 66.6 | 423.8 |
| mix | rmc | 1.32 | 1.22 | 2712 | 861 |
| mix | rustc | 2095.04 | 2097.00 | 192244 | 291864 |
| mix | ratio (rustc/rmc) | 1582.8 | 1724.5 | 70.9 | 339.0 |
| stats | rmc | 1.36 | 1.23 | 2796 | 771 |
| stats | rustc | 2083.55 | 2088.23 | 188152 | 292048 |
| stats | ratio (rustc/rmc) | 1537.3 | 1693.6 | 67.3 | 378.8 |
| refs | rmc | 1.48 | 1.36 | 2876 | 912 |
| refs | rustc | 2116.81 | 2121.92 | 192948 | 292240 |
| refs | ratio (rustc/rmc) | 1428.9 | 1555.7 | 67.1 | 320.4 |
| heap | rmc | 1.33 | 1.20 | 2800 | 930 |
| heap | rustc | 2138.48 | 2141.30 | 188652 | 292384 |
| heap | ratio (rustc/rmc) | 1603.2 | 1777.0 | 67.4 | 314.4 |
| callrec | rmc | 1.64 | 1.49 | 2768 | 613 |
| callrec | rustc | 2160.50 | 2165.46 | 187156 | 291680 |
| callrec | ratio (rustc/rmc) | 1313.8 | 1456.3 | 67.6 | 475.8 |
| forloops | rmc | 1.35 | 1.23 | 2804 | 739 |
| forloops | rustc | 2079.17 | 2084.01 | 189160 | 292480 |
| forloops | ratio (rustc/rmc) | 1543.4 | 1695.7 | 67.5 | 395.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 922 | 291776 | 4541208 | 1183168 |
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
| hello | rmc | 0.22 | 0.12 | 20 |
| hello | rustc | 0.85 | 0.74 | 2100 |
| hello | ratio (rustc/rmc) | 3.90 | 6.36 | 105.00 |
| fib | rmc | 7.17 | 7.04 | 24 |
| fib | rustc | 7.12 | 6.97 | 2080 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 86.67 |
| mandel | rmc | 92.08 | 91.88 | 24 |
| mandel | rustc | 105.35 | 105.12 | 2040 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 85.00 |
| sieve | rmc | 1.61 | 1.45 | 1584 |
| sieve | rustc | 2.51 | 2.36 | 3668 |
| sieve | ratio (rustc/rmc) | 1.56 | 1.62 | 2.32 |
| mix | rmc | 16.88 | 16.70 | 20 |
| mix | rustc | 15.57 | 15.38 | 2072 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 103.60 |
| stats | rmc | 3.25 | 3.13 | 24 |
| stats | rustc | 3.72 | 3.58 | 2076 |
| stats | ratio (rustc/rmc) | 1.14 | 1.14 | 86.50 |
| refs | rmc | 3.21 | 3.08 | 24 |
| refs | rustc | 3.86 | 3.72 | 2060 |
| refs | ratio (rustc/rmc) | 1.20 | 1.20 | 85.83 |
| heap | rmc | 3.22 | 3.10 | 4712 |
| heap | rustc | 3.42 | 3.29 | 6716 |
| heap | ratio (rustc/rmc) | 1.06 | 1.06 | 1.43 |
| callrec | rmc | 2.90 | 2.78 | 24 |
| callrec | rustc | 2.85 | 2.74 | 2084 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.99 | 86.83 |
| forloops | rmc | 2.06 | 1.96 | 24 |
| forloops | rustc | 3.33 | 3.20 | 2008 |
| forloops | ratio (rustc/rmc) | 1.61 | 1.63 | 83.67 |

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
