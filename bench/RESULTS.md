# Benchmark results: rmc vs rustc

Recorded 2026-11-09 06:38 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.40 | 1.22 | 2872 | 478 |
| hello | rustc | 2074.65 | 2077.56 | 188720 | 289416 |
| hello | ratio (rustc/rmc) | 1482.6 | 1697.3 | 65.7 | 605.5 |
| fib | rmc | 1.51 | 1.35 | 3020 | 616 |
| fib | rustc | 2126.83 | 2128.12 | 186136 | 291696 |
| fib | ratio (rustc/rmc) | 1404.5 | 1577.6 | 61.6 | 473.5 |
| mandel | rmc | 1.56 | 1.44 | 3032 | 941 |
| mandel | rustc | 2129.80 | 2133.73 | 190496 | 291776 |
| mandel | ratio (rustc/rmc) | 1365.5 | 1477.7 | 62.8 | 310.1 |
| sieve | rmc | 1.48 | 1.35 | 3000 | 699 |
| sieve | rustc | 2094.56 | 2099.12 | 192252 | 292016 |
| sieve | ratio (rustc/rmc) | 1416.6 | 1558.4 | 64.1 | 417.8 |
| mix | rmc | 1.91 | 1.76 | 3016 | 869 |
| mix | rustc | 2080.00 | 2083.94 | 187624 | 291864 |
| mix | ratio (rustc/rmc) | 1091.8 | 1183.4 | 62.2 | 335.9 |
| stats | rmc | 1.50 | 1.38 | 3012 | 770 |
| stats | rustc | 2129.02 | 2132.97 | 187876 | 292048 |
| stats | ratio (rustc/rmc) | 1421.1 | 1541.2 | 62.4 | 379.3 |
| refs | rmc | 1.64 | 1.51 | 3036 | 776 |
| refs | rustc | 2105.27 | 2108.37 | 192252 | 292240 |
| refs | ratio (rustc/rmc) | 1281.1 | 1396.3 | 63.3 | 376.6 |
| heap | rmc | 1.53 | 1.40 | 3024 | 954 |
| heap | rustc | 2126.05 | 2130.04 | 188888 | 292384 |
| heap | ratio (rustc/rmc) | 1393.3 | 1523.6 | 62.5 | 306.5 |
| callrec | rmc | 1.43 | 1.31 | 3032 | 620 |
| callrec | rustc | 2111.62 | 2115.54 | 186628 | 291680 |
| callrec | ratio (rustc/rmc) | 1481.5 | 1617.4 | 61.6 | 470.5 |
| forloops | rmc | 1.61 | 1.46 | 3020 | 737 |
| forloops | rustc | 2107.91 | 2112.18 | 188244 | 292480 |
| forloops | ratio (rustc/rmc) | 1309.5 | 1447.7 | 62.3 | 396.9 |

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
| hello | rmc | 0.27 | 0.14 | 40 |
| hello | rustc | 0.85 | 0.73 | 2040 |
| hello | ratio (rustc/rmc) | 3.11 | 5.01 | 51.00 |
| fib | rmc | 7.24 | 7.08 | 40 |
| fib | rustc | 7.14 | 6.99 | 2016 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 50.40 |
| mandel | rmc | 92.40 | 92.18 | 40 |
| mandel | rustc | 105.47 | 105.25 | 2120 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 53.00 |
| sieve | rmc | 1.56 | 1.43 | 1600 |
| sieve | rustc | 2.47 | 2.35 | 3636 |
| sieve | ratio (rustc/rmc) | 1.59 | 1.64 | 2.27 |
| mix | rmc | 14.81 | 14.65 | 36 |
| mix | rustc | 15.63 | 15.42 | 2100 |
| mix | ratio (rustc/rmc) | 1.06 | 1.05 | 58.33 |
| stats | rmc | 3.26 | 3.12 | 36 |
| stats | rustc | 3.77 | 3.62 | 2112 |
| stats | ratio (rustc/rmc) | 1.15 | 1.16 | 58.67 |
| refs | rmc | 31.09 | 30.87 | 40 |
| refs | rustc | 3.92 | 3.76 | 2124 |
| refs | ratio (rustc/rmc) | 0.13 | 0.12 | 53.10 |
| heap | rmc | 3.20 | 3.07 | 4728 |
| heap | rustc | 3.48 | 3.34 | 6728 |
| heap | ratio (rustc/rmc) | 1.09 | 1.09 | 1.42 |
| callrec | rmc | 2.90 | 2.76 | 40 |
| callrec | rustc | 2.86 | 2.72 | 2036 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.99 | 50.90 |
| forloops | rmc | 1.48 | 1.35 | 40 |
| forloops | rustc | 3.36 | 3.23 | 2104 |
| forloops | ratio (rustc/rmc) | 2.26 | 2.40 | 52.60 |

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
