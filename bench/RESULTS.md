# Benchmark results: rmc vs rustc

Recorded 2026-11-09 03:46 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.36 | 1.23 | 2960 | 478 |
| hello | rustc | 2074.91 | 2076.19 | 189864 | 289416 |
| hello | ratio (rustc/rmc) | 1520.7 | 1686.6 | 64.1 | 605.5 |
| fib | rmc | 1.43 | 1.29 | 3012 | 616 |
| fib | rustc | 2102.04 | 2105.94 | 187372 | 291696 |
| fib | ratio (rustc/rmc) | 1472.9 | 1628.7 | 62.2 | 473.5 |
| mandel | rmc | 1.52 | 1.39 | 3004 | 941 |
| mandel | rustc | 2107.51 | 2111.38 | 189684 | 291776 |
| mandel | ratio (rustc/rmc) | 1390.9 | 1520.1 | 63.1 | 310.1 |
| sieve | rmc | 1.50 | 1.35 | 2988 | 699 |
| sieve | rustc | 2115.58 | 2119.71 | 191888 | 292016 |
| sieve | ratio (rustc/rmc) | 1411.6 | 1569.0 | 64.2 | 417.8 |
| mix | rmc | 1.51 | 1.39 | 2976 | 869 |
| mix | rustc | 2119.73 | 2123.68 | 186684 | 291864 |
| mix | ratio (rustc/rmc) | 1399.5 | 1531.1 | 62.7 | 335.9 |
| stats | rmc | 1.52 | 1.40 | 2980 | 770 |
| stats | rustc | 2116.00 | 2120.08 | 187716 | 292048 |
| stats | ratio (rustc/rmc) | 1388.4 | 1511.1 | 63.0 | 379.3 |
| refs | rmc | 1.62 | 1.49 | 3056 | 776 |
| refs | rustc | 2112.92 | 2117.10 | 193232 | 292240 |
| refs | ratio (rustc/rmc) | 1307.6 | 1419.0 | 63.2 | 376.6 |
| heap | rmc | 1.51 | 1.40 | 3000 | 954 |
| heap | rustc | 2141.93 | 2145.31 | 188340 | 292384 |
| heap | ratio (rustc/rmc) | 1417.1 | 1537.9 | 62.8 | 306.5 |
| callrec | rmc | 1.47 | 1.34 | 2964 | 620 |
| callrec | rustc | 2115.67 | 2119.34 | 187276 | 291680 |
| callrec | ratio (rustc/rmc) | 1440.7 | 1581.6 | 63.2 | 470.5 |
| forloops | rmc | 1.48 | 1.35 | 2980 | 737 |
| forloops | rustc | 2124.33 | 2128.29 | 188024 | 292480 |
| forloops | ratio (rustc/rmc) | 1432.9 | 1570.7 | 63.1 | 396.9 |

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
| hello | rustc | 0.83 | 0.70 | 2088 |
| hello | ratio (rustc/rmc) | 3.45 | 5.73 | 65.25 |
| fib | rmc | 7.14 | 7.02 | 28 |
| fib | rustc | 7.08 | 6.95 | 2008 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 71.71 |
| mandel | rmc | 92.73 | 92.49 | 32 |
| mandel | rustc | 105.33 | 105.10 | 2064 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 64.50 |
| sieve | rmc | 1.58 | 1.45 | 1592 |
| sieve | rustc | 2.58 | 2.45 | 3624 |
| sieve | ratio (rustc/rmc) | 1.63 | 1.70 | 2.28 |
| mix | rmc | 14.77 | 14.62 | 32 |
| mix | rustc | 15.57 | 15.39 | 1996 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 62.38 |
| stats | rmc | 3.26 | 3.14 | 32 |
| stats | rustc | 3.78 | 3.63 | 2040 |
| stats | ratio (rustc/rmc) | 1.16 | 1.15 | 63.75 |
| refs | rmc | 31.27 | 31.07 | 32 |
| refs | rustc | 3.87 | 3.73 | 2044 |
| refs | ratio (rustc/rmc) | 0.12 | 0.12 | 63.88 |
| heap | rmc | 3.21 | 3.08 | 4720 |
| heap | rustc | 3.44 | 3.30 | 6676 |
| heap | ratio (rustc/rmc) | 1.07 | 1.07 | 1.41 |
| callrec | rmc | 2.95 | 2.81 | 32 |
| callrec | rustc | 2.91 | 2.76 | 1992 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.98 | 62.25 |
| forloops | rmc | 1.48 | 1.37 | 32 |
| forloops | rustc | 3.34 | 3.23 | 2088 |
| forloops | ratio (rustc/rmc) | 2.25 | 2.36 | 65.25 |

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
