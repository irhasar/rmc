# Benchmark results: rmc vs rustc

Recorded 2026-11-04 10:38 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 6.19 | 6.05 | 2708 | 3304 |
| hello | rustc | 2117.72 | 2117.18 | 188168 | 289416 |
| hello | ratio (rustc/rmc) | 342.3 | 349.8 | 69.5 | 87.6 |
| fib | rmc | 6.01 | 5.93 | 2688 | 3400 |
| fib | rustc | 2139.83 | 2141.40 | 189624 | 291696 |
| fib | ratio (rustc/rmc) | 356.3 | 361.2 | 70.5 | 85.8 |
| mandel | rmc | 6.22 | 6.12 | 2704 | 3688 |
| mandel | rustc | 2110.10 | 2113.59 | 190472 | 291776 |
| mandel | ratio (rustc/rmc) | 339.0 | 345.4 | 70.4 | 79.1 |
| sieve | rmc | 10.32 | 10.03 | 4372 | 3432 |
| sieve | rustc | 2118.12 | 2122.54 | 187332 | 292016 |
| sieve | ratio (rustc/rmc) | 205.2 | 211.6 | 42.8 | 85.1 |
| mix | rmc | 6.13 | 6.07 | 2716 | 3520 |
| mix | rustc | 2148.62 | 2151.64 | 188776 | 291864 |
| mix | ratio (rustc/rmc) | 350.3 | 354.5 | 69.5 | 82.9 |
| stats | rmc | 7.70 | 7.56 | 2736 | 3512 |
| stats | rustc | 2135.19 | 2139.45 | 187712 | 292048 |
| stats | ratio (rustc/rmc) | 277.3 | 283.1 | 68.6 | 83.2 |
| refs | rmc | 6.40 | 6.33 | 2812 | 4016 |
| refs | rustc | 2119.83 | 2124.52 | 193264 | 292240 |
| refs | ratio (rustc/rmc) | 331.4 | 335.4 | 68.7 | 72.8 |
| heap | rmc | 6.28 | 6.16 | 2736 | 3424 |
| heap | rustc | 2114.66 | 2118.77 | 190300 | 292384 |
| heap | ratio (rustc/rmc) | 336.8 | 343.8 | 69.6 | 85.4 |
| callrec | rmc | 6.81 | 6.61 | 2656 | 3400 |
| callrec | rustc | 2124.80 | 2127.24 | 187668 | 291680 |
| callrec | ratio (rustc/rmc) | 312.1 | 322.0 | 70.7 | 85.8 |

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
| hello | rmc | 0.27 | 0.14 | 20 |
| hello | rustc | 0.86 | 0.74 | 2032 |
| hello | ratio (rustc/rmc) | 3.16 | 5.48 | 101.60 |
| fib | rmc | 7.34 | 7.17 | 24 |
| fib | rustc | 7.16 | 7.01 | 2080 |
| fib | ratio (rustc/rmc) | 0.98 | 0.98 | 86.67 |
| mandel | rmc | 97.03 | 96.83 | 24 |
| mandel | rustc | 105.67 | 105.41 | 2076 |
| mandel | ratio (rustc/rmc) | 1.09 | 1.09 | 86.50 |
| sieve | rmc | 1.60 | 1.47 | 1584 |
| sieve | rustc | 2.54 | 2.41 | 3636 |
| sieve | ratio (rustc/rmc) | 1.59 | 1.64 | 2.30 |
| mix | rmc | 20.98 | 18.86 | 24 |
| mix | rustc | 15.51 | 15.36 | 2032 |
| mix | ratio (rustc/rmc) | 0.74 | 0.81 | 84.67 |
| stats | rmc | 3.35 | 3.22 | 24 |
| stats | rustc | 3.75 | 3.62 | 2052 |
| stats | ratio (rustc/rmc) | 1.12 | 1.12 | 85.50 |
| refs | rmc | 1.78 | 1.65 | 24 |
| refs | rustc | 3.85 | 3.72 | 2028 |
| refs | ratio (rustc/rmc) | 2.17 | 2.25 | 84.50 |
| heap | rmc | 3.24 | 3.11 | 4708 |
| heap | rustc | 3.51 | 3.40 | 6692 |
| heap | ratio (rustc/rmc) | 1.08 | 1.09 | 1.42 |
| callrec | rmc | 4.81 | 4.66 | 24 |
| callrec | rustc | 2.89 | 2.76 | 2004 |
| callrec | ratio (rustc/rmc) | 0.60 | 0.59 | 83.50 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

Run time is not uniformly in rmc's favour. On loop-heavy integer code
rmc is competitive, and on `refs` it wins by more than 2x because LICM
reassociates the loop-invariant part of the accumulation into one
hoisted value. On call-heavy code (`callrec`) it *loses*: a
stack-based generator with no register allocator pays the argument
setup and register save/restore on every call that LLVM avoids. The
per-program ratio above is the honest answer; there is no single
speedup factor.

Run-time ratios are also noisy and machine-dependent. Programs near
parity (notably `mandel`) flip between a small rmc win and a small rmc
loss from run to run and from host to host - a single recorded ratio
for such a program should not be read as a stable property. Treat the
run column as indicative, and re-run on the target machine before
drawing a conclusion about a specific workload.

Peak RSS for a compiled rmc binary is a few kB because the binary
links no libc and is mapped by the kernel directly. A dynamically
linked rustc binary pulls in ld.so and libc, so its RSS floors at a
couple of MB regardless of what the program does. That gap is real but
is mostly the C library's footprint, not the generated code's.
