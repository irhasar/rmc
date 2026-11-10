# Benchmark results: rmc vs rustc

Recorded 2026-11-09 03:39 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 6.99 | 6.93 | 2676 | 3304 |
| hello | rustc | 2103.22 | 2105.90 | 188528 | 289416 |
| hello | ratio (rustc/rmc) | 301.1 | 304.0 | 70.5 | 87.6 |
| fib | rmc | 6.50 | 6.42 | 2688 | 3400 |
| fib | rustc | 2098.11 | 2101.92 | 189008 | 291696 |
| fib | ratio (rustc/rmc) | 322.6 | 327.6 | 70.3 | 85.8 |
| mandel | rmc | 6.66 | 6.53 | 2736 | 3688 |
| mandel | rustc | 2140.08 | 2143.47 | 190960 | 291776 |
| mandel | ratio (rustc/rmc) | 321.5 | 328.0 | 69.8 | 79.1 |
| sieve | rmc | 10.81 | 10.69 | 4380 | 3432 |
| sieve | rustc | 2103.30 | 2107.06 | 187044 | 292016 |
| sieve | ratio (rustc/rmc) | 194.6 | 197.0 | 42.7 | 85.1 |
| mix | rmc | 6.43 | 6.37 | 2708 | 3520 |
| mix | rustc | 2123.73 | 2127.43 | 189864 | 291864 |
| mix | ratio (rustc/rmc) | 330.1 | 334.2 | 70.1 | 82.9 |
| stats | rmc | 6.52 | 6.46 | 2756 | 3512 |
| stats | rustc | 2131.53 | 2135.42 | 188392 | 292048 |
| stats | ratio (rustc/rmc) | 326.7 | 330.5 | 68.4 | 83.2 |
| refs | rmc | 6.69 | 6.61 | 2876 | 3904 |
| refs | rustc | 2097.32 | 2101.72 | 193312 | 292240 |
| refs | ratio (rustc/rmc) | 313.5 | 317.9 | 67.2 | 74.9 |
| heap | rmc | 6.54 | 6.45 | 2760 | 3424 |
| heap | rustc | 2101.40 | 2105.75 | 187924 | 292384 |
| heap | ratio (rustc/rmc) | 321.3 | 326.3 | 68.1 | 85.4 |
| callrec | rmc | 6.42 | 6.29 | 2728 | 3400 |
| callrec | rustc | 2098.71 | 2102.88 | 186896 | 291680 |
| callrec | ratio (rustc/rmc) | 327.0 | 334.1 | 68.5 | 85.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 3304 | 289416 | 4537096 | 1180416 |
| fib | 3400 | 291696 | 4541112 | 1183104 |
| mandel | 3688 | 291776 | 4541208 | 1183168 |
| sieve | 3432 | 292016 | 4541408 | 1183392 |
| mix | 3520 | 291864 | 4541416 | 1183328 |
| stats | 3512 | 292048 | 4541392 | 1183360 |
| refs | 3904 | 292240 | 4541992 | 1183648 |
| heap | 3424 | 292384 | 4542208 | 1183840 |
| callrec | 3400 | 291680 | 4541144 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.27 | 0.14 | 32 |
| hello | rustc | 0.90 | 0.77 | 2032 |
| hello | ratio (rustc/rmc) | 3.40 | 5.52 | 63.50 |
| fib | rmc | 7.43 | 7.25 | 32 |
| fib | rustc | 7.27 | 7.09 | 2100 |
| fib | ratio (rustc/rmc) | 0.98 | 0.98 | 65.62 |
| mandel | rmc | 95.89 | 95.68 | 32 |
| mandel | rustc | 105.61 | 105.32 | 2048 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 64.00 |
| sieve | rmc | 1.58 | 1.44 | 1592 |
| sieve | rustc | 2.49 | 2.36 | 3624 |
| sieve | ratio (rustc/rmc) | 1.58 | 1.64 | 2.28 |
| mix | rmc | 14.80 | 14.63 | 32 |
| mix | rustc | 15.64 | 15.47 | 2060 |
| mix | ratio (rustc/rmc) | 1.06 | 1.06 | 64.38 |
| stats | rmc | 3.37 | 3.25 | 32 |
| stats | rustc | 3.75 | 3.63 | 2092 |
| stats | ratio (rustc/rmc) | 1.11 | 1.12 | 65.38 |
| refs | rmc | 32.86 | 32.68 | 32 |
| refs | rustc | 3.90 | 3.78 | 2060 |
| refs | ratio (rustc/rmc) | 0.12 | 0.12 | 64.38 |
| heap | rmc | 3.26 | 3.12 | 4720 |
| heap | rustc | 3.54 | 3.37 | 6708 |
| heap | ratio (rustc/rmc) | 1.09 | 1.08 | 1.42 |
| callrec | rmc | 2.94 | 2.80 | 32 |
| callrec | rustc | 2.89 | 2.74 | 2048 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.98 | 64.00 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive, and on
`forloops` it wins because the rotated `for` compares the counter
against the bound directly. `refs` is the counterexample: its loop
reads a fixed struct through `&self` accessors, and LICM will not
hoist a read through a reference (it can fault on a loop that never
runs), so the whole accessor sum is recomputed every iteration and rmc
loses to rustc by roughly 8x. A frame-based loop of the same shape is
hoisted and wins. On call-heavy code
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
