# Benchmark results: rmc vs rustc

Recorded 2026-11-05 08:42 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.20 | 1.08 | 2796 | 478 |
| hello | rustc | 2046.27 | 2049.77 | 188616 | 289416 |
| hello | ratio (rustc/rmc) | 1707.3 | 1903.2 | 67.5 | 605.5 |
| fib | rmc | 1.27 | 1.15 | 2776 | 616 |
| fib | rustc | 2115.55 | 2119.97 | 186608 | 291696 |
| fib | ratio (rustc/rmc) | 1670.6 | 1851.5 | 67.2 | 473.5 |
| mandel | rmc | 1.40 | 1.28 | 2792 | 941 |
| mandel | rustc | 2092.47 | 2097.23 | 190660 | 291776 |
| mandel | ratio (rustc/rmc) | 1491.1 | 1643.6 | 68.3 | 310.1 |
| sieve | rmc | 1.31 | 1.19 | 2800 | 699 |
| sieve | rustc | 2091.46 | 2096.28 | 191924 | 292016 |
| sieve | ratio (rustc/rmc) | 1600.4 | 1761.6 | 68.5 | 417.8 |
| mix | rmc | 1.39 | 1.27 | 2812 | 869 |
| mix | rustc | 2088.96 | 2093.73 | 187272 | 291864 |
| mix | ratio (rustc/rmc) | 1503.4 | 1652.5 | 66.6 | 335.9 |
| stats | rmc | 1.37 | 1.26 | 2760 | 770 |
| stats | rustc | 2118.31 | 2122.88 | 189400 | 292048 |
| stats | ratio (rustc/rmc) | 1543.6 | 1687.5 | 68.6 | 379.3 |
| refs | rmc | 1.50 | 1.39 | 2864 | 901 |
| refs | rustc | 2117.89 | 2122.56 | 193508 | 292240 |
| refs | ratio (rustc/rmc) | 1409.3 | 1532.5 | 67.6 | 324.4 |
| heap | rmc | 1.38 | 1.26 | 2828 | 954 |
| heap | rustc | 2101.09 | 2106.12 | 193448 | 292384 |
| heap | ratio (rustc/rmc) | 1517.5 | 1667.6 | 68.4 | 306.5 |
| callrec | rmc | 1.31 | 1.18 | 2708 | 620 |
| callrec | rustc | 2113.55 | 2117.31 | 186640 | 291680 |
| callrec | ratio (rustc/rmc) | 1614.6 | 1792.8 | 68.9 | 470.5 |
| forloops | rmc | 1.33 | 1.21 | 2772 | 737 |
| forloops | rustc | 2095.37 | 2100.12 | 188472 | 292480 |
| forloops | ratio (rustc/rmc) | 1575.2 | 1741.4 | 68.0 | 396.9 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 478 | 289416 | 4537096 | 1180416 |
| fib | 616 | 291696 | 4541112 | 1183104 |
| mandel | 941 | 291776 | 4541208 | 1183168 |
| sieve | 699 | 292016 | 4541424 | 1183392 |
| mix | 869 | 291864 | 4541416 | 1183328 |
| stats | 770 | 292048 | 4541392 | 1183360 |
| refs | 901 | 292240 | 4542008 | 1183648 |
| heap | 954 | 292384 | 4542224 | 1183840 |
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
| hello | rmc | 0.25 | 0.13 | 24 |
| hello | rustc | 0.82 | 0.71 | 2040 |
| hello | ratio (rustc/rmc) | 3.25 | 5.43 | 85.00 |
| fib | rmc | 7.12 | 7.00 | 24 |
| fib | rustc | 7.10 | 6.96 | 2060 |
| fib | ratio (rustc/rmc) | 1.00 | 0.99 | 85.83 |
| mandel | rmc | 92.35 | 92.12 | 24 |
| mandel | rustc | 105.11 | 104.89 | 2104 |
| mandel | ratio (rustc/rmc) | 1.14 | 1.14 | 87.67 |
| sieve | rmc | 1.55 | 1.43 | 1584 |
| sieve | rustc | 2.43 | 2.31 | 3652 |
| sieve | ratio (rustc/rmc) | 1.57 | 1.62 | 2.31 |
| mix | rmc | 14.71 | 14.55 | 24 |
| mix | rustc | 15.42 | 15.28 | 2024 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 84.33 |
| stats | rmc | 3.24 | 3.11 | 24 |
| stats | rustc | 3.81 | 3.66 | 2088 |
| stats | ratio (rustc/rmc) | 1.17 | 1.17 | 87.00 |
| refs | rmc | 1.74 | 1.61 | 24 |
| refs | rustc | 3.89 | 3.75 | 2048 |
| refs | ratio (rustc/rmc) | 2.23 | 2.33 | 85.33 |
| heap | rmc | 3.17 | 3.05 | 4712 |
| heap | rustc | 3.44 | 3.25 | 6728 |
| heap | ratio (rustc/rmc) | 1.09 | 1.07 | 1.43 |
| callrec | rmc | 2.91 | 2.79 | 24 |
| callrec | rustc | 3.91 | 3.56 | 2116 |
| callrec | ratio (rustc/rmc) | 1.34 | 1.28 | 88.17 |
| forloops | rmc | 1.48 | 1.34 | 24 |
| forloops | rustc | 3.29 | 3.17 | 2012 |
| forloops | ratio (rustc/rmc) | 2.22 | 2.37 | 83.83 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive: it wins on
`forloops` because the rotated `for` compares the counter against the
bound directly, and on `refs` because LICM reassociates the
loop-invariant part of the accumulation into one hoisted value. On call-heavy code
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
