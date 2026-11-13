# Benchmark results: rmc vs rustc (LLVM) vs rustc (Cranelift)

Recorded 2026-11-13 06:24 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.
Cranelift backend: `rustc 1.102.0-nightly (3ef0c22cd 2026-11-11)`.

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
`--bounds-checks` (bounds checks on, overflow checks off - Rust's
release semantics, which rmc does not default to) and rustc with
`-C overflow-checks=off` (already the `-O` default, stated explicitly).
See the run section for why this matters.

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 2.06 | 1.94 | 3812 | 495 |
| hello | rustc | 2055.33 | 2059.12 | 188496 | 289416 |
| hello | cranelift | 54.31 | 59.84 | 88796 | 337328 |
| hello | ratio (rustc/rmc) | 998.1 | 1059.8 | 49.4 | 584.7 |
| hello | ratio (cranelift/rmc) | 26.4 | 30.8 | 23.3 | 681.5 |
| fib | rmc | 2.17 | 2.02 | 3900 | 789 |
| fib | rustc | 2142.07 | 2145.19 | 191596 | 291696 |
| fib | cranelift | 67.36 | 74.72 | 105980 | 347280 |
| fib | ratio (rustc/rmc) | 985.1 | 1064.1 | 49.1 | 369.7 |
| fib | ratio (cranelift/rmc) | 31.0 | 37.1 | 27.2 | 440.2 |
| mandel | rmc | 2.43 | 2.25 | 3936 | 1124 |
| mandel | rustc | 2158.21 | 2161.38 | 192524 | 291776 |
| mandel | cranelift | 66.79 | 74.36 | 106496 | 347376 |
| mandel | ratio (rustc/rmc) | 888.5 | 961.0 | 48.9 | 259.6 |
| mandel | ratio (cranelift/rmc) | 27.5 | 33.1 | 27.1 | 309.1 |
| sieve | rmc | 2.26 | 2.12 | 3852 | 1041 |
| sieve | rustc | 2138.68 | 2142.87 | 191028 | 292032 |
| sieve | cranelift | 69.63 | 76.75 | 106328 | 347632 |
| sieve | ratio (rustc/rmc) | 944.4 | 1009.4 | 49.6 | 280.5 |
| sieve | ratio (cranelift/rmc) | 30.7 | 36.2 | 27.6 | 333.9 |
| mix | rmc | 2.39 | 2.25 | 3920 | 1029 |
| mix | rustc | 2090.84 | 2095.10 | 187356 | 291880 |
| mix | cranelift | 68.00 | 75.05 | 106452 | 347720 |
| mix | ratio (rustc/rmc) | 873.8 | 932.0 | 47.8 | 283.7 |
| mix | ratio (cranelift/rmc) | 28.4 | 33.4 | 27.2 | 337.9 |
| stats | rmc | 2.29 | 2.15 | 3880 | 1015 |
| stats | rustc | 2127.10 | 2131.64 | 187136 | 292048 |
| stats | cranelift | 68.12 | 75.59 | 107432 | 347872 |
| stats | ratio (rustc/rmc) | 929.2 | 992.4 | 48.2 | 287.7 |
| stats | ratio (cranelift/rmc) | 29.8 | 35.2 | 27.7 | 342.7 |
| refs | rmc | 2.47 | 2.33 | 3864 | 1194 |
| refs | rustc | 2130.16 | 2134.80 | 191600 | 292512 |
| refs | cranelift | 69.13 | 76.35 | 108624 | 348256 |
| refs | ratio (rustc/rmc) | 862.1 | 915.0 | 49.6 | 245.0 |
| refs | ratio (cranelift/rmc) | 28.0 | 32.7 | 28.1 | 291.7 |
| heap | rmc | 2.44 | 2.29 | 3944 | 1567 |
| heap | rustc | 2123.08 | 2126.74 | 192844 | 292384 |
| heap | cranelift | 69.87 | 77.27 | 109248 | 350696 |
| heap | ratio (rustc/rmc) | 869.4 | 929.9 | 48.9 | 186.6 |
| heap | ratio (cranelift/rmc) | 28.6 | 33.8 | 27.7 | 223.8 |
| callrec | rmc | 2.14 | 2.02 | 3904 | 792 |
| callrec | rustc | 2146.99 | 2150.57 | 187136 | 291680 |
| callrec | cranelift | 66.68 | 73.72 | 106160 | 347280 |
| callrec | ratio (rustc/rmc) | 1002.5 | 1062.5 | 47.9 | 368.3 |
| callrec | ratio (cranelift/rmc) | 31.1 | 36.4 | 27.2 | 438.5 |
| forloops | rmc | 2.22 | 2.10 | 3908 | 930 |
| forloops | rustc | 2130.56 | 2132.09 | 192488 | 292480 |
| forloops | cranelift | 70.24 | 77.17 | 108188 | 348264 |
| forloops | ratio (rustc/rmc) | 959.7 | 1013.3 | 49.3 | 314.5 |
| forloops | ratio (cranelift/rmc) | 31.6 | 36.7 | 27.7 | 374.5 |
| light | rmc | 2.24 | 2.12 | 3892 | 1113 |
| light | rustc | 2077.87 | 2082.34 | 187500 | 291904 |
| light | cranelift | 67.94 | 75.34 | 107256 | 347648 |
| light | ratio (rustc/rmc) | 926.5 | 984.1 | 48.2 | 262.3 |
| light | ratio (cranelift/rmc) | 30.3 | 35.6 | 27.6 | 312.4 |
| heavy | rmc | 2.24 | 2.09 | 3888 | 865 |
| heavy | rustc | 2133.43 | 2135.75 | 187164 | 291792 |
| heavy | cranelift | 67.89 | 75.37 | 106412 | 347344 |
| heavy | ratio (rustc/rmc) | 953.8 | 1019.9 | 48.1 | 337.3 |
| heavy | ratio (cranelift/rmc) | 30.4 | 36.0 | 27.4 | 401.6 |

## Artifact size against each rustc build

| program | rmc (B) | rustc LLVM fat-LTO stripped (B) | rustc Cranelift stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | -------------------------------: | ---------------------------: | -----------------: | ---------------------------: |
| hello | 495 | 289416 | 337328 | 4537096 | 1180416 |
| fib | 789 | 291696 | 347280 | 4541112 | 1183104 |
| mandel | 1124 | 291776 | 347376 | 4541208 | 1183168 |
| sieve | 1041 | 292032 | 347632 | 4541424 | 1183392 |
| mix | 1029 | 291880 | 347720 | 4541432 | 1183328 |
| stats | 1015 | 292048 | 347872 | 4541392 | 1183360 |
| refs | 1194 | 292512 | 348256 | 4542248 | 1183904 |
| heap | 1567 | 292384 | 350696 | 4542176 | 1183776 |
| callrec | 792 | 291680 | 347280 | 4541144 | 1183104 |
| forloops | 930 | 292480 | 348264 | 4541808 | 1183808 |
| light | 1113 | 291904 | 347648 | 4541264 | 1183232 |
| heavy | 865 | 291792 | 347344 | 4541120 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.23 | 0.12 | 24 |
| hello | rustc | 0.81 | 0.70 | 2064 |
| hello | cranelift | 0.83 | 0.71 | 2232 |
| hello | ratio (rustc/rmc) | 3.53 | 5.85 | 86.00 |
| hello | ratio (cranelift/rmc) | 3.62 | 5.94 | 93.00 |
| fib | rmc | 6.79 | 6.63 | 24 |
| fib | rustc | 7.12 | 6.98 | 2108 |
| fib | cranelift | 17.22 | 17.07 | 2244 |
| fib | ratio (rustc/rmc) | 1.05 | 1.05 | 87.83 |
| fib | ratio (cranelift/rmc) | 2.54 | 2.57 | 93.50 |
| mandel | rmc | 95.77 | 95.53 | 24 |
| mandel | rustc | 105.22 | 104.99 | 2088 |
| mandel | cranelift | 110.17 | 109.95 | 2244 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 87.00 |
| mandel | ratio (cranelift/rmc) | 1.15 | 1.15 | 93.50 |
| sieve | rmc | 1.64 | 1.53 | 1584 |
| sieve | rustc | 2.41 | 2.29 | 3676 |
| sieve | cranelift | 2.65 | 2.54 | 3800 |
| sieve | ratio (rustc/rmc) | 1.47 | 1.50 | 2.32 |
| sieve | ratio (cranelift/rmc) | 1.62 | 1.66 | 2.40 |
| mix | rmc | 14.74 | 14.58 | 24 |
| mix | rustc | 15.52 | 15.36 | 2112 |
| mix | cranelift | 17.71 | 17.55 | 2240 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 88.00 |
| mix | ratio (cranelift/rmc) | 1.20 | 1.20 | 93.33 |
| stats | rmc | 3.47 | 3.34 | 24 |
| stats | rustc | 3.78 | 3.63 | 2068 |
| stats | cranelift | 4.47 | 4.32 | 2228 |
| stats | ratio (rustc/rmc) | 1.09 | 1.09 | 86.17 |
| stats | ratio (cranelift/rmc) | 1.29 | 1.29 | 92.83 |
| refs | rmc | 3.27 | 3.12 | 24 |
| refs | rustc | 5.50 | 5.37 | 2116 |
| refs | cranelift | 6.87 | 6.73 | 2188 |
| refs | ratio (rustc/rmc) | 1.68 | 1.72 | 88.17 |
| refs | ratio (cranelift/rmc) | 2.10 | 2.15 | 91.17 |
| heap | rmc | 3.22 | 3.09 | 4712 |
| heap | rustc | 3.34 | 3.20 | 6716 |
| heap | cranelift | 3.42 | 3.29 | 6852 |
| heap | ratio (rustc/rmc) | 1.04 | 1.04 | 1.43 |
| heap | ratio (cranelift/rmc) | 1.06 | 1.06 | 1.45 |
| callrec | rmc | 2.77 | 2.64 | 24 |
| callrec | rustc | 2.83 | 2.69 | 2116 |
| callrec | cranelift | 7.16 | 7.01 | 2240 |
| callrec | ratio (rustc/rmc) | 1.02 | 1.02 | 88.17 |
| callrec | ratio (cranelift/rmc) | 2.59 | 2.65 | 93.33 |
| forloops | rmc | 2.08 | 1.96 | 24 |
| forloops | rustc | 3.32 | 3.19 | 2044 |
| forloops | cranelift | 3.99 | 3.86 | 2212 |
| forloops | ratio (rustc/rmc) | 1.59 | 1.63 | 85.17 |
| forloops | ratio (cranelift/rmc) | 1.92 | 1.97 | 92.17 |
| light | rmc | 2.13 | 2.01 | 24 |
| light | rustc | 2.62 | 2.50 | 2068 |
| light | cranelift | 3.72 | 3.58 | 2224 |
| light | ratio (rustc/rmc) | 1.23 | 1.25 | 86.17 |
| light | ratio (cranelift/rmc) | 1.75 | 1.79 | 92.67 |
| heavy | rmc | 606.02 | 605.82 | 24 |
| heavy | rustc | 1031.99 | 1031.71 | 2104 |
| heavy | cranelift | 1065.56 | 1065.28 | 2240 |
| heavy | ratio (rustc/rmc) | 1.70 | 1.70 | 87.67 |
| heavy | ratio (cranelift/rmc) | 1.76 | 1.76 | 93.33 |

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
Second, the two builds use matching safety semantics: rmc is invoked
with `--bounds-checks` and rustc with `-C overflow-checks=off`, so both
carry bounds checks and neither carries overflow checks. rmc defaults
bounds checks off, so without the flag it would skip a per-index check
rustc emits; the cost of the check is around 1% on the array-heavy
programs, so it does not manufacture any win either way.
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

## Verdict

This is the end-to-end comparison: rmc's own front end *and* back end
against rustc/LLVM, one source each. There is no shared front end to
excuse a tie here, so a tie is a real limit of the whole compiler.
Mean ratio per axis, rustc divided by rmc (> 1 means rmc better):

| axis | mean rustc/rmc | verdict |
| ---- | -------------: | ------- |
| compile_wall | 932.76 | win |
| compile_cpu | 995.29 | win |
| compile_rss | 48.76 | win |
| size | 314.99 | win |
| run_wall | 1.46 | win |
| run_cpu | 1.67 | win |
| run_rss | 72.84 | win |

Every axis is a win.
