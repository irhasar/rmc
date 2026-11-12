# Benchmark results: rmc vs rustc

Recorded 2026-11-12 01:40 UTC on Linux-6.8.0-1055-gke-x86_64-with-glibc2.41.
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
| hello | rmc | 1.40 | 1.26 | 3104 | 495 |
| hello | rustc | 2056.21 | 2058.62 | 186708 | 289416 |
| hello | ratio (rustc/rmc) | 1469.5 | 1636.4 | 60.2 | 584.7 |
| fib | rmc | 1.45 | 1.32 | 3092 | 692 |
| fib | rustc | 2085.57 | 2089.61 | 186848 | 291696 |
| fib | ratio (rustc/rmc) | 1435.0 | 1581.8 | 60.4 | 421.5 |
| mandel | rmc | 1.59 | 1.46 | 3064 | 1041 |
| mandel | rustc | 2085.30 | 2089.30 | 190176 | 291776 |
| mandel | ratio (rustc/rmc) | 1312.4 | 1431.0 | 62.1 | 280.3 |
| sieve | rmc | 1.95 | 1.82 | 3140 | 953 |
| sieve | rustc | 2069.92 | 2074.17 | 187232 | 292032 |
| sieve | ratio (rustc/rmc) | 1061.5 | 1139.0 | 59.6 | 306.4 |
| mix | rmc | 1.52 | 1.39 | 3076 | 941 |
| mix | rustc | 2091.26 | 2095.51 | 186764 | 291880 |
| mix | ratio (rustc/rmc) | 1379.5 | 1508.6 | 60.7 | 310.2 |
| stats | rmc | 1.65 | 1.51 | 3040 | 924 |
| stats | rustc | 2101.01 | 2105.73 | 188112 | 292048 |
| stats | ratio (rustc/rmc) | 1271.3 | 1394.5 | 61.9 | 316.1 |
| refs | rmc | 1.72 | 1.61 | 3144 | 1115 |
| refs | rustc | 2090.36 | 2091.62 | 188288 | 292512 |
| refs | ratio (rustc/rmc) | 1214.2 | 1299.9 | 59.9 | 262.3 |
| heap | rmc | 1.61 | 1.48 | 3148 | 1165 |
| heap | rustc | 2082.95 | 2086.35 | 193184 | 292384 |
| heap | ratio (rustc/rmc) | 1290.6 | 1406.8 | 61.4 | 251.0 |
| callrec | rmc | 1.50 | 1.37 | 3128 | 695 |
| callrec | rustc | 2072.77 | 2075.63 | 187128 | 291680 |
| callrec | ratio (rustc/rmc) | 1385.7 | 1514.0 | 59.8 | 419.7 |
| forloops | rmc | 1.52 | 1.40 | 3112 | 851 |
| forloops | rustc | 2081.70 | 2085.90 | 188248 | 292480 |
| forloops | ratio (rustc/rmc) | 1372.4 | 1487.8 | 60.5 | 343.7 |

## Artifact size against each rustc build

| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |
| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |
| hello | 495 | 289416 | 4537096 | 1180416 |
| fib | 692 | 291696 | 4541112 | 1183104 |
| mandel | 1041 | 291776 | 4541208 | 1183168 |
| sieve | 953 | 292032 | 4541424 | 1183392 |
| mix | 941 | 291880 | 4541432 | 1183328 |
| stats | 924 | 292048 | 4541392 | 1183360 |
| refs | 1115 | 292512 | 4542248 | 1183904 |
| heap | 1165 | 292384 | 4542176 | 1183776 |
| callrec | 695 | 291680 | 4541144 | 1183104 |
| forloops | 851 | 292480 | 4541808 | 1183808 |

rmc links no libc and no runtime, so a plain `rustc -O` build (which
statically links libstd, the panic machinery and the allocator) is the
closest thing to a like-for-like size reference; the stripped static
build is the closest *runtime* one. Both are far larger than the
fat-LTO stripped build quoted in the headline table.

## Run: speed, CPU, RAM of the compiled binary

| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |
| ------- | -------- | --------: | -------: | ------------: |
| hello | rmc | 0.23 | 0.12 | 28 |
| hello | rustc | 0.82 | 0.71 | 2084 |
| hello | ratio (rustc/rmc) | 3.55 | 5.96 | 74.43 |
| fib | rmc | 7.14 | 7.01 | 28 |
| fib | rustc | 7.13 | 7.00 | 2104 |
| fib | ratio (rustc/rmc) | 1.00 | 1.00 | 75.14 |
| mandel | rmc | 95.68 | 95.48 | 28 |
| mandel | rustc | 105.11 | 104.89 | 2092 |
| mandel | ratio (rustc/rmc) | 1.10 | 1.10 | 74.71 |
| sieve | rmc | 1.83 | 1.70 | 1588 |
| sieve | rustc | 2.43 | 2.29 | 3640 |
| sieve | ratio (rustc/rmc) | 1.33 | 1.34 | 2.29 |
| mix | rmc | 14.96 | 14.79 | 28 |
| mix | rustc | 15.45 | 15.29 | 2000 |
| mix | ratio (rustc/rmc) | 1.03 | 1.03 | 71.43 |
| stats | rmc | 3.25 | 3.14 | 28 |
| stats | rustc | 3.74 | 3.61 | 2112 |
| stats | ratio (rustc/rmc) | 1.15 | 1.15 | 75.43 |
| refs | rmc | 3.23 | 3.11 | 28 |
| refs | rustc | 5.48 | 5.34 | 2104 |
| refs | ratio (rustc/rmc) | 1.70 | 1.72 | 75.14 |
| heap | rmc | 3.27 | 3.15 | 4716 |
| heap | rustc | 3.31 | 3.17 | 6708 |
| heap | ratio (rustc/rmc) | 1.01 | 1.01 | 1.42 |
| callrec | rmc | 2.90 | 2.76 | 28 |
| callrec | rustc | 2.81 | 2.69 | 2076 |
| callrec | ratio (rustc/rmc) | 0.97 | 0.98 | 74.14 |
| forloops | rmc | 1.47 | 1.35 | 28 |
| forloops | rustc | 3.31 | 3.18 | 2064 |
| forloops | ratio (rustc/rmc) | 2.25 | 2.36 | 73.71 |

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
