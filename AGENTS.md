# AGENTS.md

Repository-specific notes for future sessions. Keep this file current.

## What this is

`rmc` is an experimental ahead-of-time compiler for a small Rust subset. It emits
x86-64 assembly directly, then assembles and links it in-process into a static
ELF64 executable (`src/elf.rs`) against a freestanding assembly runtime (raw
Linux syscalls, no libc, no LLVM, no external crates, and by default no `as` or
`ld`). `--binutils` restores the old `as`+`ld` path. The `bench/` directory
compares it against real `rustc`.

## Build and run

```sh
export RUSTUP_HOME=/workspace/.rustup          # toolchain lives here
export PATH=/workspace/.cargo/bin:$PATH        # rustc/cargo are installed here
cargo build --release --manifest-path compiler/Cargo.toml

# compile a program
./compiler/target/release/rmc bench/programs/fib.rms -o /tmp/fib
/tmp/fib 30

# inspect generated assembly
./compiler/target/release/rmc bench/programs/fib.rms --emit-asm

# full benchmark suite (writes binaries to /tmp/rmcbench)
python3 bench/run.py

# benchmark suite, also recording the dated result to the repo
python3 bench/run.py --markdown bench/RESULTS.md

# differential test suite: rmc vs rustc on twin programs
python3 tests/run.py -v

# differential fuzzer: random programs vs rustc (reproducible seeds)
python3 tools/fuzz.py --mode expr --n 500 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1
python3 tools/fuzz.py --mode opt --n 500 --seed 1
python3 tools/fuzz.py --mode arith --n 500 --seed 1
python3 tools/fuzz.py --mode nest --n 500 --seed 1
python3 tools/fuzz.py --mode pat --n 500 --seed 1
python3 tools/fuzz.py --mode order --n 500 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1 --bounds --overflow
```

## Source map

- `compiler/src/lib.rs` — library entry point: `compile_to_asm(src, options)`.
  The compiler is a library plus a thin CLI, so it can be embedded.
- `compiler/src/lexer.rs` — byte scanner, tokens carry byte positions.
- `compiler/src/parser.rs` — recursive descent; `is_block_like` allows
  `if`/blocks as statements without a trailing `;`. `parse_cond` sets
  `no_struct_lit` so a `{` after a condition starts the body, not a struct.
  Precedence, loosest to tightest: `||`, `&&`, comparison, `|`, `^`, `&`,
  `<< >>`, `+ -`, `* / %`. Note shifts bind *looser* than `+`/`-`, matching
  Rust: `parse_shift` consumes `parse_add`, and `parse_bitand` consumes
  `parse_shift`. Swapping these silently mis-parses `1 + 2 << 3`.
  `match` is parsed here and *desugared to a chain of `if`s* (`desugar_match`),
  so sema and codegen have no `match` case. A non-`Var`/`Int` scrutinee is bound
  to a hidden `let` first, so a call in the scrutinee runs once. An arm after
  `_` is unreachable and dropped (the `_` body becomes the `else`).
  An arm pattern may be an integer, a *range* `lo..hi`/`lo..=hi`, an
  `Enum::Variant`, an *or-pattern* `a | b | c`, or `_`. A range desugars to
  `Bin(And, Le, Lt/Le)`, an or-pattern to `Bin(Or, ..)` over the alternatives'
  tests with the *same* body (`or_cond` builds only the test, so the body is not
  duplicated), so neither needs a sema or codegen case. An or-pattern with a
  `_` alternative, and one whose alternatives cover every variant, are handled
  by `pat_irrefutable` and `or_leaves` (in sema) respectively. A *binding*
  alternative is rejected, as rustc's E0408 rejects it.
  `while let PAT = EXPR { body }` is parsed here and desugared by
  `desugar_while_let` to `loop { if let PAT = EXPR { body } else { break; } }`,
  reusing the `if let` path, so sema and codegen gain no case. The scrutinee sits
  inside the loop, so it is re-evaluated each iteration and `continue` re-tests
  it. The `;` after `return`/`break`/`continue` may be omitted at the end of a
  block (`eat_semi_at_block_end`), as in Rust.
  `if let PAT = EXPR { .. } else { .. }` is parsed here too and desugared by
  `desugar_if_let` into a two-arm `match` (`PAT => then, _ => else_or_unit`),
  reusing `desugar_match_arms` and the `parse_pattern` shared with `match` arms,
  so sema and codegen gain no new case. The `_` fallback arm is what makes an
  `if let` need not be exhaustive; a non-unit `then` with no `else` then trips
  the ordinary branch-type check, as in rustc. But note the two desugarings
  differ in how an *irrefutable* arm (`_` or a binding) treats what follows it.
  `match` drops the dead arms, as before. `if let` must not: rustc type-checks
  the dead `else` and rejects `if let y = o { 1 } else { }` (E0308), so
  `desugar_match_arms` takes a `check_dead_arms` flag and `irrefutable_arm`
  wraps the dead arm in `if true { body } else { rest }` to carry that check
  through the ordinary branch-type rule. Dropping the dead `else` for `if let`
  accepts a program rustc rejects; wrapping the dead arms for `match` instead
  rejects a program rustc accepts (`match o { _ => 5, 3 => 9 }` in value
  position), which is why the flag exists.
  `sema` also unifies a diverging branch: `if c { 1 } else { return 2 }` is
  well typed because `!` unifies with any type. `block_diverges` decides it,
  and `stmt_diverges` treats `loop { .. }` as diverging unless `block_has_break`
  finds a `break` that targets it (a nested loop captures its own `break`s).
  Definite initialization follows: only the non-diverging branch's writes
  survive the `if`.
- `compiler/src/ast.rs` — AST and `Type` (`I64`, `Bool`, `Str`, `Unit`, `Array`,
  `Struct`, and the parser-only placeholder `Named`). `Str` is the type of a
  string literal: one slot holding a pointer to NUL-terminated static data. It
  is a scalar (travels in a register) but is deliberately *not* `i64`, so a
  string cannot be added to an integer or passed where an `i64` is expected.
  There is no way to spell the type in source, so only a literal produces it.
- `compiler/src/sema.rs` — type checking, struct layout, stack-slot allocation,
  builtin sigs, and the soundness checks (mutability, definite initialization,
  constant array bounds, string typing — see "Soundness checks" below). Resolves
  `Type::Named`, fills `Field`/`FieldInit` offsets, and allocates the hidden
  slots for aggregate arguments and returns.
- `compiler/src/consts.rs` — shared constant folding: `const_i64`/`const_binop`
  (used by sema to fold a constant index or operand) and `resolve`/`eval_i64`
  (used by the parser to evaluate `const` items). A `const` item has no storage
  and no codegen; its value is substituted at every use.
- `compiler/src/inline.rs` — AST inlining of small, call-free, scalar functions
  (a loop is allowed, an early exit is not), run after sema and before codegen.
  See the inlining gotcha below.
- `compiler/src/licm.rs` — loop-invariant code motion, run after inlining and
  before codegen. See the LICM gotcha below.
- `compiler/src/dce.rs` — dead-function elimination, run after LICM and before
  codegen. Keeps only the functions reachable from `main` (see "Runtime pruning"
  below for why this composes with runtime selection).
- `compiler/src/codegen.rs` — AST to AT&T x86-64 assembly.
- `compiler/src/runtime.rs` — freestanding assembly runtime (`_start`, I/O,
  `alloc`/`free`, `arg_i64`, the traps). Stored as a table of selectable
  `PIECES`; `for_program(user_asm)` links only the pieces the program reaches
  (see "Runtime pruning" below).
- `compiler/src/elf.rs` — the in-process assembler and ELF64 linker. Parses the
  assembly text `codegen`/`runtime` produce and emits a static executable with
  two `PT_LOAD` segments (R+X for text/rodata, R+W for data). See "In-process
  assembler" below before adding an instruction or a codegen form.
- `compiler/src/main.rs` — CLI: reads source, calls the library, writes the ELF.
  Flags: `--emit-asm`, `--keep-asm`, `--binutils`, `--no-strip`, `--min-size`,
  `--overflow-checks`, `--bounds-checks`, `--no-inline`, `--no-licm`,
  `--no-tco`.
- `compiler/tests/compile.rs` — unit tests for the front end and codegen; these
  run without `as`/`ld`.

## Things that are easy to get wrong

- **`loop` is an expression, and codegen must report its own type, not `I64`.**
  `loop { .. }` parses to `Expr::Loop(body, slot, ty)`; a `loop` in statement
  position is a `Stmt::Expr` around it with `slot == -2`, so its value is
  discarded and it is typed `()`. Sema fills `ty` with the type of the loop's
  `break`s (or `Unit` for a `!` loop, matching every other diverging
  expression), and codegen returns that `ty`. Hardcoding `Type::I64` there
  compiles but silently mis-types a loop that yields a `bool` (so `!loop { break
  true }` becomes a bitwise `not` instead of a logical one) and a loop that
  yields an aggregate (so an enum-constructor argument or `value_copies_aggregate`
  takes the wrong path). A value-yielding loop carries its result in a hidden
  slot sized by the result's word count (`slot >= 0`); `break value` stores into
  it and the loop loads it after `break_label`. The slot's `-1`/`-2` sentinels
  are load-bearing: `-1` means "no value to carry", `-2` means "statement
  position, value discarded".
- **Do not weight `promote`'s mention count by loop depth.** It looks obviously
  right — a variable inside a loop runs many more times than one in straight-line
  code — but it loses: weights 2, 3, 4 and 8 all moved mandel from ~95 ms to
  ~101 ms and heap from 3.20 to ~3.4 ms. The five registers are a fixed budget,
  so promoting an innermost-loop local demotes an outer one, and mandel's inner
  loop is division-bound and does not notice its own memory traffic while the
  outer counters `x`/`y`/`size`/`acc` pay a store and reload on every iteration.
  `promote` ranks by the raw mention count on purpose; measure before touching it
  (the full account is in `docs/design.md`).
- **A local whose address is taken must be forced into memory before promotion.**
  The promotion pass picks the hottest scalar locals and keeps them only in
  registers, so their frame slot goes stale. `&x` needs `x` to be in memory, so
  `count_expr` calls `mark_addressed` on an `AddrOf` operand to set `mem[]` for
  that slot *before* promotion runs. Miss this and `lea slot(%rbp)` reads a
  garbage word. `mark_addressed` walks `Field`/`Index` bases and marks the whole
  aggregate, and it must run in the dry pass too (it does: `count_expr` is shared
  by both passes).
- **A reference is a scalar, but an aggregate referent is named by its address.**
  `Type::Ref` is a scalar (one slot, one register) even when the referent is a
  struct or array. So `gen_expr` on a `Ref` local is a plain `mov`, while `Deref`
  of an aggregate is a no-op after evaluating the pointer — the pointer *is* the
  value. `gen_addr` on a `Deref` yields the pointer, and on a `Field` whose base
  is `Ref`-typed it adds the offset to the pointer (`ref_base`), never to the
  slot holding the pointer.
- **`&e` on a non-lvalue materialises a temporary.** `sema` gives `AddrOf` a
  hidden slot (`slot >= 0`) for a literal, call result, or aggregate-valued
  operand, and codegen stores the value there and yields that address. An
  aggregate operand is copied with `copy_words_rdi_rsi`, a scalar with one `mov`.
  `slot == -1` means the operand is an lvalue and `gen_addr` is used directly.
- **`&mut T` coerces to `&T`; the reverse does not.** `ref_coerces` is the only
  coercion and is checked in `let`, assignment and call arguments. Both are one
  pointer, so the coercion emits no code. `&mut` and `&` are otherwise distinct
  types, which is why `Sig` carries `ref_self_mut`.
- **A field read through a reference folds to one `disp(%reg)` load.**
  `field_operand` recognises a reference local kept in a register (`p.x` where
  `p: &T` and `p` is promoted) and a dereference of one (`(*q).x`), emitting
  `mov off(%reg), %rax` instead of `mov %reg,%rax; add $off,%rax; mov (%rax),%rax`.
  Before this, an `&self` accessor over an 8-field struct emitted ~3 instructions
  per field and the `refs` benchmark ran at 69 ms; the fold brought rmc's own
  runtime down ~1.4x to ~50 ms. Inlining the accessors then removed the calls
  entirely, taking `refs` to ~20 ms, and LICM hoisted the now-inlined struct
  reads out of the loop, taking it to ~6 ms. Flattening the accumulator fold over
  the nested sum LICM leaves (`acc + t + (t + p) + p`), and unwrapping the
  inliner's spliced block so the fold can see it, took `refs` to ~4.7 ms. Against
  an honest rustc twin (which `black_box`es a term, or LLVM closes the whole loop
  into `128 * n`) that is ~0.8x, not the ~0.14x an earlier run reported from the
  folded loop; rustc is only ~1.2x ahead now, keeping the struct in registers
  across the loop, which rmc does not. The fold is deliberately *one* level deep:
  a `*p` used as a reference has its pointer in memory, so `ptr_reg` only matches
  a reference *local*, never a `Deref`. `addr_reg` (local or one `Deref` of a
  local) is the form `gen_addr` folds for a `lea`. Extending either to a deeper
  chain would read the inner pointer from a register it is not in — the bug that
  `(**ppr).x` in `tests/cases/references.rms` guards against.
- **A scalar `*p` read is a memory operand.** `simple_operand` returns `(%reg)`
  for a non-aggregate `Deref` whose pointer is a reference local, so `*p < 5`
  is one `cmpq $5, (%reg)` and `*p` in a comparison needs no load. An aggregate
  `Deref` is *not* a simple operand: its value is an address, not data.
- **A store through a computed address folds a register/immediate value.**
  `(*np).next = head` with both operands in registers emits one `mov %r, (%rax)`
  rather than spilling `head` to a frame slot, storing the address, and reloading
  (`mov %rbx, -80(%rbp); mov %rax, -72(%rbp); mov -72(%rbp), %rcx; ...`). The
  fold guards on the value operand starting with `%` or `$` so a *memory* value
  never becomes an illegal memory-to-memory `mov`; a frame-slot operand falls
  back to the `%rax` round trip. The same fold applies to a field in a promoted
  register (`mov %r, %r`) and to `field_operand`'s `disp(%reg)` form. Measured
  +5% on `heap`. The two store-shape tests (`a_write_through_a_dereference...`,
  `field_write_targets_the_right_slot`) assert the folded form.
- **A simple initialiser or assignment stores straight into its home.**
  `store_simple` collapses `mov X, %rax; mov %rax, dst` into `mov X, dst` when
  `X` is already a bare operand: a small immediate (`movq $k, dst`) or a
  register (`mov %r, dst`). It is used from all four store sites — a `let` into
  an unpromoted local, an assignment to one, a struct field in a promoted
  register (via `gen_expr_into_reg`, so `mov $k, %rbx`), and a field in memory —
  and never accepts a memory operand, which would make an illegal
  `mov mem, mem`. A promoted *local* still uses `gen_expr_into_reg`, so its
  constant becomes `mov $k, %r` directly. Neither an immediate nor a register
  copy can trap, so the fold never changes when a trap would fire; it only ever
  removes a `%rax` bounce. `refs` loses eight instructions from its struct
  literal, and the frame store of a promoted field copied into an address-taken
  local loses one. A *store-to-load forwarding* rewrite of the same shape
  (`mov %r, X; mov X, %rY` -> `mov %r, %rY`) was measured perf-neutral on the
  whole suite and not kept. A fuller textual version that tracked the live
  register per frame slot (`forward_stores`) removed one instruction across the
  whole corpus and cost 3-30% of compile time (`div.rms` 1.30x) for no runtime
  change, so it was reverted too: the generator already keeps the value in a
  register or reloads where a load is genuinely needed, and a whole-body rewrite
  pass is the wrong price for the project's headline metric. `gen_expr_to_slot(e, slot)` is the one entry point
  for "evaluate into a local": the promoted register when there is one, else a
  folded frame store. `let`, assignment and both `for` bounds go through it, so
  a constant `for` start is `mov $0, %rbx` rather than a bounce through `%rax`.
- **An `if` whose whole body is `break`/`continue` branches straight to the
  loop label.** In statement position (`discard`), `bare_exit` recognises a
  single-statement body that is a lone `break` or `continue` and has the `if`
  emit `gen_branch_true(cond, exit)` instead of branch-over-a-jump
  (`jle .L12; jmp .L11; .L12:`). The taken path is one jump, the fall-through
  path none. This is the escape test in `mandel`'s inner loop and the guards in
  `sieve`/`forloops`. Guarded on `discard` so a value-position `if` still
  materialises its unit.
- **`trim_dead` drops unreachable instructions, self-moves, inverse-move pairs,
  and a jump to the next label.** After each function body is generated it
  removes instructions following an unconditional `jmp`/`ret` up to the next
  label, `mov %r, %r`, a `mov` that exactly undoes the previous `mov` (`mov %rax,
  X` then `mov X, %rax`, the store/reload pair a spill-and-restore emits), and a
  `jmp L` whose target label `L:` is the very next line (the fallthrough already
  reaches it — a function whose last expression is a value jumps to the
  epilogue label that directly follows). A `return` inside an expression emits
  `jmp epilogue` then the rest of the expression; the pass is a one-sweep
  fixpoint because removing a line creates neither a label nor a new terminator.
  The inverse-move check compares the full source and destination strings, so it
  only fires on `mov` (there are no `movq $imm` register forms to confuse it)
  and never across a label or any other instruction. It runs on the real body
  only (the dry spill pass's output is truncated anyway), so it costs nothing at
  compile time.
- **A statement-position `if` without `else` emits no unit value.** `gen_stmt_expr`
  sets a `discard` flag while generating an expression whose value is thrown
  away, and `Expr::If` reads it: with no `else` the `xor %eax, %eax` that
  materialises unit (and the `jmp` over the empty else) is dead, so it is
  omitted. In `fib` that removes a dead instruction from the hot path. Only a
  bare `if` is special-cased; a block or `match` in statement position is left
  alone because an `if` in *its* tail is a value. The flag is saved and restored,
  so a discarded outer expression does not silence an inner value `if`.
  Runtime-neutral (the loops are latency-bound) but trims ~40 bytes per artifact.
- **Inlining is a post-sema AST pass, and slots are frame indices.** Each
  inlined call gets a fresh region of the caller's frame (`base = f.nslots`,
  grown as it goes), the callee's slots are shifted by that base, and a read of
  a parameter's slot is replaced by the argument. Two expansions must not share
  a region or an inlined local would clobber another's. An argument with a side
  effect must be bound to one slot, never substituted into a body that reads the
  parameter twice — `f(sc(5))` with `f(x) = x + x` must call `sc` once. A
  parameter the body assigns must also be bound to a fresh slot even when the
  argument is a plain variable, because a parameter is a local copy: replacing
  it by the caller's variable would make `x = x + 1` write through to the caller.
  A candidate body must be call-free with no early exit; because it is call-free,
  inlining it cannot expose new calls, but the pass still runs to a *fixpoint*
  because a function only becomes call-free after its own callees are inlined
  (the `refs` wrapper `sum_via_ref` needs round two). A loop is allowed — a small
  helper like `gcd` has one and is still worth inlining, since that removes the
  call from the caller's hot loop (measured 1.12x on a `tri(i % 50)` shape). A
  `break`/`continue` is *not* allowed: once spliced into the caller it would bind
  to the caller's enclosing loop. A non-final `return` is not allowed either, for
  the same reason. `Sub` already rewrites `while`/`loop`/`for` bodies and their
  counter slots, and `assigned_slots` already descends into them, so a
  loop-bearing body needs no extra splice work. A statement-free block is unwrapped to
  its tail after expansion: the spliced body of an accessor would otherwise stay
  wrapped in an `Expr::Block`, and later passes (`simple_operand`, the
  accumulator fold) cannot see through a block, so the hot loop would bounce its
  sum through `%rax` and the spill slots. `--no-inline` disables it; the tests
  that assert on the calling convention (`call f`, argument setup) use it.
  `MAX_NODES` (48) must stay above a two-level helper — `sum_via_ref` is 35
  nodes once its accessors land — and `count_expr` must count a block's tail,
  or a candidate's size is understated and it is wrongly admitted.
- **LICM hoists a loop body's invariant arithmetic, and `for` slots are
  loop-carried.** The pass rewrites only a loop *body*; a `while` condition and
  a `for`'s bounds run once per iteration by design. A `for`'s variable and
  bound are assigned by the loop itself, not in the body, so they are handed to
  the effect scan as explicit `carried` slots — miss this and `if i == 3` in a
  `for` body gets hoisted and the loop runs wrong. It hoists only a pure,
  non-trapping scalar: no call, division/modulo, index, dereference, block or
  `if`, and a field read only when the loop writes no memory at all. Any call in
  the loop disables the whole loop, since a call can write anything. The effect
  scan must also see writes that do not name a slot directly: a `while`
  condition and a `for`'s start/end run as part of the loop, taking a local's
  address (`&mut x`) marks that slot as written (an inlined callee's deref store
  reaches it), an assignment target is scanned as an expression
  (`a[next(&mut x)] = 0`), and assigning a whole aggregate marks the loop as
  writing memory. The pass is off under `--overflow-checks` (a hoisted checked
  `+`/`-`/`*` would trap at the wrong time); `--no-licm` disables it outright.
- **Stack slots grow downward.** A local in slot `s` lives at
  `-(s+1+nregs)*8(%rbp)`, where `nregs` is the number of registers holding
  promoted locals. Address them positively and you overwrite the saved
  `%rbp`/return address and segfault. See `Codegen::off`.
- **Spills live in frame slots, not on the stack.** `push_rax`/`pop_to` write a
  slot instead of `push`/`pop`, so `%rsp` never moves and no alignment pad is
  needed around a call. The first few spills are held in callee-saved registers
  (`spill_regs`); only spills past those reach the frame, and they are packed at
  `spill_base + (sp_off - spill_regs.len())` so a register-held spill leaves no
  hole. `spill_slots` is sized by a dry run of the body; if the real pass spilled
  deeper than the reservation, spills would overwrite locals. Keeping the two
  passes identical is what makes the reservation exact.
- **The frame is sized to the slots actually used, not to `nslots`.** The parser
  sizes `nslots` for every local a function *could* declare; `off`/`array_off`
  record the deepest local slot the emitted body actually names (excluding spill
  slots, which are counted separately) and the prologue allocates only that much
  plus `spill_slots`. A promoted-local-only leaf like `fib` then needs no frame
  at all: dropping its 16-byte `sub`/`lea` pair took `fib` from 13.5 ms to
  10.6 ms. The measurement is a `Cell` because `off`/`array_off` take `&self`;
  the dry pass measures and the final pass is seeded with the result, so the
  prologue (emitted before the body) allocates the exact size.
- **Two-pass codegen must reset every counter.** `gen_fn` runs the body twice and
  truncates the output between passes; the label counter, string table, spill
  high-water mark and frame-size measurement are part of that reset. Miss one and
  the two passes diverge.
- **Cold blocks keep the hot path straight.** The 64-bit divide slow path
  (the `i64::MIN`/`-1` check and `idiv`) is correct but almost never taken, so
  `emit_div` routes it through `cold_start`/`cold_end` into a buffer appended
  after the function body. Inlined, its ~30 bytes sat between the hot loop's
  `div` and its back edge. The hot path then falls through to the join point and
  `trim_dead` drops the `jmp` it used to need; the cold block keeps its own jump
  back. The measured win is small (~0.5% on `mix`) but consistent, and the
  layout no longer depends on where the slow path happens to land.
- **The dry pass skips `trim_dead`.** The dry run only measures the spill
  high-water mark and the deepest slot, and the dead/round-trip instructions
  `trim_dead` removes change neither, so `Codegen::dry` turns the pass off for
  it. That is a pure compile-time saving on a function whose body is one long
  straight line: `div.rms` (a 531-line `main`) drops 6% of its compile time, and
  the emitted assembly is byte-for-byte identical to running the pass twice.
- **A callee-saved *temporary* is not worth it; a callee-saved *spill* is.** An
  early attempt kept every short-lived value in `%r12`-`%r15` and measured
  *slower* — the unconditional save/restore in the prologue/epilogue cost more
  than the spills it removed. The shipped `spill_regs` is narrower: it holds
  only the temporaries that must survive a call (the dry run reports how many),
  so a function with no spill reserves no register and pays no push/pop. On the
  recursive `fib` (which has exactly one such value per call) that is +23%
  (`--no-tco`: 13.1 ms to 10.1 ms); it is invisible in the default benchmark
  table because `tco` has already rewritten `fib`'s recursion into a loop with
  no memory operation to remove. Do not widen it back to all temporaries, and
  re-measure with `--no-tco` before touching it, or the win hides.
- **Don't gate promotion on a use count.** A single-use local like `gcd`'s `b`
  is copied into `%rbx` once, read once in the loop, and costs a push/pop per
  call, so promoting only slots used more than once looked like a win on `mix`.
  It measured neutral (0.998–1.009× across the suite, `mix` unchanged): the
  extra frame store/load replaces the register push/pop one-for-one. Keep the
  `> 0` filter.
- **Saved registers sit between `%rbp` and the frame.** The epilogue must point
  `%rsp` at the lowest saved register (or add the frame size back) before
  popping, otherwise `pop` reads the frame and the return address is lost.
  `regs.rms` and `calls.rms` fail if this regresses.
- **Only whole-value structs keep all their fields in memory.** A scalar field
  of a struct is promoted like any other local, *unless* the struct is ever
  copied, returned or passed by value, in which case the whole-value copy reads
  and writes memory and every field must stay there. `promote` tracks that
  during its walk (`copied` and the `let` targets) and zeroes those slots. A
  promoted field that stays in memory on the copy path but is read from its
  register on the field path is a silent wrong-answer bug, which is what
  `fieldpromo.rms` guards.
- **A struct's base slot is its last field, and it stays promotable unless the
  struct is used as a whole.** Field `k` lives at `slot + fields - 1 - k`, so
  field `fields - 1` *is* `slot`. A whole-value use (copy, return, pass by
  value, `let` of a whole copy) already marks every slot via `copied`, so the
  base slot needs no special case; a field-by-field use reaches every field by
  address off `array_off` or by register, so the last field competes for a
  register like any other. `field_operand` must test the *field's own* slot
  (`slot + n - 1 - offset/8`), not the struct's base slot, before trusting a
  `%rbp` operand — otherwise a promoted last field reads a stale frame word.
  A payload enum is the exception: it is read as a whole through its base word,
  so that word always stays in memory. Promoting `stats`'s last field is worth
  0.88x -> 1.14x of its rustc twin.
- **A base-less SIB operand must use mod=00 with a disp32.** `lea 0(,%rax,8)` is
  SIB base field 101, which means "no base" only at mod=00; at mod=01/10 it
  selects `%rbp` instead. Emitting a disp8 there reads the wrong address (a
  silent miscompile that surfaced as `4 * c` printing a stack pointer). The
  mirror case is `%rbp`/`%r13` as a base (rm 101), which is invalid at mod=00
  and needs a disp8 0. `elf.rs` unit tests pin both.
- **Field-to-slot uses the slot count, not the field count.** A struct with an
  array field has more slots than fields (`WithArr` is 2 fields, 4 slots), so
  `fields.len()` gives the wrong map. Use `size / 8`.
- **Arrays cannot be promoted.** An element is reached by `base + i*8` through
  memory, so no element slot may be promoted, and neither may an index base.
  `promote` excludes *every* slot an array spans, not just its first.
- **Aggregate element 0 is at the lowest address.** An array or struct in slots
  `s..s+n` has element/field 0 at `-(s+n+nregs)*8(%rbp)`, with item `i` at
  `base + i*8`. Mixing this up makes aggregate writes run upward past `%rbp`.
  `aggregate_base` returns this base for both arrays and structs; use it
  everywhere, including the struct-literal store in `gen_stmt` (an earlier bug
  used `off(slot)`, the wrong end of the range).
- **`for` is not a `while`.** `continue` must run the increment, so `for` keeps
  its own node with a dedicated continue label (`cont`). Desugaring it to `while`
  would make `continue` skip the increment and loop forever. It *is* rotated like
  `while`, but with a different shape: the guard tests the *un-incremented*
  counter at the bottom and loops back to `top`, and the increment sits on the
  fall-through path between the body and the guard. The entry `jmp` therefore
  reaches the guard with `var == start`, the first body runs before any
  increment, and `continue` targets `cont` so it still runs the increment exactly
  once. Duplicating the guard at the top instead (the naive rotation) is what
  would skip the first iteration or double the increment.
- **A `for` bound is compared directly when it is a constant, and through a
  hidden slot otherwise.** `0..=1000` needs no `end_slot`: the guard is
  `cmp $1000, %rax`, so no frame word is written and no reload happens each
  iteration. A non-constant bound (e.g. `0..n`) is evaluated once into `end_slot`
  and compared register-to-register. `count_expr` must count the hidden counter
  and bound slots so they compete for a callee-saved register like any other
  loop-carried local; otherwise the bound is reloaded from the frame every pass.
- **`..=` exits on `jle` back-edge, not `jl`.** The guard is the loop-back
  condition, so the inclusive form loops while `var <= end` (`jle`) and the
  exclusive one while `var < end` (`jl`); a `jg`/`jge` *exit* test is the
  un-rotated mirror of the same thing. `forloop.rms` and
  `an_inclusive_range_uses_a_strict_exit_test` pin both bounds, and `..=` is a
  three-character token, so it must be lexed in the three-char table (before
  `..`), or it lexes as `..` then `=`.
- **Loop bodies are aligned to 16 bytes.** Each loop's body label is preceded
  by `.p2align 4`. Without it the body's start address drifts whenever unrelated
  code elsewhere in the function changes size, and a one- or two-byte shift can
  flip a hot loop between fetch-friendly and straddling alignments - a
  two-byte *shrink* once cost `forloops` 0.69x. Aligning pins the body so the
  same loop compiles to the same speed regardless of what moves around it. This
  needs `trim_dead` to keep directives: it must test the *trimmed* line for a
  leading `.`, because `.p2align` is emitted indented and otherwise looks like
  dead text after the entry `jmp`. `a_loop_body_starts_on_a_sixteen_byte_boundary`
  pins it.
- **A divisor that fits i32 uses `mov` not `movabs`.** `gen_div_const`
  materialises the divisor and the magic multiplier; `mov_imm64` picks the
  7-byte `mov $k, reg` when the value is a sign-extended 32-bit immediate and
  the 10-byte `movabs` only when it is not. Small divisors are common, so this
  trims the divide sequence and the artifact. The in-process ELF encoder
  (`emit_mov_imm` in `elf.rs`) already made this choice, so the default path is
  unchanged and only the `--binutils` artifact and the emitted `.s` shrink
  (~0.7% across the corpus). `a_small_divisor_uses_a_short_immediate` and
  `a_wide_magic_multiplier_keeps_movabs` pin both directions.
- **`mov $imm, %reg` width follows the destination register.** The encoder
  (`enc_mov_generic`/`emit_mov_imm`) picks the encoding from the register name
  and the mnemonic: an unsuffixed `mov` to a 32-bit register (`%eax`) is the
  5-byte `B8+rd` form with no REX.W, an unsuffixed `mov` to a 64-bit register
  (`%rax`) is `C7 /0` (a sign-extended 32-bit immediate) unless the value does
  not fit i32, `movq` is always 64-bit, and `movabs` is always the 10-byte
  `B8+rd` form even for a small value. Getting this wrong is a silent
  disagreement with `as`, which the `--encoding` fuzz oracle and the
  `tests/cases/strcmp` case (a `mov $1, %eax`) catch; `elf.rs`'s unit tests pin
  each form.
- **`while` is rotated.** The test is emitted at the bottom as the back edge
  (`gen_branch_true`, a `jl`/`jne`) with an entry `jmp` reusing the same test, so
  an iteration is one branch fewer than the top-tested form. `continue` targets
  the bottom test and `break` the label it falls into. If you change the loop
  shape, keep both: a rotation that forgets the entry test loops forever on a
  false condition, and one that targets the body for `continue` skips the test.
- **Promotion counting is sparse, not a dense table over `nslots`.** `counts`
  and `mem` are a `HashMap`/`HashSet` keyed by slot, and only slots something
  actually names get an entry. A dense `vec![_; nslots]` plus a sort over
  `0..nslots` is O(array length) — and the array's *element* slots are never
  named, so the table is almost all zeros. A 200k-element array made that path
  take ~1.9 ms (codegen was 90% of the compile); the sparse form is ~20 µs.
  `large_array_compiles_in_linear_time` guards it.
- **An array never needs its element slots marked in memory.** Only the base
  slot can be named (elements are reached as `base + i*8`), so `Expr::Var` on an
  array, a `let` of one, and `mark_addressed` all mark just the base. Marking
  every element is O(length) per mention and promotes nothing that was not
  already unpromotable.
- **Struct layout must agree between read and write.** `sema` resolves a field
  to an offset from the aggregate base; codegen must add that offset to the same
  base used for reads (`field_operand`) and writes.
- **Aggregates cross calls by hidden pointer.** An aggregate argument is copied
  into a `sema` slot and its address is passed; an aggregate result is written
  through a buffer the caller passes in `%r10`. Two orderings are load-bearing:
  the callee must save incoming pointers on the stack *before* any copy, because
  the copy clobbers `%rsi`/`%rdi` (which still hold later arguments). The caller
  evaluates arguments strictly left-to-right and pushes each one (aggregate: its
  slot address after the copy; scalar: its value), then pops them into the ABI
  registers right-to-left, so an earlier argument's source is already copied
  before a later argument runs. Evaluating all aggregates before the scalars
  would run side effects out of order: `f(g(), mk())` must print `2` then `1`,
  not the reverse. `aggfn.rms`/`aggbig.rms` and `argorder.rms` cover this.
- **`%r10` carries the return buffer.** It is caller-saved and not an argument
  register, so it survives argument setup. Do not use `%r10` as scratch anywhere
  argument setup can run after the return pointer is set.
- **`sema` persists the inferred type on every `let`.** Codegen selects the
  aggregate path from `Stmt::Let`'s `ty`, so leaving it `None` for `let a = [7; 2]`
  silently initialises only one word.
- **Struct-literal fields stay in source order.** Their layout offsets are
  attached per field, so reordering them into declaration order would change the
  order initializer side effects run in.
- **Assignment evaluates its value before its place.** rustc orders assignment
  operands right-to-left (the one expression that does), so `a[li(1)] = li(5)`
  must run `li(5)` before `li(1)`. Codegen's general index/deref store path and
  `store_through_addr` evaluate the value first and stash it across the address
  computation whenever that computation can call (`contains_call`), and the
  aggregate copy is factored into `assign_aggregate` for the same reason. The
  `contains_call` walk must reach every place form (`Deref`, `AddrOf`,
  `EnumPayload`, ...), or a target like `*f()` is mis-ordered. `assignorder.rms`
  and `tools/fuzz.py --mode order` pin this.
- **A compound assignment binds its modifying operand first.** `desugar_compound`
  evaluates the right-hand operand into a temporary ahead of the subscript
  temporary, so `a[next()] += val()` runs `val()` before `next()`, matching the
  right-to-left rule above. The subscript is still bound once, so `a[next()] += 1`
  advances the counter once, not twice, and the cheap two-instruction form is
  kept when the subscript has no call.
- **Statements vs. tail expressions.** Assignments inside a block must be
  detected before the trailing-`;` check in `parse_block`, otherwise `x = ...;`
  is parsed as an expression and rejected.
- **Spill slots are frame slots.** `push_rax`/`pop_to` spill to a frame slot
  rather than `push`/`pop`, so `%rsp` is static and a call needs no alignment
  pad. This measured faster on every real program (`fib` +3.3%) because the pad
  costs two instructions per call.
- **`alloc`'s fast paths are a leaf.** The free-list scan, the arena carve and
  the failure return touch only caller-saved registers, so `alloc` keeps no
  frame pointer and saves no callee-saved registers; the prologue/epilogue that
  a `push %rbp` + four `push`es cost was pure overhead on every allocation.
  Measured +7% on `heap` (3.61 → 3.37 ms). Only the mmap slow path needs state
  across the syscall: the total block size rides the stack (the syscall clobbers
  `%rcx`/`%r11`) and the chunk length stays in `%rsi`. Keep new code off the
  callee-saved set, or the whole-function frame comes back.
- **Linking.** By default `src/elf.rs` assembles and links in-process; `as` and
  `ld` are only used under `--binutils`. The in-process path is what makes rmc
  ~4–5x faster to compile than the binutils path (the two child processes were
  ~80% of the wall time) and gives a *smaller* artifact, because it never emits
  a symbol table or a section-header table to begin with. It writes exactly two
  `PT_LOAD` segments — one R+X for `.text`/`.rodata`, one R+W for `.data` — so
  there is no W^X segment. Writable sections start on their own page: the kernel
  maps each `PT_LOAD` rounded down to a page, so a RW segment sharing a page with
  the RX one would have its `mprotect`-like mapping replace the execute
  permission. The runtime's writable globals are all zero, so they are emitted
  as `.bss` (`p_filesz = 0`, `p_memsz` set): mapping them from the file would
  cost a page of padding and about 4 KB on every artifact. When adding an
  instruction or a codegen form, remember the encoder only implements the subset
  the code generator emits and errors on anything else — run `tests/run.py`,
  which compiles every case through both paths and requires identical output.
  The `--binutils` path still uses `ld --build-id=none -z noseparate-code
  --strip-all`; `--no-strip` keeps the symbol table, `--min-size` also runs
  `strip --strip-section-headers`. Do not change the `--binutils` link back to
  `-n` (nmagic): it packs every section into one read+write+execute `LOAD`
  segment, a W^X violation, and is *larger*. `tests/run.py`'s `check_stripped`
  pins the size ordering of all four variants, `check_no_rwx` fails on any
  writable+executable `LOAD` segment, and `check_link_paths_agree` requires the
  two link paths to behave identically.
- **In-process assembler gotchas.** The encoder in `src/elf.rs` is deliberately
  narrow; these are the encodings that are easy to get wrong and were each a
  real bug:
  - Group opcodes: `inc`/`dec` are group-5 (`FF /0,/1`) but `not`/`neg` are
    group-3 (`F7 /2,/3`). `FF /3` is `call far`, which faults at run time.
  - ModRM direction: for `mov src, dst` and `op src, dst` the *source* is the
    reg field (REX.R) and the *destination* is r/m (REX.B). Swapping them
    silently moves the wrong register.
  - SIB: a byte is required for an index, for no base, and for a base of
    `%rsp`/`%r12` (rm encoding 100). `(%base,%index)` is two parts with an
    implied scale of 1; parsing only three-part forms drops the index (a
    `lea 0(%r,%r)` used as `x*2` came out as `x`).
  - A base of `%rbp`/`%r13` (rm encoding 101) needs an explicit displacement,
    or mod=00/rm=101 means "disp32, no base".
  - RIP-relative memory (`sym(%rip)`) writes the disp32 *after* the ModRM, not
    before the opcode; the fixup is recorded at the trailing four bytes.
  - Byte operands: `mov $imm, mem` and byte `mov %dl, mem` use the byte opcodes
    (the mnemonic `mov` carries no `b` suffix but a byte-width register makes it
    a byte move). A byte register `%sil`/`%dil` (>= 4) needs a bare `0x40` REX
    prefix to mean itself; `setcc` does *not* (a byte name is not an extended
    register). `movzbq` sets REX.W, `movzbl` does not.
  - `mov $imm, %r64` sign-extends a 32-bit immediate (`C7 /0`); a negative value
    through the zero-extending `B8+rd` form would clear the high half.
  - Symbol immediates (`mov $__rmc_overflow_len, %rdx`) come from `.set name,
    . - other`; resolve them after all labels are placed, not during the walk.
  - Operand size comes from the register the caller names, *not* from the
    mnemonic. `div %ebx` is the 32-bit form (`F7 /6`, no REX.W) and `div %rbx`
    is the 64-bit one; forcing REX.W turned every 32-bit divide into the 64-bit
    instruction. On the divide-bound `mix` loop that was **14% slower** at run
    time, and it made the in-process binary disagree with `as`+`ld` for the same
    source. The generator picks the 32-bit form deliberately (see `emit_div`),
    so the encoder must honour it. `enc_div` now derives REX.W from
    `reg_is_64`; a `None` REX byte is omitted entirely, matching `as`.
  - Register-operand width must survive encoding. A 32-bit register name
    selects the narrow form and must not set REX.W: `xor %eax,%eax` is the
    two-byte zeroing idiom (`31 C0`), `mov %ebx,%eax` is `89 D8`. Forcing W
    gave `xor %rax,%rax`/`mov %rbx,%rax`, which grows the artifact and
    disagrees with `as`. A bare `0x40` REX byte (no W, no R/X/B) is dropped
    entirely, matching `as`; an extended register (`%r8d`) still needs its
    REX.R. `reg_is_64`/`reg_is_32` decide, and `rex_if_needed` omits the
    no-op byte.
  - An unsuffixed `mov $imm, mem` is a *32-bit* store. `as` has no register
    operand to infer a width from, so it picks `movl` and warns; only an
    explicit `movq` stores 64 bits. The generator must therefore size every
    immediate-to-memory store (`store_operand` does), and the encoder must
    reproduce `as`'s default rather than promoting the store to 64 bits.
    Getting this wrong leaves the destination's high four bytes stale, which
    is only observable when the slot previously held a value with a set high
    half — the payload-enum path zeroes the slot first, so the encoding test,
    not a behaviour test, is what catches it.
  - `.p2align n` means "align to 2^n", `.balign`/`.align n` mean "align to n
    bytes". They shared one arm that treated both as a byte count, so every loop
    the generator padded with `.p2align 4` came out 4-byte aligned in-process
    where `as` gave 16. The bug was invisible to the behaviour-only
    in-process-vs-`as` test; only a byte-level check catches it. `run.py`'s
    `check_encoding_matches_as` now compares the decoded instruction streams
    (folding branch targets and dropping alignment `nop`s), so an operand-size
    or alignment regression fails the suite.
- **`.balign`/`.p2align` pads executable sections with `nop`, not zero.** The
  in-process encoder fills an aligned gap with `0x90` in a section whose ELF
  flags include `X` and with `0x00` elsewhere. A zero byte decodes as
  `add %al,(%rax)`, which faults if a jump lands in the pad; the one-byte `nop`
  is safe (GNU `as` would emit a multi-byte `nop`, but the encoder only has the
  single-byte form). `pad_to_align` is shared with `.rodata`/`.data`, where
  zeros are what a reader wants, so the fill is chosen from the section flags.
- **Runtime pruning.** `runtime.rs` stores the runtime as `PIECES`, each naming
  the symbols it defines; `for_program(user_asm)` keeps `_start` plus every
  piece the program (or a kept piece) names a symbol of, to a fixpoint. A new
  runtime helper is a new piece, and its `defines` list must include *every*
  symbol the linker needs — the function label *and* any global it owns (e.g.
  `alloc`'s piece must name `__rmc_arena`/`__rmc_freelist`, or the arena
  vanishes and the link fails with an undefined symbol). Symbol matching is
  word-boundary aware and strips `#` comments, so `free` does not match inside
  `__rmc_freelist` or in `alloc`'s "free list" comment. If a trap stops being
  emitted, its piece drops out — that is the point, but a test that asserts on
  runtime text must first force the construct that pulls the piece in (see
  `the_bounds_handler_is_in_an_executable_section`).
- **`for_program` must tokenise, not rescan per symbol.** The piece texts are
  static, so their identifier sets are built once (`piece_idents`, a
  `OnceLock`); the program is tokenised once (`idents`) and each symbol looked
  up in the set. The old form called `mentions` for every (piece, symbol) pair
  over the *whole* text — O(text × symbols), the single largest cost in a
  compile. Keep the whole-identifier matching: a substring test would pull in
  `free` for `__rmc_freelist`.
- **Dead-function elimination feeds runtime pruning.** `dce.rs` runs just before
  codegen and drops every function `main` cannot reach, so a dead function that
  called `print_str`/`alloc` no longer drags those pieces into the binary
  (`unused_helper` calling both: 1688 → 1016 bytes). Reachability
  over-approximates on purpose — a call in a branch that cannot run still keeps
  its callee — because dropping a live function is a miscompile while keeping a
  dead one is only size. A program with no `main` is left untouched so the
  "undefined reference to `main`" link error survives. Unit tests that inspect a
  helper's own codegen must now either have `main` call it or use
  `asm_no_inline`; twelve such tests were updated when the pass landed.
- **A runtime function must be emitted in `.text`.** `-n` merged sections so a
  misplaced handler still ran under the CLI link, but with any sane link (the
  library path, or the CLI's `-z noseparate-code`) a handler left in `.rodata`
  is mapped without execute permission and faults. `__rmc_bounds` follows a
  `.rodata` message, so it re-opens `.text` first; check this whenever a trap is
  added after a `.section .rodata` block. `tests/run.py` links out-of-range
  programs with a plain `ld` to catch it, across every optimiser configuration (a
  hoist or inline that drops the `cmp`/`jae` would let the program read or write
  out of range), and across the index shapes: a block or call base, a local
  array, a negative and a computed index, a struct-field array, and a store.
- **`array_len_of` must cover every array-typed base.** It reads the length from
  a `Var`/`Field`/`Call` type and from a block's or `if`'s tail. A base it does
  not recognise silently skips the bounds check; extend it alongside any new
  array-valued expression form.
- **Evaluate the base before checking bounds.** Rust runs the base and index
  before the bounds check, so a call or block base with a side effect must run
  even when the index is out of range. `check_bounds` uses no register, so
  emitting `gen_expr(base)` first leaves the address in `%rax` for the load.
- **A repeat literal `[v; n]` has no address.** Codegen only materialises an
  `ArrayLit` in `let`-initialiser position (the splat path); anywhere else it
  must be built into a buffer first, or it is emitted as a bare scalar and
  dereferenced as a pointer. Sema rewrites a value-position `[v; n]` to
  `{ let __rep = [v; n]; __rep }` via `check_let_init`, which reuses the splat.
  The list form `[a, b, c]` is unaffected: the parser already desugars it to a
  block. Keep `check_let_init` special-casing only the *direct* initialiser; a
  flag around the whole initialiser leaks into nested literals (a struct
  literal's fields are checked while it is set).
- **`gen_addr` must handle every aggregate-valued base.** A field or index
  whose base is a call result, a block, an `if`, or a struct literal is a
  *temporary*: its value is its own address. `gen_addr` matches those forms and
  evaluates the base, leaving the address in `%rax`. Only `Var`/`Field`/`Index`
  are true lvalues; the catch-all is `unreachable!`, so a new aggregate-valued
  expression form must be added here too or the compiler aborts on
  `mk().y`-style access.
- **Benchmark inputs must be opaque.** Programs read their workload via
  `arg_i64` (rmc) or `env::args()` + `black_box` (Rust), so LLVM cannot
  constant-fold the workload away. Do not "simplify" them to constants.
- **`print_bool` prints 0/1**, not `true`/`false`, despite the name.

## Benchmark gotchas

- Runtime is process-spawn dominated for small programs; use large enough
  workloads (see `ARGS` in `bench/run.py`) for a meaningful runtime comparison.
  At the recorded sizes the small-program ratios swing several percent run to
  run (a re-measure of `fib`/`mandel` at the same `ARGS` landed at 0.99/1.00
  where `RESULTS.md` had 0.80/0.85), so treat the run column as indicative and
  never chase a sub-percent runtime difference without many repetitions.
- **A `*` of two runtime values is not provably non-negative.** `(x*x)/C` looks
  like it can skip the signed-division sign fixup, but `i64` multiplication
  wraps, so `x*x` may be negative and rustc keeps the fixup. Removing it is
  unsound; do not add a "square is non-negative" case to `gen_div_const`.
- `bench/run.py` measures both compilers and both binaries the same way: wall
  and CPU time from `wait4()`, and peak RAM from the child's own `VmHWM`. Do not
  read peak RAM from `wait4()`'s `ru_maxrss`: a freshly spawned child shares
  Python's address space before `exec`, so it floors at Python's RSS (~15 MB) and
  hides the real number. `VmHWM` is sampled until the child exits and taken as
  the max over reps, because a sub-millisecond run can exit between samples and
  read back as zero. It raises if a produced binary exits non-zero, which
  catches the classic "measured a nonexistent path" bug. `--markdown PATH` writes
  the dated result to a file; the current one lives at `bench/RESULTS.md`.
- **The sampler yields, it does not spin.** The `VmHWM` poll calls
  `os.sched_yield()` between reads. A tight `wait4(WNOHANG)` spin keeps the
  harness runnable for the whole child lifetime and competes with the benchmark
  on a single-core host, inflating the wall time it is trying to measure. A
  `sleep` would fix that but drops the sample rate enough to miss the peak of a
  sub-millisecond process (`hello` read ~0.8 MB instead of the true ~2 MB), so
  the loop yields and keeps polling.
- **`heap` compares equal strides, not equal allocators.** rmc's `alloc` returns
  a payload 8 bytes past a size header, so its 16-byte nodes are 24 bytes apart.
  The Rust twin must use the same 24-byte stride or its smaller working set
  flatters the walk; it still does not run a free list, so the ratio is about the
  generated pointer code, not the allocators.
- **`refs` is not an instruction-level tie.** Keeping the Rust loop real needs
  an in-loop `black_box`, which costs a store/reload per iteration; rmc has no
  such barrier. rmc's `licm` reassociates the loop-invariant part of the
  accumulation into one hoisted value, so its loop does a single add per
  iteration, and it wins the ratio by more than 2x. Read the number as "two real
  loops doing the same arithmetic", not as an instruction-for-instruction tie.
- **`callrec` is the call-heavy case, and it used to lose.** Non-tail recursion
  (`tree(n) = 1 + tree(n-1) + tree(n-2)`) cannot be inlined by either compiler,
  so it measures raw call overhead plus argument setup, where the stack-based
  generator with no register allocator is weakest. It was ~0.60x rustc until the
  self-recursion pass learned to fold the right spine of `1 + f(n-1) + f(n-2)`
  (not just the `f(g) + f(h)` shape), which leaves one call per level like rustc;
  it now sits at parity. The shape the pass still does not match is the losing
  case, so keep this benchmark: it is what stops the runtime story from being a
  uniform "rmc wins" claim.
- **Artifact size depends entirely on the rustc reference build.** The headline
  table compares against `-O` + fat LTO + `codegen-units=1` + `panic=abort` +
  stripped, which is the *smallest* rustc output (~290 kB) and therefore the
  least flattering to rmc (~85x). A plain `rustc -O` is ~4.5 MB (~1335x) and a
  stripped static build is ~1.2 MB (~350x). `bench/run.py` records all three
  (`measure_rustc_plain`, `measure_rustc_static`) so no single choice overstates
  the gap; report the reference whenever quoting a size ratio.
- **Run-time ratios are noisy and machine-dependent.** A program near parity
  (`fib`, `mandel`) flips between a small rmc win and a small rmc loss from run
  to run and host to host. Do not quote a single ratio for such a program as a
  stable property, and re-measure on the target machine before drawing a
  workload-specific conclusion.
- **A compiled rmc binary's few-kB RSS is mostly "no libc".** Its ~24 kB peak is
  real but is a property of linking no C library, not of the generated code
  being leaner: a dynamically linked rustc binary floors at ~2 MB because of
  `ld.so` + `libc`. Say so when the RAM gap comes up, or it reads as a
  code-quality claim it is not.

## Tests

`tools/fuzz.py --mode reject` compiles a list of programs `rustc` rejects and
requires `rmc` to reject each one cleanly; it pins the soundness checks below,
which the differential fuzzers cannot see (they only compare programs both
compilers accept).

`tests/run.py` compiles each `tests/cases/NAME.{rms,rs}` pair with both compilers
and diffs stdout and the exit status. Every case is then recompiled with the
optimiser passes disabled in turn (`--no-inline`, `--no-licm`, `--no-tco`, and
all three at once), because a bug in one pass can hide behind another or only
show on the unoptimised path; the Rust twin is built once and reused. `aggsem`
(by-value aggregate assignment,
inferred aggregates, nested fields, indexing a returned array), `cmpsem`
(single evaluation of a side-effecting subscript in a compound assignment,
struct-literal field order), `utf8` (non-ASCII string literals), `inlmulti`
(a callee that assigns its parameter, a non-trivial argument evaluated once, and
nested inlining with a call argument), `assignorder` (value-before-place
assignment order, plain and compound), `licmref` (a field read that must not
be hoisted past a `&mut` store in the loop), `iflet` (the `if let`
desugaring: integer, binding, payload-binding, `else if let` chain,
side-effecting scrutinee, and value position) and `consts` (top-level `const`
items in expression position, as array lengths and repeat counts, as a `for`
bound, in conditions, and as `match` patterns, including a negative constant and
one defined in terms of another) guard the
front-end/codegen fixes below. When adding a case, add both twins, write
them to behave identically, and remember that `rustc` infers `i32` for unsuffixed
literals, so large literals need an `i64` suffix on the Rust side. `args.json`
maps a case name to a list of argv vectors. Verify a new case actually detects
the bug it guards: revert the fix and confirm the case fails.

The parser bounds nesting depth (`MAX_DEPTH` in `parser.rs`, currently 100). A
recursive-descent parser with no bound overflows the native stack and aborts on
deeply nested or very long input, which is a crash on untrusted input. Each
expression level threads through every precedence function, so a debug build
costs on the order of 10 KiB per level and the full 100 levels is close to the
2 MiB a default test thread gets; the two depth tests in `tests/compile.rs`
therefore run on a dedicated 32 MiB thread via `with_big_stack`, so they test
the limit rather than the harness's thread size. The CLI runs on the 8 MiB main
thread. `fuzz.py --mode robust` guards the limit end to end. If you add a new
recursive construct, count a level with `bump_depth` on entry.

An enum is a set of unit variants represented by its `i64` tag (the declaration
index). `Expr::EnumVariant` keeps its node and its enum type so re-checking it
(as a method receiver is) stays consistent; codegen emits the tag. Enum names
resolve to `Type::Enum` through `resolve_ty`, which every type site uses, and
enum patterns are a `Pat::Variant` the parser desugars into an `EnumVariant`
comparison. `as` may not target an enum, matching rustc.

A pattern is a wildcard, an integer or `bool` literal, an `Enum::Variant`, or a
bare binding name. A `bool` pattern (`true`/`false`) desugars to a `BoolTest`,
a node like `VariantTest` that compares the scrutinee word; a chain of two of
them covering both values is exhaustive, so `match b { true => .., false => .. }`
is a value. The node exists precisely so a hand-written `if b == true { .. }
else if b == false { .. }` (a plain `Bin(Eq, ..)`) is *not* mistaken for an
exhaustive chain — rustc rejects that in value position, and rmc must too. A
`bool` pattern on a non-`bool` value is rejected. rmc's `match` desugaring also
evaluates a non-trivial scrutinee once through a `\0mt<n>` temporary, so a side
effect in the scrutinee runs exactly once regardless of the arm count.

A `match` is desugared into a chain of `if`s, so a `match` with no `_` arm has
no final `else` and, used as a value, would trip the "`if` without `else`" check
in sema. When the arms name *every* variant of one enum it is total, and rustc
accepts it as a value, so sema recognises exactly that case:
`Sema::if_chain_is_exhaustive` walks the chain, requires every condition to be a
`VariantTest` over one enum (or a `BoolTest` over one variable), and compares the
number of distinct variants (or bool values) tested against the enum's variant
count (or two). Because the desugared chain is checked
outside-in, the innermost `if` is seen first; `Sema::match_chain`/`match_total`
carry the verdict down the chain so the last `if` (no `else`) may stand for a
value. The two flags are set only on the `else` path of a *total* chain, so a
wildcard arm body that is itself an `if` is still checked as an ordinary root.

Every `match` must be exhaustive, in *any* position, exactly as rustc requires
(E0004). The value-position case is caught by the `if`-without-`else` check; the
statement-position case is not, because the arms are unit-typed and the
fall-through to `()` is harmless, so `Sema`'s `MatchBoundary` arm also decides
exhaustiveness (`match_chain_is_exhaustive`, plus the catch-all flag the parser
records) and errors when it fails. Do not relax this to statement position only:
rustc rejects a non-exhaustive `match` there too, and rmc must match rustc.

The `match` desugaring wraps its first step in `Expr::MatchBoundary`, a
transparent node that marks where the chain begins. Its `bool` field records
whether some arm is a catch-all (`_` or a binding), which makes the match
exhaustive regardless of the variants named. The marker exists because a wildcard
arm body can itself be a `match`: `match a { A => 1, _ => match a { B => 2 } }`
and `match a { A => 1, B => 2 }` desugar to the *same* `if` chain, yet only the
second is exhaustive. Without the marker, `if_chain_is_exhaustive` walks through
the wildcard body into the nested match and counts both matches' variants
together, wrongly accepting a program rustc rejects. The boundary stops that
walk and resets `match_chain`/`match_total`, so each `match` is judged on its
own. It is transparent everywhere else: codegen, the inliner, LICM and TCO treat
it exactly like `Expr::Block`, and `is_block_like` lists it so a `match`
statement still needs no trailing `;`. A subtle-scrutinee comparison would *not*
work: the two programs above share the same scrutinee, so the boundary is the
only thing that keeps them apart.
`tests/cases/exhmat.{rms,rs}` covers value position, reordering, nesting,
references and a loop; a non-exhaustive or duplicate-variant match still errors,
and `tools/fuzz.py --mode reject` pins the nested-wildcard and statement-position
cases.

A method is an ordinary free function whose name is mangled to `Type$method` and
whose first parameter is `self` of the impl's struct type; the parser produces
it, sema inserts the `self` parameter before building signatures, and a method
call is rewritten by sema into a `Call` with the receiver prepended. Codegen
therefore has no method case — keep it that way. Method resolution accepts a
receiver whose type is `Struct`, `Enum` (unit) or `EnumData` (payload); all
three give the same mangled prefix, and a `self` method on an aggregate receiver
passes it by pointer. `Self` resolves through the parser's `current_impl`.

An array's element type is a scalar: `i64`, `bool` or a unit enum. Nested
arrays, arrays of structs and arrays of payload enums are rejected. A unit enum
is one 8-byte slot emitted like `i64`, so `[E; N]` lays out and indexes
correctly; rustc likewise accepts a unit enum and rejects a payload enum and a
reference as array elements. A list literal `[a, b, c]` is parsed as
`Expr::ArrayList` and rewritten by sema, which infers the element type from the
values — the parser must not guess it, because it does not type-check, and
guessing `i64` is what made `[true, false]` fail.

## Soundness checks

The differential fuzzers only compare programs that *both* compilers accept, so
they are blind to the dangerous direction: a program `rmc` accepts but `rustc`
rejects. Such a program is never run against the reference (the reference does
not build), so a wrong-answer or memory-safety bug it triggers is invisible to
the suite. `tools/fuzz.py --mode reject` pins this class instead: a deterministic
list of programs `rustc` rejects, each of which `rmc` must also reject (cleanly,
exit 1). When you find a new `rmc`-accepts/`rustc`-rejects divergence, add it
there. The hand-audit harness that found the original set lived outside the repo
(`/tmp/audit5/`); the durable record is the reject corpus.

The checks, all in `sema.rs`:

- **Mutability (`E0384`/`E0594`/`E0596`).** A write must target a mutable
  *place*. The check is `Sema::place_writable`, which walks a target to its root
  binding: a `mut` binding, or a reference whose referent is mutable
  (`&mut T`), is writable; a shared reference (`&T`) is not; an uninitialized
  binding is writable when the caller passes `allow_uninit` (an assignment
  initializes it — whole, by field, or by element, matching rustc's partial
  initializer), but not for a `&mut place` borrow (borrowing an uninitialized
  place is itself E0381). A bare-`Var` write of a non-`mut` binding that is
  already initialized is E0384; a field/element write through a non-`mut` root
  is E0594; `&mut x` on a non-`mut` binding is E0596. Checking the *place* rather
  than just the target node is what closes field/element assignment and
  `&mut`-borrow divergences that a bare-`Var` check missed. `mut` on a parameter
  is recorded in `FnDef::param_mut` (the parser used to accept and discard it)
  and a parameter is otherwise immutable: `fn f(x: i64) { x = 1; }` is rejected,
  `fn f(mut x: i64) { x = 1; }` is not. The scope entry is a `Binding` struct
  carrying `mutable` and `init`. Because a `&mut self` receiver has type `&mut
  Self`, `self.n = ..` stays legal without `mut self`.
- **Definite initialization (`E0381`).** `Binding::init` records whether a
  binding is initialized at the current point, and is set only by a whole-binding
  write — a field or element write does not initialize the aggregate, matching
  rustc's "partially assigned" rule, so reading it afterwards errors. A read of
  an uninitialized one errors. An `if`/`else` keeps a binding initialized only if both arms (or the
  entry state) initialized it; a branch with no `else` and a loop body do not
  count, because they may not run. This is deliberately conservative — it rejects
  the uninitialized-array-filled-in-a-loop pattern (`let mut a: [i64; N]; while
  ... { a[i] = ..; } a[0]`), which rustc also rejects. Write the array with `= [0;
  N]` instead; the corpus cases do.
- **Constant array bounds.** A constant index outside the array (`a[5]` on a
  `[i64; 2]`, the negated `a[-1]`, or an expression like `a[1 + 4]`) is a compile
  error, with or without `--bounds-checks`, matching rustc (`E0080`/`E0608`).
  The index is folded with `const_i64`, which evaluates the integer operators
  (`+ - * / % & | ^ << >>`) as well as unary `-`, so an index written as an
  expression is caught too — an earlier version folded only literals and let
  `a[1 + 4]` read adjacent stack memory. `Sema::const_of` extends this to a
  constant *local*: `let i = -1; a[i]` is folded the way rustc's constant
  propagation folds it, which an earlier version did not, letting `a[i]` read
  past the array. A *variable* index is still unchecked by default (see Scope).
- **String typing.** A string literal is `Type::Str`, not `i64`. It cannot be an
  arithmetic operand (`1 + "hi"` was accepted as integer addition on the
  string's address, `E0369`) and cannot be passed where an `i64` is expected
  (`print_str(42)` used to compile and then segfault, dereferencing the integer
  as a pointer). `print_str` takes `Type::Str`.
- **String comparison.** A `str` value is the address of its `{len, bytes}`
  record, so a machine `cmp` compares *pointers*: `"abc" == "abc"` answered
  false because the two literals live at different addresses (a silent
  wrong-answer bug — rustc says true). `Sema::str_cmp` rewrites `==`/`!=`/`<`/
  `<=`/`>`/`>=` on two strings into calls to the runtime helper `__rmc_str_lt`,
  which walks the bytes, so ordering matches Rust's. `str` has no source syntax
  (no type, no return type), so both operands are pure literals or variables and
  the helper is never double-evaluated. Comparing a `str` with an `i64` is
  rejected (`E0308`), as rustc does.
- **Parameter names (`E0415`) and `main` (`E0580`).** A function's parameters
  must be uniquely named; `check_fn` rejects a duplicate (rmc used to bind both
  to one slot). `main` takes no parameters, checked in `analyze`.
- **Casts (`E0054`/`E0605`/`E0606`).** `check_cast` restricts `as` to the
  conversions rustc allows plus rmc's two documented extensions. A reference may
  only become a reference to the same type (`&mut T` may lose mutability to
  `&T`, never gain it), and never a scalar; any other scalar may not become a
  reference. The extensions are `i64 as bool` (rustc spells it `x != 0`) and the
  raw pointer cast `i64 as &T` / `i64 as &mut T` (rustc needs `as *const T`
  first), both used by the corpus (`boolops`, `listptr`) and the heap benchmark.
  Before this, `r as i64`, `r as &mut T`, `&T as &U` and `bool as &T` all
  compiled, reinterpreting a pointer as a value or forging mutability.
- **Constant arithmetic (`E0080`-like).** `const_binop` folds an operator whose
  operands are both constant (through `Sema::const_of`) and rejects the cases
  rustc denies: division/remainder by zero, `MIN / -1`, `+`/`-`/`*` overflow,
  and a shift amount outside `0..64`. Under the overflow checks rmc emits each
  of these traps, so an accepted one would fault at run time (rmc used to emit an
  `idiv` that raised SIGFPE). A constant local counts as constant, so `let d =
  0; 1 / d` and `let x = i64::MAX; x + 1` are diagnosed like the literals, as
  rustc does.
- **Constant-local propagation, matching rustc.** `Sema::const_locals` maps a
  slot to a known constant, and `const_of` reads it. A local is entered only
  when rustc would know it: its initializer is constant and it is assigned at
  most once in the whole function and never taken by `&mut`. `collect_assigns`
  pre-scans the body (before slots exist, so it keys on names) for `=`
  assignments and `&mut` borrows. The whole-function, order-independent test is
  what keeps the fold in step with rustc: `let mut x = i64::MAX; x = x + 1;` is
  *accepted* because `x` is assigned twice, so rustc does not propagate its
  constant and neither must rmc (folding it would wrongly reject the program).
  A deferred single init (`let i; i = -1;`) is the one assignment that is
  folded, via the `Assign` arm, which is also where the target's slot is
  resolved. This is the safe direction: rmc folds a strict subset of what rustc
  folds, so it never accepts a program rustc rejects.

`!` on an integer is bitwise not, as in Rust: sema accepts `bool` and `i64`
operands (it yields the operand's type) and codegen emits `not %rax` for the
integer form and a `sete` for the `bool` form, selecting on the operand's
generated type. `licm::expr_ty` follows the operand for a `!` so a hoisted
bitwise not is bound to an `i64` slot, not a `bool` one.

The `i64::MIN` literal `-9223372036854775808` is accepted: the lexer parses the
magnitude `9223372036854775808`, which exceeds `i64::MAX`, as a distinct
`IntMinMagnitude` token rather than an error, and the parser folds a `-` that
directly precedes it into `Int(i64::MIN)`. Anywhere else the magnitude is out of
range and rejected, as in rustc — `9223372036854775808` alone, `0x10000000000000000`,
and `- -9223372036854775808` all error. Negating a constant `i64::MIN` (from the
literal or a constant local) is rejected by the same `arithmetic_overflow` check
rmc applies to constant arithmetic, matching rustc's deny lint. The magnitude
test is structural (`is_i64_min_magnitude`), covering decimal, hex, binary and
octal; a spaced `- 9223372036854775808` still folds, as in rustc.

`tools/fuzz.py` is the differential fuzzer. `expr` mode builds random integer
expressions with calls, guarded division and comparisons (`(x < y) as i64`,
which is the only generator that feeds `emit_cmp`'s operand-class dispatch:
immediate/immediate, immediate/memory, memory/immediate, memory/memory);
`stmt` mode builds loops,
conditionals, `match`, arrays and aggregate calls, and deliberately places
side-effecting scalar and aggregate arguments in every position (this is what
found the argument-evaluation-order bug); `opt` mode builds a self-recursive
right-spine function (TCO), a loop with an invariant subexpression (LICM) and
calls to small helpers with trivial and non-trivial arguments (inlining) — the
shapes the other two modes never emit, so the optimiser passes are exercised
with randomised inputs rather than only the fixed corpus cases. `arith` mode
builds overflowing arithmetic near `i64::MAX` — a `lea`/`imul` shape and a
loop-carried accumulator, the two strength reductions `gen_expr_into_reg` makes
— so the `--overflow-checks` trap actually fires: the other modes use small
values, so no generated program ever traps and the trap was reached only by the
`ovf` corpus case. `nest` mode builds nested aggregates — a struct holding
another struct and an array, and an enum whose payload is a struct — with a
deep copy (`let mut o1 = o0`) and a nested-path write (`o1.mid.f = ...`,
`o1.arr[0] = ...`) that the copy must not alias: `stmt` mode only ever gives a
struct plain `i64` fields, so the field-offset arithmetic for a nested path and
the aggregate copy were never generated. `pat` mode builds `match`
expressions: a multi-variant unit enum, a variant with two payload fields, a
binding pattern, a negative literal, a nested `match`, an `if let` (integer,
binding, payload-binding, `else if let` chain and a side-effecting scrutinee),
and a non-trivial
side-effecting scrutinee (a call) tested against several arms — the last is
what exercises `desugar_match`'s temporary, which must evaluate the scrutinee
exactly once (re-evaluating it per arm repeats a side effect). It compares
stdout and exit status against `rustc -O`, and it recompiles each program with
every optimiser pass off in turn (`--no-inline`, `--no-licm`, `--no-tco`, all
three), the same matrix the corpus suite uses — a pass-specific bug is otherwise
invisible, because the fuzzer used to run only the default configuration.
`--bounds` adds `--bounds-checks` runs (every generated index is in bounds, so a
spurious trap or a check-codegen miscompile shows up) and `--overflow` adds
`--overflow-checks` runs, each compared against a `rustc` built with
`-C overflow-checks=on` (the default `rustc -O` wraps, so it would disagree by
design). The temp directory is per-PID: two concurrent fuzz runs sharing one
would overwrite each other's `p.s` and report a bogus assembler failure.
A generated shift amount must stay `i64`-typed: rustc infers
`i32` for an unconstrained literal and masks the count to 5 bits, while rmc is
`i64`-only and masks to 6, so an untyped count reports a divergence that is not a
bug. `stmt` mode's reference build also has to be valid Rust in its own right:
`rustc` indexes an array with `usize`, so every generated `arr[...]` in the
reference carries an `as usize` cast (rmc is `i64`-only), and the `fn h` helper
is inserted at the *front* of the reference — inserting it after the struct's
`#[derive(Copy, Clone)]` puts that attribute on a function, which rustc rejects
outright. When that happened the whole `stmt` mode was vacuous: rustc refused
every generated program, the fuzzer skipped all of them, and the mode reported a
clean run while testing nothing. If a fuzz mode ever reports a suspiciously
pristine run, check its skip rate rather than trusting the summary.

`--encoding` makes the fuzzer compile each program both ways and compare the
decoded `.text` instruction-for-instruction (branch targets folded, alignment
nops dropped, immediates reduced to a placeholder), the same normalisation the
corpus suite's `check_encoding_matches_as` uses. This is what catches an
operand-size or alignment bug that both builds execute identically — the
in-process encoder and `as`/`ld` can produce the same behaviour from different
bytes, and only the decoded stream shows it.

The fuzzer's grammar is narrower than the language, so it does not reach every
construct; three divergences found by hand-probing were an enum `match` with no
`_` arm used as a *value* (not a statement), a method on a *payload* enum, and an
array of a unit enum. `stmt` mode now emits all three, so a regression in any of
them fails the fuzz job at `iter=0`. When adding a language feature, extend the
generator to exercise it — a clean fuzz run only covers the shapes it emits.

## Scope

This is a small, self-contained compiler, not a general Rust compiler. Do not
claim otherwise. It supports only the subset in `docs/language.md` (scalars,
arrays, structs, enums, `for`, aggregates across calls, references `&T`/`&mut T`
with no borrow checking). It has no *general* register allocator: whole-function
locals are promoted to callee-saved registers by mention count (`promote`), and
temporaries crossing a call use `SPILL_REGS`, but a dying temporary's register is
not reused for a later local (roadmap item 1). The optimiser is an AST inlining
pass (small, straight-line, scalar functions), a self-recursion-to-loop pass
(`f(x) = t(x) op f(h(x))` with `op` associative and `t` any pure expression that
may contain a self-call becomes a loop with one call per level, halving the call
tree of both `fib` and `tree`), a loop-invariant code motion pass, plus a
small peephole pass (operand/comparison/array-address folding, constant folding,
division by a constant, a 32-bit `div` fast path for a variable divisor). A *constant* index out of
range is a compile error; a *variable* index is unchecked by default (used
as-is, like C), so an out-of-range one reads or writes adjacent stack memory.
`--bounds-checks` turns on a per-index runtime check that traps (see the codegen
notes and "Soundness checks").
The README's roadmap lists the intended next steps.

## Codegen notes

- Every expression leaves its result in `%rax`; `%rcx`/`%rdx` are the only
  scratch registers used. No other register is written by expression code, which
  is what makes the direct-argument call shortcut safe (see `gen_call`).
- **A benchmark twin must `black_box` its workload.** rmc's `arg_i64` is opaque
  to rmc, but the Rust twin's `std::env::args().nth(1).parse()` is not opaque to
  LLVM: it sees the whole program and can fold a loop into a closed form. The
  `refs` twin did exactly that — LLVM reduced the loop to `acc = 128 * n`, so the
  recorded "rustc 0.83 ms" was its own constant folding and the ratio read 0.18x
  instead of the honest ~1.0x. Every twin with a loop now threads its trip count
  or a term through `std::hint::black_box`. When a rustc twin's time matches its
  `hello` time, suspect folding before celebrating. The `heap` twin uses a bump
  arena rather than `Box`/`malloc`, so the comparison is the generated pointer
  code, not rmc's free list versus glibc.
- **A destination address must survive the value it is assigned.** Assigning a
  whole aggregate to a field or element computes the destination first, and
  `gen_expr(value)` may clobber `%rdi` (a call, or another aggregate copy), so
  the address is pushed to a frame slot and popped back after the value. Writing
  `%rdi` before evaluating the value silently corrupts the destination.
- **The direct-argument call shortcut needs `%rdi`/`%rsi` preserved.**
  `contains_call` is not enough: a block's `let` initialiser can copy a whole
  aggregate (`copy_words_rdi_rsi`) with no call in sight. `clobbers_arg_regs`
  catches that case; add to it, not just to `contains_call`, when a new
  expression form can copy an aggregate.
- **A value-returning function must not fall off the end.** `check_fn` rejects a
  `Unit` body (no tail, or a tail like an `if` without an `else`) unless every
  path returns, using `block_diverges`. It is deliberately conservative: an
  `if` with no `else`, a `while`, and a `loop` are not treated as guaranteed
  returns.
- **With `--bounds-checks`, an array element is never a bare operand.**
  `simple_operand` returns `None` for `Expr::Index` when `self.bounds` is set, so
  every access is materialised through `gen_expr`/`gen_addr`, which emit the
  check. If you add a code path that folds an element into an instruction
  operand, it must also emit the check, or an out-of-range index can bypass it.
- **`lea_shape` must keep its displacement in the signed 32-bit range.** Each
  constant in a folded `k*x + c` chain is representable alone, but their sum is
  not; `disp_ok` guards the fold, and a rejection falls back to ordinary code.
- **Generated temporaries use unspellable names.** Compound-assignment
  desugaring introduces `let`s whose names contain a NUL byte, which no source
  identifier can contain, so they can never shadow a user variable. Do not
  switch them back to a spellable `__rmc_...` name.
- **`copy_words_rdi_rsi` must not touch `%rax`.** It copies through `%r8`, so a
  caller can hold a live value in `%rax` across the copy (a pending operand, or
  an argument already loaded into a register). It also clobbers `%rdi`/`%rsi`,
  which is why `contains_call` treats a struct/array literal as call-like: a
  literal is materialised through this copy, so it must not be selected by the
  direct-argument shortcut that requires register preservation.
- **`gen_expr_into_reg` folds arithmetic into one `lea`.** `lea_shape` recognises
  `k*x + c` for the scales `lea` can express (1, 2, 3, 4, 5, 8, 9) and a constant
  offset, so `2*i+3` becomes one instruction. It is only valid when overflow
  checks are off (a `lea` cannot trap); with `--overflow-checks` it falls through
  to `gen_expr`, which emits the `jo`.
- **`x * k` for a small constant `k` is a `lea`, not an `imul`.** The immediate
  arm of `gen_bin`'s `Mul` calls `lea_mul_scale`: for a positive `k` in
  {1,2,3,4,5,8,9} it emits one `lea 0(,%rax,scale)` (1 cycle) instead of
  `imul $k` (3 cycles), and `k == 1` drops the multiply entirely. This is the
  `2 * zr` in `mandel`'s inner loop — worth ~3% there. It needs the same
  overflow-checks guard as the `lea_shape` fold, because `lea` sets no flags and
  `imul`'s `OF` is what the trap test reads. `imul` is still used for a
  non-`lea` scale (`7 * x`) and for a negative constant (`-2 * x`, where the
  scale cannot be encoded). `gen_expr_into_reg(e, dst)` is also the
  safe way to materialise an expression straight into a destination register
  during argument setup, and a scalar `let`/assignment writes straight into a
  promoted local's register rather than bouncing through `%rax`.
  - **Retargeting a trailing `mov SRC, %rax` to `dst` was measured and
    rejected.** When `gen_expr` ends with a move into `%rax` (the modulo-by-
    constant tail's `mov %r8, %rax`), emitting `mov SRC, dst` instead of
    `mov SRC, %rax; mov %rax, dst` saves an instruction, but it measured neutral
    at run time (the loops are latency-bound, and register renaming hides the
    extra move) and shrank the artifact by only 8 bytes on two of eight programs.
    The transform also has to be careful: truncating to the pre-expression
    offset wipes the whole expression, not just the final move.
- **A constant initialiser stores straight to its frame slot.** `store_imm`
  folds `let x: i64 = 7` and an unpromoted struct field `P { x: 1 }` into one
  `movq $7, off(%rbp)` instead of the `mov $7, %rax` + `mov %rax, off(%rbp)`
  bounce the value would otherwise take through the accumulator. It only fires
  when the destination is memory (a promoted local keeps its register store) and
  only for a constant the `small_imm` path accepts, so no overflow check is
  owed. This is the eight-field struct literal in `refs` and the small scalars
  in `stats`/`mandel`; the instruction count drops without changing behaviour.
- **`emit_cmp` folds a register operand.** `cmp %r, %r` and `cmp %r, mem` are
  legal, so a comparison between two promoted locals is one instruction; only a
  `mem, mem` compare must materialise one side into `%rax`.
- **Aggregates are copied, not aliased.** Whole-aggregate assignment, struct
  fields of aggregate type, array elements of aggregate type, and struct-literal
  fields all copy their words. A field or element read of aggregate type yields
  an *address*, not a value, and `simple_operand` returns `None` for it so no
  scalar path can treat the address as data.
- `simple_operand` returns operands usable with no register at all: immediates,
  scalar locals, constant-index array elements, and struct fields of a local.
  `array_elem_operand` returns the `disp(%rbp,%rax,8)` form whose index must
  first be loaded into `%rax`.
- Folding arithmetic memory operands into `add`/`imul` and the plain two-operand
  in-place `x = x + e` form were both tried and measured *slower* on the Zen
  cores this was tuned on, so they were removed. Keep the plain two-register
  form unless you re-measure on the target CPU.
  - Folding a *register* operand (never a memory one) into `+`, `-`, `*`, `&`,
    `|`, `^` is the one exception and was measured faster: it drops the
    `mov y, %rcx` bounce, taking `mandel` from 1.03× to 1.10×. The distinction is
    the load: a memory operand lands in the instruction's dependency chain (and
    regressed `mandel` by ~19% when tried), a register operand does not. Only
    `simple_operand` results that start with `%` fold.
  - **Commutative spill fold.** When neither operand is simple, the
    order-preserving path evaluates the right operand, moves it to `%rcx`, then
    restores the left from the spill. For a commutative op (`+`, `*`, `&`, `|`,
    `^`) the left can be popped straight into `%rcx` instead, one move fewer;
    `eval_operands` takes a `commutative` flag and `sub`/`div`/`mod`/comparisons
    pass `false`. Measured ~10% on `stats`.
  - **`idiv` reads a promoted-local divisor directly.** `a / y` no longer bounces
    `y` into `%rcx`; the divisor is already in a callee-saved register and
    `gen_expr(a)` plus `cqto` only touch `%rax`/`%rcx`/`%rdx`, so it stays live.
  - The multi-term **accumulation fold** (`gen_accumulate`) is a narrower form
    that was measured neutral: it applies only when `x` is promoted, no term
    reads `x`'s register, and every term is already a single operand. It cuts
    instruction count (a running sum over locals drops to one `add`/`sub` per
    term; the `refs` loop goes 22 → 12) with no wall-clock regression, because
    these loops are latency-bound on the serial add chain and register renaming
    hides the extra moves.
  - **A term that needs evaluating folds too, when overflow checks are off.**
    `sum = sum + f(i)` emits `call f; add %rax, dst` — one `add` on the
    accumulator — instead of `call f; mov %rax, %rcx; mov dst, %rax; add %rcx,
    %rax; mov %rax, dst`. That is three instructions fewer per iteration in
    `mix` (the call-heavy benchmark), byte-identical output for `refs`/`stats`
    (their loops are fully inlined and have no such term), and neutral-to-better
    at run time. Terms are still emitted in source order, so a call that writes
    a local an earlier term read is unaffected (`mut_aliasing` tests cover it).
    An earlier rejection of this idea (a ~12% `refs` regression) predates the
    inliner: that `refs` loop still held three calls, and folding them reordered
    reads around them. With the loop inlined there is nothing to reorder.
  - **CSE into frame slots was measured and rejected.** De-duplicating repeated
    inlined subexpressions *within* a statement by binding each to a fresh slot
    (`refs`: 18 → 28 ms) loses to recomputing: the extra slot loses the
    five-register promotion contest and the loop round-trips through memory.
    Hoisting the same value out of the loop entirely (LICM) is what wins, because
    the slot is written once, not per iteration.
- Division or modulo by a literal divisor does not use `idiv`. `gen_div_const`
  emits the Hacker's Delight magic multiply (the same algorithm LLVM uses),
  with a sign fixup and, for roughly a quarter of divisors, an extra add of the
  dividend. Powers of two use a shift with a `2^k-1` sign bias.
  - **`imul` clobbers `%rax`** with the low product, so the fixups must read the
    dividend from the `%r8` copy taken before the multiply, not from `%rax`.
    Using `%rax` here silently produces wrong results for large dividends.
  - **The sign bias is required** for power-of-two divisors; without it a
    negative dividend rounds toward negative infinity instead of toward zero.
  - `small_imm` accepts a negated literal, so `x / -1000` takes the fast path.
  - `tests/cases/div.{rms,rs}` covers both variants, powers of two, negatives,
    and boundary dividends, and is the regression guard for all of the above.
- Division or modulo by a *variable* divisor uses `idiv`, but only after a
  32-bit fast path. `emit_div` tests whether both operands fit in 32 bits by
  ORing them and shifting the top 32 bits out (`mov %rax, %rdx` + `or <rd>`,
  `%rdx` + `shr $32, %rdx` + `jnz <slow>`); when both fit, `div <reg>d`
  computes the same result in ~20 cycles instead of ~30–40, and when either does
  not, the 64-bit `cqto` + `idiv <reg>` runs. Its `gcd` operands are always
  small, so `mix` takes the narrow path every iteration — this is what keeps
  `mix` close, though it is still the documented weak spot (0.92×, division-
  bound; see `bench/RESULTS.md`).
  - **The fit test must be unsigned.** ORing the two operands and reading the
    top 32 bits is an unsigned test: a negative operand has a high bit set and
    the shift leaves a non-zero result, so it falls through to the 64-bit path,
    exactly as the earlier `cmp $2^32`/`jae` pair did. A signed test would send
    a negative dividend down the 32-bit path and truncate it. (This replaced the
    pair of `cmp`/`jae` against `mov $4294967296, %rdx`; the OR form is one
    instruction fewer and needs no 10-byte `movabs` in the loop. Copying the
    dividend into `%rdx` first is what lets the shift run without destroying
    `%rax`, which the old 2^32 form could not do.)
  - **The 32-bit divide reads `%rdx` as the dividend's high half**, and the
    `shr $32, %rdx` above already left it zero, so no `xor` is needed before
    `div <reg>d`. `%rdx` is scratch on the fast path because `cqto` has not run.
  - The divisor is read from its own register (`idiv %rbx`) when it is a
    promoted local, and from `%rcx` otherwise; `r32` maps each to its 32-bit
    half. A one-off sweep of 1042 `q(a,b)` pairs across the full `i64` range —
    negatives, the 2^32 boundary, and near-`i64::MAX` — matched rustc exactly.
- **Under `--overflow-checks`, a shift count is range-checked before the shift.**
  `check_shift_count` emits `cmp $63, <count>; ja __rmc_shift_overflow`, because
  the hardware masks a 64-bit shift count to 6 bits: without the check `x << 64`
  silently shifts by 0, while Rust panics. The count is only known to be in range
  when it is a compile-time constant (`const_of`); a run-time count always gets
  the check. With overflow checks off, the shift is unchecked, matching Rust's
  default `<<` (which does not panic on an out-of-range count in release).
- **`tests/cases/divtrap.rms` and `shovf.rms` pin the trap codegen.** `divtrap`
  exercises `MIN / -1` (SIGFPE on x86 `idiv`) and division by zero, both of which
  trap *without* `--overflow-checks` (Rust panics on them by default); it covers
  the constant `-1` divisor (strength-reduced) and a runtime divisor (`idiv`).
  `shovf` (with a `.overflow` marker) exercises an out-of-range shift count under
  `--overflow-checks`. Both are differential (stdout and exit status must match
  the rustc twin) and are what catch a trap that fires when it should not, or
  fails to fire when it should.
- **The `MIN / -1` guard sits in the 64-bit slow path, not before the split.**
  `emit_div` splits on whether both operands fit in 32 bits; a dividend that does
  is never `MIN` (its high bit is clear), so the `movabs $0x8000...; cmp; jne`
  guard only runs on the `idiv` path. Putting it before the split cost every
  division a `movabs`+`cmp`+branch for nothing. The zero-divisor check *must*
  stay before the split: zero fits in 32 bits and would otherwise reach `div`.
- **The self-recursion pass must synthesise the AST the parser would build.** It
  runs after sema, so it has no `Sema` to allocate slots or resolve names. It
  reuses the parameter's own `(name, slot, ty)` for the loop variable, and takes
  one fresh slot (`f.nslots`, then `f.nslots += 1`) for the accumulator; the
  synthesized `Call` carries `arg_slots: vec![(-1, 0)]` and `ret_slot: -1`
  directly, because a scalar `i64` argument needs no hidden buffer and a scalar
  return needs no hidden pointer. Every synthesized `Expr::Var` must carry the
  right `slot`/`ty`, or codegen emits a load from the wrong frame word.
- **The pass is guarded to be exact, not approximate.** Only `+`/`*` (associative)
  qualify; `cond`, `guard` and `h` must be call-free and return-free, and `t` must
  be return-free and call only the function being transformed, or the loop would
  duplicate a side effect or leave without adding the accumulator. It
  is also skipped entirely under `--overflow-checks`, since reassociating checked
  arithmetic traps on a different intermediate than the recursion. A shape that
  does not match is left completely alone. `tests/cases/tco.{rms,rs}` diffs `fib`
  and `fact` against rustc and `tests/cases/callrec.{rms,rs}` covers the
  `1 + f(n-1) + f(n-2)` shape; unit tests pin the non-associative
  `f(x-1) - f(x-2)` case, a `return` in an argument, a foreign call in the left
  operand, and overflow checks.

## Pushing

Pushing needs a token with `contents: write`. The `GITHUB_TOKEN` provided in
this environment is **read-only**: it authenticates as the repo owner and the
REST API even reports `admin` on the repo, yet `git push` and the Git Data API
both return `403 Resource not accessible by integration` (`receive-pack` is
refused while `git fetch` works). That combination — API says `push: true`, push
says "not accessible by integration" — is the signature of a read-only token,
not a missing scope. Do not re-derive it: ask the user for a token with
`contents: write` and push with it once, inline in the URL
(`git push https://<token>@github.com/irhasar/rmc.git <branch>`), so it is not
written to `.git/config`. Never persist the token in a remote URL or a file.
