# Benchmark results: rmc vs rustc

Recorded 2026-11-12 05:32 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.62 | 1.49 | 3620 | 503 |
| hello | rustc | 2110.17 | 2112.41 | 188164 | 289416 |
| hello | ratio (rustc/rmc) | 1305.9 | 1414.9 | 52.0 | 575.4 |
| fib | rmc | 1.72 | 1.58 | 3616 | 870 |
| fib | rustc | 2146.25 | 2147.34 | 188484 | 291696 |
| fib | ratio (rustc/rmc) | 1247.6 | 1357.4 | 52.1 | 335.3 |
| mandel | rmc | 1.89 | 1.77 | 3652 | 1209 |
| mandel | rustc | 2131.60 | 2135.78 | 186868 | 291776 |
| mandel | ratio (rustc/rmc) | 1128.1 | 1208.0 | 51.2 | 241.3 |
| sieve | rmc | 1.86 | 1.73 | 3644 | 1137 |
| sieve | rustc | 2167.75 | 2168.35 | 186700 | 292032 |
| sieve | ratio (rustc/rmc) | 1167.7 | 1255.6 | 51.2 | 256.8 |
| mix | rmc | 1.91 | 1.77 | 3556 | 1117 |
| mix | rustc | 2174.82 | 2178.58 | 187356 | 291880 |
| mix | ratio (rustc/rmc) | 1139.7 | 1230.1 | 52.7 | 261.3 |
| stats | rmc | 1.86 | 1.74 | 3696 | 1104 |
| stats | rustc | 2177.36 | 2180.97 | 187852 | 292048 |
| stats | ratio (rustc/rmc) | 1171.8 | 1254.9 | 50.8 | 264.5 |
| refs | rmc | 2.04 | 1.92 | 3784 | 1279 |
| refs | rustc | 2149.20 | 2152.70 | 188976 | 292512 |
| refs | ratio (rustc/rmc) | 1053.6 | 1122.4 | 49.9 | 228.7 |
| heap | rmc | 1.91 | 1.78 | 3636 | 1335 |
| heap | rustc | 2183.21 | 2187.70 | 188380 | 292384 |
| heap | ratio (rustc/rmc) | 1143.4 | 1231.8 | 51.8 | 219.0 |
| callrec | rmc | 1.75 | 1.63 | 3688 | 873 |
| callrec | rustc | 2150.84 | 2155.09 | 186816 | 291680 |
| callrec | ratio (rustc/rmc) | 1227.7 | 1324.6 | 50.7 | 334.1 |
| forloops | rmc | 1.81 | 1.69 | 3724 | 1011 |
| forloops | rustc | 2201.07 | 2205.34 | 187856 | 292480 |
| forloops | ratio (rustc/rmc) | 1217.1 | 1308.0 | 50.4 | 289.3 |
| light | rmc | 1.83 | 1.72 | 3748 | 1209 |
| light | rustc | 2154.45 | 2158.88 | 189948 | 291904 |
| light | ratio (rustc/rmc) | 1174.7 | 1253.7 | 50.7 | 241.4 |
| heavy | rmc | 1.76 | 1.65 | 3624 | 969 |
| heavy | rustc | 2207.43 | 2211.42 | 187236 | 291792 |
| heavy | ratio (rustc/rmc) | 1251.6 | 1340.3 | 51.7 | 301.1 |

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
| light | 1209 | 291904 | 4541264 | 1183232 |
| heavy | 969 | 291792 | 4541120 | 1183104 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.21 | 0.12 | 24 |
| hello | rustc | 0.80 | 0.70 | 2104 |
| hello | ratio (rustc/rmc) | 3.73 | 5.85 | 87.67 |
| fib | rmc | 6.72 | 6.61 | 24 |
| fib | rustc | 7.12 | 6.96 | 2040 |
| fib | ratio (rustc/rmc) | 1.06 | 1.05 | 85.00 |
| mandel | rmc | 95.75 | 95.50 | 24 |
| mandel | rustc | 105.17 | 104.95 | 2096 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 87.33 |
| sieve | rmc | 1.64 | 1.52 | 1584 |
| sieve | rustc | 2.39 | 2.26 | 3660 |
| sieve | ratio (rustc/rmc) | 1.46 | 1.48 | 2.31 |
| mix | rmc | 14.71 | 14.56 | 24 |
| mix | rustc | 15.50 | 15.36 | 2076 |
| mix | ratio (rustc/rmc) | 1.05 | 1.06 | 86.50 |
| stats | rmc | 3.42 | 3.30 | 24 |
| stats | rustc | 3.72 | 3.59 | 2092 |
| stats | ratio (rustc/rmc) | 1.09 | 1.09 | 87.17 |
| refs | rmc | 4.68 | 4.56 | 24 |
| refs | rustc | 5.48 | 5.35 | 2104 |
| refs | ratio (rustc/rmc) | 1.17 | 1.17 | 87.67 |
| heap | rmc | 3.19 | 3.06 | 4712 |
| heap | rustc | 3.31 | 3.19 | 6716 |
| heap | ratio (rustc/rmc) | 1.04 | 1.04 | 1.43 |
| callrec | rmc | 2.74 | 2.62 | 24 |
| callrec | rustc | 2.83 | 2.71 | 2088 |
| callrec | ratio (rustc/rmc) | 1.04 | 1.03 | 87.00 |
| forloops | rmc | 1.46 | 1.35 | 24 |
| forloops | rustc | 3.31 | 3.18 | 2084 |
| forloops | ratio (rustc/rmc) | 2.26 | 2.35 | 86.83 |
| light | rmc | 2.09 | 1.98 | 24 |
| light | rustc | 2.63 | 2.50 | 2092 |
| light | ratio (rustc/rmc) | 1.26 | 1.26 | 87.17 |
| heavy | rmc | 766.19 | 765.93 | 24 |
| heavy | rustc | 1030.97 | 1030.60 | 2060 |
| heavy | ratio (rustc/rmc) | 1.35 | 1.35 | 85.83 |

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
| compile_wall | 1185.74 | win |
| compile_cpu | 1275.13 | win |
| compile_rss | 51.27 | win |
| size | 295.70 | win |
| run_wall | 1.47 | win |
| run_cpu | 1.65 | win |
| run_rss | 72.66 | win |

ALL AXES WON