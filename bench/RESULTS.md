# Benchmark results: rmc vs rustc

Recorded 2026-11-09 03:42 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.23 | 1.11 | 2756 | 478 |
| hello | rustc | 2057.85 | 2061.13 | 185576 | 289416 |
| hello | ratio (rustc/rmc) | 1672.2 | 1853.5 | 67.3 | 605.5 |
| fib | rmc | 1.32 | 1.21 | 2868 | 616 |
| fib | rustc | 2108.23 | 2107.70 | 186600 | 291696 |
| fib | ratio (rustc/rmc) | 1593.5 | 1736.2 | 65.1 | 473.5 |
| mandel | rmc | 1.43 | 1.31 | 2872 | 941 |
| mandel | rustc | 2102.91 | 2107.30 | 190536 | 291776 |
| mandel | ratio (rustc/rmc) | 1470.5 | 1608.6 | 66.3 | 310.1 |
| sieve | rmc | 1.35 | 1.24 | 2852 | 699 |
| sieve | rustc | 2115.25 | 2116.63 | 191840 | 292016 |
| sieve | ratio (rustc/rmc) | 1565.2 | 1702.8 | 67.3 | 417.8 |
| mix | rmc | 1.37 | 1.26 | 2832 | 869 |
| mix | rustc | 2082.57 | 2086.97 | 189984 | 291864 |
| mix | ratio (rustc/rmc) | 1523.1 | 1662.9 | 67.1 | 335.9 |
| stats | rmc | 1.40 | 1.29 | 2876 | 770 |
| stats | rustc | 2092.23 | 2096.29 | 188276 | 292048 |
| stats | ratio (rustc/rmc) | 1490.9 | 1627.6 | 65.5 | 379.3 |
| refs | rmc | 2.00 | 1.81 | 2864 | 776 |
| refs | rustc | 2106.20 | 2109.02 | 192404 | 292240 |
| refs | ratio (rustc/rmc) | 1055.2 | 1165.2 | 67.2 | 376.6 |
| heap | rmc | 1.38 | 1.26 | 2844 | 954 |
| heap | rustc | 2083.70 | 2088.12 | 188200 | 292384 |
| heap | ratio (rustc/rmc) | 1509.5 | 1650.7 | 66.2 | 306.5 |
| callrec | rmc | 1.29 | 1.17 | 2840 | 620 |
| callrec | rustc | 2054.89 | 2059.34 | 186668 | 291680 |
| callrec | ratio (rustc/rmc) | 1594.4 | 1764.6 | 65.7 | 470.5 |
| forloops | rmc | 1.38 | 1.26 | 2804 | 737 |
| forloops | rustc | 2090.29 | 2094.02 | 189728 | 292480 |
| forloops | ratio (rustc/rmc) | 1517.1 | 1665.9 | 67.7 | 396.9 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 616 | 291696 | 4541112 | 1183104 |
| mandel | 941 | 291776 | 4541208 | 1183168 |
| sieve | 699 | 292016 | 4541408 | 1183392 |
| mix | 869 | 291864 | 4541416 | 1183328 |
| stats | 770 | 292048 | 4541392 | 1183360 |
| refs | 776 | 292240 | 4541992 | 1183648 |
| heap | 954 | 292384 | 4542208 | 1183840 |
| callrec | 620 | 291680 | 4541144 | 1183104 |
| forloops | 737 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.24 | 0.12 | 32 |
| hello | rustc | 0.83 | 0.71 | 2000 |
| hello | ratio (rustc/rmc) | 3.52 | 5.81 | 62.50 |
| fib | rmc | 7.17 | 7.02 | 32 |
| fib | rustc | 7.13 | 6.97 | 2068 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 64.62 |
| mandel | rmc | 91.98 | 91.80 | 32 |
| mandel | rustc | 105.42 | 105.21 | 2016 |
| mandel | ratio (rustc/rmc) | 1.15 | 1.15 | 63.00 |
| sieve | rmc | 1.53 | 1.42 | 1592 |
| sieve | rustc | 2.47 | 2.33 | 3580 |
| sieve | ratio (rustc/rmc) | 1.62 | 1.64 | 2.25 |
| mix | rmc | 14.75 | 14.59 | 32 |
| mix | rustc | 15.47 | 15.32 | 2008 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 62.75 |
| stats | rmc | 3.25 | 3.09 | 32 |
| stats | rustc | 3.70 | 3.57 | 2060 |
| stats | ratio (rustc/rmc) | 1.14 | 1.15 | 64.38 |
| refs | rmc | 30.99 | 30.81 | 32 |
| refs | rustc | 3.79 | 3.67 | 2036 |
| refs | ratio (rustc/rmc) | 0.12 | 0.12 | 63.62 |
| heap | rmc | 3.19 | 3.06 | 4720 |
| heap | rustc | 3.43 | 3.30 | 6716 |
| heap | ratio (rustc/rmc) | 1.07 | 1.08 | 1.42 |
| callrec | rmc | 2.96 | 2.79 | 32 |
| callrec | rustc | 2.91 | 2.73 | 2024 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.98 | 63.25 |
| forloops | rmc | 1.45 | 1.34 | 32 |
| forloops | rustc | 3.34 | 3.21 | 2052 |
| forloops | ratio (rustc/rmc) | 2.30 | 2.39 | 64.12 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive: it wins on
`forloops` because the rotated `for` compares the counter against the
bound directly. `refs` is the counterexample: its loop reads a fixed
struct through `&self` accessors, and LICM will not hoist a read through
a reference (it can fault on a loop that never runs), so the whole
accessor sum is recomputed every iteration and rmc loses to rustc by
roughly 8x. A frame-based loop of the same shape is hoisted and wins.
On call-heavy code
(`callrec`) rmc now keeps up because the self-recursion pass folds the
right spine into a loop, leaving one call per level - the same
transform rustc applies. A register-only function also drops the frame
pointer, so a leaf call carries no prologue beyond saving the registers
it actually uses. That pass is narrow (one `i64` parameter, an
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
