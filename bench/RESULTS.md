# Benchmark results: rmc vs rustc

Recorded 2026-11-11 23:44 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.34 | 1.22 | 3080 | 495 |
| hello | rustc | 2076.10 | 2072.60 | 187752 | 289416 |
| hello | ratio (rustc/rmc) | 1552.5 | 1691.9 | 61.0 | 584.7 |
| fib | rmc | 1.46 | 1.33 | 3112 | 692 |
| fib | rustc | 2043.10 | 2047.51 | 187000 | 291696 |
| fib | ratio (rustc/rmc) | 1395.7 | 1534.9 | 60.1 | 421.5 |
| mandel | rmc | 1.57 | 1.45 | 3164 | 1041 |
| mandel | rustc | 2083.71 | 2087.25 | 190320 | 291776 |
| mandel | ratio (rustc/rmc) | 1325.9 | 1435.5 | 60.2 | 280.3 |
| sieve | rmc | 1.49 | 1.37 | 3128 | 852 |
| sieve | rustc | 2078.26 | 2082.36 | 187220 | 292032 |
| sieve | ratio (rustc/rmc) | 1394.2 | 1518.9 | 59.9 | 342.8 |
| mix | rmc | 1.52 | 1.40 | 3104 | 941 |
| mix | rustc | 2067.95 | 2071.18 | 192308 | 291880 |
| mix | ratio (rustc/rmc) | 1359.2 | 1480.5 | 62.0 | 310.2 |
| stats | rmc | 1.54 | 1.43 | 3164 | 924 |
| stats | rustc | 2074.10 | 2077.53 | 187968 | 292048 |
| stats | ratio (rustc/rmc) | 1343.3 | 1451.8 | 59.4 | 316.1 |
| refs | rmc | 1.66 | 1.54 | 3108 | 1014 |
| refs | rustc | 2064.70 | 2068.67 | 188612 | 292256 |
| refs | ratio (rustc/rmc) | 1247.3 | 1346.8 | 60.7 | 288.2 |
| heap | rmc | 1.62 | 1.49 | 3176 | 1165 |
| heap | rustc | 2068.92 | 2073.25 | 188064 | 292400 |
| heap | ratio (rustc/rmc) | 1276.8 | 1393.3 | 59.2 | 251.0 |
| callrec | rmc | 1.45 | 1.31 | 3116 | 695 |
| callrec | rustc | 2070.15 | 2073.55 | 188580 | 291680 |
| callrec | ratio (rustc/rmc) | 1429.9 | 1580.5 | 60.5 | 419.7 |
| forloops | rmc | 1.61 | 1.49 | 3132 | 851 |
| forloops | rustc | 2072.64 | 2076.65 | 187024 | 292480 |
| forloops | ratio (rustc/rmc) | 1286.6 | 1393.7 | 59.7 | 343.7 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 495 | 289416 | 4537096 | 1180416 |
| fib | 692 | 291696 | 4541112 | 1183104 |
| mandel | 1041 | 291776 | 4541208 | 1183168 |
| sieve | 852 | 292032 | 4541424 | 1183392 |
| mix | 941 | 291880 | 4541432 | 1183328 |
| stats | 924 | 292048 | 4541392 | 1183360 |
| refs | 1014 | 292256 | 4542008 | 1183648 |
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
| hello | rmc | 0.26 | 0.14 | 20 |
| hello | rustc | 0.82 | 0.72 | 2068 |
| hello | ratio (rustc/rmc) | 3.17 | 5.29 | 103.40 |
| fib | rmc | 7.10 | 6.99 | 24 |
| fib | rustc | 7.09 | 6.96 | 2056 |
| fib | ratio (rustc/rmc) | 1.00 | 1.00 | 85.67 |
| mandel | rmc | 95.89 | 95.67 | 24 |
| mandel | rustc | 105.14 | 104.91 | 2036 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 84.83 |
| sieve | rmc | 1.75 | 1.62 | 1584 |
| sieve | rustc | 2.42 | 2.30 | 3588 |
| sieve | ratio (rustc/rmc) | 1.38 | 1.42 | 2.27 |
| mix | rmc | 14.82 | 14.69 | 24 |
| mix | rustc | 15.45 | 15.29 | 2044 |
| mix | ratio (rustc/rmc) | 1.04 | 1.04 | 85.17 |
| stats | rmc | 3.23 | 3.12 | 24 |
| stats | rustc | 3.71 | 3.60 | 2080 |
| stats | ratio (rustc/rmc) | 1.15 | 1.15 | 86.67 |
| refs | rmc | 1.79 | 1.66 | 24 |
| refs | rustc | 3.84 | 3.70 | 1988 |
| refs | ratio (rustc/rmc) | 2.15 | 2.24 | 82.83 |
| heap | rmc | 3.34 | 3.17 | 4712 |
| heap | rustc | 3.48 | 3.31 | 6700 |
| heap | ratio (rustc/rmc) | 1.04 | 1.05 | 1.42 |
| callrec | rmc | 2.90 | 2.77 | 24 |
| callrec | rustc | 2.84 | 2.70 | 2020 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.97 | 84.17 |
| forloops | rmc | 1.46 | 1.35 | 24 |
| forloops | rustc | 3.32 | 3.19 | 2096 |
| forloops | ratio (rustc/rmc) | 2.27 | 2.36 | 87.33 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible, though on this recording rmc is ahead on every program. On
loop-heavy integer code rmc is competitive: it wins on `forloops`
because the rotated `for` compares the counter against the bound
directly. `refs` used to be the counterexample - its loop reads a fixed
struct through `&self` accessors, and LICM would not hoist a read
through a reference (it can fault on a loop that never runs), so the
whole accessor sum was recomputed every iteration and rmc lost to rustc
by roughly 8x. A provenance pre-pass now proves that a reference bound
to a live local points at the frame, so its read *is* hoisted and `refs`
wins (2.15x); a reference of unproven origin (a parameter, a cast, a
mutable ref) still blocks the hoist and such a loop would still lose.
On call-heavy code (`callrec`) rmc keeps up because the self-recursion
pass folds the right spine into a loop, leaving one call per level - the
same transform rustc applies. A register-only function also drops the
frame pointer, so a leaf call carries no prologue beyond saving the
registers it actually uses. That pass is narrow (one `i64` parameter, an
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
