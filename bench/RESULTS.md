# Benchmark results: rmc vs rustc

Recorded 2026-11-12 02:31 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.69 | 1.58 | 3424 | 503 |
| hello | rustc | 2025.23 | 2026.34 | 188508 | 289416 |
| hello | ratio (rustc/rmc) | 1200.5 | 1285.8 | 55.1 | 575.4 |
| fib | rmc | 1.78 | 1.66 | 3720 | 870 |
| fib | rustc | 2066.43 | 2070.33 | 187416 | 291696 |
| fib | ratio (rustc/rmc) | 1160.6 | 1248.7 | 50.4 | 335.3 |
| mandel | rmc | 1.89 | 1.78 | 3580 | 1209 |
| mandel | rustc | 2079.98 | 2084.04 | 190932 | 291776 |
| mandel | ratio (rustc/rmc) | 1099.2 | 1170.2 | 53.3 | 241.3 |
| sieve | rmc | 1.85 | 1.72 | 3776 | 1137 |
| sieve | rustc | 2083.24 | 2087.38 | 188560 | 292032 |
| sieve | ratio (rustc/rmc) | 1123.9 | 1211.5 | 49.9 | 256.8 |
| mix | rmc | 1.88 | 1.73 | 3636 | 1117 |
| mix | rustc | 2070.73 | 2074.75 | 187376 | 291880 |
| mix | ratio (rustc/rmc) | 1101.5 | 1201.4 | 51.5 | 261.3 |
| stats | rmc | 1.90 | 1.78 | 3740 | 1104 |
| stats | rustc | 2087.56 | 2089.65 | 187192 | 292048 |
| stats | ratio (rustc/rmc) | 1101.4 | 1176.6 | 50.1 | 264.5 |
| refs | rmc | 2.04 | 1.91 | 3688 | 1279 |
| refs | rustc | 2080.14 | 2084.64 | 188040 | 292512 |
| refs | ratio (rustc/rmc) | 1020.8 | 1090.3 | 51.0 | 228.7 |
| heap | rmc | 2.38 | 2.21 | 3752 | 1335 |
| heap | rustc | 2095.03 | 2098.73 | 191420 | 292384 |
| heap | ratio (rustc/rmc) | 879.7 | 950.5 | 51.0 | 219.0 |
| callrec | rmc | 1.78 | 1.65 | 3724 | 873 |
| callrec | rustc | 2063.37 | 2067.59 | 186848 | 291680 |
| callrec | ratio (rustc/rmc) | 1156.4 | 1250.1 | 50.2 | 334.1 |
| forloops | rmc | 1.86 | 1.74 | 3576 | 1011 |
| forloops | rustc | 2067.92 | 2072.43 | 188360 | 292480 |
| forloops | ratio (rustc/rmc) | 1111.4 | 1194.5 | 52.7 | 289.3 |

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
| hello | rmc | 0.24 | 0.13 | 36 |
| hello | rustc | 0.82 | 0.71 | 2112 |
| hello | ratio (rustc/rmc) | 3.34 | 5.61 | 58.67 |
| fib | rmc | 6.77 | 6.62 | 36 |
| fib | rustc | 7.12 | 6.99 | 2104 |
| fib | ratio (rustc/rmc) | 1.05 | 1.06 | 58.44 |
| mandel | rmc | 95.87 | 95.62 | 36 |
| mandel | rustc | 105.30 | 105.08 | 2128 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 59.11 |
| sieve | rmc | 1.64 | 1.52 | 1596 |
| sieve | rustc | 2.40 | 2.28 | 3656 |
| sieve | ratio (rustc/rmc) | 1.46 | 1.49 | 2.29 |
| mix | rmc | 14.68 | 14.55 | 36 |
| mix | rustc | 15.46 | 15.31 | 2124 |
| mix | ratio (rustc/rmc) | 1.05 | 1.05 | 59.00 |
| stats | rmc | 3.42 | 3.31 | 36 |
| stats | rustc | 3.73 | 3.59 | 2060 |
| stats | ratio (rustc/rmc) | 1.09 | 1.08 | 57.22 |
| refs | rmc | 4.72 | 4.59 | 36 |
| refs | rustc | 5.46 | 5.34 | 2108 |
| refs | ratio (rustc/rmc) | 1.16 | 1.16 | 58.56 |
| heap | rmc | 3.20 | 3.09 | 4724 |
| heap | rustc | 3.35 | 3.21 | 6724 |
| heap | ratio (rustc/rmc) | 1.05 | 1.04 | 1.42 |
| callrec | rmc | 2.75 | 2.62 | 36 |
| callrec | rustc | 2.84 | 2.71 | 2108 |
| callrec | ratio (rustc/rmc) | 1.03 | 1.04 | 58.56 |
| forloops | rmc | 1.49 | 1.36 | 36 |
| forloops | rustc | 3.32 | 3.20 | 2116 |
| forloops | ratio (rustc/rmc) | 2.24 | 2.36 | 58.78 |

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
