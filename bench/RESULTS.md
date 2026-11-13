# Benchmark results: rmc vs rustc (LLVM) vs rustc (Cranelift)

Recorded 2026-11-13 05:09 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.
Cranelift backend: `rustc 1.102.0-nightly (8eb778d04 2026-11-12)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

The rustc (LLVM) column is an aggressive configuration (`-O` with fat
LTO, `codegen-units=1`, `panic=abort`, stripped). That is the right
target for *compile cost* - it is the slowest rustc build - but it is
the *smallest* rustc artifact, so the size ratio here understates the
gap against a default build. See the size table below for both.

The rustc (Cranelift) column is the fair comparison for the
no-LLVM claim: it runs the same rustc frontend rmc is measured
against, but a backend that also does not use LLVM. Any gap between
it and rmc is rmc's own codegen and link step, not the optimiser.
Cranelift rejects `lto`, so that flag is omitted; the rest of the
flag set matches the LLVM column. It is a nightly-only component,
so the column is present only where the host has it installed.

Every compiler is invoked with matching safety semantics: rmc with
bounds checks on (the default; Rust's release semantics) and rustc with
`-C overflow-checks=off` (already the `-O` default, stated explicitly).
See the run section for why this matters.

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 1.71 | 1.58 | 3308 | 495 |
| hello | rustc | 2079.79 | 2082.18 | 187224 | 289416 |
| hello | cranelift | 51.91 | 57.53 | 88608 | 337328 |
| hello | ratio (rustc/rmc) | 1214.6 | 1314.5 | 56.6 | 584.7 |
| hello | ratio (cranelift/rmc) | 30.3 | 36.3 | 26.8 | 681.5 |
| fib | rmc | 1.79 | 1.68 | 3292 | 692 |
| fib | rustc | 2072.75 | 2077.05 | 188088 | 291696 |
| fib | cranelift | 64.56 | 71.41 | 106744 | 347152 |
| fib | ratio (rustc/rmc) | 1156.2 | 1239.3 | 57.1 | 421.5 |
| fib | ratio (cranelift/rmc) | 36.0 | 42.6 | 32.4 | 501.7 |
| mandel | rmc | 1.97 | 1.84 | 3312 | 1041 |
| mandel | rustc | 2111.94 | 2116.43 | 190268 | 291776 |
| mandel | cranelift | 65.00 | 71.46 | 107532 | 347248 |
| mandel | ratio (rustc/rmc) | 1073.9 | 1147.7 | 57.4 | 280.3 |
| mandel | ratio (cranelift/rmc) | 33.1 | 38.8 | 32.5 | 333.6 |
| sieve | rmc | 2.03 | 1.90 | 3248 | 953 |
| sieve | rustc | 2070.68 | 2074.54 | 189708 | 292032 |
| sieve | cranelift | 65.76 | 72.01 | 107336 | 347504 |
| sieve | ratio (rustc/rmc) | 1020.5 | 1090.7 | 58.4 | 306.4 |
| sieve | ratio (cranelift/rmc) | 32.4 | 37.9 | 33.0 | 364.6 |
| mix | rmc | 1.91 | 1.77 | 3272 | 941 |
| mix | rustc | 2060.27 | 2064.75 | 192180 | 291880 |
| mix | cranelift | 64.79 | 71.24 | 108168 | 347592 |
| mix | ratio (rustc/rmc) | 1079.9 | 1165.9 | 58.7 | 310.2 |
| mix | ratio (cranelift/rmc) | 34.0 | 40.2 | 33.1 | 369.4 |
| stats | rmc | 1.90 | 1.78 | 3276 | 924 |
| stats | rustc | 2097.28 | 2101.61 | 189388 | 292048 |
| stats | cranelift | 65.23 | 72.27 | 108268 | 347744 |
| stats | ratio (rustc/rmc) | 1106.0 | 1182.0 | 57.8 | 316.1 |
| stats | ratio (cranelift/rmc) | 34.4 | 40.6 | 33.0 | 376.3 |
| refs | rmc | 2.12 | 1.98 | 3352 | 1115 |
| refs | rustc | 2086.76 | 2091.03 | 187712 | 292512 |
| refs | cranelift | 66.63 | 73.46 | 109344 | 348128 |
| refs | ratio (rustc/rmc) | 985.9 | 1055.0 | 56.0 | 262.3 |
| refs | ratio (cranelift/rmc) | 31.5 | 37.1 | 32.6 | 312.2 |
| heap | rmc | 2.07 | 1.91 | 3368 | 1553 |
| heap | rustc | 2080.66 | 2082.02 | 192144 | 292384 |
| heap | cranelift | 66.99 | 73.90 | 110408 | 350568 |
| heap | ratio (rustc/rmc) | 1002.8 | 1088.9 | 57.0 | 188.3 |
| heap | ratio (cranelift/rmc) | 32.3 | 38.7 | 32.8 | 225.7 |
| callrec | rmc | 1.75 | 1.63 | 3304 | 695 |
| callrec | rustc | 2062.58 | 2066.75 | 187536 | 291680 |
| callrec | cranelift | 64.63 | 71.50 | 106896 | 347152 |
| callrec | ratio (rustc/rmc) | 1180.1 | 1266.4 | 56.8 | 419.7 |
| callrec | ratio (cranelift/rmc) | 37.0 | 43.8 | 32.4 | 499.5 |
| forloops | rmc | 1.83 | 1.72 | 3312 | 851 |
| forloops | rustc | 2063.19 | 2067.54 | 188380 | 292480 |
| forloops | cranelift | 66.60 | 73.40 | 109068 | 348136 |
| forloops | ratio (rustc/rmc) | 1126.0 | 1204.2 | 56.9 | 343.7 |
| forloops | ratio (cranelift/rmc) | 36.3 | 42.7 | 32.9 | 409.1 |

## Artifact size against each rustc build

| program | rmc (B) | rustc LLVM fat-LTO stripped (B) | rustc Cranelift stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | -------------------------------: | ---------------------------: | -----------------: | ---------------------------: |
| hello | 495 | 289416 | 337328 | 4537096 | 1180416 |
| fib | 692 | 291696 | 347152 | 4541112 | 1183104 |
| mandel | 1041 | 291776 | 347248 | 4541208 | 1183168 |
| sieve | 953 | 292032 | 347504 | 4541424 | 1183392 |
| mix | 941 | 291880 | 347592 | 4541432 | 1183328 |
| stats | 924 | 292048 | 347744 | 4541392 | 1183360 |
| refs | 1115 | 292512 | 348128 | 4542248 | 1183904 |
| heap | 1553 | 292384 | 350568 | 4542176 | 1183776 |
| callrec | 695 | 291680 | 347152 | 4541144 | 1183104 |
| forloops | 851 | 292480 | 348136 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.22 | 0.12 | 24 |
| hello | rustc | 0.84 | 0.70 | 2016 |
| hello | cranelift | 0.83 | 0.71 | 2196 |
| hello | ratio (rustc/rmc) | 3.79 | 5.80 | 84.00 |
| hello | ratio (cranelift/rmc) | 3.74 | 5.88 | 91.50 |
| fib | rmc | 7.17 | 7.05 | 24 |
| fib | rustc | 7.10 | 6.97 | 2076 |
| fib | cranelift | 17.28 | 17.13 | 2156 |
| fib | ratio (rustc/rmc) | 0.99 | 0.99 | 86.50 |
| fib | ratio (cranelift/rmc) | 2.41 | 2.43 | 89.83 |
| mandel | rmc | 95.89 | 95.70 | 24 |
| mandel | rustc | 105.34 | 105.15 | 2056 |
| mandel | cranelift | 110.28 | 110.08 | 2208 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 85.67 |
| mandel | ratio (cranelift/rmc) | 1.15 | 1.15 | 92.00 |
| sieve | rmc | 1.82 | 1.70 | 1584 |
| sieve | rustc | 2.40 | 2.27 | 3620 |
| sieve | cranelift | 2.64 | 2.51 | 3772 |
| sieve | ratio (rustc/rmc) | 1.32 | 1.34 | 2.29 |
| sieve | ratio (cranelift/rmc) | 1.45 | 1.48 | 2.38 |
| mix | rmc | 14.83 | 14.69 | 24 |
| mix | rustc | 15.43 | 15.29 | 2064 |
| mix | cranelift | 17.78 | 17.59 | 2200 |
| mix | ratio (rustc/rmc) | 1.04 | 1.04 | 86.00 |
| mix | ratio (cranelift/rmc) | 1.20 | 1.20 | 91.67 |
| stats | rmc | 3.25 | 3.12 | 24 |
| stats | rustc | 3.73 | 3.60 | 2012 |
| stats | cranelift | 4.48 | 4.36 | 2160 |
| stats | ratio (rustc/rmc) | 1.15 | 1.15 | 83.83 |
| stats | ratio (cranelift/rmc) | 1.38 | 1.40 | 90.00 |
| refs | rmc | 3.22 | 3.10 | 24 |
| refs | rustc | 5.47 | 5.33 | 2092 |
| refs | cranelift | 6.82 | 6.68 | 2168 |
| refs | ratio (rustc/rmc) | 1.70 | 1.72 | 87.17 |
| refs | ratio (cranelift/rmc) | 2.12 | 2.15 | 90.33 |
| heap | rmc | 3.23 | 3.09 | 4712 |
| heap | rustc | 3.33 | 3.21 | 6624 |
| heap | cranelift | 3.42 | 3.29 | 6804 |
| heap | ratio (rustc/rmc) | 1.03 | 1.04 | 1.41 |
| heap | ratio (cranelift/rmc) | 1.06 | 1.06 | 1.44 |
| callrec | rmc | 2.88 | 2.76 | 24 |
| callrec | rustc | 2.82 | 2.69 | 2004 |
| callrec | cranelift | 7.11 | 6.95 | 2212 |
| callrec | ratio (rustc/rmc) | 0.98 | 0.98 | 83.50 |
| callrec | ratio (cranelift/rmc) | 2.47 | 2.52 | 92.17 |
| forloops | rmc | 1.45 | 1.35 | 24 |
| forloops | rustc | 3.33 | 3.21 | 2076 |
| forloops | cranelift | 7.26 | 7.11 | 2168 |
| forloops | ratio (rustc/rmc) | 2.29 | 2.38 | 86.50 |
| forloops | ratio (cranelift/rmc) | 4.99 | 5.28 | 90.33 |

Ratios are the other compiler divided by rmc, so a value above 1 means
rmc is better on that dimension. Both the LLVM and the Cranelift ratios
use rmc as the denominator.

The Cranelift run column is the one to read when asking whether rmc's
*generated code* is good, as opposed to how fast it compiles. LLVM is
a much stronger optimiser than either rmc or Cranelift, so a run-time
loss to LLVM may only mean LLVM optimised harder. Where rmc and
Cranelift are close on run time, rmc's codegen is doing its job; where
Cranelift is clearly faster, rmc is leaving performance on the table.

Run time is not uniformly in rmc's favour, and a ratio below 1 is
possible. On loop-heavy integer code rmc is competitive: it wins on
`forloops` because the rotated `for` compares the counter against the
bound directly, and on `refs` because the accessor sum is loop-invariant
and rmc's LICM hoists a read through a reference once a pre-pass proves
the referent is a live local that the loop does not write.

Two benchmark shapes were changed to make the run comparison fair, and
both changes are worth knowing when reading the table. First, no program
uses `black_box` inside a hot loop: rmc lowers it to the identity while
rustc makes it opaque, so an in-loop `black_box` would charge rustc a
store/reload rmc never emits. `refs` instead seeds the struct from an
opaque value and reduces the accumulator with a data-dependent branch,
which keeps the loop real for both compilers (it defeats LLVM's
loop-closing and vectoriser) without a barrier only one side pays for.
Second, the two builds use matching safety semantics: rmc checks bounds
by default and rustc is invoked with `-C overflow-checks=off`, so both
carry bounds checks and neither carries overflow checks. The check costs
about 1% on the array-heavy programs, so it does not manufacture any win
either way.
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
