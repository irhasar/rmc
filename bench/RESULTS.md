# Benchmark results: rmc vs rustc

Recorded 2026-11-12 04:49 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
CPU: AMD EPYC 9B14.
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`.

Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.

## Compile: build time, CPU, RAM, artifact size

The rustc column is an aggressive configuration (`-O` with fat LTO,
`codegen-units=1`, `panic=abort`, stripped). That is the right target
for *compile cost* - it is the slowest rustc build - but it is the
*smallest* rustc artifact, so the size ratio here understates the gap
against a default build. See the size table below for both.

Both compilers are invoked with matching safety semantics: rmc with
`--bounds-checks` (bounds checks on, overflow checks off - Rust's
release semantics, which rmc does not default to) and rustc with
`-C overflow-checks=off` (already the `-O` default, stated explicitly).
See the run section for why this matters.

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |
| ------- | -------- | --------: | -------: | ------------: | -----------: |
| hello | rmc | 1.64 | 1.50 | 3500 | 503 |
| hello | rustc | 2095.43 | 2098.78 | 188100 | 289416 |
| hello | ratio (rustc/rmc) | 1278.6 | 1397.3 | 53.7 | 575.4 |
| fib | rmc | 1.78 | 1.66 | 3608 | 870 |
| fib | rustc | 2118.09 | 2122.17 | 188844 | 291696 |
| fib | ratio (rustc/rmc) | 1191.5 | 1280.0 | 52.3 | 335.3 |
| mandel | rmc | 1.90 | 1.77 | 3552 | 1209 |
| mandel | rustc | 2103.61 | 2108.17 | 189136 | 291776 |
| mandel | ratio (rustc/rmc) | 1107.9 | 1193.8 | 53.2 | 241.3 |
| sieve | rmc | 1.81 | 1.70 | 3708 | 1137 |
| sieve | rustc | 2122.53 | 2124.72 | 190660 | 292032 |
| sieve | ratio (rustc/rmc) | 1169.8 | 1251.3 | 51.4 | 256.8 |
| mix | rmc | 1.82 | 1.70 | 3736 | 1117 |
| mix | rustc | 2127.58 | 2131.66 | 190980 | 291880 |
| mix | ratio (rustc/rmc) | 1168.1 | 1256.1 | 51.1 | 261.3 |
| stats | rmc | 1.85 | 1.71 | 3564 | 1104 |
| stats | rustc | 2119.54 | 2124.07 | 186796 | 292048 |
| stats | ratio (rustc/rmc) | 1147.2 | 1242.9 | 52.4 | 264.5 |
| refs | rmc | 2.02 | 1.90 | 3708 | 1279 |
| refs | rustc | 2142.45 | 2146.99 | 191908 | 292512 |
| refs | ratio (rustc/rmc) | 1059.0 | 1130.0 | 51.8 | 228.7 |
| heap | rmc | 1.91 | 1.79 | 3644 | 1335 |
| heap | rustc | 2110.59 | 2115.44 | 189408 | 292384 |
| heap | ratio (rustc/rmc) | 1107.4 | 1183.1 | 52.0 | 219.0 |
| callrec | rmc | 1.83 | 1.69 | 3684 | 873 |
| callrec | rustc | 2124.70 | 2128.50 | 189468 | 291680 |
| callrec | ratio (rustc/rmc) | 1157.9 | 1261.0 | 51.4 | 334.1 |
| forloops | rmc | 1.81 | 1.69 | 3616 | 1011 |
| forloops | rustc | 2132.46 | 2134.87 | 188008 | 292480 |
| forloops | ratio (rustc/rmc) | 1176.8 | 1266.2 | 52.0 | 289.3 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 503 | 289416 | 4537096 | 1180416 |
| fib | 870 | 291696 | 4541112 | 1183104 |
| mandel | 1209 | 291776 | 4541208 | 1183168 |
| sieve | 1137 | 292032 | 4541424 | 1183392 |
| mix | 1117 | 291880 | 4541432 | 1183328 |
| stats | 1104 | 292048 | 4541392 | 1183360 |
| refs | 1279 | 292512 | 4542248 | 1183904 |
| heap | 1335 | 292384 | 4542176 | 1183776 |
| callrec | 873 | 291680 | 4541144 | 1183104 |
| forloops | 1011 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.24 | 0.12 | 20 |
| hello | rustc | 0.80 | 0.69 | 2108 |
| hello | ratio (rustc/rmc) | 3.39 | 5.74 | 105.40 |
| fib | rmc | 6.74 | 6.61 | 24 |
| fib | rustc | 7.12 | 6.98 | 2072 |
| fib | ratio (rustc/rmc) | 1.06 | 1.06 | 86.33 |
| mandel | rmc | 95.65 | 95.45 | 24 |
| mandel | rustc | 105.19 | 104.97 | 2104 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 87.67 |
| sieve | rmc | 1.65 | 1.54 | 1584 |
| sieve | rustc | 2.38 | 2.27 | 3660 |
| sieve | ratio (rustc/rmc) | 1.45 | 1.48 | 2.31 |
| mix | rmc | 14.68 | 14.53 | 24 |
| mix | rustc | 15.49 | 15.34 | 2080 |
| mix | ratio (rustc/rmc) | 1.06 | 1.06 | 86.67 |
| stats | rmc | 3.44 | 3.31 | 24 |
| stats | rustc | 3.71 | 3.57 | 2068 |
| stats | ratio (rustc/rmc) | 1.08 | 1.08 | 86.17 |
| refs | rmc | 4.74 | 4.61 | 24 |
| refs | rustc | 5.50 | 5.36 | 2104 |
| refs | ratio (rustc/rmc) | 1.16 | 1.16 | 87.67 |
| heap | rmc | 3.19 | 3.06 | 4712 |
| heap | rustc | 3.33 | 3.21 | 6712 |
| heap | ratio (rustc/rmc) | 1.04 | 1.05 | 1.42 |
| callrec | rmc | 2.73 | 2.61 | 24 |
| callrec | rustc | 2.83 | 2.71 | 2084 |
| callrec | ratio (rustc/rmc) | 1.04 | 1.04 | 86.83 |
| forloops | rmc | 1.47 | 1.34 | 24 |
| forloops | rustc | 3.28 | 3.17 | 2104 |
| forloops | ratio (rustc/rmc) | 2.24 | 2.36 | 87.67 |

Ratios are rustc divided by rmc, so a value above 1 means rmc is
better on that dimension.

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
| compile_wall | 1156.42 | win |
| compile_cpu | 1246.17 | win |
| compile_rss | 52.14 | win |
| size | 300.58 | win |
| run_wall | 1.46 | win |
| run_cpu | 1.71 | win |
| run_rss | 71.81 | win |

ALL AXES WON