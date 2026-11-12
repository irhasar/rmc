# Making rmc a rustc back end: status and roadmap

This is the milestone plan for the experiment "rustc is the front end, rmc is
the back end". It records what the current pipeline actually does (measured, not
assumed), where it stands against LLVM, and the ordered work to close the gap.

Regenerate the numbers with:

```sh
python3 bench/run_fair.py    # same source, both back ends
python3 bench/run_mir.py     # three pipelines, separate sources
```

## What the pipeline is today

`rmc --rustc-frontend foo.rs` runs `rustc --emit=mir -O -Zno-codegen`, parses
the printed MIR text (`compiler/src/mir.rs`), and lowers it to x86-64
(`compiler/src/mir_codegen.rs`), reusing `elf.rs` and `runtime.rs`. There is no
LLVM and no libc in the back end.

Two architectural limits follow from the text interface, and they are the
reason this is a prototype rather than a `-Zcodegen-backend`:

* **The interface is the printed MIR, not the MIR data structures.** rmc
  re-parses a lossy text dump: spans, some generic instantiations and promoted
  constants arrive as opaque strings. A real back end links `rustc_private` and
  implements `rustc_codegen_ssa::CodegenBackend`, receiving `mir::Body` directly.
* **The front end still pays full rustc start-up.** Both pipelines run rustc, so
  the ~28 ms driver start and the ~84 MB front-end RSS are shared. Only codegen
  is swapped, which is why the compile-RSS axis ties (see below).

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
| compile wall | **1.085×** (rmc better) | **1.13×** |
| compile CPU | **1.120×** | **1.15×** |
| compile RSS | 0.996 (tie — shared rustc front end) | **1.41×** |
| artifact size | **18.7×** | — |
| run wall | **1.100×** | — |
| run CPU | **1.109×** | — |
| run RSS | **1.306×** | — |

The verdict is "some axes lost" for exactly two programs, and they are the
honest picture of the remaining work:

* **`hello` compile wall 0.99** — a tie inside noise; both pay rustc start-up.
* **`light` run wall 0.81** — LLVM's generated code is genuinely faster on this
  cache-resident kernel. This is a **codegen-quality gap**, not a measurement
  artefact, and it is the one run-time loss on the whole suite.
* **Back-end compile, `refs` 0.83 / `sieve` 0.88 / `mandel` 0.94** — `rmcmir`
  is *slower than `ld`* to assemble+link these three. rmc's assembler is a
  single-pass text encoder; `ld` is not doing much for an already-object file,
  so a slow path in `rmcmir` shows through. Worth a profile.

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

**P3 — close the run-time codegen gap.**
`light` is 0.81× LLVM. The likely causes are register allocation and addressing
mode selection in `mir_codegen`; the AST path already wins the same-shaped
kernel, so the MIR path is leaving something on the table. Target: no program
below ~0.95×.

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

**P5 — the architecturally correct interface.**
Replace the printed-MIR text parser with a `rustc_private`
`CodegenBackend` implementation, so the back end sees `mir::Body` directly and
can be selected with `-Zcodegen-backend`. This is a rewrite of the front-end
adapter, not of the code generator, and it is what turns the prototype into a
real drop-in.

## What "beat LLVM on every axis" would require, honestly

The compile-time, memory and size axes are already won, by a wide margin, and
the win is structural: skipping LLVM codegen and optimisation, and linking no
libc. The run-time axis is not a slam dunk and should not be promised as one:
rmc's back end has no register allocator beyond the stack machine, so kernels
where LLVM's scheduler and allocator matter (`light`) can lose. P3 is the work
that closes that, and the fair harness is what will say whether it did.
