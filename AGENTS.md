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
./compiler/target/release/rmc bench/rust/fib.rs -o /tmp/fib
/tmp/fib 30

# inspect generated assembly
./compiler/target/release/rmc bench/rust/fib.rs --emit-asm

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
python3 tools/fuzz.py --mode shift --n 500 --seed 1
python3 tools/fuzz.py --mode cast --n 500 --seed 1
python3 tools/fuzz.py --mode assoc --n 500 --seed 1
python3 tools/fuzz.py --mode ref --n 500 --seed 1
python3 tools/fuzz.py --mode update --n 500 --seed 1
python3 tools/fuzz.py --mode letdestr --n 500 --seed 1
python3 tools/fuzz.py --mode fordestr --n 500 --seed 1
python3 tools/fuzz.py --mode nestpat --n 500 --seed 1
python3 tools/fuzz.py --mode arrpat --n 500 --seed 1
python3 tools/fuzz.py --mode parampat --n 500 --seed 1
python3 tools/fuzz.py --mode enumpat --n 500 --seed 1
python3 tools/fuzz.py --mode atpat --n 500 --seed 1
python3 tools/fuzz.py --mode tuple --n 500 --seed 1
python3 tools/fuzz.py --mode intmeth --n 500 --seed 1
python3 tools/fuzz.py --mode nestedfn --n 500 --seed 1
python3 tools/fuzz.py --mode typealias --n 500 --seed 1
python3 tools/fuzz.py --mode printfmt --n 500 --seed 1
python3 tools/fuzz.py --mode ptr --n 500 --seed 1
python3 tools/fuzz.py --mode char --n 500 --seed 1
python3 tools/fuzz.py --mode semi --n 500 --seed 1
python3 tools/fuzz.py --mode boolbit --n 500 --seed 1
python3 tools/fuzz.py --mode localconst --n 500 --seed 1
python3 tools/fuzz.py --mode stmt --n 500 --seed 1 --bounds --overflow

# robustness / soundness: rmc must error cleanly, never crash
python3 tools/fuzz.py --mode robust
python3 tools/fuzz.py --mode reject
python3 tools/fuzz.py --mode mutate --n 500 --seed 1
```

## Source map

- `compiler/src/lib.rs` — library entry point: `compile_to_asm(src, options)`.
  The compiler is a library plus a thin CLI, so it can be embedded.
- `compiler/src/lexer.rs` — byte scanner, tokens carry byte positions. A byte
  literal `b'A'` lexes to a `Tok::Int` (rmc has no `u8`); `b".."` is not a
  literal it recognises, so it is rejected downstream.
- `compiler/src/parser.rs` — recursive descent; `is_block_like` allows
  `if`/blocks as statements without a trailing `;`. `parse_cond` sets
  `no_struct_lit` so a `{` after a condition starts the body, not a struct.
  The flag is scoped to the head itself: `parse_bracketed_expr` clears it while
  parsing inside `(...)`/`[...]`, so `for p in [P { x: 1 }] { .. }` (and a struct
  literal in a call argument or index) still parses, matching rustc.
  Precedence, loosest to tightest: `||`, `&&`, comparison, `|`, `^`, `&`,
  `<< >>`, `+ -`, `* / %`. Note shifts bind *looser* than `+`/`-`, matching
  Rust: `parse_shift` consumes `parse_add`, and `parse_bitand` consumes
  `parse_shift`. Swapping these silently mis-parses `1 + 2 << 3`.
  Comparison is *non-associative*: `parse_cmp` parses one comparison and then
  rejects a second comparison operator at the same level, because rustc treats
  `a < b < c` as a hard error rather than `(a < b) < c`. A parenthesised inner
  comparison (`(a < b) == c`) still parses, since the `)` ends the first.
  `match` is parsed here and *desugared to a chain of `if`s* (`desugar_match`),
  so sema and codegen have no `match` case. A non-`Var`/`Int` scrutinee is bound
  to a hidden `let` first, so a call in the scrutinee runs once. An arm after
  `_` is unreachable and dropped (the `_` body becomes the `else`).
  An arm *body* is parsed by `parse_arm_body`, not `parse_expr`: assignment is
  only a statement here, so a bare `0 => acc = 1` would otherwise stop at the
  `=` and fail, and `return`/`break`/`continue` would be rejected outright.
  `parse_arm_body` builds an `Assign` for `expr = value` and `expr op= value`
  (via `desugar_compound`), or a `Return`/`Break`/`Continue` statement, and
  wraps it in a `Block`; the block is what `match_arm` places as the branch
  expression. A terminator's optional value stops at the arm's `,`/`}`
  (`arm_terminator_value`), so `0 => return 5,` keeps the comma for the arm
  list. Only `match` arms use it: `if let`/`while let` bodies are blocks.
  An arm pattern may be an integer, a *range* `lo..hi`/`lo..=hi`, a *string
  literal*, an `Enum::Variant`, an *or-pattern* `a | b | c`, or `_`. A range
  desugars to `Bin(And, Le, Lt/Le)`, an or-pattern to `Bin(Or, ..)` over the
  alternatives' tests with the *same* body (`or_cond` builds only the test, so
  the body is not duplicated), so neither needs a sema or codegen case. A
  *string* pattern desugars to `Bin(Eq, scrutinee, Str(s))`, which sema already
  lowers to the byte-wise `__rmc_str_lt` helper: the scrutinee is a pointer, so
  a machine compare would never match two distinct-but-equal literals. A
  *scalar* pattern (a literal, a range or a bool) desugars to a plain compare
  too, but match ergonomics must deref a `&T` scrutinee (`match &x { 5 => .. }`
  reads the `i64` under the `&i64`), and the parser does not know the
  scrutinee's type. So `scalar_scrutinee` wraps it in `Expr::PatScrutinee`, a
  marker sema rewrites: `check_expr` replaces it with a `Deref` for a `Ref`
  scrutinee, or with the bare expression otherwise (a scalar scrutinee stays
  untouched). Only a pattern auto-derefs; a bare `r < 6` still needs `*r`, as in
  rustc. The rewrite moves the inner expression out with `std::mem::replace`
  rather than re-checking it, so a side-effecting scrutinee is type-checked and
  its slots allocated exactly once. The marker never survives to codegen; the
  later passes (`inline`, `tco`, `licm`, `dce`, `codegen`) treat it as
  unreachable, like `ArrayList` and `MethodCall`. `TupleLit` is the same kind of
  marker: sema's `check_expr` rewrites it to a `StructLit` of the anonymous
  struct that lays the tuple out (so it is visible only to the sema pre-pass,
  `collect_mut_borrows`, which recurses into its elements).
  An or-pattern with a
  `_` alternative, and one whose alternatives cover every variant, are handled
  by `pat_irrefutable` and `or_leaves` (in sema) respectively. A *binding*
  alternative is rejected, as rustc's E0408 rejects it.
  A *struct pattern* `S { f, g: h, i: _, .. }` is irrefutable, so it also has no
  sema or codegen case: `struct_pat_binds` turns each bound field into a `let`
  whose initialiser is `Expr::Field { base: scrutinee, .. }`, which sema already
  types and locates (so a field may auto-deref a `&S` scrutinee). `mut` is a
  per-binding flag carried in `Pat::Struct` (`f: mut h`, `mut f`), so a bound
  field becomes a `let mut`. The prescan records
  each struct's field names before the real parse (`struct_fields`), so a field
  name is checked even for `i: _` (which emits no read) and a later-declared
  struct still matches; a field the struct lacks, or one bound twice, is a parse
  error. A field's value may be a *nested* struct sub-pattern (`a: I { x }`), to
  any depth: `parse_nested_struct_pat` recognises `Ident {` after the `:` and
  recurses into `parse_struct_pat_fields` (the field-list loop, factored out of
  `parse_pattern_primary`), which checks the inner struct against `struct_fields`
  too. `struct_pat_binds` recurses with the field read as the sub-scrutinee, so a
  nested name expands to an ordinary chain of field reads (`s.a.x`) and sema and
  codegen gain no case; `mut` is rejected on a sub-pattern, since it only applies
  to a binding. A field whose type is an array may also be destructured by an
  array sub-pattern, `S { a: [x, y], b }`: `struct_pat_binds` recurses into
  `array_pat_binds` with the field read as the sub-scrutinee, so the field read
  and the index read compose (`s.a[0]`) and the two nesting directions (a struct
  inside an array element, an array inside a struct field) both work.
  `first_bound_name` walks the pattern (through struct and or
  alternatives) to keep rejecting a binding in an or-pattern (rustc's E0408) one
  level down. A struct pattern is a catch-all arm, so it joins
  `pat_irrefutable`, and `or_cond` treats it as `true` (a *binding* struct
  pattern in an or-pattern is rejected like any other binding alternative).
  *`let` destructuring* `let S { f, g: h, .. } = e;` reuses the same parser
  (`parse_pattern_primary`) and the same `struct_pat_binds` helper. Because the
  bindings must outlive the statement, the expansion cannot be a nested block:
  `parse_stmt` was split into `parse_stmt_into(&mut Vec<Stmt>)`, and a
  destructuring `let` pushes the hidden scrutinee (a `\0lp<n>` temporary, unless
  the scrutinee is already a plain place, in which case its field reads read it
  directly, as in `desugar_match_arms`) followed by the per-field bindings into
  the enclosing block. `let mut S { .. }` is rejected (rustc's "`mut` must be
  attached to each individual binding"); a type annotation goes on the temporary.
  The temporary is forced when a binding shadows the name the initializer reads
  (`let (x, y) = x;`), so every element read still sees the original value; the
  prelude's `force` flag and the `pat_bound_names`/`init_shadowed_by` helpers
  decide that. No sema or codegen case.
  *Array patterns* `[a, b, .., c]` are the third irrefutable aggregate pattern,
  alongside struct patterns: `Pat::Array { elems, rest, front }` is parsed by
  `parse_array_pat` (`[` is routed from `parse_pattern_primary`) and desugared by
  `array_pat_binds` into one `let` per bound element whose initialiser is
  `Expr::Index { base: scrutinee, index: .. }`, which sema already types, bounds
  checks and locates. No sema or codegen case. `elems` stays in source order and
  is `Option<(bool, Pat)>` per element (`None` only for the `..` slot the builder
  skips): `front` counts the elements before `..`, and an element at pattern
  index `j` reads `subj[j]` when `j < front` (or with no rest) and
  `subj[subj.len() - (behind - (j - front))]` otherwise, so a trailing binding
  reads relative to the length (which folds to the constant, see `sema`'s
  `arr.len()`). An element may be `_` (no read), a `mut` binding (the per-element
  `mut` flag), or a nested array/struct sub-pattern (recursion, with the index
  read as the sub-scrutinee). `parse_array_pat` rejects a second `..`, `mut` on a
  non-binding, and any *refutable* element (`[1, y]`) — the last would need a
  per-element test rmc does not generate, so it is rejected rather than
  mis-bound. The pattern is irrefutable when every element is, so it joins
  `pat_irrefutable`, `first_bound_name` (recursing so a binding element in an
  or-pattern is still rejected) and the `let`/`for`/`match`/`if let` paths through
  `let_destructure_array_stmts` (the `let_destructure_prelude` helper shares the
  hidden-scrutinee logic with the struct path).   `or_cond` does not treat it as
  true: an array pattern is not a supported or-alternative.
  *Tuple patterns* `(a, b)` are the fourth irrefutable aggregate pattern:
  `Pat::Tuple(Vec<(bool, Pat)>)` is parsed by `parse_tuple_pat` (`(` is routed
  from `parse_pattern_primary`; `()` is the unit pattern and `(a)` is a
  parenthesised sub-pattern, so a one-tuple needs a trailing comma `(a,)`, as in
  Rust) and desugared by `tuple_pat_binds` into one `let` per bound element whose
  initialiser is a *field read* `Expr::Field { name: "k" }`. A tuple is an
  aggregate lowered to an anonymous struct (element `k` is field `"k"`), so a
  tuple pattern reuses the whole struct path: `tuple_pat_binds` mirrors
  `struct_pat_binds` (a nested struct/tuple/array element recurses with the field
  read as the sub-scrutinee). `tuple_pat_binds` emits an `Expr::PatArity`
  marker first (and `array_pat_binds` does too, unless a `..` rest makes the
  count flexible), which sema checks so a pattern naming the wrong number of
  elements is rejected as rustc does (E0308/E0527) rather than silently reading
  the first `k` and ignoring the rest; `()` (the unit type) counts as the empty
  tuple. A tuple pattern never matches an array even at equal length (E0308), so
  the array arm of the check is guarded by `!is_tuple`. No sema or codegen case
  beyond that marker. It is irrefutable when
  every element is, so it joins `pat_irrefutable`, `first_bound_name`, `or_cond`
  and the
  `let`/`for`/`match`/parameter paths through `let_destructure_tuple_stmts` (the
  `let_destructure_prelude` helper shares the hidden-scrutinee logic).
  In a *refutable* position (a `match` arm or an `if let` / `while let`
  scrutinee) a tuple element may be a scalar test. The parser tracks this with
  `pat_refutable_ok`, set by `parse_refutable_pattern` at the four refutable
  call sites; `parse_tuple_pat` rejects a non-irrefutable element when it is
  clear. `tuple_pat_test` builds the arm's test as the `&&` of each element's
  test (`pat_test`), each read through the element's field; `None` when every
  element is irrefutable, so the arm stays a catch-all. A refutable element may
  not bind or be a variant (`tuple_element_ok` rejects `E::V(x)`), since the
  test carries no payload. `pat_test` also handles an or-pattern element
  (`(0 | 1, x)`) by folding `or_cond` with `||`, and `or_cond` handles a tuple
  alternative the same way; `parse_tuple_pat` therefore parses each element with
  `parse_pattern` (not `parse_pattern_primary`) so an or-pattern is collected.
  A tuple *type* `(A, B)` is lowered by `lower_tuple_type` (in `parse_type`) to
  the same anonymous struct, named by the canonical element-type keys
  (`ast::tuple_struct_name`), and a tuple *literal* `(a, b)` stays `Expr::TupleLit`
  until sema resolves its element types and rewrites it to a `StructLit` of that
  struct (`tuple_struct_for` introduces the layout if the parser did not). An
  element is written `.0`, `.1`, ...: `parse_postfix` accepts a decimal literal
  after `.` as the field name.
  *At-bindings* `name @ subpat` bind the whole value to `name` and also match it
  against `subpat`. `Pat::At(String, bool, Box<Pat>)` is parsed by
  `parse_pattern_primary` (an identifier followed by `@`), so it is reachable in
  every pattern position; `mut` before the name (`mut n @ sub`) sets the flag.
  The name is an ordinary binding (the whole scrutinee) and `subpat`'s own names
  are bound from the same value, which is exactly what `pat_binds(pat, m, subj,
  out)` does: the uniform binding helper every position calls, which emits a
  `Stmt::Let` for the name then recurses on `subpat`. `pat_irrefutable` treats an
  at-binding as irrefutable only when its sub-pattern is, and `first_bound_name`
  returns the name. In a refutable position (`pat_test`, `arm_pattern_parts`,
  `or_cond`) the sub-pattern contributes its own test and the name binds when it
  passes. The irrefutable positions (`let`, `for`, parameters, a struct field's
  sub-pattern) each reject a refutable sub-pattern rather than mis-bind it, so
  `let n @ 1..=5 = e;` is a refutable local binding (E0005), as in rustc. A
  binding alternative inside an or-pattern stays rejected: `n @ 1 | 2` is E0408.
  The element-level `mut` of an array or tuple element (`[mut n @ _, m]`,
  `(mut n @ _, m)`) is OR-ed into the binding's flag, since `mut` before the
  element attaches to the element binding. An at-binding whose sub-pattern is a
  `..` rest (`first @ ..`) is rejected cleanly: rmc has no slice type.
  An arm may carry a *guard*, `PAT if COND => body`. `parse_match` reads the
  optional `if COND` after the pattern (`parse_cond`, so a struct literal needs
  parentheses) and threads it through `desugar_arm` as an `Option<Expr>`. The
  guard folds into the arm's condition rather than becoming a node, so sema and
  codegen gain no case; see the exhaustiveness notes below for the three rules a
  guard changes. When any arm has a guard, `desugar_match_arms` snapshots a
  variable scrutinee into a temporary before the arm chain, so a guard that
  assigns the matched value (`match x { n if { x = 2; true } => n }`) or one of
  its fields (`match p { P { x } if { p.x = 9; true } => x }`) cannot change what
  the arm body's binding reads: the guard and the body share the snapshot.
  Without a guard nothing runs between an arm's test and its body, so a plain
  variable stays inline and the lowering matches the equivalent field reads.
  rustc's borrow checker rejects both mutating programs (E0506), so no
  differential twin exists; the values are pinned by end-to-end tests.
  `while let PAT = EXPR { body }` is parsed here and desugared by
  `desugar_while_let` to `loop { if let PAT = EXPR { body } else { break; } }`,
  reusing the `if let` path, so sema and codegen gain no case. The scrutinee sits
  inside the loop, so it is re-evaluated each iteration and `continue` re-tests
  it. The `;` after `return`/`break`/`continue` may be omitted at the end of a
  block (`eat_semi_at_block_end`), as in Rust. An assignment and a compound
  assignment are `()`-valued expressions, so `parse_block_body` and the
  statement arm of `parse_stmt_into` end them with `eat_semi_at_block_end`
  too: `{ x = 1 }` and `{ x += 1 }` are accepted as a block's last statement,
  and a bare `;` (an empty statement) is skipped before every statement.
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
  `check_block` returns `Never` for a block that cannot complete, and every
  consumer of a block type accepts `Never` where a value is expected (`let`,
  `return`, the function-body check, and both arms of an `if`). That is what
  makes `fn f(e: E) -> i64 { match e { E::A => { return 1; } E::B => { return 2;
  } } }` and `let x: i64 = if c { return 1; } else { return 2; };` compile, as
  in rustc. The one case a block type cannot express is a `match` chain whose
  every arm diverges: its last step is an `if` with no `else`, which completes
  as `()`, so `Sema::match_chain_diverges` walks the chain and the no-`else`
  branch propagates `Never` for a `total` chain.
  An *unannotated* binding whose initializer diverges keeps `Never`, and rmc
  has no usage-based inference to refine it, so using it as a place is an error:
  `let x = { return 3; }; x + 1` and `let S { a } = { return 3; };` are rejected
  where rustc accepts them. This is a documented limitation, not a regression —
  an annotation (`let x: i64 = ..`) makes the type concrete and both compile.
  `let_destructuring_{accepts,rejects}_an_*_diverging_initialiser` pin it.
- Type aliases (`type NAME = T;`) are resolved entirely in the parser and never
  reach sema or codegen, so they cost nothing at run time and cannot affect
  layout. `prescan` records each alias name and the token index where its target
  begins (skipping to the `;`), then, after `resolve_consts`, parses each target
  (a second pass expands a target that named a not-yet-parsed alias, so
  `type A = B; type B = i64;` works in either order). `parse_type_inner`
  rewrites a `Tok::Ident` through `aliases` via `expand_alias_ty`, so every use
  site (a `let`, a parameter, a return, a struct field, a cast) is already
  concrete. A struct name is not an alias and stays `Type::Named`. A cycle is
  reported (`expand_alias_ty` bounds the walk), matching rustc's `E0391`.
  A *tuple* or array alias whose element names a later alias needs one more
  step: the first pass bakes the tuple struct's element as the still-unresolved
  `Named`, so after the map is complete every target is re-parsed with the
  tuple-struct table cleared (`type Pair = (Later, i64); type Later = i64;`
  otherwise keeps the placeholder's layout). `a_tuple_alias_may_name_an_alias_
  declared_later` and the `PairF` case in `tests/cases/typealias.*` pin it.
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
  slots for aggregate arguments and returns. Field init shorthand
  (`P { x, y }`) is resolved by the *parser*, which rewrites a bare field name
  to `x: x`; sema then checks the `Var` it built like any other, so an unbound
  name reports `unknown variable` rather than a missing-field error.
- `compiler/src/consts.rs` — shared constant folding: `const_i64`/`const_binop`
  (used by sema to fold a constant index or operand) and `resolve`/`eval_i64`
  (used by the parser to evaluate `const` items). A `const` item has no storage
  and no codegen; its value is substituted at every use. An immutable `static`
  is parsed by `parse_static` and lowered to the same `ConstDef`, so it shares
  this path; `static mut` is rejected, since rmc has no mutable global.
  A `const`/immutable-`static` declared inside a block is folded by
  `scan_block_consts` when the block begins (rustc hoists block items), so it is
  in scope for the whole block and shadows an outer one until the block ends.
  Because such a constant never reaches sema, the parser must enforce the checks
  sema applies to a top-level one: `fold_const_item` resolves the declared type
  and requires a foldable kind (a block-scoped `const S: str` is rejected like a
  top-level one), and `reject_const_shadow` rejects a `let`/`@`/struct-field
  binding that reuses the name (rustc's `E0530`).
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
  (the full account is in `docs/design.md`). A depth *tiebreak* (same count,
  prefer the deeper loop) fails the same way: mandel's five locals tie at four
  mentions, the tiebreak promotes the innermost `i` and demotes `x`, and mandel
  goes 31.7 -> 32.4 ms.
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
  `(**ppr).x` in `tests/cases/references.rs` guards against.
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
  whole corpus and cost 3-30% of compile time (`div.rs` 1.30x) for no runtime
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
  A trivial argument (a name or literal) is substituted in place only when no
  *later* argument can write memory (`has_effect`); otherwise it is copied to a
  fresh slot, so `sum(x, set(&mut x))` reads the `x` from before `set` ran.
  The same holds for a parameter whose *address* the body takes (`let r = &x;`,
  `&mut x`): it must be bound to a fresh slot too, or a substituted variable
  argument would let `*r = ..` write through to the caller (`f(y)` with
  `f(mut x) { let r = &mut x; *r = 5; }` left `y` at 5 instead of 3), and a
  substituted non-lvalue argument (a literal, a call) would produce `&3`, an
  address with no place to name, which `gen_addr` rejects. `escaped_slots`
  collects both kinds — a slot assigned directly and a slot whose address is
  taken — and its `Expr::AddrOf` arm records the operand slot when it is a bare
  `Var`, so the copy is forced.
  A candidate body must be call-free with no early exit; because it is call-free,
  inlining it cannot expose new calls, but the pass still runs to a *fixpoint*
  because a function only becomes call-free after its own callees are inlined
  (the `refs` wrapper `sum_via_ref` needs round two). A loop is allowed — a small
  helper like `gcd` has one and is still worth inlining, since that removes the
  call from the caller's hot loop (measured 1.12x on a `tri(i % 50)` shape). A
  `break`/`continue` is *not* allowed: once spliced into the caller it would bind
  to the caller's enclosing loop. A non-final `return` is not allowed either, for
  the same reason. `Sub` already rewrites `while`/`loop`/`for` bodies and their
  counter slots, and `escaped_slots` already descends into them, so a
  loop-bearing body needs no extra splice work. A statement-free block is unwrapped to
  its tail after expansion: the spliced body of an accessor would otherwise stay
  wrapped in an `Expr::Block`, and later passes (`simple_operand`, the
  accumulator fold) cannot see through a block, so the hot loop would bounce its
  sum through `%rax` and the spill slots. `--no-inline` disables it; the tests
  that assert on the calling convention (`call f`, argument setup) use it.
  `MAX_NODES` (48) must stay above a two-level helper — `sum_via_ref` is 35
  nodes once its accessors land — and `count_expr` must count a block's tail,
  or a candidate's size is understated and it is wrongly admitted.
- **LICM never hoists a read through a reference.** A scalar field read is
  hoisted only when its base is a local aggregate (`frame_field_base`); a field
  read through `&T` dereferences a pointer that may be null (`0 as &S`) or
  dangling, so hoisting it would fault on a loop whose body never runs. Any
  field in the chain whose own type is a reference holds such a pointer, so
  `h.p.n` (where `p: &S`) is not a frame access either. Local aggregate fields
  are frame accesses and stay hoistable.
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
- **A function that addresses no frame slot also omits `%rbp`.** When
  `frame_slots == 0 && spill_slots == 0` nothing is ever addressed through the
  frame pointer, so `push %rbp`/`mov %rsp, %rbp`/`leave` are dropped and the
  epilogue `add`s the frame back instead of pointing `%rsp` at the saved
  registers. This is the same reasoning the hand-written `alloc` already uses
  (see below), generalised to compiled functions. On the current benchmark set
  it is a wash at run time — the out-of-order core hides a three-instruction
  prologue — but it is strictly fewer instructions and a couple of bytes smaller
  on a register-only leaf like `fib`. `uses_frame` is read *before* the body is
  emitted, because emitting re-measures `frame_slots` and `off` is `&self`.
- **The no-frame-pointer path needs its own alignment pad.** The call ABI wants
  `%rsp ≡ 8 (mod 16)` at a call. With a frame pointer `push %rbp` supplies the
  8; without it, the saved registers plus the frame must total 8 mod 16, so an
  even number of saved registers (16 bytes) is followed by an 8-byte `sub`.
  A misalignment is silent — rmc's runtime makes no aligned SSE access, so it
  never faults — which is why it is checked statically over every program in
  `bench/` and `tests/cases/` (every call-site function must show
  `%rsp ≡ 8 (mod 16)`) rather than trusted to the behavioural suite.
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
  straight line: `div.rs` (a 531-line `main`) drops 6% of its compile time, and
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
  `regs.rs` and `calls.rs` fail if this regresses.
- **Only whole-value structs keep all their fields in memory.** A scalar field
  of a struct is promoted like any other local, *unless* the struct is ever
  copied, returned or passed by value, in which case the whole-value copy reads
  and writes memory and every field must stay there. `promote` tracks that
  during its walk (`copied` and the `let` targets) and zeroes those slots. A
  promoted field that stays in memory on the copy path but is read from its
  register on the field path is a silent wrong-answer bug, which is what
  `fieldpromo.rs` guards.
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
- **A wide element has stride `size()/8`, not 1.** An array of nested arrays or
  of structs addresses element `i` at `base + i * stride`, so `array_off` must be
  called with the *slot* count `n * stride`, never the element count `n`. Every
  load, store, copy and splat scales by that stride. The stride comes from
  `elem_stride(base)`, which reads `array_elem_ty(base)`; that must look through
  an `Index` base (`m[i][j]` uses the inner array's element type), or the inner
  index silently uses stride 1 and reads a neighbouring element. `sema`'s
  list-literal and repeat-literal desugars must also allocate `n * stride` slots:
  allocating `n` makes wide elements overlap.
- **A splat copies one value into every element.** In `splat_aggregate`'s
  counting loop only the destination pointer advances; the source stays put. An
  earlier version stepped both, so element `i` was read from `v + i*elem_size`
  and only element 0 held `v` — invisible for a scalar splat, wrong for a wide
  element over the 8-word small-path threshold.
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
- **A loop label selects the target loop, not the innermost one.** `sema` and
  `codegen` each keep a stack of the enclosing loops (`loop_stack` / `loops`)
  with the loop's label, and resolve a `break 'a` / `continue 'a` by searching
  that stack from the top for the label (an unlabelled `break`/`continue` takes
  the innermost entry). The label is carried through the desugarings that build
  a loop: `'a: for x in arr` copies the label onto the index `for` it expands
  to, and `'a: while let PAT = EXPR { .. }` passes the label to the `loop` it
  desugars to, so `break 'a` / `continue 'a` in the body target it. The
  `if`-with-a-lone-`break`/`continue`
  fast path in `bare_exit` only fires for an *unlabelled* `break`/`continue`,
  since a labelled one may leave an outer loop.
- **`'` starts a `char` literal or a loop label, and the lexer decides by lookahead.**
  It is a `char` literal when the byte after `'` is a backslash escape, an
  immediate `'` (`''`), or a full UTF-8 sequence followed by `'`; otherwise it
  is a label. This mirrors rustc, where a label is never followed by a lone
  quote. A `char` *range* pattern (`'a'..='z'`) is built in `maybe_char_range`:
  after a `char` literal, a following `..`/`..=` consumes a second `char`
  literal as the upper bound. The upper bound must be a `char` literal (a bare
  `5` is rejected) and an empty range (`'a'..'a'`, `'z'..'a'`) is rejected, as
  rustc rejects them (E0579/E0308). `Pat::CharRange` lowers to the same
  `lo <= s && s <= hi` comparisons as `Pat::Range`, so sema and codegen need no
  new case.
- **A `for` bound is compared directly when it is a constant, and through a
  hidden slot otherwise.** `0..=1000` needs no `end_slot`: the guard is
  `cmp $1000, %rax`, so no frame word is written and no reload happens each
  iteration. A non-constant bound (e.g. `0..n`) is evaluated once into `end_slot`
  and compared register-to-register. `count_expr` must count the hidden counter
  and bound slots so they compete for a callee-saved register like any other
  loop-carried local; otherwise the bound is reloaded from the frame every pass.
- **`for x in ARRAY` desugars to an index loop**, not a new node:
  `{ let a = EXPR; for i in 0..a.len() { let x = a[i]; body } }`. `parse_for`
  binds `EXPR` to a hidden `\0fa<n>` temporary so an array expression with a
  side effect (a call, a literal) runs once, and `a.len()` folds to the array's
  compile-time length, so the loop bound is a constant and `end_slot` is unused
  (see the note above). `x` is a fresh `let` per iteration, so it is immutable
  and a stale read across iterations is impossible. `continue`/`break` work
  because the desugared loop is the ordinary `For` node. A **struct pattern**
  `for S { f, .. } in arr` reuses `let_destructure_stmts` (the `let`
  destructuring expansion) in place of the single `let`, so the element is read
  into a hidden temporary and the fields are bound from it; the element type is
  `S`, so this is the same read-through as `let`. The pattern must be
  irrefutable: a binding, `_`, or a struct pattern — including a nested
  sub-pattern, which `struct_pat_binds` recurses into like the `let`/`match`
  paths (a refutable pattern like an enum variant is rejected rather than
  mis-bound). `for x in
  &arr` is rejected — Rust iterates a reference by reference (`x: &i64`), so
  accepting it would silently mistype `x`. `parse_for` also checks the pattern
  name against the constant table *before* `parse_pattern`, because a bare
  constant name there resolves to its value (not a binding), which would turn
  `for N in 0..3` into an iteration over a literal and lose the E0530 error.
  `tests/cases/forarr.rs` and the `stmt` fuzzer cover it.
- **`..=` exits on `jle` back-edge, not `jl`.** The guard is the loop-back
  condition, so the inclusive form loops while `var <= end` (`jle`) and the
  exclusive one while `var < end` (`jl`); a `jg`/`jge` *exit* test is the
  un-rotated mirror of the same thing. `forloop.rs` and
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
  not the reverse. `aggfn.rs`/`aggbig.rs` and `argorder.rs` cover this.
- **`%r10` carries the return buffer.** It is caller-saved and not an argument
  register, so it survives argument setup. Do not use `%r10` as scratch anywhere
  argument setup can run after the return pointer is set.
- **The direct-argument shortcut must decline every clobbering argument.**
  `gen_call` writes scalar arguments straight into the ABI registers when each
  argument is a simple operand, or when there are at most two and none calls or
  copies an aggregate. `contains_call` must therefore recurse into *every*
  expression that can evaluate a child — including `EnumCtor`, `VariantTest`
  and `EnumPayload` — and `clobbers_arg_regs`/`stmt_clobbers` must flag every
  statement path that runs `copy_words_rdi_rsi` (whole-aggregate `let`, and
  aggregate `Var`/`Field`/`Index` stores). A missed case leaves an earlier
  argument in `%rdi`/`%rsi` that a later argument overwrites.
- **At most six parameters.** The System V integer argument registers are the
  only ones this compiler uses, so `sema` rejects a function (or a method after
  its receiver is inserted) with more than six parameters rather than letting
  `zip(ARG_REGS)` silently drop the seventh.
- **`mut` is enforced.** `Stmt::Let` carries a `mutable` flag and `FnDef` a
  `param_mut` vector; `sema` records both in its scope and rejects an assignment
  to a non-`mut` binding. A plain `let` is written exactly once — its initialiser
  or its first assignment — so a later write is `cannot assign twice to
  immutable variable`, and an immutable parameter reports `cannot assign to
  immutable argument`, both matching rustc. The check is a may-analysis: an
  `if`/`else` merges a flag with OR. Whether a write repeats is decided per
  path: `Sema` threads the enclosing statement's continuation (`loop_cont`)
  into every nested block, so a write is a second write only when *some* path
  from it reaches the loop's top again — a `continue`, a loop body's
  fall-through end, or the code after its statement. `x = 1; break;` is a
  single write even when a sibling path `continue`s, and a `loop` whose every
  path `break`s/`return`s runs at most once. A `break` merges the writes on its
  path into the post-loop state. Keep generated `let`s `mutable: true` so they
  never trip this. Three edges the analysis must get right: every loop entry
  pushes a `loop_cont` entry (not just `loop_reach`), or a nested block's
  `saved_cont` defaults to `false` and a write in the nested block of a
  repeating loop is missed; a `break` merges only the *prefix* of its snapshot
  that lines up with the bindings present at loop entry, so a break path that
  declared extra locals still contributes its outer-binding writes; and a
  `loop` with no reachable `break` does not fall through, so an infinite inner
  loop must not make the outer loop look repeating. Two more: an `if`/`while`
  condition is evaluated like any other operand, so the break/reach walks
  descend into condition expressions and every other evaluated child
  (`expr_has_break`/`expr_contains_continue`), or `loop { loop { if { break;
  true } {} } x = 1; }` wrongly accepts the second write; and a labelled
  `continue` inside a nested loop still targets the outer loop, so the reach
  walk descends into a nested loop body, threading a `nested` flag so an
  unlabelled `continue` there no longer counts. A diverging condition
  (`{ break; }`, `{ return; }`, `{ continue; }`) has type `!`, which rustc
  accepts where `bool` is expected; the `if`/`while` condition type check must
  not reject it. An *unlabelled* `break`/`continue` in a `while` condition has
  no valid target (E0590), so the condition check rejects it via
  `cond_has_unlabelled_exit`; a labelled one is fine.
- **A compound assignment evaluates its place once.** `desugar_compound` binds
  any component of the target that contains a call — the base or the subscript —
  to a temporary and reuses it for both the read and the write, so `make().x += 1`
  calls `make` once. A call-free `Var`/`Field`/`Index` is cloned directly.
- **`sema` persists the inferred type on every `let`.** Codegen selects the
  aggregate path from `Stmt::Let`'s `ty`, so leaving it `None` for `let a = [7; 2]`
  silently initialises only one word.
- **Struct-literal fields stay in source order.** Their layout offsets are
  attached per field, so reordering them into declaration order would change the
  order initializer side effects run in.
- **A bounds check sees through a reference to an array.** `array_len_of` /
  `array_len_ty` unwrap an `Expr::Ref` base, so `let r: &[i64; 3] = &a; r[i]`
  traps on an out-of-range `i` under `--bounds-checks` instead of reading
  adjacent memory. A field store through a shared `&T` is rejected by
  `place_writable` ("cannot assign through a shared reference; use `&mut`"), and
  a `&mut` binding itself is not reassignable unless declared `mut`.
- **Assignment evaluates its value before its place.** rustc orders assignment
  operands right-to-left (the one expression that does), so `a[li(1)] = li(5)`
  must run `li(5)` before `li(1)`. Codegen's general index/deref store path and
  `store_through_addr` evaluate the value first and stash it across the address
  computation whenever that computation can have an effect (`place_has_effect`,
  which is `contains_call` plus a block/if that assigns a local), and the
  aggregate copy is factored into `assign_aggregate` for the same reason. The
  walk must reach every place form (`Deref`, `AddrOf`, `EnumPayload`, ...) and
  must descend through `Bin`/`Un`/`Cast` and an `if` condition, or a target like
  `r[{ y = 2; 0 } + 0]` is mis-ordered: `contains_call` alone misses a block
  that assigns without calling. `assignorder.rs`, `refidxorder.rs` and
  `tools/fuzz.py --mode order` pin this.
- **A compound assignment binds its modifying operand first.** `desugar_compound`
  evaluates the right-hand operand into a temporary ahead of the subscript
  temporary, so `a[next()] += val()` runs `val()` before `next()`, matching the
  right-to-left rule above. The subscript is still bound once, so `a[next()] += 1`
  advances the counter once, not twice, and the cheap two-instruction form is
  kept when the subscript has no call.
- **A struct update evaluates its fields before its base.** `S { f: v, ..base }`
  is desugared by `desugar_struct_update` into
  `{ let v = <field value>; ...; let mut t = base; t.f = v; ...; t }`: every
  written field value is bound to a temporary *before* the base runs, which is
  rustc's order (verified against `rustc -O`). Evaluating the base first is a
  silent wrong answer when a field value mutates the base, so
  `tools/fuzz.py --mode update` emits `S { x: mutate(&mut base, ..), ..base }`
  and pins it. The result is a fresh local, so it never aliases the base, and a
  field written twice is rejected at parse time (the plain-literal path catches
  that in sema). A comma may not follow the base (rustc rejects it), and a
  struct literal *is* allowed as the base (`..P { ..a }`), unlike the
  struct-literal-in-condition restriction.
- **Statements vs. tail expressions.** Assignments inside a block must be
  detected before the trailing-`;` check in `parse_block`, otherwise `x = ...;`
  is parsed as an expression and rejected. An assignment's own `;` may be
  omitted when it ends a block (`eat_semi_at_block_end`), so the check is not a
  bare `expect`; a leading `;` is skipped first as an empty statement.
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
  cost a page of padding and about 4 KB on every artifact. Only a *writable*
  section may be demoted to `.bss`. A read-only `.rodata` that happens to be all
  zeros — a program whose only string literal is `""` emits just `.quad 0` — is
  still file-backed and part of the RX segment; demoting it left `rx_end`
  reading its unassigned `file_off` of 0, so the RX segment covered 8 bytes and
  the binary segfaulted on entry while the `--binutils` path worked. When adding an
  instruction or a codegen form, remember the encoder only implements the subset
  the code generator emits and errors on anything else — run `tests/run.py`,
  which compiles every case through both paths and requires identical output.
  The `--binutils` path still uses `ld --build-id=none -z noseparate-code
  --strip-all`; `--no-strip` keeps the symbol table, `--min-size` also runs
  `strip --strip-section-headers`. `--no-strip` and `--min-size` request
  opposite artifacts, so `parse_args` rejects the pair instead of letting
  `--min-size` strip the symbols `--no-strip` asked to keep. Do not change the
  `--binutils` link back to `-n` (nmagic): it packs every section into one
  read+write+execute `LOAD` segment, a W^X violation, and is *larger*.
  `tests/run.py`'s `check_stripped` pins the size ordering of all four variants,
  `check_no_rwx` fails on any writable+executable `LOAD` segment, and
  `check_link_paths_agree` requires the two link paths to behave identically.
- **In-process assembler gotchas.** The encoder in `src/elf.rs` is deliberately
  narrow; these are the encodings that are easy to get wrong and were each a
  real bug:
  - Group opcodes: `inc`/`dec` are group-5 (`FF /0,/1`) but `not`/`neg` are
    group-3 (`F7 /2,/3`). `FF /3` is `call far`, which faults at run time.
  - `bswap %r64` (`0F C8+rd`) names its operand in the low three opcode bits,
    *not* in ModRM: there is no ModRM byte, so the register index is REX.B
    (`rex(true, 0, reg)`), and a `%r8`-`%r15` operand needs `0x49`, not `0x44`.
    Using the REX.R helper here would silently swap the wrong register.
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
- **Every runtime piece after the first must re-open `.text`.** The bit-method
  helpers (`__rmc_count_ones`, `__rmc_leading_zeros`, `__rmc_trailing_zeros`,
  `__rmc_swap_bytes`, `__rmc_reverse_bits`, `__rmc_rotate_left`,
  `__rmc_rotate_right`) follow `__rmc_clamp`, whose last message ends in
  `.section .rodata`. Without a `.text` line they were emitted into `.rodata`
  and faulted under a link that honours section permissions (`--binutils`, or
  the library path). The fuzzers did not see it because a bit method that traps
  was never combined with the CLI's `-n` merge. When adding a piece, copy the
  leading `.text` from a neighbour; `tests/run.py` links bit-method programs with
  a plain `ld` to catch a regression.
- **A `.set` resolves against the section current when it appeared.** The
  in-process encoder recorded `.` for a `.set` against the *last* section the
  file switched to, so a `.set` after a `.section .rodata` block (the trap
  message lengths) resolved against the wrong section and mis-sized the
  immediate. Each `.set` now carries the section it was seen in.
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
- **`refs` is the reference-read loss, and it is deliberate.** The loop reads a
  fixed struct through `&self` accessors. `licm` will not hoist a read through a
  reference: rmc lets a reference outlive its referent (`0 as &i64` is accepted),
  so a speculative deref can fault on a loop that never runs, where the in-body
  read would not. The accessor sum is therefore recomputed every iteration and
  rmc loses to rustc by roughly 8x. A frame-based loop of the same shape is
  hoisted and wins. Keeping the Rust loop real needs an in-loop `black_box`
  (a store/reload per iteration), so read the ratio as "two real loops doing the
  same arithmetic", not an instruction-for-instruction tie - but do not claim a
  win here; there is not one. This was the state before the faulting-hoist fix
  (the LICM reference-read restriction); the docstrings and this note were not
  updated when the fix landed, and a stale 2.23x win sat in `RESULTS.md`.
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
compilers accept). `tools/fuzz.py --mode mutate` is the complement: it applies
random splices, deletions and duplications to the generators' programs and the
`robust` corpus and requires every mutant to fail cleanly (exit 0 or 1, no
signal, no panic). It needs no rustc, so it runs anywhere, and it reaches error
paths the hand-written corpora miss.

`tests/run.py` compiles each `tests/cases/NAME.rs` with both compilers
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
one defined in terms of another, plus a `const`/`static` declared inside a block,
usable before its declaration and shadowing an outer one) guard the
front-end/codegen fixes below. There is no separate rmc dialect: each case is a
single `.rs` file that rmc compiles directly, and rustc compiles the same file
as the reference, so rmc can stand in for rustc on the supported subset. When
adding a case, remember that `rustc` infers `i32` for unsuffixed
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

A variant may write its tag explicitly (`enum E { A = 5, B, C }`). The tag is
then the written value, not the index, so it is computed once in `layout_enum`
(a variant with no `= N` is the previous tag plus one) and carried in the fourth
field of `EnumInfo::variants`. Every tag use goes through `tag_of`/the tuple, so
a `match` arm and an `as i64` both read the written value. `layout_enum` also
rejects a duplicate tag, and a variant with no `= N` after `i64::MAX` (rustc
E0370) — the check is lazy, so a final `= i64::MAX` is fine. The parser rejects
`= N` on a payload variant and folds `-9223372036854775808` to `i64::MIN`.

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

A guard (`PAT if COND => body`) is desugared, not given a node: `desugar_arm`
folds it into the pattern test as `Bin(And, test, cond)`, and an *irrefutable*
arm's guard (no test of its own) becomes the whole `if` condition, paired with a
literal `true` test so the exhaustiveness walk can still recognise the arm. A
failed guard falls through to the next arm, so the guard is evaluated at most
once and only after its pattern matched. Three rules matter for exhaustiveness,
all matching rustc (RFC 2005):
  - a *guarded* arm covers **nothing** — the guard may be false — so
    `if_chain_is_exhaustive` skips it entirely (`arm_guard`). Counting it would
    wrongly accept `match e { E::A if c => 1, E::B => 2 }`, which rustc rejects
    (E0004).
  - a guarded *binding* arm (`n if c`) is not a catch-all, so `has_catch_all`
    is set only for an *unguarded* irrefutable arm. The parser's `true`-test
    marker is what distinguishes the two: without it, `n if c` would either be
    miscounted as a catch-all or (with no pattern test) abort the walk.
  - a repeated variant (only reachable when one of the duplicates is guarded and
    therefore skipped) is redundant, not a gap: `seen_variants` is a set, and a
    duplicate is ignored rather than treated as non-exhaustive.
`arm_pattern_test`/`arm_guard`/`is_pattern_test` peel the guard off a condition.
They must not mistake a *range* pattern (which also folds to `Bin(And, ..)`) for
a guarded one, so they strip only when the left operand is itself a pattern test.

Two exhaustiveness-walk bugs this exposed are fixed. A payload-binding arm
(`O::S(v) => ..`) wraps its `if` in a block holding the binding `let`s, and that
block used to hide the chain: `match o { O::Some(v) => v, O::None => 0 }` (no
`_`) was wrongly rejected. `chain_tail` now unwraps block wrappers while
descending the chain, and the walk no longer requires an arm's `else` block to
have empty `stmts`. `tests/cases/guard.rs` and the `pat` fuzzer cover
guards; the `reject` fuzzer's `guard_*` cases pin the exhaustiveness rules.

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
`tests/cases/exhmat.rs` covers value position, reordering, nesting,
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

A function in an `impl` block with no `self` is an **associated function**. The
parser sets `takes_self = false` and emits the same `Type$name` mangling with no
receiver parameter; sema inserts the `self` parameter only when `takes_self` is
true, so an associated function registers like a free function. A call
`Type::name(..)` arrives as `Expr::EnumCtor` (the parser cannot tell a
payload-variant constructor from an associated function), so sema checks whether
the enum has that variant; if not and `Type$name` is in `sigs`, it rewrites the
node to a `Call`. A struct qualifier always takes this path. `Qual::name` with
no call and no matching variant is reported as an associated function that must
be called (rmc has no function values), not as an unknown enum. Calling an
associated function with `.` (`p.new(1)`) passes the receiver as an extra
argument, so it fails on arity — which matches rustc's E0599 closely enough.

A **nested `fn`** (`fn f() { .. }` as a statement) is an item, not a closure:
Rust forbids it from capturing an enclosing local (`E0434`), so it sees only the
program's items. That is what makes it cheap to support: the parser hoists it to
the top level and appends it to `Program::fns`, so sema, codegen and every later
pass treat it as an ordinary free function with no new case. Two things make it
behave like rustc's block scoping, both in the parser:

- *Name resolution and shadowing.* `parse_block_inner` pushes a scope frame
  filled by `scan_block_fns`, a token-index pre-scan of the block's direct
  statements (it skips any `{ .. }`/`( .. )`/`[ .. ]` region, so a `fn` inside an
  inner block or a body is not seen here). The pre-scan gives each nested `fn` a
  unique symbol (`rmc$nf<n>$<name>`, keyed in `fn_at` by the `fn` keyword's token
  index) and records `name -> unique` in the frame. Because the frame is filled
  before the statements are parsed, a nested `fn` is in scope for its whole block
  (a call before its declaration, or a sibling declared later, both resolve), and
  a nested `fn` shadows a top-level one of the same name inside the block.
  `resolve_fn_name` rewrites a call through the innermost frame that names it.
  Two `fn`s of one name in a block is rustc's `E0428`, reported by the pre-scan.
- *The symbol.* `$` is used, not a NUL: the in-process encoder takes any bytes,
  but the `--binutils` path writes the label into GNU `as`, which rejects a NUL.
  A user identifier is `[A-Za-z0-9_]`, and a method mangles to `Type$method`
  (one `$`), so `rmc$nf<n>$name` (two `$`) cannot collide with either.

The definition is parsed under its pre-scanned name (`fn_at`), so the hoisted
function and its call sites agree. A nested `fn` captures nothing, so this is
exactly rustc's semantics for the subset — including that a nested `fn` in a
`loop` body is the same function every iteration (it is not re-created).

An array's element type is a scalar: `i64`, `bool` or a unit enum. Nested
arrays, arrays of structs and arrays of payload enums are rejected. A unit enum
is one 8-byte slot emitted like `i64`, so `[E; N]` lays out and indexes
correctly; rustc likewise accepts a unit enum and rejects a payload enum and a
reference as array elements. A list literal `[a, b, c]` is parsed as
`Expr::ArrayList` and rewritten by sema, which infers the element type from the
values — the parser must not guess it, because it does not type-check, and
guessing `i64` is what made `[true, false]` fail.

`let _ = e;` (and `let _: T = e;`, and the no-op `let _: T;`) is a **discard**.
The parser lowers it to a block holding a hidden `let _ = e;` binding, so the
initializer is still evaluated once and an annotation is still checked, then the
binding is dropped. The hidden binding is literally named `_` so a type-mismatch
diagnostic reads `variable `_``; `_` cannot be written as an ordinary binding
(`expect_ident` rejects it), so the name never clashes with user code. Bare
`let _;` with no annotation is rejected at parse time (rustc's E0282).

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
  entry state) initialized it; a branch with no `else` does not count, because it
  may not run, and a `while`/`for` body does not because the loop may run zero
  times. A `loop` body counts only when every `break` that leaves the loop
  initializes the binding: the loop runs at least once, but an exit taken before
  the assignment leaves the binding uninitialized (rustc E0381), so the state at
  each `break` is captured and merged. This is deliberately conservative — it rejects
  the uninitialized-array-filled-in-a-loop pattern (`let mut a: [i64; N]; while
  ... { a[i] = ..; } a[0]`), which rustc also rejects. Write the array with `= [0;
  N]` instead; the corpus cases do.
- **A short circuit has two outcome states, and a chained guard needs both.**
  `&&`/`||` run their right operand on only one path, so `check_expr` records
  the true and false entry states in `cond_then_init`/`cond_else_init` for the
  `if`/`while` that branches on the condition. Checking a right operand against
  the scope's merged "either path" state loses a write the guard guarantees:
  `(a && { x = 7; true }) && x == 7` reaches the comparison only where the inner
  `&&` was true, yet the merged state says `x` is uninitialized (E0381). Compose
  the outcomes per operand instead — check the right operand under the left-true
  state for `&&` (left-false for `||`), then merge for the true, false and
  post-expression states. The post-expression state stays conservative, so a
  value use (`let b = a && { x = 7; true }`) still cannot assume `x` is set.
- **Constant array bounds.** A constant index outside the array (`a[5]` on a
  `[i64; 2]`, the negated `a[-1]`, or an expression like `a[1 + 4]`) is a compile
  error, with or without `--bounds-checks`, matching rustc (`E0080`/`E0608`).
  The index is folded with `const_i64`, which evaluates the integer operators
  (`+ - * / % & | ^ << >>`) as well as unary `-`, so an index written as an
  expression is caught too — an earlier version folded only literals and let
  `a[1 + 4]` read adjacent stack memory. `Sema::const_of` extends this to a
  constant *local*: `let i = -1; a[i]` is folded the way rustc's constant
  propagation folds it, which an earlier version did not, letting `a[i]` read
  past the array. Both folders see through the identity cast `i as usize` (the
  usual spelling of an index in real Rust), so `a[i as usize]` is caught
  exactly like `a[i]`. A *variable* index is still unchecked by default (see
  Scope).
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
  which walks the bytes, so ordering matches Rust's. Both operands are pure
  literals or variables, so the helper is never double-evaluated. Comparing a
  `str` with an `i64` is rejected (`E0308`), as rustc does. `str` is a builtin
  type nameable in a signature, field or `let` (`fn f(s: str) -> str`,
  `struct M { s: str, n: i64 }`): `resolve_ty_rec`/`resolve_in` map `Named("str")`
  to `Type::Str`, one word passed by value, rather than reporting "unknown type".
- **`str::len` and array `.len()`.** `len()` is special-cased in the `MethodCall`
  arm of sema *before* the struct/enum method lookup, because `str` is a pointer:
  a `str` receiver lowers to `Call __rmc_str_len` (the record's first word), and
  an array receiver folds to the constant `N`, keeping the receiver only for side
  effects (dropped entirely when it is a named local). A user `len` on a struct or
  enum still falls through to the mangled-method path. rmc's `.len()` is `i64`
  where rustc's is `usize`, so a differential twin casts with `as i64`.
- **Built-in `i64` methods
  (`abs`/`min`/`max`/`clamp`/`signum`/`is_positive`/`is_negative`/
  `div_euclid`/`rem_euclid`/`midpoint`/`count_ones`/`count_zeros`/`leading_zeros`/
  `trailing_zeros`/`leading_ones`/`trailing_ones`/`swap_bytes`/`reverse_bits`/
  `rotate_left`/`rotate_right`).**
  Like `len()`, they
  are special-cased in the `MethodCall` arm of sema before the struct/enum lookup,
  and only when the receiver's type is `i64`, so a user `min` on a struct is
  untouched. `desugar_i64_method` rewrites the call into an expression over the
  operators rmc already lowers, so codegen gains no case
  (`clamp`'s `min > max` panic and the ten bit methods are the exceptions — a
  call to the `__rmc_clamp` / `__rmc_count_ones` / `__rmc_leading_zeros` /
  `__rmc_trailing_zeros` / `__rmc_swap_bytes` / `__rmc_reverse_bits` /
  `__rmc_rotate_left` / `__rmc_rotate_right` runtime
  pieces, each linked only when its method appears):
  `abs` is `if x < 0 { -x } else { x }` (so `i64::MIN` maps to itself, matching
  `rustc -O`; under `--overflow-checks` the `neg` traps, matching a debug build),
  `signum` is `(x > 0) as i64 - (x < 0) as i64` (comparisons only, so `i64::MIN`
  never overflows), `min`/`max` compare `y` against `x`
  (`if y < x { y } else { x }` for `min`), returning `x` on a tie,
  `clamp(lo, hi)` is `if lo > hi { __rmc_clamp() } else if x < lo { lo } else if
  x > hi { hi } else { x }` (the `lo`/`hi` binds run in argument order, then the
  range check, so `g().clamp(g(), g())` calls `g` three times as rustc does and
  the inverted-range panic fires before either bound is used), and
  `is_positive`/`is_negative` are the bare comparisons `x > 0` / `x < 0` (so
  `0` is neither, and both return `bool`, unlike the `i64`-valued seven).
  `div_euclid`/`rem_euclid` bind `q = x / y` and `r = x % y`, then round: when
  `r < 0`, `div_euclid` is `q - 1` (positive `y`) or `q + 1` (negative `y`), and
  `rem_euclid` is `r + y` (positive `y`) or `r - y` (negative `y`), the latter
  arm written as a subtraction so no synthesized `-y` needs the neg lint exempted.
  `rem_euclid` is phrased `r ± y` (not `r + abs(y)`) precisely so a `MIN` divisor
  does not overflow a dead arm: with `y == MIN` the remainder is never negative,
  so `r - y` never runs. They reuse `/` and `%`, so a zero divisor and
  `MIN.div_euclid(-1)` panic at run time as the operators do.
  `midpoint(y)` is the mean rounded toward zero, which is *not* the floor: it is
  `(x & y) + ((x ^ y) >> 1)` (the floor, since `x + y = (x & y) + (x | y)` and
  `(x | y) = (x & y) + (x ^ y)`) with a one-step correction for an odd negative
  sum — `(x ^ y) & 1` is 1 exactly when the sum is odd, and the half-sum is
  negative exactly when the sum is, so the correction is
  `((x ^ y) & 1) & ((half < 0) as i64)`, added once. Nothing is negated and no
  arm can overflow (the `&`, `^`, `>>` and `+` of the floor stay in range, and
  the correction is 0 or 1), so `MAX.midpoint(MAX)` is `MAX` and
  `MIN.midpoint(MAX)` is `0` with or without `--overflow-checks`. The half-sum is
  bound to a temp so the sign test reads the same value the `+` does. This is
  exactly `rustc`'s definition, so no helper is added.
  `count_ones`/`leading_zeros`/`trailing_zeros` take no argument and lower to a
  `Call __rmc_<name>` on the receiver (no operators): the usual `x & (x - 1)`
  bit trick overflows for `i64::MIN`, and rustc's answer for `MIN` is exact
  (`count_ones` 1, `leading_zeros` 0, `trailing_zeros` 63), so each helper is a
  plain loop over the 64 bits that neither negates nor decrements — the result
  is the same with and without `--overflow-checks`. `swap_bytes` is the same
  shape with a single `bswap` (`enc_bswap` in the encoder: `0F C8+rd`), and
  `reverse_bits` is a per-bit loop. `leading_ones`/`trailing_ones`/`count_zeros`
  need no piece of their own: `leading_ones(x)` is `leading_zeros(!x)`,
  `trailing_ones(x)` is `trailing_zeros(!x)` and `count_zeros(x)` is
  `count_ones(!x)`, so the desugar wraps the receiver in `!` (`not`, which
  cannot overflow) and calls the matching helper. `rotate_left`/`rotate_right`
  are the one bit family that takes an argument: the desugar binds the receiver
  then the count and calls `__rmc_rotate_left` / `__rmc_rotate_right`, which are
  a single `rol`/`ror` (`enc_shift` gained the `/0` and `/1` group-2 encodings).
  The hardware masks the count to 6 bits, which is exactly Rust's `n % 64`, so
  every count is accepted (no panic) and no count is negated — the result is the
  same with and without `--overflow-checks`. They reuse the same
  bind-receiver-once path, so `g().count_ones()` calls `g` once, and the
  `call`/`defines` pruning drops each helper unless its method appears.
  Each desugar reads an operand more than once, so every non-trivial operand is
  first bound to a fresh `\0im<n>` temporary (`Sema::tmp`), receiver before
  argument — `g().abs()` calls `g` once, as rustc does; a plain variable or
  literal is reused directly. A method with an argument snapshots a *variable*
  receiver into a temp too, even though `bind` would leave it as a direct read:
  rustc evaluates and copies the scalar receiver before its arguments, so
  `x.min({ x = 9; 7 })` must compare against the original `x` (5), not the value
  the argument's block wrote. A literal receiver cannot be mutated, and any
  other receiver shape already gets a temp. The euclidean *receiver* instead goes
  through `Sema::force_temp`, which binds it even when it is a plain variable or
  literal: a literal or a const-tracked variable would let the synthesized
  `x / y` (or the dead `r - y` arm) fold and trip the overflow lint on
  `MIN.div_euclid(-1)` / `7.rem_euclid(MIN)` — programs rustc accepts, since it
  never folds the stdlib body. The divisor is deliberately left as `bind` so a
  literal divisor stays literal and `gen_div_const` can still strength-reduce
  `x % d` to a magic multiply. A `\0`-named temp is never entered into
  `const_locals` (the same reason). The desugar returns its result type (`i64`,
  or `bool` for the predicates) so the caller reports the right type without
  re-deriving it.
  `abs`'s synthesized `-x` is exempt from the compile-time neg-overflow lint
  (`Sema::in_builtin_neg`, named for the neg it suppresses), because rustc
  accepts `i64::MIN.abs()`: a written `-i64::MIN` is still rejected.
- **Parameter names (`E0415`) and `main` (`E0580`).** A function's parameters
  must be uniquely named; `check_fn` rejects a duplicate (rmc used to bind both
  to one slot). The parser applies the same check earlier
  (`check_dup_param_names`), because a parameter *pattern* binds names the
  semantic pass never sees as parameters: `fn f(S { x }: S, x: i64)` is rejected
  at parse time. `main` takes no parameters, checked in `analyze`.
- **Aggregate parameter patterns.** A parameter may be written as a struct or
  array pattern, `fn f(S { x, y }: S)` / `fn f([a, b]: [i64; 2])`. The parser
  (`parse_param`) keeps an ordinary parameter of the annotation's type (a hidden
  `\0p<n>` name) and stores the per-binding `let` statements in `FnDef::param_pats`
  (parallel to `params`); `check_fn` splices them at the top of the body before
  checking, so sema and codegen gain no case — the expansion is exactly the
  matching `let` destructuring, reading through the parameter. A refutable
  pattern (literal, variant, or-pattern) is rejected (`param_pat_binds`), and
  `mut` on the whole pattern is rejected. `sema` inserts a `None` into
  `param_pats` when it prepends the `self` receiver, keeping the vectors aligned.
- **Enum payload sub-patterns.** A payload variant's argument list takes the
  same sub-patterns a struct field or array element does: `E::A(_, y)`,
  `E::A(mut x)`, `E::A(S { x })`, `E::A([a, b])`, and a trailing `..`
  (`E::A(x, ..)`, `E::A(..)`). `Pat::Variant` carries `Option<(Vec<(bool, Pat)>,
  bool)>` — the argument patterns and the `..` flag. `desugar_arm` reads each
  argument as `EnumPayload { idx }` (the payload at its field index) and feeds it
  to `pat_binds_into`, which recurses through `struct_pat_binds` /
  `array_pat_binds`, so a nested pattern reads *through* the payload at its
  offset rather than copying the whole aggregate first. `VariantTest` carries
  `n_binds` (the argument count) and `has_rest`; `check_expr` requires
  `n_binds == fields.len()` unless `has_rest`, and rejects a payload read past
  the last field. A nested *variant* argument (`E::A(Inner::X(v))`) is
  refutable, so `parse_variant_pat_args` rejects it, as it rejects a literal
  argument, since rmc generates no per-field test.
- **Casts (`E0054`/`E0605`/`E0606`).** `check_cast` restricts `as` to the
  conversions rustc allows plus rmc's two documented extensions. A reference may
  only become a reference to the same type (`&mut T` may lose mutability to
  `&T`, never gain it), and never a scalar; any other scalar may not become a
  reference. The extensions are `i64 as bool` (rustc spells it `x != 0`) and the
  raw pointer cast `i64 as &T` / `i64 as &mut T` (rustc needs `as *const T`
  first), both used by the corpus (`boolops`, `listptr`) and the heap benchmark.
  Before this, `r as i64`, `r as &mut T`, `&T as &U` and `bool as &T` all
  compiled, reinterpreting a pointer as a value or forging mutability.
  `as` is parsed at its own precedence level (`parse_cast`, between `parse_mul`
  and `parse_unary`), looser than the unary operators and tighter than `*`. It
  used to live in `parse_postfix`, which bound it tighter than unary, so
  `*r as i64` parsed as `*(r as i64)` and a valid program failed with a
  reference-cast error. Rust's `as` sits just above `*` in the binary chain; this
  level is the same place, and the right-hand side is a type, so there is no
  operand recursion and no depth cost.
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
integer form and a `sete` for the `bool` form. The operand's type is taken from
`Codegen::static_ty` (falling back to `gen_expr`'s return value for a node it
does not model), *not* from `gen_expr` alone: `gen_bin` reports `i64` for a
`&`/`|`/`^` because the emitted instruction is the same for a 0/1 `bool` and an
integer, so `!(a | b)` on `bool` would otherwise be a bitwise `not`. The same
reasoning is why `licm::expr_ty` types a `bool` `&`/`|`/`^` as `Bool` (a hoisted
one bound to an `i64` slot turns a later `!` into a bitwise not) and follows the
operand for a `!` so a hoisted bitwise not is bound to an `i64` slot, not a
`bool` one.

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
the aggregate copy were never generated. `nest` mode also builds nested arrays
(`[[i64; c]; r]`) and arrays of structs: a list literal, a nested splat, a
variable-index read, an element write and a whole-element copy. Those pin the
stride (`size()/8` slots per element, so `array_off` takes `n * stride`) and the
splat loop, which for a wide element over the 8-word threshold copies one source
value into every element instead of walking the source along with the
destination. `ref` mode builds references (`&T` / `&mut T`): a reference to a
scalar local, a struct field, a nested field, an array element and a nested
array element, each read and written both directly and through a callee, plus a
whole aggregate read and written through a reference. A reference is one word
holding an address, so a wrong address on a *place* that is not a plain local
(`&mut o.mid.f`, `&mut a[i]`, `&mut m[i][j]`) writes a neighbouring word; no
other mode generates `&`/`*` at all, so this is the only randomised coverage of
the reference codegen. `pat` mode builds `match`
expressions: a multi-variant unit enum, a variant with two payload fields, a
binding pattern, a negative literal, a nested `match`, an `if let` (integer,
binding, payload-binding, `else if let` chain and a side-effecting scrutinee),
and a non-trivial
side-effecting scrutinee (a call) tested against several arms — the last is
what exercises `desugar_match`'s temporary, which must evaluate the scrutinee
exactly once (re-evaluating it per arm repeats a side effect). `nestpat` mode
builds nested struct sub-patterns (`S { a: A { m: B { .. }, n }, b }`) through a
chain of distinct struct types, so the pattern nests several levels deep, and
emits each pattern in `let`, `match` (through a reference), `if let` and `for`.
The expansion is a chain of field reads, so a wrong field order or a missing
intermediate read shows up as a wrong final value; `letdestr`/`fordestr` only
reach one level, so a deeper sub-pattern had no randomised coverage. `enumpat`
mode builds payload variant patterns with sub-patterns (`_`, `mut x`, a trailing
`..`, and a nested struct or array pattern inside `E::V(..)`), in `match`,
`if let` and a method, and reads them through a `&E` scrutinee (match
ergonomics), which no other mode emits; each function is called twice
so a shared slot or a leaked `mut` write is observable. It compares
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
them fails the fuzz job at `iter=0`. `expr` mode emits `str::len` and array
`.len()`. When adding a language feature, extend the
generator to exercise it — a clean fuzz run only covers the shapes it emits.

`--mode mutate` is the counterpart to the differential modes: it needs no rustc
reference at all. It seeds from the generators' programs and the `robust` corpus,
applies one to four random edits (splice a token, delete a run, duplicate a run),
and requires every mutant to fail cleanly — exit 0 or 1, no signal, no
`panicked`/`overflowed its stack` in stderr. A differential mode only compares
programs *both* compilers accept, so a crash on invalid input is invisible to it;
`mutate` and `robust` are the two modes that look for exactly that, the first
reaching error paths the fixed `robust` corpus does not. `--keep` writes a
failing mutant out for shrinking.

A new generator must be checked for *validity*, not just a clean run. The first
`ref` generator wrote `r.i0` (a field of the referent struct, not of the nested
one), which rustc rejected for every seed; the fuzzer skips a program rustc
cannot compile, so the mode reported "all match rustc" while generating nothing.
Run a new mode's programs through both compilers and count the rejections before
trusting the summary — a mode with a 100% skip rate is silent, not clean.

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

The macro surface is lowered at parse time and is deliberately tiny.
`print!`/`println!` take a literal format string and rewrite each `{}` to a
`__rmc_print` call whose argument's checked type picks the runtime writer
(`__rmc_write_i64`, `__rmc_write_bool`, `__rmc_write_str`, `__rmc_write_char`);
`println!` appends the newline itself. Every `{}` argument is evaluated before
the first byte of output, in source order (matching Rust), so the lowering
binds each argument to a fresh local first and the `{}` writes read those —
otherwise `println!("A{}B{}", f(), g())` would print `A` before `f` ran.
`format_arguments_run_before_the_first_write` and `tests/cases/fmtorder.*`
pin it; the fmtorder twin uses `println!` on both sides so the comparison
rests on the ordering rule, not on `print_str`'s trailing newline. The `char`
type is a distinct scalar (a Unicode scalar value in an 8-byte slot, so `c + 1`
is rejected); its literals take the usual escapes, `as i64` yields the code
point, and `match` accepts `'a'..='z'` / `'a'..'z'` range arms (lowered to the
same two comparisons as an integer range; an empty range or a non-`char` bound
is rejected). The `print_char(c)` builtin writes the UTF-8 bytes then a newline.
`assert!`/`assert_eq!`/`assert_ne!`/`panic!`/`unreachable!`/
`todo!`/`unimplemented!` desugar to `if !(cond) { exit(101) }` (or a bare
`exit(101)`), so a failure matches rustc's panic exit code; the message
arguments are parsed and discarded. `use` items are skipped at the top level
and inside a block; a `use` of a `std` subtree is recorded so a later
`process::exit(0)` after `use std::process;` resolves, and only
`std::process::exit` and `std::hint::black_box` are understood among `std::`
paths. A `pub` qualifier (on an item, a struct field, an `impl` method or a
nested `fn`, with any `(crate)`/`(super)`/`(in ..)` restriction) is dropped by
`skip_pub`, since every name resolves in one flat namespace; the prescan and the
block-body loop skip it too, and `scan_block_fns` looks through a `pub` before a
nested `fn`. An inline `mod NAME { .. }` is skipped by `skip_mod` (its items are
not visible); a `mod NAME;` that would load another file is a compile error. `str`
and `&str` are the same one-word value, so `ref_coerces` lets them
be exchanged at a variable, parameter, struct-field, enum-variant-field or
comparison site; there is no lifetime checking, so a `&'static str` field or
`&str` parameter may be written `str`.

There is no `Vec`, but the parser folds the few shapes the corpus uses to an
`i64` address: `Vec::with_capacity(k)`/`Vec::new()` become `alloc(k * 8)`,
`vec![e; n]` becomes `alloc(n * 8)` plus a `__rmc_fill`, `v.capacity()`/`v.len()`
fold to the constant count, and `v.as_mut_ptr()` keeps the address so a later
`.add(off)` is `p + off * elem`, scaling by the element width from the `Vec<T>`
annotation (`Vec<u8>` stays byte-addressed, and a `type Byte = u8` alias is
chased to its target). Each shape is recognised only directly
in a `let` initializer; anywhere else the `\0`-prefixed parser marker reaches
sema, which reports the source construct (see the `\0vecnew` arm). An
args-vector is never built: `std::env::args()...collect()` records the binding,
`for v in &args` desugars to `for i in skip..__rmc_nargs()`, and `args[i]`
becomes `__rmc_arg_or(i + skip, 0, 0, 0)`. The `__rmc_arg_or`, `__rmc_nargs` and
`__rmc_fill` helpers live in `runtime.rs`.

The `args().nth(i)` chain has two independent failure points, and
`finish_args_nth` tracks them in `__rmc_arg_or`'s last two arguments: `missing`
(the `Option` unwrap) and `parse` (the `Result` unwrap). `.unwrap_or(d)` leaves
both 0 and yields `d` for either failure; `.unwrap()` sets `missing`, so an
absent argument exits 101 while a present non-integer takes the fallback when
only `parse` is 0. The `and_then(|s| s.parse().ok())` chain recovers both, so it
sets neither. A present non-integer argument must not be read as zero, and the
turbofish `parse::<i64>()` type name lexes as a keyword token, not an
identifier.

Two relaxations make the pointer
shapes type: `*mut T as i64` is allowed (rmc maps `*mut T` and `&mut T` alike,
and rustc allows the raw form) and an integer/pointer coerce both ways, matching
the existing `i64 as &T` extension; a shared reference still may not become a
`&mut` one.

The `Vec`/pointer name tables (`vec_names`, `vec_elem`, `vec_len`, `vec_cap`,
`ptr_vars`, `args_vecs`) are lexically scoped like the nested-`fn` frames above:
`parse_block_inner` pushes a frame onto `name_scopes` and every binding pushes an
undo entry first (the `note_*` helpers), so `pop_name_scope` at block exit
removes a binding the block introduced and restores one it shadowed. Without
this, a nested `let v: Vec<i64>` would leave an outer `v: Vec<u8>` with an
eight-byte stride once the block ended.

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
  - `tests/cases/div.rs` covers both variants, powers of two, negatives,
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
- **`tests/cases/divtrap.rs` and `shovf.rs` pin the trap codegen.** `divtrap`
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
  does not match is left completely alone. `tests/cases/tco.rs` diffs `fib`
  and `fact` against rustc and `tests/cases/callrec.rs` covers the
  `1 + f(n-1) + f(n-2)` shape; unit tests pin the non-associative
  `f(x-1) - f(x-2)` case, a `return` in an argument, a foreign call in the left
  operand, and overflow checks.

## MIR back-end benchmark tuning (fair harness)

`bench/run_fair.py` compares the same `bench/mir/*.rs` source through both
back ends: `mir` = `rustc --emit=mir -O -Zno-codegen` + `rmcmir`, `llvm` =
`rustc --emit=obj -O -C lto=fat -C codegen-units=1` + `ld`, both linked against
rmc's own freestanding runtime. Read the harness before trusting a number.

- **Count the whole pipeline's peak RSS on both sides.** The `mir` column's RSS
  is `max(rustc --emit=mir, rmcmir)`; counting only `rustc` on the LLVM side
  (leaving out `ld`) credits LLVM with a free link step and turns a tie into a
  fake win. `measure_llvm` takes `max(c, ld)`.
- **`-Zno-codegen` is the whole point of the front end.** With it, `rustc
  --emit=mir` stops before LLVM codegen: `hello`/`stats` front-end CPU drops
  from ~267/538 ms to ~54/408 ms. rmc sets `RUSTC_BOOTSTRAP=1` on the child
  rustc; the harness mirrors that. Without the flag the MIR front end is as
  expensive as the LLVM one.
- **`rustc --emit=mir` uses ~3-5 MB *more* RSS than `--emit=obj`** (85892 vs
  85220 KB on `stats`), reproducibly, with `-Zthreads=1`, `debuginfo=0`,
  `mir-opt-level=2` and `incremental=false` all failing to close it. The
  compile-RSS axis is therefore an inherent front-end tie (~0.99), not
  something the back end can fix; the front end is rustc in *both* pipelines.
- **The compile CPU win is real and large** (~1.2x): skipping LLVM codegen.
  Wall time is a near-tie because both fronts pay rustc's ~25-30 ms startup.
- **Ratios inside +/- 0.02 are a tie.** The run axes take the minimum of 50
  interleaved samples, but two runs of the *same* binary still land within
  ~1% of each other on this host (measured spread 0.8-1.3% for `mix`/`heap`),
  so `0.99` carries no signal. `NOISE_BAND = 0.02` keeps the verdict from
  calling a tie a loss; the harness also prints the mean speedup per axis, so
  one program's tie cannot hide a regression. On the current host that mean is
  `wall 1.08, cpu 1.12, rss 1.00 (tie), size 18.8, run_wall 1.10, run_cpu 1.11,
  run_rss 1.20`: six wins and the one documented front-end tie.
- **`run_rss` above 1 is rmc *winning*.** rmc's `_start` is a bare syscall exit
  with no libc, so a short program peaks at ~36 kB while the LLVM+`ld` binary
  pays the ELF loader's ~48 kB. A `run_rss` of 1.2-1.3 is a 12-33% RAM win, not
  a loss; do not "fix" it.
- **Strip the LLVM link (`ld -s`).** rmc's linker emits no section headers, so
  its binary is symbol-free already; leaving LLVM's symtab in credits rmc with
  ~5.7 kB it did not save. `run.py` strips rustc for the same reason.

### The harness gates on output equality and interleaves sampling

Two properties make the verdict defensible, and both are easy to lose when
editing the harness:

- **Correctness gate.** `verify_outputs` runs both produced binaries once and
  aborts if stdout or exit status differ, before any run-time number is
  reported. A back end is only "faster" on a workload the other also computes
  correctly; without the gate, a back end that miscompiles (or traps early)
  would look fast.
- **Interleaved sampling.** `measure_stages` times the four compile stages
  (`mir.front`, `mir.back`, `llvm.front`, `llvm.back`) round-robin in one pass,
  and `measure_interleaved` does the same for the produced binaries, so CPU
  frequency and thermal drift cannot favour whichever pipeline ran first. A
  sequential sweep can flip a near-parity run axis (`mix`/`heap` sit at ~1.00)
  on its own.

`bench/run.py` (AST path) carries the same gate (`verify_outputs`) and
round-robin run sampling (`measure_runs`). `bench/run_mir.py` does *not*: its
`mir` column compiles `bench/mir/*.rs` while its `ast`/`llvm` columns compile
`bench/rust/*.rs`, so its run-time axes compare two different programs. Treat
its run numbers as indicative and use `run_fair.py` for the apples-to-apples
claim; the caveat is recorded in its docstring.

### Branchless min/max (`cmov`) was tried and rejected

A min/max diamond (`if v > acc { acc = v }`) lowered to a branch; LLVM emits
`cmov`. Adding `enc_cmov` to `elf.rs` and a `plan_cmovs` fold to
`mir_codegen.rs` made `stats` **1.16x slower** (3.27 -> 5.08 ms on the same
binary, `taskset -c 2`, best of 13) and was neutral (0.99-1.02) on the rest.
The reduction branches in these benchmarks are predictable — the accumulators
start at `i64::MIN`/`MAX` and the update usually takes the same side — so the
branch predicts ~perfectly while the `cmov` only adds a data dependency to a
latency-bound loop. LLVM wins `stats` because it *unrolls by two* and strength-
reduces the `i * 2654435761` multiply into two induction variables, not because
of `cmov`. Do not re-add the fold without measuring a program whose min/max
input is genuinely unpredictable.

### Recurring wide constants are hoisted into idle callee-saved registers

An i64 immediate that does not fit a sign-extended i32 needs a `movabs` to
materialise, and one named inside a loop pays that on every iteration (`stats`'s
`i * 2654435761` and the `% 1000003` magic multiplier are two per iteration).
`hoist_consts` ranks the constants that recur by loop depth (a single use in a
loop clears the bar) and materialises each into a callee-saved register once, at
function entry; `const_operand` and `gen_div_const` then read the register
instead of re-emitting the immediate.

Promotion runs first and claims the registers it wants; hoisting only fills what
is left. That ordering matters: hoisting *first* (so it claimed the spare
registers) displaced `stats`'s hot `count` local into memory, and the extra
load/store per iteration cost more than the two `movabs` it saved. A promoted
local is worth more than a hoisted constant -- a load versus a `movabs` -- so
hoisting is the tie-breaker, not the priority. It is a wash on programs whose
hot loop already uses all five callee-saved registers (`mandel`) and helps those
with registers to spare.

### No-op arithmetic (`add $0, %reg`) must be dropped

A struct field at offset 0 lowers to `add $0, %rax`. The instruction cannot
change its destination, but it *does* block the store-forwarding peephole that
would otherwise fold the pointer's round-trip through its frame slot, so every
field write keeps a reload. `drop_noop_arith` removes `add $0`/`sub $0` unless
the next instruction reads the flags it sets (`flags_used_by`). Measured on the
`heap` linked-list fill: one load per node removed, run 1.08x vs LLVM; `refs`
1.04x, `mix` 1.01x.

### A drop-glue read of a promoted caller-saved local must come from its spill slot

`gen_drop` spills the promoted caller-saved locals once, before the destructor
tree, and restores them after the whole tree. That is correct for the tree's
*calls*, but not for a read that happens *inside* the tree after one of them: the
destructor call clobbers the register, so a later `place_addr` that reads the
local gets garbage. Dropping `arr[i]` is the reachable case — the index local
feeds `Proj::IndexLocal`, and a struct whose first field drops before a second
field is addressed (`struct S { a: [Inner; 2], b: Inner }`) reads the index again
after the array's destructors have run, dropping `b` at `arr[<garbage>]`. While
`in_drop` is set, `local_operand` returns the local's caller-spill slot (written
by the tree's own spill) instead of the register. The index must be *promoted*
for this to bite: it needs a use count of at least 2 (`indexdrop.rs` raises it
with nested loops), otherwise it stays in its frame slot and the bug is latent.
Regression: `tests/mir/indexdrop.rs`.

### An aggregate literal is a variant only if a real `::variant` segment follows the type

Both aggregate-classification paths (`reclassify_enum_aggregate`, which uses the
destination local's type, and `is_enum_variant_name`, which decides by
capitalisation) must strip generics *before* looking for the variant segment. A
generic struct literal prints as `G::<T> { .. }`; splitting the raw name takes
the turbofish as the variant and adds a phantom tag word, shifting every field by
one (`G::<T>` with a module-qualified argument gave 12 instead of 119). Reducing
first makes `Option::<i64>::Some` -> `Option::Some` (a variant) and `G::<T>` ->
`G` (a struct). Regression: `tests/mir/genstruct.rs`.

### A type spelling is relative to the item that mentions it, and it can be qualified

rustc prints a type relative to the body that mentions it, and that relative
spelling is not always bare: inside `outer::get`, the nested type `outer::a::A`
prints as `a::A`, while the crate root spells it `outer::a::A`. A qualified
spelling looks like a complete path, so `canonicalize_types`' global bare-name
pass skips it (it must: `a::A` in two modules can name two distinct types). The
callee then keys a *different* type than the caller built, its layout is empty,
and field reads fall back to `k*8`, reading the wrong word past a multi-word
field. `Function::module` keeps the body's own module prefix from the raw header
(before the printer's relative spellings are resolved) and a per-body pass in
`canonicalize_types` resolves each qualified body spelling against it, longest
prefix first. Regression: `tests/mir/modrelpath.rs` (reads `1 5` instead of
`1 13` without the per-body pass). The inliner masks this: the callee needs a
large body (or `#[inline(never)]`) to be emitted at all.

## Pushing

`git push origin <branch>` needs a token with `contents: write`. The
`GITHUB_TOKEN` in this environment is sometimes such a token — pushes to
`rmc-1-core` through `rmc-4-patterns` have succeeded — and sometimes a
read-only GitHub App installation token (`ghu_` prefix, empty
`x-oauth-scopes`), which fetches fine but answers every write with `403
Resource not accessible by integration` on `receive-pack` and on the git-refs
API alike. The repo permission list can still read `push: true` for the app's
own account, so that is not a reliable signal. Check `git ls-remote` first,
then try the push; if it 403s, leave the commits local and report the block
rather than retrying. Never persist a token in `.git/config` or a file.

## PR branch stack

The four PRs stack linearly: `rmc-1-core` -> `rmc-2-optimiser` ->
`rmc-3-artifact-size` -> `rmc-4-patterns`, each targeting the one below it.
Review fixes land on the branch the finding names, then propagate *upward*:
merge the fixed lower branch into the branch above it. A fix that both a lower
and an upper branch already carry independently (because it was carried upward
before it landed below) will conflict on the duplicate hunks; resolve by keeping
either side — the content is identical — and commit the merge so the upper
branch descends from the corrected base and the PR reports `mergeable`.

Do not reach for `-X ours` (or `-X theirs`) to skip that resolution. When the
upper branch added a *method* the lower branch does not have, `-X ours` drops
the lower branch's copy of a *different* method silently, and the next build
fails with "duplicate definitions with name ..." once both are in the tree. A
plain merge surfaces the overlapping hunks; resolve them by hand. After any
such merge, rebuild and run `cargo test` plus `tests/run.py` before committing.

## Definite-initialization analysis (`sema.rs`)

The loop/branch init merge is where most rustc divergences have come from.
Rules that took real work to pin down, each verified against rustc and now
covered by a test in `compiler/tests/compile.rs`:

- **A `while` condition runs at least once.** A binding it initializes is
  initialized on the fall-through exit even if the body never runs, so
  snapshot the state *after* checking the condition and require it on every
  exit (the fall-through and each `break` that leaves the loop). Do not
  restore the pre-condition snapshot.
- **A `while` condition is re-evaluated whenever the body completes.** So a
  write to a plain `let` in the condition is a second write (E0384). Seed the
  condition's continuation (`loop_cont`) with the body's reachability before
  checking the condition, or the back edge is invisible to the condition's
  own block walk.
- **A `for` range bound is evaluated before the loop's own label enters
  scope.** A `break 'a` in a bound names the enclosing `'a` loop even when the
  `for` reuses that label, so the bound scanners must *not* apply the
  shadowing (`outward`) test; only the body is shadowed.
- **An `if` whose two branches both diverge does not fall through.** Merging
  such branches as "uninitialized" clobbers a binding that was live before the
  `if`; keep the pre-`if` state instead.
- **rustc counts unreachable breaks.** `'a: loop { loop { break; break 'a; } }`
  is rejected (E0308) because the dead `break 'a` still makes the outer loop
  `()`, not `!`. `block_has_break_at` must therefore scan *every* statement,
  including ones after a `break`/`return` — a "stop at non-falling stmt" walk
  is wrong here.
- **Check ordering matters for the message.** rustc reports an unlabelled
  `break`/`continue` in a `while` condition before it evaluates the condition,
  so that check must precede `check_expr`.
- **`&&`/`||` short-circuit, so track the two branch states, not one merge.**
  The right operand runs on only one of the two paths, so a binding it
  initializes is not initialized after the expression unless it was already.
  A *global* merge (both operands required) is wrong: rustc is path-sensitive,
  and it over-rejects valid `if a && b { .. }` (true path ran `b`) and
  `if a || b { .. }` (true path may have skipped `b`). `check_expr` instead
  records `cond_then_init`/`cond_else_init` — the state on entry to the true
  and false branch — and `check_stmt`'s `if`/`while` restore them before
  checking each branch. For a single `a && b`, the true state is the
  right operand's outcome and the false state merges both ways it can be false;
  `a || b` is the mirror. The two states are composed per operand, not read off
  the scope — see the chained-guard rule below. Leave the scope with the
  "either path" state for a value use (`let x = a && b`). Only a *top-level*
  short circuit splits the branches; consume the slots on every condition so a
  nested one cannot leak.
- **A chained guard checks its right operand under the left-true state, so
  compose outcomes rather than reading the scope.** `(a && { x = 7; true }) &&
  x == 7` reaches the comparison only where the inner `&&` was true — exactly
  where `x` was assigned — but the inner short circuit has already written the
  merged "either path" state onto the scope, so checking the comparison there
  wrongly rejects `x` (E0381). Each operand therefore has a *true* and a
  *false* outcome state: `check_expr` leaves a logical operand's pair in the
  `cond_*` slots, and any other operand's single exit state is both. The right
  operand is checked under the left-true state for `&&` (left-false for `||`),
  and the two outcomes are merged for the resulting true, false and
  post-expression states. `outcome_states` consumes the slots for every
  operand, so a nested short circuit cannot leak. The post-expression state is
  still the conservative merge, so a value use cannot assume the right operand
  ran.
- **A `for` bound `continue` targets the enclosing loop even when the label is
  reused.** The bounds are evaluated before the `for`'s own label enters scope,
  so `stmt_contains_continue` must inspect `start`/`end` regardless of the
  shadowing test and guard only the body. Missing this hid the back edge to a
  `while` condition, letting a repeated immutable write pass (E0384).
- **rustc's `arithmetic_overflow` lint is best-effort, and rmc is stricter.**
  rmc folds a constant expression and rejects an overflow it can see. rustc's
  lint can silently give up once a function body grows (a second statement in
  `main` suppresses the diagnostic for an overflowing `*`), so a large generated
  program where rustc accepts but rmc rejects is a known false-rejection, not a
  bug in the fold. The differential fuzzers keep generated values small so this
  never triggers under CI.

Tests that claim to guard a runtime bug must *execute* the program and assert
the result (`run_exit_code` / `run_output`); an `asm(...).contains("main:")`
assertion passes even when the emitted code is wrong.
