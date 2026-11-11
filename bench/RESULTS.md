# Benchmark results: rmc vs rustc

Recorded 2026-11-11 13:46 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.75 | 1.59 | 3080 | 495 |
| hello | rustc | 2023.29 | 2026.37 | 186388 | 289416 |
| hello | ratio (rustc/rmc) | 1158.6 | 1276.1 | 60.5 | 584.7 |
| fib | rmc | 1.89 | 1.74 | 3104 | 692 |
| fib | rustc | 2087.81 | 2091.17 | 186500 | 291696 |
| fib | ratio (rustc/rmc) | 1102.0 | 1202.5 | 60.1 | 421.5 |
| mandel | rmc | 1.57 | 1.43 | 3116 | 1041 |
| mandel | rustc | 2081.03 | 2085.15 | 192368 | 291776 |
| mandel | ratio (rustc/rmc) | 1321.7 | 1457.1 | 61.7 | 280.3 |
| sieve | rmc | 1.52 | 1.39 | 3080 | 852 |
| sieve | rustc | 2130.61 | 2133.97 | 188476 | 292032 |
| sieve | ratio (rustc/rmc) | 1400.8 | 1533.0 | 61.2 | 342.8 |
| mix | rmc | 1.53 | 1.41 | 3108 | 941 |
| mix | rustc | 2098.38 | 2102.12 | 188264 | 291880 |
| mix | ratio (rustc/rmc) | 1367.7 | 1494.0 | 60.6 | 310.2 |
| stats | rmc | 1.54 | 1.43 | 3160 | 942 |
| stats | rustc | 2154.39 | 2156.99 | 188052 | 292048 |
| stats | ratio (rustc/rmc) | 1401.4 | 1506.3 | 59.5 | 310.0 |
| refs | rmc | 1.81 | 1.59 | 3148 | 892 |
| refs | rustc | 2096.40 | 2100.23 | 193676 | 292256 |
| refs | ratio (rustc/rmc) | 1160.0 | 1317.6 | 61.5 | 327.6 |
| heap | rmc | 1.60 | 1.47 | 3132 | 1165 |
| heap | rustc | 2136.26 | 2140.36 | 187812 | 292400 |
| heap | ratio (rustc/rmc) | 1334.9 | 1451.1 | 60.0 | 251.0 |
| callrec | rmc | 1.46 | 1.34 | 3048 | 695 |
| callrec | rustc | 2109.60 | 2113.63 | 187596 | 291680 |
| callrec | ratio (rustc/rmc) | 1445.2 | 1579.7 | 61.5 | 419.7 |
| forloops | rmc | 1.60 | 1.43 | 3088 | 851 |
| forloops | rustc | 2106.54 | 2110.00 | 188548 | 292480 |
| forloops | ratio (rustc/rmc) | 1317.3 | 1477.6 | 61.1 | 343.7 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 495 | 289416 | 4537096 | 1180416 |
| fib | 692 | 291696 | 4541112 | 1183104 |
| mandel | 1041 | 291776 | 4541208 | 1183168 |
| sieve | 852 | 292032 | 4541424 | 1183392 |
| mix | 941 | 291880 | 4541432 | 1183328 |
| stats | 942 | 292048 | 4541392 | 1183360 |
| refs | 892 | 292256 | 4542008 | 1183648 |
| heap | 1165 | 292400 | 4542224 | 1183840 |
| callrec | 695 | 291680 | 4541144 | 1183104 |
| forloops | 851 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.22 | 0.12 | 24 |
| hello | rustc | 0.81 | 0.70 | 2024 |
| hello | ratio (rustc/rmc) | 3.73 | 5.97 | 84.33 |
| fib | rmc | 7.12 | 7.00 | 24 |
| fib | rustc | 7.07 | 6.93 | 2100 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 87.50 |
| mandel | rmc | 95.70 | 95.48 | 24 |
| mandel | rustc | 105.14 | 104.94 | 2100 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 87.50 |
| sieve | rmc | 1.77 | 1.64 | 1584 |
| sieve | rustc | 2.39 | 2.27 | 3648 |
| sieve | ratio (rustc/rmc) | 1.35 | 1.38 | 2.30 |
| mix | rmc | 14.83 | 14.69 | 20 |
| mix | rustc | 15.53 | 15.35 | 2024 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 101.20 |
| stats | rmc | 3.93 | 3.81 | 24 |
| stats | rustc | 3.73 | 3.61 | 2096 |
| stats | ratio (rustc/rmc) | 0.95 | 0.95 | 87.33 |
| refs | rmc | 30.84 | 30.69 | 24 |
| refs | rustc | 3.84 | 3.71 | 2020 |
| refs | ratio (rustc/rmc) | 0.12 | 0.12 | 84.17 |
| heap | rmc | 3.18 | 3.04 | 4712 |
| heap | rustc | 3.37 | 3.27 | 6684 |
| heap | ratio (rustc/rmc) | 1.06 | 1.07 | 1.42 |
| callrec | rmc | 2.87 | 2.76 | 24 |
| callrec | rustc | 2.83 | 2.72 | 2052 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.98 | 85.50 |
| forloops | rmc | 1.44 | 1.33 | 24 |
| forloops | rustc | 3.30 | 3.18 | 1984 |
| forloops | ratio (rustc/rmc) | 2.29 | 2.39 | 82.67 |

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
