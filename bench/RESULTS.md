# Benchmark results: rmc vs rustc

Recorded 2026-11-04 17:59 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.45 | 1.27 | 2776 | 478 |
| hello | rustc | 2072.38 | 2073.71 | 185944 | 289416 |
| hello | ratio (rustc/rmc) | 1430.7 | 1627.7 | 67.0 | 605.5 |
| fib | rmc | 1.31 | 1.19 | 2804 | 609 |
| fib | rustc | 2117.21 | 2121.35 | 187216 | 291696 |
| fib | ratio (rustc/rmc) | 1617.8 | 1784.1 | 66.8 | 479.0 |
| mandel | rmc | 1.39 | 1.28 | 2792 | 911 |
| mandel | rustc | 2069.21 | 2073.83 | 190144 | 291776 |
| mandel | ratio (rustc/rmc) | 1487.2 | 1624.0 | 68.1 | 320.3 |
| sieve | rmc | 1.30 | 1.18 | 2816 | 689 |
| sieve | rustc | 2095.32 | 2099.72 | 191824 | 292016 |
| sieve | ratio (rustc/rmc) | 1611.3 | 1779.4 | 68.1 | 423.8 |
| mix | rmc | 1.35 | 1.23 | 2740 | 845 |
| mix | rustc | 2101.67 | 2106.03 | 187860 | 291864 |
| mix | ratio (rustc/rmc) | 1554.8 | 1716.4 | 68.6 | 345.4 |
| stats | rmc | 1.36 | 1.23 | 2748 | 765 |
| stats | rustc | 2111.98 | 2116.24 | 188108 | 292048 |
| stats | ratio (rustc/rmc) | 1556.4 | 1726.1 | 68.5 | 381.8 |
| refs | rmc | 1.48 | 1.36 | 2796 | 888 |
| refs | rustc | 2161.62 | 2166.47 | 192624 | 292240 |
| refs | ratio (rustc/rmc) | 1462.9 | 1588.3 | 68.9 | 329.1 |
| heap | rmc | 1.40 | 1.26 | 2788 | 930 |
| heap | rustc | 2170.66 | 2175.56 | 192992 | 292384 |
| heap | ratio (rustc/rmc) | 1549.3 | 1728.0 | 69.2 | 314.4 |
| callrec | rmc | 1.27 | 1.13 | 2752 | 613 |
| callrec | rustc | 2107.95 | 2112.31 | 186596 | 291680 |
| callrec | ratio (rustc/rmc) | 1658.4 | 1867.6 | 67.8 | 475.8 |
| forloops | rmc | 1.33 | 1.21 | 2776 | 733 |
| forloops | rustc | 2148.18 | 2148.51 | 188892 | 292480 |
| forloops | ratio (rustc/rmc) | 1620.5 | 1781.5 | 68.0 | 399.0 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 911 | 291776 | 4541208 | 1183168 |
| sieve | 689 | 292016 | 4541424 | 1183392 |
| mix | 845 | 291864 | 4541416 | 1183328 |
| stats | 765 | 292048 | 4541392 | 1183360 |
| refs | 888 | 292240 | 4542008 | 1183648 |
| heap | 930 | 292384 | 4542224 | 1183840 |
| callrec | 613 | 291680 | 4541144 | 1183104 |
| forloops | 733 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.25 | 0.13 | 24 |
| hello | rustc | 0.83 | 0.71 | 2080 |
| hello | ratio (rustc/rmc) | 3.29 | 5.63 | 86.67 |
| fib | rmc | 8.98 | 8.72 | 24 |
| fib | rustc | 7.15 | 7.00 | 2068 |
| fib | ratio (rustc/rmc) | 0.80 | 0.80 | 86.17 |
| mandel | rmc | 92.30 | 92.11 | 24 |
| mandel | rustc | 105.32 | 105.12 | 2092 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 87.17 |
| sieve | rmc | 1.51 | 1.41 | 1584 |
| sieve | rustc | 2.44 | 2.31 | 3604 |
| sieve | ratio (rustc/rmc) | 1.61 | 1.64 | 2.28 |
| mix | rmc | 16.83 | 16.66 | 24 |
| mix | rustc | 15.47 | 15.31 | 2064 |
| mix | ratio (rustc/rmc) | 0.92 | 0.92 | 86.00 |
| stats | rmc | 3.34 | 3.22 | 24 |
| stats | rustc | 3.74 | 3.62 | 2060 |
| stats | ratio (rustc/rmc) | 1.12 | 1.12 | 85.83 |
| refs | rmc | 1.76 | 1.62 | 24 |
| refs | rustc | 3.85 | 3.73 | 2028 |
| refs | ratio (rustc/rmc) | 2.19 | 2.30 | 84.50 |
| heap | rmc | 3.31 | 3.17 | 4712 |
| heap | rustc | 3.51 | 3.38 | 6692 |
| heap | ratio (rustc/rmc) | 1.06 | 1.06 | 1.42 |
| callrec | rmc | 2.91 | 2.78 | 24 |
| callrec | rustc | 2.82 | 2.71 | 2004 |
| callrec | ratio (rustc/rmc) | 0.97 | 0.97 | 83.50 |
| forloops | rmc | 1.46 | 1.35 | 24 |
| forloops | rustc | 3.31 | 3.20 | 2076 |
| forloops | ratio (rustc/rmc) | 2.26 | 2.37 | 86.50 |

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
