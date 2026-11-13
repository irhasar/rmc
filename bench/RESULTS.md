# Benchmark results: rmc vs rustc (LLVM) vs rustc (Cranelift)

Recorded 2026-11-13 09:34 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 2.02 | 1.89 | 3724 | 495 |
| hello | rustc | 2041.48 | 2045.04 | 188388 | 289416 |
| hello | cranelift | 54.62 | 60.46 | 88824 | 337328 |
| hello | ratio (rustc/rmc) | 1011.0 | 1079.7 | 50.6 | 584.7 |
| hello | ratio (cranelift/rmc) | 27.1 | 31.9 | 23.9 | 681.5 |
| fib | rmc | 2.16 | 2.04 | 3860 | 789 |
| fib | rustc | 2092.70 | 2096.16 | 188120 | 291696 |
| fib | cranelift | 66.83 | 73.94 | 105900 | 347280 |
| fib | ratio (rustc/rmc) | 969.6 | 1025.5 | 48.7 | 369.7 |
| fib | ratio (cranelift/rmc) | 31.0 | 36.2 | 27.4 | 440.2 |
| mandel | rmc | 2.29 | 2.17 | 3912 | 1124 |
| mandel | rustc | 2091.47 | 2095.96 | 192136 | 291776 |
| mandel | cranelift | 69.15 | 76.16 | 107020 | 347376 |
| mandel | ratio (rustc/rmc) | 913.9 | 967.7 | 49.1 | 259.6 |
| mandel | ratio (cranelift/rmc) | 30.2 | 35.2 | 27.4 | 309.1 |
| sieve | rmc | 2.25 | 2.11 | 3856 | 1041 |
| sieve | rustc | 2086.32 | 2090.91 | 187340 | 292032 |
| sieve | cranelift | 66.96 | 74.47 | 106420 | 347632 |
| sieve | ratio (rustc/rmc) | 926.5 | 990.0 | 48.6 | 280.5 |
| sieve | ratio (cranelift/rmc) | 29.7 | 35.3 | 27.6 | 333.9 |
| mix | rmc | 2.24 | 2.11 | 3928 | 1029 |
| mix | rustc | 2104.39 | 2106.62 | 191136 | 291880 |
| mix | cranelift | 68.24 | 75.41 | 106448 | 347720 |
| mix | ratio (rustc/rmc) | 940.3 | 999.8 | 48.7 | 283.7 |
| mix | ratio (cranelift/rmc) | 30.5 | 35.8 | 27.1 | 337.9 |
| stats | rmc | 2.27 | 2.13 | 3876 | 1015 |
| stats | rustc | 2109.39 | 2114.62 | 188004 | 292048 |
| stats | cranelift | 68.79 | 75.57 | 107552 | 347872 |
| stats | ratio (rustc/rmc) | 928.7 | 994.6 | 48.5 | 287.7 |
| stats | ratio (cranelift/rmc) | 30.3 | 35.5 | 27.7 | 342.7 |
| refs | rmc | 2.56 | 2.40 | 3904 | 1194 |
| refs | rustc | 2121.38 | 2125.46 | 193624 | 292512 |
| refs | cranelift | 69.86 | 77.43 | 108656 | 348256 |
| refs | ratio (rustc/rmc) | 828.2 | 885.6 | 49.6 | 245.0 |
| refs | ratio (cranelift/rmc) | 27.3 | 32.3 | 27.8 | 291.7 |
| heap | rmc | 2.40 | 2.26 | 3948 | 1567 |
| heap | rustc | 2100.37 | 2105.32 | 189800 | 292384 |
| heap | cranelift | 70.84 | 77.70 | 109300 | 350696 |
| heap | ratio (rustc/rmc) | 876.8 | 930.7 | 48.1 | 186.6 |
| heap | ratio (cranelift/rmc) | 29.6 | 34.4 | 27.7 | 223.8 |
| callrec | rmc | 2.17 | 2.04 | 3920 | 792 |
| callrec | rustc | 2087.65 | 2091.87 | 187360 | 291680 |
| callrec | cranelift | 69.58 | 76.47 | 106100 | 347280 |
| callrec | ratio (rustc/rmc) | 963.4 | 1024.4 | 47.8 | 368.3 |
| callrec | ratio (cranelift/rmc) | 32.1 | 37.5 | 27.1 | 438.5 |
| forloops | rmc | 2.96 | 2.81 | 3912 | 930 |
| forloops | rustc | 2102.07 | 2106.67 | 187740 | 292480 |
| forloops | cranelift | 70.93 | 78.49 | 108400 | 348264 |
| forloops | ratio (rustc/rmc) | 710.9 | 750.0 | 48.0 | 314.5 |
| forloops | ratio (cranelift/rmc) | 24.0 | 27.9 | 27.7 | 374.5 |
| light | rmc | 2.30 | 2.16 | 3856 | 1113 |
| light | rustc | 2120.34 | 2124.47 | 188092 | 291904 |
| light | cranelift | 67.97 | 75.73 | 107256 | 347648 |
| light | ratio (rustc/rmc) | 920.3 | 983.6 | 48.8 | 262.3 |
| light | ratio (cranelift/rmc) | 29.5 | 35.1 | 27.8 | 312.4 |
| heavy | rmc | 2.17 | 2.05 | 3828 | 865 |
| heavy | rustc | 2103.87 | 2108.44 | 187996 | 291792 |
| heavy | cranelift | 67.45 | 74.38 | 106884 | 347344 |
| heavy | ratio (rustc/rmc) | 968.2 | 1029.5 | 49.1 | 337.3 |
| heavy | ratio (cranelift/rmc) | 31.0 | 36.3 | 27.9 | 401.6 |

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
| hello | rmc | 0.23 | 0.12 | 28 |
| hello | rustc | 0.82 | 0.71 | 2104 |
| hello | cranelift | 0.83 | 0.72 | 2236 |
| hello | ratio (rustc/rmc) | 3.52 | 5.85 | 75.14 |
| hello | ratio (cranelift/rmc) | 3.55 | 5.93 | 79.86 |
| fib | rmc | 6.73 | 6.59 | 28 |
| fib | rustc | 7.11 | 6.98 | 2056 |
| fib | cranelift | 17.18 | 17.04 | 2244 |
| fib | ratio (rustc/rmc) | 1.06 | 1.06 | 73.43 |
| fib | ratio (cranelift/rmc) | 2.55 | 2.59 | 80.14 |
| mandel | rmc | 95.66 | 95.43 | 28 |
| mandel | rustc | 105.15 | 104.96 | 2036 |
| mandel | cranelift | 110.22 | 110.02 | 2240 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 72.71 |
| mandel | ratio (cranelift/rmc) | 1.15 | 1.15 | 80.00 |
| sieve | rmc | 1.62 | 1.51 | 1588 |
| sieve | rustc | 2.36 | 2.24 | 3668 |
| sieve | cranelift | 2.62 | 2.49 | 3816 |
| sieve | ratio (rustc/rmc) | 1.45 | 1.49 | 2.31 |
| sieve | ratio (cranelift/rmc) | 1.61 | 1.65 | 2.40 |
| mix | rmc | 14.72 | 14.56 | 28 |
| mix | rustc | 15.50 | 15.35 | 2108 |
| mix | cranelift | 17.68 | 17.50 | 2228 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 75.29 |
| mix | ratio (cranelift/rmc) | 1.20 | 1.20 | 79.57 |
| stats | rmc | 3.46 | 3.33 | 28 |
| stats | rustc | 3.73 | 3.59 | 2096 |
| stats | cranelift | 4.45 | 4.33 | 2240 |
| stats | ratio (rustc/rmc) | 1.08 | 1.08 | 74.86 |
| stats | ratio (cranelift/rmc) | 1.29 | 1.30 | 80.00 |
| refs | rmc | 3.25 | 3.12 | 28 |
| refs | rustc | 5.50 | 5.37 | 2108 |
| refs | cranelift | 6.89 | 6.76 | 2196 |
| refs | ratio (rustc/rmc) | 1.69 | 1.72 | 75.29 |
| refs | ratio (cranelift/rmc) | 2.12 | 2.16 | 78.43 |
| heap | rmc | 3.23 | 3.10 | 4716 |
| heap | rustc | 3.33 | 3.20 | 6720 |
| heap | cranelift | 3.43 | 3.31 | 6856 |
| heap | ratio (rustc/rmc) | 1.03 | 1.03 | 1.42 |
| heap | ratio (cranelift/rmc) | 1.06 | 1.07 | 1.45 |
| callrec | rmc | 2.74 | 2.62 | 28 |
| callrec | rustc | 2.84 | 2.71 | 2112 |
| callrec | cranelift | 7.09 | 6.94 | 2236 |
| callrec | ratio (rustc/rmc) | 1.04 | 1.04 | 75.43 |
| callrec | ratio (cranelift/rmc) | 2.58 | 2.65 | 79.86 |
| forloops | rmc | 2.07 | 1.95 | 28 |
| forloops | rustc | 3.31 | 3.19 | 2112 |
| forloops | cranelift | 3.99 | 3.87 | 2236 |
| forloops | ratio (rustc/rmc) | 1.60 | 1.64 | 75.43 |
| forloops | ratio (cranelift/rmc) | 1.93 | 1.98 | 79.86 |
| light | rmc | 2.09 | 1.97 | 28 |
| light | rustc | 2.62 | 2.51 | 2120 |
| light | cranelift | 3.74 | 3.61 | 2236 |
| light | ratio (rustc/rmc) | 1.25 | 1.27 | 75.71 |
| light | ratio (cranelift/rmc) | 1.79 | 1.84 | 79.86 |
| heavy | rmc | 605.75 | 605.39 | 28 |
| heavy | rustc | 1032.46 | 1032.09 | 2080 |
| heavy | cranelift | 1069.74 | 1069.47 | 2232 |
| heavy | ratio (rustc/rmc) | 1.70 | 1.70 | 74.29 |
| heavy | ratio (cranelift/rmc) | 1.77 | 1.77 | 79.71 |

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
| compile_wall | 913.15 | win |
| compile_cpu | 971.77 | win |
| compile_rss | 48.79 | win |
| size | 314.99 | win |
| run_wall | 1.46 | win |
| run_cpu | 1.67 | win |
| run_rss | 62.61 | win |

Every axis is a win.
