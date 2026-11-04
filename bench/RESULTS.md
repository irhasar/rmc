# Benchmark results: rmc vs rustc

Recorded 2026-11-04 17:23 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.17 | 1.07 | 2748 | 478 |
| hello | rustc | 2055.22 | 2058.01 | 186000 | 289416 |
| hello | ratio (rustc/rmc) | 1749.7 | 1928.8 | 67.7 | 605.5 |
| fib | rmc | 1.28 | 1.16 | 2796 | 609 |
| fib | rustc | 2071.73 | 2075.70 | 188076 | 291696 |
| fib | ratio (rustc/rmc) | 1616.5 | 1786.3 | 67.3 | 479.0 |
| mandel | rmc | 1.38 | 1.26 | 2780 | 916 |
| mandel | rustc | 2098.17 | 2102.40 | 190320 | 291776 |
| mandel | ratio (rustc/rmc) | 1515.3 | 1663.3 | 68.5 | 318.5 |
| sieve | rmc | 1.33 | 1.21 | 2796 | 689 |
| sieve | rustc | 2146.48 | 2150.59 | 187000 | 292016 |
| sieve | ratio (rustc/rmc) | 1611.4 | 1778.8 | 66.9 | 423.8 |
| mix | rmc | 1.39 | 1.23 | 2816 | 845 |
| mix | rustc | 2118.96 | 2123.16 | 188744 | 291864 |
| mix | ratio (rustc/rmc) | 1523.6 | 1731.8 | 67.0 | 345.4 |
| stats | rmc | 1.32 | 1.19 | 2760 | 765 |
| stats | rustc | 2072.23 | 2077.02 | 188660 | 292048 |
| stats | ratio (rustc/rmc) | 1568.9 | 1742.5 | 68.4 | 381.8 |
| refs | rmc | 1.48 | 1.35 | 2788 | 888 |
| refs | rustc | 2123.79 | 2111.12 | 193272 | 292240 |
| refs | ratio (rustc/rmc) | 1436.7 | 1558.0 | 69.3 | 329.1 |
| heap | rmc | 1.36 | 1.24 | 2800 | 930 |
| heap | rustc | 2107.21 | 2111.32 | 187852 | 292384 |
| heap | ratio (rustc/rmc) | 1551.9 | 1701.3 | 67.1 | 314.4 |
| callrec | rmc | 1.33 | 1.18 | 2820 | 613 |
| callrec | rustc | 2114.64 | 2119.42 | 186476 | 291680 |
| callrec | ratio (rustc/rmc) | 1593.4 | 1794.6 | 66.1 | 475.8 |
| forloops | rmc | 1.32 | 1.20 | 2744 | 739 |
| forloops | rustc | 2088.65 | 2091.33 | 188744 | 292480 |
| forloops | ratio (rustc/rmc) | 1586.5 | 1738.4 | 68.8 | 395.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 609 | 291696 | 4541112 | 1183104 |
| mandel | 916 | 291776 | 4541208 | 1183168 |
| sieve | 689 | 292016 | 4541424 | 1183392 |
| mix | 845 | 291864 | 4541416 | 1183328 |
| stats | 765 | 292048 | 4541392 | 1183360 |
| refs | 888 | 292240 | 4542008 | 1183648 |
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
| hello | rmc | 0.26 | 0.15 | 24 |
| hello | rustc | 0.86 | 0.72 | 2096 |
| hello | ratio (rustc/rmc) | 3.30 | 4.83 | 87.33 |
| fib | rmc | 7.18 | 7.02 | 24 |
| fib | rustc | 7.10 | 6.98 | 2028 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 84.50 |
| mandel | rmc | 92.75 | 92.55 | 24 |
| mandel | rustc | 105.38 | 105.14 | 2028 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 84.50 |
| sieve | rmc | 1.55 | 1.42 | 1584 |
| sieve | rustc | 2.43 | 2.30 | 3632 |
| sieve | ratio (rustc/rmc) | 1.56 | 1.62 | 2.29 |
| mix | rmc | 16.86 | 16.72 | 24 |
| mix | rustc | 15.44 | 15.29 | 2052 |
| mix | ratio (rustc/rmc) | 0.92 | 0.91 | 85.50 |
| stats | rmc | 3.36 | 3.22 | 24 |
| stats | rustc | 3.77 | 3.64 | 2096 |
| stats | ratio (rustc/rmc) | 1.12 | 1.13 | 87.33 |
| refs | rmc | 1.75 | 1.63 | 24 |
| refs | rustc | 3.85 | 3.72 | 2024 |
| refs | ratio (rustc/rmc) | 2.20 | 2.28 | 84.33 |
| heap | rmc | 3.31 | 3.17 | 4712 |
| heap | rustc | 3.43 | 3.27 | 6708 |
| heap | ratio (rustc/rmc) | 1.04 | 1.03 | 1.42 |
| callrec | rmc | 2.88 | 2.76 | 24 |
| callrec | rustc | 2.81 | 2.70 | 1996 |
| callrec | ratio (rustc/rmc) | 0.97 | 0.98 | 83.17 |
| forloops | rmc | 2.09 | 1.96 | 24 |
| forloops | rustc | 3.33 | 3.20 | 2024 |
| forloops | ratio (rustc/rmc) | 1.60 | 1.63 | 84.33 |

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
