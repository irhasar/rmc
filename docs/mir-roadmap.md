# Making rmc a rustc back end: status and roadmap

This is the milestone plan for the experiment "rustc is the front end, rmc is
the back end". It records what the current pipeline actually does (measured, not
assumed), where it stands against LLVM, and the ordered work to close the gap.

Regenerate the numbers with:

```sh
# The backend links one exact nightly's `rustc_private`. The channel is pinned
# in `backend/rust-toolchain.toml`, and every harness reads the pin from there,
# so a fresh checkout needs only:
rustup toolchain install "$(sed -n 's/^channel *= *"\(.*\)"/\1/p' \
    backend/rust-toolchain.toml)" \
    --component rustc-dev --component llvm-tools --component rustc-codegen-cranelift
cargo build --release --manifest-path compiler/Cargo.toml --example rmcmir
./backend/build.sh

python3 bench/run_fair.py     # same source, two back ends (rmc vs LLVM)
python3 bench/run_codegen.py  # four pipelines, one source (adds cb and Cranelift)
python3 bench/run_four.py     # the same four, via run_codegen's measurement code
python3 bench/run_mir.py      # three pipelines, separate sources
```

None of the harnesses hard-codes a mangled symbol name any more: the reference
link's panic stubs are read from the object with `nm -u` (`tests/mir/run.py`,
`bench/run_fair.py`) or generated for the probe crate's disambiguator
(`bench/run_codegen.py`), so they keep working on a toolchain the author never
ran.

## What the pipeline is today

There are two front ends onto the same back end:

* `rmc --rustc-frontend foo.rs` runs `rustc --emit=mir -O -Zno-codegen`, parses
  the printed MIR text (`compiler/src/mir.rs`), and lowers it to x86-64
  (`compiler/src/mir_codegen.rs`), reusing `elf.rs` and `runtime.rs`. This is the
  standalone prototype (`rmcmir`) and the correctness oracle.
* `backend/` is the real thing: a `rustc_private` `CodegenBackend` dylib that
  rustc loads with `-Zcodegen-backend`, so rmc receives `mir::Body` directly, in
  one rustc process, with no MIR text and no second process. `bench/run_codegen.py`
  measures it as `cb`; see P5 below.

Neither uses LLVM or libc in the back end.

One architectural limit is shared: **the front end still pays full rustc
start-up.** Both pipelines run rustc, so the ~28 ms driver start and the ~84 MB
front-end RSS are common to both back ends. Only codegen is swapped, which is why
the compile-RSS axis ties (see below). The printed-MIR path additionally pays a
lossy text round-trip: spans, some generic instantiations and promoted constants
arrive as opaque strings, which is why the `cb` path exists.


## What compiles today (correctness-verified)

Measured by compiling each construct through `rustc --emit=mir` + `rmcmir` and
running it, comparing stdout to a native `rustc -O` twin. All of the following
**produce correct results**, not merely compile:

| area | supported |
| ---- | --------- |
| integers | `i8`–`i64`, `u8`–`u64`, `usize`/`isize`; add/sub/mul/div/rem, shifts (incl. variable), bitwise, wrapping, `as` casts and chains |
| control flow | `if`/`else`, `while`, `for`, `loop` with `break`/`continue`, nested loops, `match` on integers and ranges |
| functions | calls, recursion, multiple `return`s, nested `fn`, monomorphised generics (`fn id<T>`, `struct W<T>`) |
| aggregates | local/global arrays, bounds-checked indexing, slices, `.len()`, tuples and structs **returned by value**, nested structs, arrays of structs, 2-D arrays |
| references | `&T`, `&mut T` (through the reference), `*const T`/`*mut T` |
| floats | `f64`/`f32` arithmetic, comparison and loops (see limits) |
| `i128` | arithmetic, division, comparison (see limits) |
| closures | capturing and non-capturing closures **called directly** (see limits) |
| dispatch | inherent `impl` methods, statically-dispatched trait methods, `for` over slice iterators |
| enums | fieldless and payload variants, `match` |
| std types | `Option`/`Result` construction and `match`, `&str` literals, `.len()` |

### Limits (things that do **not** compile)

Grouped by cause, because the cause decides the fix:

* **Front-end reach** — needs `alloc`/`core` surface the `#![no_std]` stub does
  not provide: `Vec`, `Box`, `String`, iterator adapters with closure state
  (`map`, `step_by`), `str::chars()`. Fix: a larger freestanding `core`/`alloc`
  shim, or accept these as out of scope.
* **Back-end gaps** — `mir_codegen` bails: `dyn Trait` (trait objects),
  `Range` as a value (an aggregate constant it cannot materialise), promoted
  aggregate constants. Function pointers and indirect calls are now
  **supported** (see P4).
* **Wrong or stale messages** — `classify_unsupported` in `mir.rs` still claims
  "floating-point types are not implemented" and "128-bit integers are not
  implemented", but `f64` arithmetic/comparison/loops and `i128`
  arithmetic/division/comparison all work and are correctness-verified above.
  The list is a maintenance hazard: it will reject-with-a-message, or silently
  pass, based on a stale picture. **This is the first thing to fix.**

## Where it stands against LLVM (fair harness, same source)

`bench/run_fair.py` compiles the **same** `bench/mir/*.rs` through both back
ends and links the **same** freestanding runtime, so a difference is the code
generator. On the reference host (AMD EPYC 9B14, `rustc 1.99.1`):

| axis | whole pipeline (mean) | back-end only (mean) |
| ---- | --------------------- | -------------------- |
| compile wall | **1.076×** (rmc better) | **1.10×** |
| compile CPU | **1.107×** | **1.12×** |
| compile RSS | 1.003 (tie — shared rustc front end) | **1.40×** |
| artifact size | **18.8×** | — |
| run wall | **1.118×** | — |
| run CPU | **1.132×** | — |
| run RSS | **1.306×** | — |

Every whole-pipeline axis is a win or a tie. Two cells are inside the noise
band and are the honest picture of the remaining work:

* **`hello` compile wall 0.99** — a tie inside noise; both pay rustc start-up.
* **Back-end compile, `refs` 0.81 / `sieve` 0.85 / `mandel` 0.89** — `rmcmir`
  is *slower than `ld`* to assemble+link these three. rmc's assembler is a
  single-pass text encoder; `ld` is not doing much for an already-object file,
  so a slow path in `rmcmir` shows through. Worth a profile. It does not affect
  the `cb` pipeline, whose one-process compile beats LLVM on every program
  (`sieve` 1.15, `mandel` 1.11, `refs` 1.09).

The run-time loss is closed. `light` used to run at 0.81× LLVM; the
`fold_lea_terms` peephole pass (P3, below) folds the array-index address chain
into one addressing mode, and `light` now runs at **1.11×**.

## Roadmap, in priority order

**P0 — runtime primitives the back end needs. (done)**
LLVM lowers a wide zero-init to a `memset` call, so the same source that rmc
compiles to an inline `rep stos` reached the linker as an undefined `memset`.
Added freestanding `memset`/`memcpy`/`memmove`/`memcmp` as runtime pieces
(`compiler/src/runtime.rs`), kept only when a program names them, so rmc's own
output is byte-identical. Verified against a C harness (including `memmove`'s
overlap direction) and pinned by `runtime::tests`.

**P1 — make the supported surface honest and tested. (done)**
`classify_unsupported` no longer claims float or 128-bit integers are
unimplemented; the float rule now fires only for `%` and the intrinsics rustc
lowers to library calls. Three panics on *expected* input are fixed: the
standalone `rmcmir` driver used `.expect("mir parse")` / `.expect("assemble")`,
so any unsupported construct crashed with a backtrace note instead of exiting 1
(`f_minmax` panicked at `assemble` on an undefined float intrinsic). A `dyn
Trait` coercion used to pass as an identity pointer cast and surface as a
confusing `elf: undefined symbol`; it is now a parse error naming the construct.
Coverage added: `tests/mir/float.rs` and `tests/mir/wideint.rs` (float and
128-bit arithmetic, the two features the stale messages had hidden), three
`tests/mir/reject/` cases asserting a *clean named* refusal, and
`tools/mir_survey.py` now actually passes `--rustc-frontend` (it had been
surveying the AST path, so the crashes were invisible) and runs in CI. Measured
after the fix: 80/121 realistic Rust snippets compile through the MIR path, 38
refuse cleanly, zero crashes.

**P2 — close the back-end compile-time gaps.**
Profile `rmcmir` on `refs`/`sieve`/`mandel` and remove the slow path. Target:
back-end-only compile wall ≥ 1.0 on every program, not just on the mean.

**P3 — close the run-time codegen gap. (done)**
`light` was 0.81× LLVM. The cause was addressing-mode selection: an array
element access was generated as `lea base(%rbp), %rax; mov index, %rcx; lea
(%rax,%rcx,8), %rax`, three instructions where x86-64 fits the base, the scaled
index and the displacement in one. The `fold_lea_terms` peephole pass
(`compiler/src/mir_codegen.rs`) fuses the chain into `lea base(%rbp, index, 8),
%rax` and drops the index copy, guarded on the scratch register not still
holding a live value. `light` now runs at **1.11×** LLVM (measured: run wall
0.81 → 1.11), and the whole-suite run mean is 1.118. No program is below 0.98×.

**P4 — widen the front-end-reachable surface. (function pointers done)**
Function pointers and indirect calls are implemented. Taking a function as a
value (`add as fn(i64, i64) -> i64`, MIR `PointerCoercion(ReifyFnPointer)`)
reifies its address as a `lea add(%rip), %reg`; a call through a `fn`-typed
place is lowered to an indirect `call *%reg` (the callee is evaluated into a
frame slot and reached memory-indirect, because `%r11` is the scratch register
for array copies and division). Covered: a pointer passed as an argument,
returned from a function, stored in a struct field, and held in an array
(`tests/mir/fnptr.rs`). The `reject/fnptr` case is retired.

Three latent bugs had to be fixed for this to work, each of which would have
broken any operand-shaped callee:

* `split_top_args` counted the `>` of a `->` as a closing angle bracket, so a
  signature containing a `fn(..) -> ..` parameter was parsed as a *single*
  argument and every later parameter was dropped.
* The inliner cloned `CallTarget::Indirect` verbatim, so an inlined indirect
  call still named the *caller's* local (its own `_1`) instead of the inlined
  argument.
* Every MIR use/def and rewrite pass (`copy_propagate`, `rewrite_term`,
  use-counting, `other_place`) ignored the callee operand, so a `fn` pointer
  local looked dead and its definition was dropped.

Still open in P4: closures passed as `impl Fn` parameters, iterator adapters,
`Range` as a value. These are `mir_codegen` gaps, so they do not depend on a
bigger `core` shim.

**P5 — the architecturally correct interface. (done)**
The printed-MIR text parser is no longer the only route: `backend/` is a
`rustc_private` `CodegenBackend` implementation, selected with
`-Zcodegen-backend=librmc_codegen_backend.so`. One rustc process, no MIR text,
no second process, no `ld` — rmc receives `mir::Body` directly and writes the
ELF itself. `bench/run_codegen.py` measures it as `cb`: against LLVM it wins
every mean axis (wall 1.33, cpu 1.36, rss 1.11, size 21.0, run_wall 1.09,
run_cpu 1.09, run_rss 1.25), and against the printed-MIR prototype it is faster
to compile (wall 1.25, cpu 1.24, rss 1.08) at identical run time. The
printed-MIR path is kept as the correctness oracle and for the standalone
`rmcmir` driver.

`tests/cb/` is the differential suite for the backend path specifically: it
drives the real `-Zcodegen-backend` route (skipping cleanly when nightly
`rustc-dev` is absent) over the shapes the prototype never sees — defining vs
visible paths, bare unit-struct constants, split promoted chains, the
`integer_min`/`integer_max` intrinsics, the `todo!`/`unreachable!` message
helpers, and `const ZeroSized` closures. With those handled, the
prototype/backend divergence scan over the survey corpus is clean: every case
the prototype compiles now compiles identically under the backend.

## What "beat LLVM on every axis" would require, honestly

The compile-time, memory and size axes are won, and the win is structural:
skipping LLVM codegen and optimisation, and linking no libc. The run-time axis
was the open question — rmc's back end has no register allocator beyond the
stack machine — and P3 closed it: the `light` kernel that used to lose now
wins, and every program is at or above 0.98× LLVM. What is *not* yet closed is
**surface**: the back end compiles 80 of 121 realistic snippets, refusing the
rest cleanly (no crashes). The largest clusters are calls that return an
aggregate by value (`Option`/`Result`-shaped, `pow`, `checked_add`), `Vec` and
other `alloc`-backed types, iterator adapters with closure state, and `dyn
Trait`. Those are P4, and they are the remaining distance between "beats LLVM on
this suite" and "can replace LLVM".

