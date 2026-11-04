# Benchmark results: rmc vs rustc

Recorded 2026-11-04 11:14 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 6.32 | 6.22 | 2688 | 3304 |
| hello | rustc | 2189.60 | 2192.63 | 187412 | 289416 |
| hello | ratio (rustc/rmc) | 346.5 | 352.7 | 69.7 | 87.6 |
| fib | rmc | 6.12 | 6.05 | 2668 | 3400 |
| fib | rustc | 2102.52 | 2105.88 | 187116 | 291696 |
| fib | ratio (rustc/rmc) | 343.5 | 347.9 | 70.1 | 85.8 |
| mandel | rmc | 6.47 | 6.39 | 2716 | 3688 |
| mandel | rustc | 2133.47 | 2137.74 | 190284 | 291776 |
| mandel | ratio (rustc/rmc) | 330.0 | 334.4 | 70.1 | 79.1 |
| sieve | rmc | 8.74 | 8.62 | 4420 | 3432 |
| sieve | rustc | 2125.23 | 2127.12 | 189252 | 292016 |
| sieve | ratio (rustc/rmc) | 243.2 | 246.7 | 42.8 | 85.1 |
| mix | rmc | 6.23 | 6.14 | 2680 | 3520 |
| mix | rustc | 2110.16 | 2114.80 | 187104 | 291864 |
| mix | ratio (rustc/rmc) | 338.9 | 344.3 | 69.8 | 82.9 |
| stats | rmc | 6.41 | 6.27 | 2756 | 3512 |
| stats | rustc | 2102.33 | 2106.25 | 187660 | 292048 |
| stats | ratio (rustc/rmc) | 328.1 | 335.7 | 68.1 | 83.2 |
| refs | rmc | 6.68 | 6.54 | 2816 | 4016 |
| refs | rustc | 2153.18 | 2157.38 | 192884 | 292240 |
| refs | ratio (rustc/rmc) | 322.3 | 330.1 | 68.5 | 72.8 |
| heap | rmc | 6.43 | 6.31 | 2776 | 3424 |
| heap | rustc | 2133.79 | 2138.32 | 188468 | 292384 |
| heap | ratio (rustc/rmc) | 331.8 | 338.6 | 67.9 | 85.4 |
| callrec | rmc | 6.18 | 6.10 | 2732 | 3400 |
| callrec | rustc | 2093.53 | 2097.62 | 188492 | 291680 |
| callrec | ratio (rustc/rmc) | 339.0 | 343.8 | 69.0 | 85.8 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 3304 | 289416 | 4537096 | 1180416 |
| fib | 3400 | 291696 | 4541112 | 1183104 |
| mandel | 3688 | 291776 | 4541208 | 1183168 |
| sieve | 3432 | 292016 | 4541424 | 1183392 |
| mix | 3520 | 291864 | 4541416 | 1183328 |
| stats | 3512 | 292048 | 4541392 | 1183360 |
| refs | 4016 | 292240 | 4542008 | 1183648 |
| heap | 3424 | 292384 | 4542224 | 1183840 |
| callrec | 3400 | 291680 | 4541144 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.37 | 0.18 | 24 |
| hello | rustc | 0.91 | 0.79 | 2060 |
| hello | ratio (rustc/rmc) | 2.46 | 4.38 | 85.83 |
| fib | rmc | 7.60 | 7.48 | 24 |
| fib | rustc | 7.21 | 7.03 | 2096 |
| fib | ratio (rustc/rmc) | 0.95 | 0.94 | 87.33 |
| mandel | rmc | 96.62 | 96.42 | 24 |
| mandel | rustc | 107.00 | 106.80 | 2088 |
| mandel | ratio (rustc/rmc) | 1.11 | 1.11 | 87.00 |
| sieve | rmc | 1.59 | 1.46 | 1584 |
| sieve | rustc | 2.76 | 2.61 | 3640 |
| sieve | ratio (rustc/rmc) | 1.74 | 1.79 | 2.30 |
| mix | rmc | 14.80 | 14.64 | 24 |
| mix | rustc | 15.52 | 15.38 | 2032 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 84.67 |
| stats | rmc | 3.51 | 3.35 | 24 |
| stats | rustc | 3.98 | 3.85 | 2080 |
| stats | ratio (rustc/rmc) | 1.13 | 1.15 | 86.67 |
| refs | rmc | 2.04 | 1.87 | 24 |
| refs | rustc | 4.12 | 3.94 | 2112 |
| refs | ratio (rustc/rmc) | 2.02 | 2.11 | 88.00 |
| heap | rmc | 3.48 | 3.34 | 4712 |
| heap | rustc | 3.70 | 3.58 | 6704 |
| heap | ratio (rustc/rmc) | 1.06 | 1.07 | 1.42 |
| callrec | rmc | 2.91 | 2.79 | 24 |
| callrec | rustc | 2.89 | 2.75 | 2036 |
| callrec | ratio (rustc/rmc) | 0.99 | 0.99 | 84.83 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive, and on `refs`
it wins by more than 2x because LICM reassociates the loop-invariant
part of the accumulation into one hoisted value. On call-heavy code
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
