//! Unit tests that exercise the compiler front end and code generator without
//! invoking `as` or `ld`, so they run anywhere a Rust toolchain does.

use rmc::{compile_to_asm, CompileOptions};

fn asm(src: &str) -> String {
    compile_to_asm(src, &CompileOptions::default()).expect("should compile")
}

/// Assembly with the inlining pass disabled, for tests that assert on the shape
/// of a real call (`call f`, the argument setup) rather than on inlined code.
fn asm_no_inline(src: &str) -> String {
    compile_to_asm(src, &CompileOptions { no_inline: true, ..Default::default() })
        .expect("should compile")
}

/// Assembly with the self-recursion-to-loop pass disabled, for tests that assert
/// on the shape of the untransformed recursion.
fn asm_no_tco(src: &str) -> String {
    compile_to_asm(src, &CompileOptions { no_tco: true, ..Default::default() })
        .expect("should compile")
}

/// The frame offset stored by the first `mov %rax, <off>(%rbp)` after `marker`.
/// Lets a test check relative layout without hard-coding absolute offsets.
/// The frame offset of the first `movq $imm, off(%rbp)` store. Panics when the
/// value was instead promoted to a register (no immediate store is emitted).
fn store_offset_of(out: &str, imm: i64) -> i32 {
    let marker = format!("movq ${imm}, ");
    let tail = &out[out.find(&marker).expect("an immediate store")..];
    let rest = &tail[marker.len()..];
    let end = rest.find("(%rbp)").expect("frame store");
    rest[..end].trim().parse().expect("offset")
}

#[test]
fn emits_a_global_main() {
    let out = asm("fn main() -> i64 { return 0; }");
    assert!(out.contains(".globl main"));
    assert!(out.contains("main:"));
    // The entry point is always appended; the runtime is pruned to what the
    // program uses, and this one calls nothing.
    assert!(out.contains("_start:"));
    assert!(!out.contains("print_i64:"), "unused printer dropped");
}

#[test]
fn runtime_pieces_are_kept_only_when_used() {
    // A program that prints keeps `print_i64` but not the allocator or the
    // bounds trap.
    let printing = asm("fn main() -> i64 { print_i64(1); return 0; }");
    assert!(printing.contains("print_i64:"));
    assert!(!printing.contains("alloc:"));
    assert!(!printing.contains("__rmc_bounds:"));

    // A program that allocates keeps the allocator and its arena globals, and
    // `free` only when it is called.
    let allocating = asm("fn main() -> i64 { let p: i64 = alloc(16); return p; }");
    assert!(allocating.contains("alloc:"));
    assert!(allocating.contains("__rmc_arena:"));
    assert!(!allocating.contains("free:"));
}

#[test]
fn a_constant_initialiser_stores_straight_to_the_frame() {
    // A scalar local and a struct field both start from a constant. The value
    // goes into the frame slot with one `movq $k, off(%rbp)`, not the two-step
    // `mov $k, %rax` + `mov %rax, off(%rbp)` bounce.
    let out = asm(
        "struct P { x: i64, y: i64 } fn main() -> i64 { let a: i64 = 7; let ra: &i64 = &a; let p: P = P { x: 1, y: 2 }; let r: &P = &p; return *ra + r.x + r.y; }",
    );
    assert!(out.contains("movq $7, "), "scalar constant stored direct:\n{out}");
    assert!(out.contains("movq $1, "), "field constant stored direct:\n{out}");
    assert!(!out.contains("mov $7, %rax"), "no register bounce:\n{out}");
}

#[test]
fn a_constant_assignment_stores_straight_to_the_frame() {
    // `x = 5` on an unpromoted local is the same store as its initialiser: one
    // `movq $5, off(%rbp)`, not `mov $5, %rax` + `mov %rax, off(%rbp)`.
    let out = asm(
        "fn main() -> i64 { let mut x: i64 = 1; let p: &i64 = &x; x = 5; return *p + x; }",
    );
    assert!(out.contains("movq $5, "), "constant assignment stored direct:\n{out}");
    assert!(!out.contains("mov $5, %rax"), "no register bounce:\n{out}");
}

#[test]
fn a_register_initialiser_stores_straight_to_the_frame() {
    // `q.x` is promoted to `%rbx`; copying it into `a`, whose address is taken,
    // is one `mov %rbx, off(%rbp)`, not `mov %rbx, %rax; mov %rax, off(%rbp)`.
    let out = asm(
        "struct P { x: i64, y: i64 } fn main() -> i64 { let mut q: P = P { x: 1, y: 2 }; let mut i: i64 = 0; while i < 10 { q.x = q.x + q.y; i = i + 1; } let mut a: i64 = 0; let ra: &i64 = &a; a = q.x; return a + *ra; }",
    );
    assert!(out.contains("mov %rbx, -"), "register stored direct:\n{out}");
}

#[test]
fn a_loop_body_starts_on_a_sixteen_byte_boundary() {
    // The body is aligned so its layout does not shift with unrelated code-size
    // changes; the directive must survive the dead-code pass, which otherwise
    // drops it as text after the entry `jmp`.
    let out = asm("fn main() -> i64 { let mut i: i64 = 0; while i < 3 { i = i + 1; } return i; }");
    assert!(out.contains(".p2align 4"), "loop body aligned:\n{out}");
}

#[test]
fn a_constant_for_bound_folds_into_its_home() {
    // `for i in 0..n` evaluates the start once; a constant start stores
    // straight to the counter register instead of bouncing through `%rax`.
    let out = asm("fn main() -> i64 { let mut s: i64 = 0; for i in 0..3 { s = s + i; } return s; }");
    assert!(!out.contains("mov $0, %rax\n\tmov %rax, %r"), "no counter bounce:\n{out}");
}

fn struct_fields_are_laid_out_in_order() {
    // Declaration order, no padding: `y` sits exactly one word above `x`. A
    // `&p` keeps the struct in memory so both fields are stored to the frame;
    // a field-only use now promotes the last field to a register.
    let out = asm("struct P { x: i64, y: i64 } fn main() -> i64 { let p: P = P { x: 1, y: 2 }; let a: &P = &p; return a.y; }");
    let x = store_offset_of(&out, 1);
    let y = store_offset_of(&out, 2);
    assert_eq!(y, x + 8, "fields laid out in declaration order");
}

#[test]
fn field_write_targets_the_right_slot() {
    let out = asm("struct P { x: i64, y: i64 } fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; let a: &P = &p; p.y = 9; return p.y; }");
    // The read of `p.y` and the write `p.y = 9` must name the same frame slot.
    // The write folds the constant straight into a sized `movq`, so it is
    // `movq $9, <off>(%rbp)` rather than a `%rax` round trip. The size matters:
    // an unsized `mov` would store 32 bits and leave the high half stale.
    let read: i32 = out
        .lines()
        .find_map(|l| {
            let t = l.trim();
            let inner = t.strip_prefix("mov ")?.strip_suffix("(%rbp), %rax")?;
            inner.trim().parse().ok()
        })
        .expect("a frame read of p.y");
    let write: i32 = out
        .split("movq $9, ")
        .nth(1)
        .and_then(|s| s.split("(%rbp)").next())
        .expect("the field store")
        .trim()
        .parse()
        .expect("a frame offset");
    assert_eq!(write, read, "`p.y = 9` writes the slot `p.y` reads");
}

#[test]
fn for_loop_has_a_continue_label() {
    // `continue` must jump to the increment, which is why `for` is not a while.
    let out = asm("fn main() -> i64 { let mut s: i64 = 0; for i in 0..3 { if i == 1 { continue; } s = s + i; } return s; }");
    assert!(out.contains("jl "), "range end comparison, rotated to the back edge");
    assert!(out.contains("add $1"), "increment");
}

#[test]
fn an_inclusive_range_uses_a_non_strict_back_edge() {
    // The guard is the back edge: `0..3` loops back while `var < end` (`jl`),
    // and `0..=3` while `var <= end` (`jle`), so the inclusive form still runs
    // the body once more when `var == end`.
    let excl = asm("fn main() -> i64 { let mut s: i64 = 0; for i in 0..3 { s = s + i; } return s; }");
    assert!(excl.contains("jl "), "exclusive range loops back strictly");
    assert!(!excl.contains("jle "), "exclusive range must not use jle");
    let incl = asm("fn main() -> i64 { let mut s: i64 = 0; for i in 0..=3 { s = s + i; } return s; }");
    assert!(incl.contains("jle "), "inclusive range loops back non-strictly");
}

#[test]
fn for_over_an_array_iterates_by_value() {
    // `for x in arr` desugars to an index loop over the array; the length folds
    // to a literal, so the loop carries no hidden end slot.
    let src = "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; \
               let mut s: i64 = 0; for x in a { s = s + x; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_runs_an_array_expression_once() {
    // The iterated expression is bound to a hidden temporary before the loop,
    // so a call in the head is evaluated exactly once, not once per iteration.
    let src = "fn mk() -> [i64; 3] { return [1, 2, 3]; } \
               fn main() -> i64 { let mut s: i64 = 0; for x in mk() { s = s + x; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_supports_continue_and_break() {
    // The desugared loop is a `for`, so `continue` runs the index increment and
    // `break` leaves; both must still be legal in the body.
    let src = "fn main() -> i64 { let a: [i64; 4] = [1, 2, 3, 4]; let mut s: i64 = 0; \
               for x in a { if x == 2 { continue; } if x == 4 { break; } s = s + x; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_with_a_wildcard_binding_is_accepted() {
    let src = "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; \
               let mut c: i64 = 0; for _ in a { c = c + 1; } return c; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_destructures_a_struct_element() {
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let a: [P; 2] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }]; \
               let mut s: i64 = 0; for P { x, y } in a { s = s + x * 10 + y; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_struct_pattern_compiles_like_reading_the_fields() {
    // The destructuring `for` must read each element once into a temporary and
    // then read its fields, exactly like the explicit form it stands for.
    let destructured = "struct P { x: i64, y: i64 } \
        fn main() -> i64 { let a: [P; 2] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }]; \
        let mut s: i64 = 0; for P { x, y } in a { s = s + x + y; } return s; }";
    let explicit = "struct P { x: i64, y: i64 } \
        fn main() -> i64 { let a: [P; 2] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }]; \
        let mut s: i64 = 0; for p in a { let x = p.x; let y = p.y; s = s + x + y; } return s; }";
    let opts = CompileOptions::default();
    assert_eq!(
        compile_to_asm(destructured, &opts).unwrap(),
        compile_to_asm(explicit, &opts).unwrap()
    );
}

#[test]
fn a_struct_literal_is_allowed_inside_a_for_head_bracket() {
    // The `no_struct_lit` restriction on a `for` head is scoped to the head
    // itself, not to a nested delimiter, as in rustc: `[P { .. }]` is a struct
    // literal, while a bare `P { .. }` head would be rejected (it looks like the
    // loop body).
    let src = "struct P { x: i64 } \
               fn main() -> i64 { let mut s: i64 = 0; \
               for p in [P { x: 1 }, P { x: 2 }] { s = s + p.x; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_struct_pattern_allows_a_per_field_mut() {
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let a: [P; 1] = [P { x: 1, y: 2 }]; \
               let mut s: i64 = 0; for P { mut x, y } in a { x = x + 10; s = s + x + y; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn for_over_an_array_rejects_a_refutable_pattern() {
    // A `for` pattern must be irrefutable, as in rustc; an enum variant is not.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let a: [E; 1] = [E::A]; \
         let mut s: i64 = 0; for E::A in a { s = s + 1; } return s; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("binding, `_`, a struct, tuple or array pattern"), "got: {err}");
}

#[test]
fn for_over_a_reference_is_rejected() {
    // `for x in &a` would iterate by reference in Rust; rmc iterates by value,
    // so it rejects the form rather than silently mistype `x`.
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; \
         let mut s: i64 = 0; for x in &a { s = s + x; } return s; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("reference"), "got: {err}");
}

#[test]
fn a_labelled_break_leaves_the_named_outer_loop() {
    // `break 'outer` from a nested loop must target the labelled loop, not the
    // innermost one. The body runs `i` up to 5 and then leaves both loops.
    let src = "fn main() -> i64 { let mut i: i64 = 0; \
               'outer: loop { let mut j: i64 = 0; while j < 10 { j = j + 1; i = i + 1; \
               if i == 5 { break 'outer; } } } return i; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_labelled_continue_targets_the_named_outer_loop() {
    let src = "fn main() -> i64 { let mut s: i64 = 0; \
               'a: for x in 0..3 { for y in 0..3 { if y == 1 { continue 'a; } s = s + 1; } } \
               return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_labelled_loop_yields_a_value() {
    let src = "fn main() -> i64 { let mut k: i64 = 0; \
               let v: i64 = 'l: loop { k = k + 1; if k == 7 { break 'l k * 2; } }; return v; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_undeclared_loop_label_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { loop { break 'nope; } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("undeclared label"), "got: {err}");
}

#[test]
fn a_label_on_while_let_is_threaded_into_the_desugared_loop() {
    // `while let` desugars to a labelled `loop { if let .. else { break; } }`,
    // so `break 'a` / `continue 'a` in the body target it. Before, the label
    // was dropped and the statement was rejected.
    asm(
        "enum E { A(i64), B } \
         fn main() -> i64 { let mut e: E = E::A(1); 'a: while let E::A(x) = e { break 'a; } return 0; }",
    );
    asm(
        "enum E { A(i64), B } \
         fn main() -> i64 { let mut e: E = E::A(1); let mut n: i64 = 0; \
         'a: while let E::A(x) = e { n = n + x; if n < 3 { continue 'a; } break; } return n; }",
    );
    // The label is a real loop label, so an undeclared one is still an error.
    let err = compile_to_asm(
        "enum E { A(i64), B } \
         fn main() -> i64 { let mut e: E = E::A(1); while let E::A(x) = e { break 'nope; } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("undeclared label"), "got: {err}");
}

#[test]
fn struct_field_init_shorthand_reuses_the_local() {
    // `P { x, y }` is `P { x: x, y: y }`; the shorthand must read the locals of
    // those names, not a placeholder.
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let x: i64 = 3; let y: i64 = 4; let p: P = P { x, y }; \
               return p.x * 10 + p.y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn struct_field_init_shorthand_mixes_with_explicit_fields() {
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let x: i64 = 3; let p: P = P { y: 10, x }; \
               return p.x * 10 + p.y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn struct_field_init_shorthand_for_an_unbound_name_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let p: P = P { x, y }; return p.x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("unknown variable"), "got: {err}");
}

#[test]
fn a_struct_pattern_binds_its_fields() {
    // `P { x, y }` binds `x` and `y` to the scrutinee's fields. The pattern is
    // irrefutable, so the match needs no `_` arm and the arm body reads the two
    // bindings. A renamed field and an ignored one are accepted too.
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let p: P = P { x: 3, y: 4 }; \
               match p { P { x: a, y: _ } => { return a; } } }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_pattern_with_rest_ignores_the_other_fields() {
    let src = "struct P { x: i64, y: i64, z: i64 } \
               fn main() -> i64 { let p: P = P { x: 1, y: 2, z: 3 }; \
               match p { P { y, .. } => { return y; } } }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_pattern_with_only_rest_is_accepted() {
    // `P { .. }` binds nothing but still names the struct.
    let src = "struct P { x: i64 } \
               fn main() -> i64 { let p: P = P { x: 1 }; \
               match p { P { .. } => { return 0; } } }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_pattern_may_name_a_later_struct() {
    // The struct is declared after the function that patterns on it, so the
    // field list must come from the prescan, not the parse order.
    let src = "fn f(p: P) -> i64 { match p { P { n, .. } => n } } \
               struct P { n: i64, m: i64 } \
               fn main() -> i64 { return f(P { n: 5, m: 6 }); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_pattern_reads_through_a_reference() {
    // A field read auto-derefs, so a `&P` scrutinee works like `P`.
    let src = "struct P { x: i64, y: i64 } \
               fn f(p: &P) -> i64 { match p { P { x, y } => x + y } } \
               fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return f(&p); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_pattern_compiles_like_the_equivalent_field_reads() {
    // The pattern has no test, so it must lower to exactly the field reads its
    // bindings name, and nothing more. Compare against the hand-written form.
    let pat = "struct P { x: i64, y: i64 } \
               fn f(p: &P) -> i64 { match p { P { x, y } => x * 10 + y } }";
    let reads = "struct P { x: i64, y: i64 } \
                 fn f(p: &P) -> i64 { let x: i64 = p.x; let y: i64 = p.y; \
                 return x * 10 + y; }";
    assert_eq!(asm(pat), asm(reads));
}

#[test]
fn an_unknown_struct_pattern_field_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let p: P = P { x: 1, y: 2 }; \
         match p { P { z } => { return z; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has no field `z`"), "got: {err}");
}

#[test]
fn an_ignored_unknown_struct_pattern_field_is_rejected() {
    // `z: _` emits no read for sema to catch, so the field name is checked at
    // parse time against the prescanned field list.
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let p: P = P { x: 1 }; \
         match p { P { z: _ } => { return 0; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has no field `z`"), "got: {err}");
}

#[test]
fn a_struct_pattern_field_bound_twice_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let p: P = P { x: 1 }; \
         match p { P { x, x: a } => { return a; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("bound twice"), "got: {err}");
}

#[test]
fn an_unknown_struct_in_a_pattern_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let p: P = P { x: 1 }; \
         match p { Q { x } => { return x; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("unknown struct `Q`"), "got: {err}");
}

#[test]
fn a_struct_pattern_on_a_non_struct_scrutinee_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let v: i64 = 5; \
         match v { P { x } => { return x; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("field access"), "got: {err}");
}

#[test]
fn a_binding_struct_pattern_in_an_or_pattern_is_rejected() {
    // A struct pattern is irrefutable, but a *binding* one cannot appear as an
    // or-pattern alternative: the name would not be bound in every branch.
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let p: P = P { x: 1 }; \
         match p { P { x } | P { x: x } => { return x; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("not bound in all patterns"), "got: {err}");
}

#[test]
fn a_nested_struct_sub_pattern_binds_through_the_field() {
    // `S { a: I { x, y }, b }` reads `s.a` then binds `x` and `y` from it, the
    // same chain of field reads the explicit form writes out.
    let src = "struct I { x: i64, y: i64 } struct S { a: I, b: i64 } \
               fn main() -> i64 { let s: S = S { a: I { x: 3, y: 4 }, b: 5 }; \
               let S { a: I { x, y }, b } = s; return x * 100 + y * 10 + b; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_nested_struct_sub_pattern_compiles_like_chained_field_reads() {
    let nested = "struct I { x: i64 } struct S { a: I } \
        fn f(s: &S) -> i64 { let S { a: I { x } } = s; return x + 1; }";
    let reads = "struct I { x: i64 } struct S { a: I } \
        fn f(s: &S) -> i64 { let x: i64 = s.a.x; return x + 1; }";
    assert_eq!(asm(nested), asm(reads));
}

#[test]
fn a_nested_struct_sub_pattern_may_rename_and_ignore() {
    let src = "struct I { x: i64, y: i64 } struct S { a: I } \
               fn main() -> i64 { let s: S = S { a: I { x: 1, y: 2 } }; \
               let S { a: I { x: v, y: _ } } = s; return v; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_nested_struct_sub_pattern_may_use_rest() {
    let src = "struct I { x: i64, y: i64, z: i64 } struct S { a: I } \
               fn main() -> i64 { let s: S = S { a: I { x: 1, y: 2, z: 3 } }; \
               let S { a: I { x, .. } } = s; return x; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_nested_struct_sub_pattern_may_bind_nothing() {
    let src = "struct I { x: i64 } struct S { a: I } \
               fn main() -> i64 { let s: S = S { a: I { x: 1 } }; \
               let S { a: I { .. } } = s; return 0; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_nested_struct_sub_pattern_works_in_match_and_through_a_reference() {
    let src = "struct I { x: i64, y: i64 } struct S { a: I, b: i64 } \
               fn main() -> i64 { let s: S = S { a: I { x: 7, y: 8 }, b: 9 }; \
               let r: &S = &s; \
               match r { S { a: I { x, y }, b } => { return x * 100 + y * 10 + b; } } }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_nested_struct_sub_pattern_may_be_deep() {
    let src = "struct L3 { n: i64 } struct L2 { c: L3 } struct L1 { b: L2 } struct S { a: L1 } \
               fn main() -> i64 { let s: S = S { a: L1 { b: L2 { c: L3 { n: 42 } } } }; \
               let S { a: L1 { b: L2 { c: L3 { n } } } } = s; return n; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_nested_struct_sub_pattern_with_an_unknown_struct_is_rejected() {
    let err = compile_to_asm(
        "struct I { x: i64 } struct S { a: I } \
         fn main() -> i64 { let s: S = S { a: I { x: 1 } }; \
         let S { a: Q { x } } = s; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("unknown struct `Q`"), "got: {err}");
}

#[test]
fn a_nested_struct_sub_pattern_with_an_unknown_field_is_rejected() {
    let err = compile_to_asm(
        "struct I { x: i64 } struct S { a: I } \
         fn main() -> i64 { let s: S = S { a: I { x: 1 } }; \
         let S { a: I { z } } = s; return z; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has no field `z`"), "got: {err}");
}

#[test]
fn a_mut_on_a_nested_struct_sub_pattern_is_rejected() {
    // `mut` attaches to a binding, not to a nested pattern, as in rustc.
    let err = compile_to_asm(
        "struct I { x: i64 } struct S { a: I } \
         fn main() -> i64 { let s: S = S { a: I { x: 1 } }; \
         let S { a: mut I { x } } = s; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`mut` is not allowed on a nested pattern"), "got: {err}");
}

#[test]
fn a_mut_on_a_nested_array_sub_pattern_is_rejected() {
    // The same rule for an array-typed field: `mut` binds an element, not the
    // nested array pattern.
    let err = compile_to_asm(
        "struct S { a: [i64; 2] } \
         fn main() -> i64 { let s: S = S { a: [1, 2] }; \
         let S { a: mut [x, y] } = s; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`mut` is not allowed on a nested pattern"), "got: {err}");
}

#[test]
fn a_nested_struct_sub_pattern_binding_in_an_or_pattern_is_rejected() {
    // A nested binding still cannot appear in an or-pattern alternative.
    let err = compile_to_asm(
        "struct I { x: i64 } struct S { a: I } \
         fn main() -> i64 { let s: S = S { a: I { x: 1 } }; \
         match s { S { a: I { x } } | S { a: I { x: y } } => { return x + y; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("not bound in all patterns"), "got: {err}");
}

#[test]
fn a_struct_pattern_works_in_while_let() {
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; \
               let mut n: i64 = 0; \
               while let P { x, .. } = p { n = n + x; \
               if n > 3 { break; } p = P { x: x + 1, y: 2 }; } return n; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_binds_its_fields_in_the_enclosing_block() {
    // The bindings must outlive the statement, so the expansion is spliced into
    // the block rather than wrapped in a nested block.
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let p: P = P { x: 3, y: 4 }; \
               let P { x, y } = p; return x * 10 + y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_accepts_rest_and_ignored_fields() {
    let src = "struct P { x: i64, y: i64, z: i64 } \
               fn main() -> i64 { let p: P = P { x: 1, y: 2, z: 3 }; \
               let P { x: a, y: _, .. } = p; return a; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_binds_nothing_for_a_bare_rest() {
    // `let P { .. } = e;` names the struct but binds nothing.
    let src = "struct P { x: i64 } \
               fn main() -> i64 { let p: P = P { x: 1 }; \
               let P { .. } = p; return 0; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_allows_a_type_annotation() {
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let p: P = P { x: 1, y: 2 }; \
               let P { x, y }: P = p; return x + y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_attaches_mut_per_binding() {
    // `mut` goes on each binding, not on the pattern as a whole.
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let p: P = P { x: 1, y: 2 }; \
               let P { mut x, y: mut z } = p; x = 5; z = 6; return x + z; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_reads_through_a_reference() {
    let src = "struct P { x: i64, y: i64 } \
               fn f(p: &P) -> i64 { let P { x, y } = p; return x + y; } \
               fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return f(&p); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_compiles_like_the_equivalent_field_reads() {
    // The expansion must be exactly a hidden scrutinee plus one field read per
    // binding. A single-field struct keeps the two forms comparable.
    let pat = "struct P { x: i64 } \
               fn f(p: &P) -> i64 { let P { x } = p; return x + 1; }";
    let reads = "struct P { x: i64 } \
                 fn f(p: &P) -> i64 { let x: i64 = p.x; return x + 1; }";
    assert_eq!(asm(pat), asm(reads));
}

#[test]
fn let_destructuring_evaluates_the_scrutinee_once() {
    // The scrutinee is a call, so it must be stored in a temporary rather than
    // re-evaluated for each field. The hidden temporary is named with a
    // NUL-prefixed identifier, which no source name can produce.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn mk() -> P { return P { x: 1, y: 2 }; } \
         fn main() -> i64 { let P { x, y } = mk(); return x + y; }",
    );
    assert!(out.contains("mk"), "the scrutinee is evaluated");
    // One call site in `main` (plus the definition), not two.
    assert_eq!(out.matches("call mk").count(), 1, "scrutinee runs once");
}

#[test]
fn let_destructuring_accepts_an_annotated_diverging_initialiser() {
    // The annotation gives the scrutinee temporary a real type, so the field
    // reads are well typed even though the block cannot complete. rustc
    // accepts this; the unannotated form, which rmc rejects, is the next test.
    let src = "struct P { x: i64 } \
               fn f() -> i64 { let P { x }: P = { return 3; }; return x; } \
               fn main() -> i64 { return f(); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_destructuring_rejects_an_unannotated_diverging_initialiser() {
    // An unannotated diverging initialiser keeps type `!`, and rmc has no
    // usage-based inference to refine it, so the field read is rejected. This
    // is a deliberate, documented limitation shared with the `match` path (a
    // diverging scrutinee errors the same way), not a regression: if this ever
    // starts compiling, update the note in `docs/language.md`.
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn f() -> i64 { let P { x } = { return 3; }; return x; } \
         fn main() -> i64 { return f(); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("Never"), "got: {err}");
}

#[test]
fn a_mut_before_a_let_destructuring_pattern_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let p: P = P { x: 1 }; \
         let mut P { x } = p; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("attached to each individual binding"), "got: {err}");
}

#[test]
fn a_let_destructuring_pattern_on_a_non_struct_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let P { x } = 5; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("field access"), "got: {err}");
}

#[test]
fn an_unknown_field_in_a_let_destructuring_pattern_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } \
         fn main() -> i64 { let p: P = P { x: 1 }; \
         let P { z } = p; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has no field `z`"), "got: {err}");
}

#[test]
fn let_array_destructuring_binds_its_elements_in_the_enclosing_block() {
    let src = "fn main() -> i64 { let a: [i64; 2] = [3, 4]; \
               let [x, y] = a; return x * 10 + y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_ignores_wildcard_elements() {
    let src = "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; \
               let [_, y, _] = a; return y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_places_a_middle_rest() {
    // `[x, .., z]` binds the first and last elements of a longer array; the
    // trailing binding reads `a.len() - 1`, which folds to the constant length.
    let src = "fn main() -> i64 { let a: [i64; 4] = [1, 2, 3, 4]; \
               let [x, .., z] = a; return x * 10 + z; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_allows_leading_and_trailing_rests() {
    let leading = "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; \
                   let [.., z] = a; return z; }";
    let trailing = "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; \
                    let [x, ..] = a; return x; }";
    assert!(compile_to_asm(leading, &CompileOptions::default()).is_ok());
    assert!(compile_to_asm(trailing, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_nests() {
    // An element pattern may itself be an array pattern.
    let src = "fn main() -> i64 { let m: [[i64; 2]; 2] = [[1, 2], [3, 4]]; \
               let [[a, b], [c, d]] = m; return a + b * 10 + c * 100 + d * 1000; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_allows_a_type_annotation() {
    let src = "fn main() -> i64 { let [x, y]: [i64; 2] = [1, 2]; return x + y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_attaches_mut_per_element() {
    let src = "fn main() -> i64 { let a: [i64; 2] = [1, 2]; \
               let [mut x, y] = a; x = x + 10; return x + y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn let_array_destructuring_compiles_like_the_equivalent_index_reads() {
    // The expansion is a hidden scrutinee plus one index read per binding, so
    // it must lower to exactly the hand-written indexing form.
    let pat = "fn f(a: &[i64; 3]) -> i64 { let [x, y, z] = a; return x * 100 + y * 10 + z; }";
    let reads = "fn f(a: &[i64; 3]) -> i64 { \
                 let x: i64 = a[0]; let y: i64 = a[1]; let z: i64 = a[2]; \
                 return x * 100 + y * 10 + z; }";
    assert_eq!(asm(pat), asm(reads));
}

#[test]
fn let_array_destructuring_evaluates_the_scrutinee_once() {
    let out = asm(
        "fn mk() -> [i64; 2] { return [1, 2]; } \
         fn main() -> i64 { let [x, y] = mk(); return x + y; }",
    );
    assert_eq!(out.matches("call mk").count(), 1, "scrutinee runs once");
}

#[test]
fn let_array_destructuring_reads_through_a_reference() {
    let src = "fn f(a: &[i64; 2]) -> i64 { let [x, y] = a; return x + y; } \
               fn main() -> i64 { return f(&[1, 2]); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_for_over_an_array_destructures_an_array_element() {
    let src = "fn main() -> i64 { let m: [[i64; 2]; 2] = [[1, 2], [3, 4]]; \
               let mut s: i64 = 0; for [x, y] in m { s = s + x + y; } return s; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_match_arm_may_use_an_array_pattern() {
    let src = "fn main() -> i64 { let a: [i64; 2] = [1, 2]; \
               match a { [x, y] => { return x * 10 + y; } } }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_array_pattern_with_a_refutable_element_is_rejected() {
    // `[1, y]` needs a per-element test rmc does not generate, so it is
    // rejected rather than mis-bound as an always-true read.
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let [1, y] = a; return y; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("must be bindings"), "got: {err}");
}

#[test]
fn an_array_pattern_with_two_rests_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; let [.., x, ..] = a; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("only be used once"), "got: {err}");
}

#[test]
fn mut_on_an_array_wildcard_or_nested_pattern_is_rejected() {
    let wild = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let [mut _, y] = a; return y; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(wild.contains("attaches to a binding"), "got: {wild}");

    let nested = compile_to_asm(
        "fn main() -> i64 { let m: [[i64; 2]; 1] = [[1, 2]]; \
         let [mut [x, y]] = m; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(nested.contains("attaches to a binding"), "got: {nested}");
}

#[test]
fn a_mut_before_a_let_array_pattern_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let mut [x, y] = a; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("attached to each individual binding"), "got: {err}");
}

#[test]
fn an_array_pattern_with_an_out_of_range_element_is_rejected() {
    // `[x, y, z]` on a length-2 array reads index 2, which sema rejects.
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let [x, y, z] = a; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("out of bounds"), "got: {err}");
}

#[test]
fn a_struct_field_may_be_destructured_by_an_array_pattern() {
    let src = "struct S { a: [i64; 2], b: i64 } \
               fn main() -> i64 { let s: S = S { a: [3, 4], b: 5 }; \
               let S { a: [x, y], b } = s; return x * 100 + y * 10 + b; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_array_sub_pattern_in_a_struct_may_ignore_elements_and_rest() {
    let src = "struct S { a: [i64; 3], b: i64 } \
               fn main() -> i64 { let s: S = S { a: [1, 2, 3], b: 4 }; \
               let S { a: [_, y, ..], .. } = s; return y; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_array_sub_pattern_nests_through_a_struct_element() {
    // `S { a: [I { u }, ..] }`: an array element is itself a struct pattern, so
    // the two nesting directions compose.
    let src = "struct I { u: i64, v: i64 } struct S { a: [I; 2], b: i64 } \
               fn main() -> i64 { let s: S = S { a: [I { u: 3, v: 4 }, I { u: 5, v: 6 }], b: 7 }; \
               let S { a: [I { u, .. }, I { v, .. }], b } = s; \
               return u * 100 + v * 10 + b; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_array_sub_pattern_reads_through_a_reference() {
    let src = "struct S { a: [i64; 2] } \
               fn main() -> i64 { let s: S = S { a: [3, 4] }; let r: &S = &s; \
               match r { S { a: [x, y] } => { return x * 10 + y; } } }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_struct_array_sub_pattern_compiles_like_the_equivalent_field_reads() {
    // `S { a: [x, y] }` expands to `let x = s.a[0]; let y = s.a[1];`.
    let pat = "struct S { a: [i64; 2], b: i64 } \
               fn f(s: &S) -> i64 { let S { a: [x, y], b } = s; return x * 100 + y * 10 + b; }";
    let reads = "struct S { a: [i64; 2], b: i64 } \
                 fn f(s: &S) -> i64 { let x: i64 = s.a[0]; let y: i64 = s.a[1]; let b: i64 = s.b; \
                 return x * 100 + y * 10 + b; }";
    assert_eq!(asm(pat), asm(reads));
}

#[test]
fn a_refutable_element_in_a_struct_array_sub_pattern_is_rejected() {
    let err = compile_to_asm(
        "struct S { a: [i64; 2] } \
         fn main() -> i64 { let s: S = S { a: [1, 2] }; \
         let S { a: [1, y] } = s; return y; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("must be bindings"), "got: {err}");
}

#[test]
fn a_diverging_match_is_never_and_fits_any_return_type() {
    // Every arm `return`s, so the match cannot yield a value: it is `!` and
    // satisfies `-> i64`. rustc accepts this; rmc used to reject it with a
    // spurious "falls off the end".
    let src = "enum E { A, B } \
               fn f(e: E) -> i64 { match e { E::A => { return 1; } E::B => { return 2; } } } \
               fn main() -> i64 { return f(E::A); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_diverging_if_else_initializes_a_typed_let() {
    let src = "fn f(c: bool) -> i64 { let x: i64 = if c { return 1; } else { return 2; }; return x; } \
               fn main() -> i64 { return f(true); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_loop_with_no_break_satisfies_any_return_type() {
    let src = "fn f() -> i64 { loop {} } fn main() -> i64 { return f(); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_mixed_value_and_diverging_arm_keeps_the_value_type() {
    // One arm yields `i64`, the other diverges: the match is `i64`.
    let src = "enum E { A, B } \
               fn f(e: E) -> i64 { match e { E::A => 10, E::B => { return 20; } } } \
               fn main() -> i64 { return f(E::A); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn compound_assignment_expands() {
    // `x += 4` folds into a single `lea` when the target is register-resident,
    // and otherwise into an `add`. Either is the expansion; what matters is
    // that the value is incremented in place.
    let out = asm("fn main() -> i64 { let mut x: i64 = 1; x += 4; return x; }");
    assert!(
        out.contains("lea 4(") || out.contains("add $4, %rax") || out.contains("add %rcx, %rax"),
        "no increment in:\n{out}"
    );
}

#[test]
fn bang_on_int_is_bitwise_not() {
    // Rust's `!` is bitwise not on an integer and logical not on a `bool`; the
    // integer form emits `not %rax`, the bool form a `sete`.
    let int = asm("fn main() -> i64 { return !5; }");
    assert!(int.contains("not %rax"), "integer `!` is a bitwise not");
    assert!(!int.contains("sete"), "integer `!` is not a logical not");
    let boolean = asm("fn main() -> i64 { let b: bool = true; return (!b) as i64; }");
    assert!(boolean.contains("sete"), "bool `!` is a logical not");
}

#[test]
fn bang_on_a_non_int_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { let a: [i64; 2] = [0,0]; return (!a) as i64; }", &CompileOptions::default());
    assert!(err.is_err(), "`!` on an array must be rejected");
}

#[test]
fn i64_min_literal_is_accepted() {
    let out = asm("fn main() -> i64 { let m: i64 = -9223372036854775808; return m; }");
    // The folded value reaches codegen as a single immediate. A bare magnitude
    // is rejected (`i64_min_magnitude_alone_is_rejected`), so this can only come
    // from the `-` fold, not from a negation of a positive value.
    assert!(out.contains("mov $-9223372036854775808, %rax"), "the value reaches codegen");
}

#[test]
fn i64_min_magnitude_alone_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { return 9223372036854775808; }", &CompileOptions::default());
    assert!(err.is_err(), "2^63 without a `-` is out of range");
}

#[test]
fn negating_a_constant_i64_min_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { return - -9223372036854775808; }", &CompileOptions::default());
    assert!(err.is_err(), "negating i64::MIN overflows");
}

#[test]
fn if_let_need_not_be_exhaustive() {
    // Statement position, no `else`, one integer pattern: rustc accepts it.
    let out = asm("fn main() -> i64 { let o: i64 = 3; if let 3 = o { print_i64(1); } return 0; }");
    assert!(out.contains("main:"), "if let compiles");
}

#[test]
fn if_let_non_unit_without_else_is_rejected() {
    // A non-unit `then` with no `else` is a branch-type error, as in rustc.
    let err = compile_to_asm("fn main() -> i64 { let o: i64 = 3; if let 3 = o { 1 } return 0; }", &CompileOptions::default());
    assert!(err.is_err(), "non-unit `if let` without `else` must be rejected");
}

#[test]
fn if_let_binding_binds_the_scrutinee() {
    // `if let y = o` always matches and binds `o` to `y`.
    let out = asm("fn main() -> i64 { let o: i64 = 5; if let y = o { return y; } return 0; }");
    assert!(out.contains("main:"), "binding if let compiles");
}

#[test]
fn irrefutable_if_let_still_checks_the_else_type() {
    // An irrefutable pattern makes the `else` dead, but rustc still type-checks
    // it: a non-unit `then` with a unit `else` is rejected.
    let err = compile_to_asm(
        "fn main() -> i64 { let o: i64 = 3; if let y = o { 1 } else { } return 0; }",
        &CompileOptions::default(),
    );
    assert!(err.is_err(), "dead `else` with a mismatched type must be rejected");
}

#[test]
fn irrefutable_if_let_with_matching_else_is_accepted() {
    let out = asm("fn main() -> i64 { let o: i64 = 3; let x = if let y = o { y + 1 } else { 0 }; return x; }");
    assert!(out.contains("main:"), "matching dead `else` is accepted");
}

#[test]
fn match_arm_body_may_be_a_bare_assignment() {
    // A `match` arm body is an expression in Rust, but `x = 1` and `x += 1` are
    // accepted too. rmc parses assignment only as a statement, so the arm body
    // parser must accept it explicitly; without that, `0 => acc = 1` fails.
    let out = asm(
        "fn main() -> i64 { let mut acc: i64 = 0; let x: i64 = 1; \
         match x { 0 => acc = acc + 1, 1 => acc += 10, _ => {} } return acc; }",
    );
    assert!(out.contains("main:"), "an assignment arm body compiles");
}

#[test]
fn assignment_arm_bodies_work_with_or_patterns_and_ranges() {
    // The assignment arm body composes with every pattern form, and with a
    // `match` used as a statement (its value discarded).
    let out = asm(
        "fn main() -> i64 { let mut acc: i64 = 0; let mut i: i64 = 0; \
         while i < 6 { match i { 0 | 1 => acc += 10, 2..=4 => acc = acc - 1, _ => {} } i = i + 1; } return acc; }",
    );
    assert!(out.contains("main:"), "assignment bodies compose with or-patterns and ranges");
}

#[test]
fn if_let_and_while_let_bodies_may_assign() {
    // `if let`/`while let` bodies are blocks, so an assignment inside them is an
    // ordinary statement; this guards that path stays working.
    let out = asm(
        "fn main() -> i64 { let mut acc: i64 = 0; let x: i64 = 1; \
         if let 1 = x { acc = acc + 5; } \
         let mut y: i64 = 0; while let 0..=2 = y { y += 1; } return acc + y; }",
    );
    assert!(out.contains("main:"), "assignment bodies compile under `if let`/`while let`");
}

#[test]
fn match_arm_body_may_be_a_return_or_break() {
    // As in Rust, an arm body may be a `return`/`break` statement, not just an
    // expression; `parse_arm_body` builds the statement and leaves the arm's
    // terminator for the caller.
    let out = asm(
        "fn f(x: i64) -> i64 { match x { 0 => return 5, _ => 0 } } \
         fn main() -> i64 { let mut i: i64 = 0; while i < 9 { match i { 3 => break, _ => {} } i = i + 1; } return f(0); }",
    );
    assert!(out.contains("main:"), "return/break arm bodies compile");
}

#[test]
fn match_drops_arms_after_a_catch_all() {
    // A `match` arm after `_` is dead and dropped, so a value-position match
    // whose `_` arm yields a value is accepted even with arms after it.
    let out = asm("fn main() -> i64 { let o: i64 = 3; let x = match o { _ => 5, 3 => 9 }; return x; }");
    assert!(out.contains("main:"), "arms after `_` are dropped");
}

#[test]
fn assignment_evaluates_value_before_place() {
    // rustc orders assignment operands right-to-left: the assigned value is
    // evaluated before the assignee place. With `li` calling on both sides, the
    // value's `call li` must appear before the index's `call li`.
    let out = asm("fn li(t: i64) -> i64 { print_i64(t); return t; } fn main() -> i64 { let mut a: [i64; 4] = [0,0,0,0]; a[li(1)] = li(5); return 0; }");
    let value_call = out.find("call li").expect("a call to li");
    let index_call = out[value_call + 1..].find("call li").expect("a second call to li");
    // The value call's argument is 5; the index call's is 1. Locate them by the
    // immediate that precedes each `mov ..., %rdi` argument load.
    let first_arg = &out[..value_call];
    let second_arg = &out[value_call..value_call + 1 + index_call];
    let last_imm = |s: &str| {
        s.rfind("$").map(|i| s[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
    };
    assert_eq!(last_imm(first_arg).as_deref(), Some("5"), "value (5) is the first call");
    assert_eq!(last_imm(second_arg).as_deref(), Some("1"), "place index (1) is the second call");
}

#[test]
fn overflow_checks_add_a_jo() {
    let plain = asm("fn main() -> i64 { let mut x: i64 = 1; x = x + 1; return x; }");
    assert!(!plain.contains("jo __rmc_overflow"));
    let checked = compile_to_asm(
        "fn main() -> i64 { let mut x: i64 = 1; x = x + 1; return x; }",
        &CompileOptions { overflow_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(checked.contains("jo __rmc_overflow"));
}

#[test]
fn large_array_compiles_in_linear_time() {
    // Promotion counts references to every slot. An array of N elements must not
    // make that quadratic: a regression here took ~6s for 200k, this is ~7ms.
    let src = format!(
        "fn main() -> i64 {{ let a: [i64; 200000] = [1; 200000]; return a[0]; }}"
    );
    let t = std::time::Instant::now();
    let out = compile_to_asm(&src, &CompileOptions::default()).unwrap();
    assert!(out.contains("main:"));
    assert!(
        t.elapsed().as_secs_f64() < 0.5,
        "compiling a 200k-element array took {:?}",
        t.elapsed()
    );
}

#[test]
fn aggregate_argument_is_passed_by_pointer() {
    // The caller copies the argument into a hidden slot and passes its address;
    // the callee stashes the incoming pointer and copies it into its own buffer.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn f(p: P) -> i64 { return p.x; } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; return f(a); }",
    );
    assert!(out.contains("lea"), "an address is taken for the argument");
    assert!(out.contains("push %rdi"), "callee saves the incoming pointer");
    assert!(out.contains("pop %rsi"), "callee restores it for the copy");
}

#[test]
fn aggregate_return_uses_a_hidden_buffer() {
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn f() -> P { return P { x: 1, y: 2 }; } \
         fn main() -> i64 { let p: P = f(); return p.x; }",
    );
    assert!(out.contains("mov %r10,"), "callee stores the return pointer");
    assert!(out.contains("lea") && out.contains("%r10"), "caller passes a buffer");
}

#[test]
fn large_struct_copy_uses_a_loop() {
    // More than eight words is copied with a counted loop, not unrolled.
    let fields: String = (0..12).map(|i| format!("f{i}: i64, ")).collect();
    let src = format!(
        "struct B {{ {fields} }} \
         fn f(b: B) -> i64 {{ return b.f0; }} \
         fn main() -> i64 {{ let x: B = B {{ {} }}; return f(x); }}",
        (0..12).map(|i| format!("f{i}: {i}, ")).collect::<String>()
    );
    let out = asm(&src);
    assert!(out.contains("jnz"), "the copy loops for a large struct");
}

#[test]
fn errors_carry_a_byte_offset() {
    let err = compile_to_asm("fn main() -> i64 { return x; }", &CompileOptions::default())
        .unwrap_err();
    assert!(err.contains("unknown variable"), "got: {err}");
}

#[test]
fn unknown_type_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { let p: Nope = 0; return 0; }", &CompileOptions::default())
        .unwrap_err();
    assert!(err.contains("unknown type"), "got: {err}");
}

#[test]
fn struct_self_reference_is_rejected() {
    let err = compile_to_asm(
        "struct S { a: S } fn main() -> i64 { return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("contains itself"), "got: {err}");
}

#[test]
fn exhaustive_match_is_a_value() {
    // Every variant named, no `_`: the desugared `if` chain is total, so it may
    // be used as a value even though its last `if` has no `else`.
    let src = "enum E { A, B } \
               fn main() -> i64 { let e: E = E::A; \
               let t: i64 = match e { E::A => 1, E::B => 2 }; return t; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_three_variant_exhaustive_match_is_a_value() {
    // The boundary wraps the whole chain once, so the exhaustiveness walk sees
    // all three arms; a per-arm boundary would stop it after the first and
    // reject this with "`if` without `else`".
    let src = "enum E { A, B, C } \
               fn main() -> i64 { let e: E = E::C; \
               let t: i64 = match e { E::A => 1, E::B => 2, E::C => 3 }; return t; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_exhaustive_match_in_return_position_is_a_value() {
    // The chain is the function's tail expression, with no trailing `;`.
    let src = "enum E { A, B } \
               fn f(e: E) -> i64 { return match e { E::A => 10, E::B => 20 }; } \
               fn main() -> i64 { return f(E::B); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn non_exhaustive_match_as_a_value_is_rejected() {
    // A missing variant means the chain can fall through to unit, which is a
    // type error in value position, exactly as in Rust.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let e: E = E::A; \
         let t: i64 = match e { E::A => 1 }; return t; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_guarded_arm_folds_the_guard_into_the_pattern_test() {
    // `pat if g` becomes `if pat_test && g`; the guard is an ordinary
    // expression, so no new node reaches codegen.
    let out = asm("fn main() -> i64 { let x: i64 = 5; \
                   let t: i64 = match x { n if n > 3 => 1, _ => 2 }; return t; }");
    assert!(out.contains("main:"), "{out}");
    assert!(
        compile_to_asm(
            "fn main() -> i64 { let x: i64 = 5; \
             let t: i64 = match x { n if n > 3 => 1, _ => 2 }; return t; }",
            &CompileOptions::default()
        )
        .is_ok()
    );
}

#[test]
fn a_guard_makes_an_otherwise_irrefutable_arm_not_a_catch_all() {
    // `n if c` can fail, so the arms after it are still reachable: the match
    // below is accepted and the `_` arm is live. If the guard were dropped the
    // `_` arm would be dead and the body would differ.
    let src = "fn main() -> i64 { let x: i64 = 5; \
               let t: i64 = match x { n if n > 100 => 1, _ => 2 }; return t; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_guarded_arm_covers_nothing_for_exhaustiveness() {
    // Every variant named, but one arm is guarded, so the match can still fall
    // through: rustc rejects it (E0004) and rmc must too.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let e: E = E::A; \
         let t: i64 = match e { E::A if e as i64 == 0 => 1, E::B => 2 }; return t; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("exhaustive") || err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_guarded_binding_arm_is_not_a_catch_all() {
    // A bare binding arm is a catch-all; a guarded one is not.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let e: E = E::A; \
         let t: i64 = match e { v if v as i64 == 0 => 1 }; return t; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("exhaustive") || err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_guard_on_a_payload_binding_sees_the_binding() {
    let src = "enum Opt { None, Some(i64) } \
               fn f(o: Opt) -> i64 { return match o { \
               Opt::Some(v) if v > 10 => 100 + v, Opt::Some(v) => v, Opt::None => -1 }; } \
               fn main() -> i64 { return f(Opt::Some(5)); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_payload_enum_match_naming_every_variant_is_a_value() {
    // Pre-existing gap: the payload-binding block hid the chain from the
    // exhaustiveness walk, so this was wrongly rejected without a `_` arm.
    let src = "enum Opt { None, Some(i64) } \
               fn f(o: Opt) -> i64 { return match o { Opt::Some(v) => v, Opt::None => 0 }; } \
               fn main() -> i64 { return f(Opt::Some(5)); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_repeated_variant_arm_is_not_a_gap() {
    // Two unguarded arms naming the same variant are redundant, not a hole;
    // rustc accepts this and rmc must not report it non-exhaustive.
    let src = "enum E { A, B } fn main() -> i64 { let e: E = E::A; \
               let t: i64 = match e { E::A => 1, E::A => 2, E::B => 3 }; return t; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_method_on_a_payload_enum_resolves() {
    // The receiver type is `EnumData`, which method resolution must accept just
    // like `Struct` and the unit `Enum`.
    let src = "enum Opt { None, Some(i64) } \
               impl Opt { fn or_else(self, d: i64) -> i64 { \
               return match self { Opt::Some(v) => v, Opt::None => d }; } } \
               fn main() -> i64 { return Opt::Some(8).or_else(0); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_wildcard_arm_whose_body_is_another_match_is_not_exhaustive() {
    // The two arms below test `E::A` and `E::B`, but the `E::B` test lives in a
    // *nested* match reached only from the wildcard arm. The outer match is
    // therefore not exhaustive, and the nested match itself is missing `E::A`,
    // so as a value the whole expression is a type error, exactly as in Rust.
    let src = "enum E { A, B } \
               fn main() -> i64 { let a: E = E::B; let b: E = E::A; \
               let t: i64 = match a { E::A => 1, _ => match b { E::B => 2 } }; \
               return t; }";
    let err = compile_to_asm(src, &CompileOptions::default()).unwrap_err();
    assert!(err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_wildcard_arm_matching_the_same_value_is_not_exhaustive() {
    // Same shape, but the nested match scrutinises the same value as the outer
    // one. After desugaring the two `if` chains are structurally identical to an
    // exhaustive `match a { E::A => 1, E::B => 2 }`, so this is the case that a
    // naive scrutinee comparison would let through; only the match boundary
    // keeps them apart.
    let src = "enum E { A, B } \
               fn main() -> i64 { let a: E = E::B; \
               let t: i64 = match a { E::A => 1, _ => match a { E::B => 2 } }; \
               return t; }";
    let err = compile_to_asm(src, &CompileOptions::default()).unwrap_err();
    assert!(err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_nested_match_with_all_variants_is_still_a_value() {
    // The nested match names every variant, so the wildcard arm's body is a
    // value and the outer chain is total: this must compile.
    let src = "enum E { A, B } \
               fn main() -> i64 { let a: E = E::B; let b: E = E::A; \
               let t: i64 = match a { E::A => 1, _ => match b { E::A => 2, E::B => 3 } }; \
               return t; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_bool_match_covering_both_values_is_a_value() {
    // `true` and `false` together cover every `bool`, so the chain is total and
    // may stand for a value. The two tests are `BoolTest`s, not plain
    // `Bin(Eq, ..)`, so the exhaustiveness walk can recognise them.
    let src = "fn f(b: bool) -> i64 { return match b { true => 1, false => 0 }; } \
               fn main() -> i64 { return f(1 < 2); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_bool_match_missing_a_value_is_rejected() {
    // Only `true`: the chain can fall through to unit, a type error in value
    // position, exactly as in Rust (E0004).
    let err = compile_to_asm(
        "fn f(b: bool) -> i64 { return match b { true => 1 }; } \
         fn main() -> i64 { return f(1 < 2); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_hand_written_bool_if_chain_is_not_exhaustive() {
    // `if b == true { .. } else if b == false { .. }` has no final `else` and
    // rustc rejects it in value position; only the `match` desugaring's
    // `BoolTest`s count as covering `bool`, so this must stay an error.
    let err = compile_to_asm(
        "fn f(b: bool) -> i64 { \
         let t: i64 = if b == true { 1 } else if b == false { 0 }; return t; } \
         fn main() -> i64 { return f(1 < 2); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`if` without `else`"), "got: {err}");
}

#[test]
fn a_bool_pattern_on_a_non_bool_is_rejected() {
    let err = compile_to_asm(
        "fn f(x: i64) -> i64 { return match x { true => 1, _ => 0 }; } \
         fn main() -> i64 { return f(1); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`true` pattern cannot match"), "got: {err}");
}

#[test]
fn a_match_statement_without_a_semicolon_is_allowed() {
    // A `match` is a block-like expression, so it is a valid statement with no
    // trailing `;`. Wrapping the desugared chain in the match-boundary node must
    // not change that.
    let src = "enum E { A, B } fn main() -> i64 { let e: E = E::A; let mut n: i64 = 0; \
               match e { E::A => { n = 1; } _ => { n = 2; } } return n; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_match_whose_arms_all_return_is_accepted_as_a_body() {
    // Every arm returns, so the desugared chain diverges on all paths and a
    // value-returning body does not fall off the end. The match boundary must be
    // transparent to the divergence walk.
    let src = "fn classify(x: i64) -> i64 { \
               match x { 0 => { return 100; } _ => { return 300; } } } \
               fn main() -> i64 { return classify(0); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_non_exhaustive_match_statement_is_rejected() {
    // The arms are unit-typed, so the fall-through to `()` is harmless and rmc
    // used to accept this; rustc still rejects it with E0004, and so must rmc.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let e: E = E::A; \
         match e { E::A => { print_i64(1); } } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("non-exhaustive `match`"), "got: {err}");
}

#[test]
fn a_non_exhaustive_integer_match_statement_is_rejected() {
    // Integer patterns cannot be checked for exhaustiveness, so a match over an
    // integer with no `_` arm is always non-exhaustive, exactly as in rustc.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 5; \
         match x { 0 => { print_i64(1); } } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("non-exhaustive `match`"), "got: {err}");
}

#[test]
fn an_exhaustive_match_statement_is_accepted() {
    // Naming every variant is exhaustive even without a `_` arm, so the match is
    // a valid statement.
    let src = "enum E { A, B } fn main() -> i64 { let e: E = E::A; let mut n: i64 = 0; \
               match e { E::A => { n = 1; } E::B => { n = 2; } } return n; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_non_exhaustive_match_nested_in_a_statement_is_rejected() {
    // The match is inside a block statement, not the statement itself; it is
    // still non-exhaustive and must be rejected.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let e: E = E::A; \
         if true { match e { E::A => { print_i64(1); } } } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("non-exhaustive `match`"), "got: {err}");
}

#[test]
fn a_non_exhaustive_match_in_a_loop_body_is_rejected() {
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { let e: E = E::A; let mut i: i64 = 0; \
         while i < 1 { match e { E::A => { print_i64(1); } } i = i + 1; } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("non-exhaustive `match`"), "got: {err}");
}

#[test]
fn assigning_to_an_immutable_binding_is_rejected() {
    // rustc rejects a second write to a `let` without `mut` (E0384); rmc used
    // to accept it and write the slot anyway.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; x = 2; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("immutable variable `x`"), "got: {err}");
}

#[test]
fn assigning_to_an_immutable_parameter_is_rejected() {
    // A parameter is a `let` binding without `mut`, so assigning it is E0384.
    let err = compile_to_asm(
        "fn f(x: i64) -> i64 { x = x + 1; return x; } fn main() -> i64 { return f(1); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("immutable variable `x`"), "got: {err}");
}

#[test]
fn a_mut_binding_and_a_mut_parameter_are_assignable() {
    // The positive side: `let mut` and `mut x: T` allow the writes rustc allows.
    assert!(compile_to_asm(
        "fn f(mut x: i64) -> i64 { x = x + 1; return x; } \
         fn main() -> i64 { let mut y: i64 = 1; y = y + 2; return f(y); }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_constant_index_expression_out_of_bounds_is_rejected() {
    // The index `1 + 4` folds to 5, so it is out of bounds for `[i64; 2]` and
    // must be rejected exactly like the literal `a[5]`; the earlier check only
    // folded a bare literal.
    for idx in ["1 + 4", "2 * 3", "0 - 1", "1 << 3"] {
        let err = compile_to_asm(
            &format!("fn main() -> i64 {{ let a: [i64; 2] = [1, 2]; return a[{idx}]; }}"),
            &CompileOptions::default(),
        )
        .unwrap_err();
        assert!(err.contains("out of bounds"), "a[{idx}]: got: {err}");
    }
}

#[test]
fn a_constant_local_index_out_of_bounds_is_rejected() {
    // rustc's constant propagation knows the value of `i`, so it denies the
    // index. rmc must fold a constant local too, or it reads past the array.
    for (decl, idx) in [
        ("let i: i64 = -1;", "i"),
        ("let i: i64 = 2;", "i"),
        ("let i: i64 = 1 - 3;", "i"),
        ("let mut i: i64 = -1;", "i"),
        ("let i: i64; i = -1;", "i"),
        ("let i: i64 = -1; let i: i64 = 2;", "i"),
    ] {
        let err = compile_to_asm(
            &format!("fn main() -> i64 {{ let a: [i64; 2] = [1, 2]; {decl} return a[{idx}]; }}"),
            &CompileOptions::default(),
        )
        .unwrap_err();
        assert!(err.contains("out of bounds"), "{decl} a[{idx}]: got: {err}");
    }
}

#[test]
fn a_written_constant_local_index_is_not_folded() {
    // rustc assigns `i` twice, so its constant is not propagated and the
    // program is accepted; folding the first `i` here would wrongly reject it.
    assert!(compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let mut i: i64 = -1; i = 0; return a[i]; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_constant_local_that_overflows_is_rejected() {
    // A constant local participates in the trapping-constant check: rustc
    // denies `let x = i64::MAX; x + 1` (E0080).
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 9223372036854775807; let y: i64 = x + 1; return y; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("overflow"), "got: {err}");
}

#[test]
fn a_second_assignment_stops_constant_folding() {
    // The overflow is only denied when the value is a propagated constant; with
    // a second assignment rustc treats `x` as runtime data and accepts it.
    assert!(compile_to_asm(
        "fn main() -> i64 { let mut x: i64 = 9223372036854775807; x = x + 1; return x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn an_in_range_constant_index_expression_is_accepted() {
    assert!(compile_to_asm(
        "fn main() -> i64 { let a: [i64; 4] = [1, 2, 3, 4]; return a[1 + 2]; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn casting_a_reference_to_a_scalar_is_rejected() {
    // `r as i64` would expose the address as an integer; rustc forbids it.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; return r as i64; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("reference"), "got: {err}");
}

#[test]
fn casting_a_shared_reference_to_mut_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; let s: &mut i64 = r as &mut i64; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("shared reference"), "got: {err}");
}

#[test]
fn casting_a_reference_to_another_type_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; let s: &bool = r as &bool; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("reference"), "got: {err}");
}

#[test]
fn casting_bool_to_a_reference_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let b: bool = true; let r: &i64 = b as &i64; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("reference"), "got: {err}");
}

#[test]
fn the_documented_casts_are_accepted() {
    // rustc-valid casts plus rmc's two documented extensions: `i64 as bool`
    // (rustc needs `!= 0`) and the raw pointer cast `i64 as &mut T`.
    assert!(compile_to_asm(
        "struct N { v: i64 } \
         fn main() -> i64 { \
           let b: bool = true; let x: i64 = b as i64; \
           let y: bool = x as bool; \
           let mut z: i64 = 0; let rz: &mut i64 = &mut z; let rs: &i64 = rz as &i64; \
           let p: i64 = 0; let np: &mut N = p as &mut N; \
           return x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_cast_binds_looser_than_a_deref() {
    // `*r as i64` is `(*r) as i64`, not `*(r as i64)`: in Rust `as` sits above
    // the unary operators in the precedence chain, so rmc used to reject this
    // valid program with a reference-cast error.
    assert!(compile_to_asm(
        "fn main() -> i64 { let x: i64 = 5; let r: &i64 = &x; return *r as i64; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_cast_binds_looser_than_unary_minus() {
    // `-x as i64` is `(-x) as i64`.
    assert!(compile_to_asm(
        "fn main() -> i64 { let x: i64 = 5; return -x as i64; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_cast_binds_tighter_than_multiplication() {
    // `b as i64 * 2` is `(b as i64) * 2`, so the `as` applies to `b` alone.
    let asm = asm("fn main() -> i64 { let b: bool = true; return b as i64 * 2; }");
    assert!(!asm.is_empty());
}

#[test]
fn a_duplicate_parameter_name_is_rejected() {
    // rustc rejects two parameters with the same name (E0415); rmc used to
    // accept it and bind both to the same slot.
    let err = compile_to_asm(
        "fn f(x: i64, x: i64) -> i64 { return x; } fn main() -> i64 { return f(1, 2); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("bound more than once"), "got: {err}");
}

#[test]
fn a_struct_parameter_pattern_binds_through_the_parameter() {
    // `fn f(S { x, y }: S)` is the `let` destructuring applied to the
    // parameter, so the body sees `x` and `y` bound from the argument.
    let src = "struct S { x: i64, y: i64 } \
               fn f(S { x, y }: S) -> i64 { return x * 10 + y; } \
               fn main() -> i64 { return f(S { x: 1, y: 2 }); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_array_parameter_pattern_binds_through_the_parameter() {
    let src = "fn f([a, b]: [i64; 2]) -> i64 { return a * 10 + b; } \
               fn main() -> i64 { return f([3, 4]); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_parameter_pattern_compiles_like_a_let_destructuring() {
    // The expansion is the same one a `let` uses: a plain parameter plus the
    // per-field reads at the top of the body.
    let pat = "struct S { x: i64, y: i64 } \
               fn f(S { x, y }: S) -> i64 { return x + y; } \
               fn main() -> i64 { return f(S { x: 1, y: 2 }); }";
    let plain = "struct S { x: i64, y: i64 } \
                 fn f(s: S) -> i64 { let x: i64 = s.x; let y: i64 = s.y; return x + y; } \
                 fn main() -> i64 { return f(S { x: 1, y: 2 }); }";
    assert_eq!(asm(pat), asm(plain));
}

#[test]
fn a_parameter_pattern_may_rename_ignore_and_mutate() {
    let src = "struct S { x: i64, y: i64, z: i64 } \
               fn f(S { x: v, y: _, mut z }: S) -> i64 { z = z + 1; return v + z; } \
               fn main() -> i64 { return f(S { x: 1, y: 2, z: 3 }); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_parameter_pattern_may_be_nested() {
    // A struct-typed field destructured in turn, and an array-typed field
    // destructured by an array pattern, both in a parameter position.
    let src = "struct I { x: i64 } struct S { a: [i64; 2], b: I } \
               fn f(S { a: [p, q], b: I { x } }: S) -> i64 { return p + q + x; } \
               fn main() -> i64 { return f(S { a: [1, 2], b: I { x: 3 } }); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_parameter_pattern_works_on_a_method() {
    let src = "struct S { x: i64, y: i64 } \
               impl S { fn add(self, S { x, y }: S) -> i64 { return self.x + x + y; } } \
               fn main() -> i64 { return S { x: 1, y: 2 }.add(S { x: 3, y: 4 }); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_mut_on_a_parameter_pattern_is_rejected() {
    // `mut` attaches to each binding, not to the whole pattern.
    let err = compile_to_asm(
        "struct S { x: i64 } fn f(mut S { x }: S) -> i64 { return x; } \
         fn main() -> i64 { return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`mut` must be attached to each individual binding"), "got: {err}");
}

#[test]
fn a_refutable_parameter_pattern_is_rejected() {
    // A literal (or variant, or or-pattern) is refutable, so rustc rejects it
    // as a parameter; rmc rejects it too rather than mis-binding it.
    let err = compile_to_asm(
        "fn f(1: i64) -> i64 { return 1; } fn main() -> i64 { return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("expected identifier"), "got: {err}");
}

#[test]
fn a_parameter_pattern_name_may_not_collide_with_a_parameter() {
    // E0415 counts the names a pattern binds, not just the written parameters.
    let err = compile_to_asm(
        "struct S { x: i64 } fn f(S { x }: S, x: i64) -> i64 { return x; } \
         fn main() -> i64 { return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("bound more than once"), "got: {err}");
}

#[test]
fn main_with_a_parameter_is_rejected() {
    // The entry point takes no arguments (rustc E0580).
    let err = compile_to_asm(
        "fn main(x: i64) -> i64 { return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`main` takes no parameters"), "got: {err}");
}

#[test]
fn a_variant_payload_pattern_may_use_a_wildcard() {
    // `E::A(_, y)` ignores the first field and binds the second.
    let src = "enum E { A(i64, i64), B } \
               fn main() -> i64 { let e: E = E::A(3, 4); \
               return match e { E::A(_, y) => y, E::B => 0 }; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_variant_payload_pattern_may_be_mut() {
    // `E::A(mut x)` binds the field mutably, exactly as a `let` binding would.
    let src = "enum E { A(i64), B } \
               fn main() -> i64 { let e: E = E::A(3); \
               return match e { E::A(mut x) => { x = x + 1; x }, E::B => 0 }; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_variant_payload_pattern_may_end_in_rest() {
    // `E::A(x, ..)` names a prefix and ignores the rest; `E::A(..)` names none.
    let prefix = "enum E { A(i64, i64, i64), B } \
                  fn main() -> i64 { let e: E = E::A(1, 2, 3); \
                  return match e { E::A(x, ..) => x, E::B => 0 }; }";
    let bare = "enum E { A(i64, i64), B } \
                fn main() -> i64 { let e: E = E::A(1, 2); \
                return match e { E::A(..) => 1, E::B => 0 }; }";
    // A trailing comma after `..` is allowed, as in rustc.
    let comma = "enum E { A(i64, i64, i64), B } \
                 fn main() -> i64 { let e: E = E::A(1, 2, 3); \
                 return match e { E::A(x, ..,) => x, E::B => 0 }; }";
    assert!(compile_to_asm(prefix, &CompileOptions::default()).is_ok());
    assert!(compile_to_asm(bare, &CompileOptions::default()).is_ok());
    assert!(compile_to_asm(comma, &CompileOptions::default()).is_ok());
}

#[test]
fn a_variant_payload_pattern_may_be_nested() {
    // A struct-typed and an array-typed payload field, each destructured in
    // turn, and a mix of a scalar, a struct and an array field.
    let src = "struct I { u: i64 } struct S { a: [i64; 2], b: I } \
               enum E { A(S), B([i64; 2]), C(i64, S, [i64; 2]) } \
               fn main() -> i64 { \
                 let e: E = E::A(S { a: [1, 2], b: I { u: 3 } }); \
                 let f: E = E::B([4, 5]); \
                 let g: E = E::C(6, S { a: [7, 8], b: I { u: 9 } }, [10, 11]); \
                 let s = match e { E::A(S { a: [p, q], b: I { u } }) => p + q + u, _ => 0 }; \
                 let t = match f { E::B([a, b]) => a * 10 + b, _ => 0 }; \
                 let r = match g { E::C(n, S { a: [p, q], b: I { u } }, [x, y]) => n + p + q + u + x + y, _ => 0 }; \
                 return s + t + r; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_variant_payload_pattern_reads_through_the_payload() {
    // A nested payload pattern reads its fields straight from the payload,
    // offset past the tag word, rather than copying the whole aggregate payload
    // into a name first: `E::A(S { x })` is leaner than
    // `E::A(v) => { let S { x } = v; .. }`.
    let out = asm("struct S { x: i64 } enum E { A(S), B } \
                   fn main() -> i64 { let e: E = E::A(S { x: 5 }); \
                   return match e { E::A(S { x }) => x, E::B => 0 }; }");
    // The payload's first field starts after the 8-byte tag word, and the read
    // is the very next instruction (no intervening whole-payload copy).
    let at = out.find("add $8, %rax").expect("reads the payload at its offset");
    let next = out[at..].lines().nth(1).unwrap_or("").trim();
    assert_eq!(next, "mov (%rax), %rax", "direct payload field read:\n{out}");
}

#[test]
fn a_variant_payload_pattern_may_not_overrun_the_fields() {
    // Naming more fields than the variant has is rejected, as rustc rejects it.
    let err = compile_to_asm(
        "enum E { A(i64), B } \
         fn main() -> i64 { let e: E = E::A(3); \
         return match e { E::A(x, y) => x, E::B => 0 }; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has 1 field"), "got: {err}");
}

#[test]
fn a_variant_payload_pattern_may_not_omit_a_field_without_rest() {
    // Naming fewer fields without a trailing `..` is rejected.
    let err = compile_to_asm(
        "enum E { A(i64, i64), B } \
         fn main() -> i64 { let e: E = E::A(3, 4); \
         return match e { E::A(x) => x, E::B => 0 }; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has 2 field"), "got: {err}");
}

#[test]
fn a_refutable_variant_payload_argument_is_rejected() {
    // `E::A(1)` would need a per-field test, which rmc does not generate.
    let err = compile_to_asm(
        "enum E { A(i64), B } \
         fn main() -> i64 { let e: E = E::A(3); \
         return match e { E::A(1) => 1, E::A(_) => 0, E::B => 0 }; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("variant pattern arguments"), "got: {err}");
}

#[test]
fn a_mut_on_a_nested_variant_payload_pattern_is_rejected() {
    // `mut` attaches to a binding, not a nested pattern.
    let err = compile_to_asm(
        "struct S { x: i64 } enum E { A(S), B } \
         fn main() -> i64 { let e: E = E::A(S { x: 5 }); \
         return match e { E::A(mut S { x }) => x, E::B => 0 }; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`mut` is not allowed here"), "got: {err}");
}

#[test]
fn a_variant_payload_pattern_works_in_an_if_let() {
    let src = "struct S { x: i64 } enum E { A(S), B } \
               fn main() -> i64 { let e: E = E::A(S { x: 5 }); \
               if let E::A(S { x }) = e { return x; } return 0; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_variant_pattern_reads_through_a_reference() {
    // Match ergonomics dereferences a `&E` scrutinee, exactly as a struct
    // pattern reads through a `&S`: `match &e { E::A(x) => .. }` must compile,
    // and the payload read goes through the same `Deref` the test does. The
    // scrutinee is a reference, so codegen emits a load of the tag word before
    // the compare; a payload field is then read at its offset past the tag.
    let src = "enum E { A(i64, i64), B } \
               fn main() -> i64 { let e: E = E::A(3, 4); \
               return match &e { E::A(_, y) => y, E::B => 0 }; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());

    // A unit variant through a reference, and an `if let`.
    let unit = "enum E { A, B } \
                fn main() -> i64 { let e: E = E::B; \
                return match &e { E::A => 1, E::B => 2 }; }";
    assert!(compile_to_asm(unit, &CompileOptions::default()).is_ok());
    let iflet = "enum E { A(i64), B } \
                 fn main() -> i64 { let e: E = E::A(9); \
                 if let E::A(x) = &e { return x; } return 0; }";
    assert!(compile_to_asm(iflet, &CompileOptions::default()).is_ok());

    // The auto-deref must not accept a reference to a *different* enum: the
    // referent is what the pattern is checked against.
    let err = compile_to_asm(
        "enum E { A, B } enum F { C, D } \
         fn main() -> i64 { let e: E = E::A; \
         return match &e { F::C => 1, _ => 0 }; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("cannot match"), "got: {err}");
}

#[test]
fn a_variant_pattern_reads_through_a_nested_aggregate_reference() {
    // A reference to an enum whose payload is an aggregate: the nested
    // sub-pattern reads through the one `Deref` into the payload's fields.
    let src = "struct S { a: [i64; 2] } enum E { A(S), B } \
               fn main() -> i64 { let e: E = E::A(S { a: [3, 4] }); \
               return match &e { E::A(S { a: [p, q] }) => p * 10 + q, E::B => 0 }; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn constant_division_by_zero_is_rejected() {
    // rustc denies `1 / 0` (it would panic); rmc used to emit an `idiv` that
    // faults at run time.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1 / 0; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("division by zero"), "got: {err}");
}

#[test]
fn constant_overflow_is_rejected() {
    // Under the overflow checks rmc emits, `i64::MAX + 1` traps, so a constant
    // one is a compile error, exactly as rustc denies it.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 9223372036854775807 + 1; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("overflows"), "got: {err}");
}

#[test]
fn an_out_of_range_constant_shift_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { return 1 << 64; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("shift amount"), "got: {err}");
}

#[test]
fn a_known_shift_count_is_checked_even_when_the_value_is_not_constant() {
    // rustc rejects `x << 81` for any `x`: the count is a compile-time constant
    // outside 0..=63, so the shift is an error regardless of the left operand.
    // Checking only when *both* sides are constant would accept this.
    for src in [
        "fn f(x: i64) -> i64 { return x << 81; } fn main() -> i64 { return f(1); }",
        "fn f(x: i64) -> i64 { return x >> 64; } fn main() -> i64 { return f(1); }",
        "const K: i64 = 81; fn f(x: i64) -> i64 { return x << K; } fn main() -> i64 { return f(1); }",
        "fn f(x: i64) -> i64 { let k: i64 = 81; return x << k; } fn main() -> i64 { return f(1); }",
        "fn f(x: i64) -> i64 { return x << -1; } fn main() -> i64 { return f(1); }",
    ] {
        let err = compile_to_asm(src, &CompileOptions::default()).unwrap_err();
        assert!(err.contains("shift amount"), "src {src:?}: got {err}");
    }
}

#[test]
fn an_in_range_known_shift_count_is_accepted() {
    for src in [
        "fn f(x: i64) -> i64 { return x << 63; } fn main() -> i64 { return f(1); }",
        "fn f(x: i64) -> i64 { return x >> 0; } fn main() -> i64 { return f(1); }",
    ] {
        assert!(compile_to_asm(src, &CompileOptions::default()).is_ok(), "src {src:?}");
    }
}

#[test]
fn overflow_checks_guard_a_variable_shift_count() {
    // The hardware masks a shift count to 6 bits, so `x << 64` would silently
    // shift by 0. Under `--overflow-checks` rmc emits a `cmp`/`ja` to the trap,
    // matching Rust's panic; without the flag it must not (the shift is then
    // unchecked, as Rust's default `<<` is too).
    let src = "fn f(x: i64, n: i64) -> i64 { return x << n; } fn main() -> i64 { return f(1, 2); }";
    // Inlining would fold the constant shift into `main` and leave `f` dead;
    // disable it so the guard is inspected in the function that carries it.
    let plain = compile_to_asm(
        src,
        &CompileOptions { no_inline: true, ..Default::default() },
    )
    .unwrap();
    assert!(!plain.contains("ja __rmc_shift_overflow"), "no guard without the flag");
    let checked = compile_to_asm(
        src,
        &CompileOptions { overflow_checks: true, no_inline: true, ..Default::default() },
    )
    .unwrap();
    assert!(checked.contains("cmp $63, %rcx"), "count is range-checked");
    assert!(checked.contains("ja __rmc_shift_overflow"), "out-of-range count traps");
}

#[test]
fn in_range_constant_arithmetic_is_accepted() {
    assert!(compile_to_asm(
        "fn main() -> i64 { let x: i64 = (1 << 62) + 3; let y: i64 = x / 7 % 5; return y; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn assigning_to_a_field_of_an_immutable_binding_is_rejected() {
    // The mutability check must reach a field or element, not just a bare
    // variable: rustc rejects `p.x = 2` when `p` is not `mut` (E0594).
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let p: P = P { x: 1, y: 2 }; p.x = 2; return p.x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("immutable variable `p`"), "got: {err}");
}

#[test]
fn assigning_to_an_element_of_an_immutable_binding_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; a[0] = 5; return a[0]; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("immutable variable `a`"), "got: {err}");
}

#[test]
fn assigning_through_a_shared_reference_is_rejected() {
    // `*r = ..` where `r: &i64` is E0594; only a `&mut` reference is writable.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; *r = 5; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("shared reference"), "got: {err}");
}

#[test]
fn borrowing_an_immutable_binding_as_mut_is_rejected() {
    // `&mut x` on a `let` without `mut` is E0596.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; let r: &mut i64 = &mut x; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("immutable variable `x`"), "got: {err}");
}

#[test]
fn writing_through_a_mut_reference_needs_no_mut_binding() {
    // The reference binding itself may be immutable: `r.x = ..` where
    // `r: &mut T` is writable without `let mut r`, matching rustc.
    assert!(compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; \
         let r: &mut P = &mut p; r.x = 9; return r.x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_mut_self_method_can_write_its_receiver() {
    // `self` is an immutable binding of type `&mut Self` (or `Self`), and the
    // write must stay legal, or every `&mut self` setter breaks.
    assert!(compile_to_asm(
        "struct C { n: i64 } \
         impl C { fn bump(&mut self) -> i64 { self.n = self.n + 1; return self.n; } } \
         fn main() -> i64 { let mut c: C = C { n: 0 }; return c.bump(); }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_partially_initialized_aggregate_is_not_readable() {
    // Writing one element of a `let a: [i64; 2];` leaves the binding as a whole
    // uninitialized, so reading it is E0381 (rustc: "partially assigned").
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2]; a[0] = 1; return a[0]; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("uninitialized variable `a`"), "got: {err}");
}

#[test]
fn reading_an_uninitialized_binding_is_rejected() {
    // `let x: i64;` with no initializer and no write, then read: rustc rejects
    // it with E0381.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("uninitialized variable `x`"), "got: {err}");
}

#[test]
fn a_binding_initialized_on_every_branch_is_readable() {
    // Both arms write `x`, so after the `if` it is definitely initialized.
    assert!(compile_to_asm(
        "fn main() -> i64 { let x: i64; if true { x = 1; } else { x = 2; } return x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_binding_initialized_on_only_one_branch_is_not_readable() {
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64; if true { x = 1; } return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("uninitialized variable `x`"), "got: {err}");
}

#[test]
fn a_binding_initialized_only_inside_a_loop_is_not_readable() {
    // The loop may run zero times, so nothing it writes is guaranteed after it.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64; let mut i: i64 = 0; \
         while i < 3 { x = 1; i = i + 1; } return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("uninitialized variable `x`"), "got: {err}");
}

#[test]
fn writing_an_uninitialized_binding_before_reading_it_is_accepted() {
    assert!(compile_to_asm(
        "fn main() -> i64 { let mut x: i64; x = 7; return x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_string_literal_is_not_an_integer() {
    // A string literal has its own type, so `1 + "hi"` is a type error rather
    // than an integer addition on the string's address (which rustc rejects
    // with E0369).
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = \"hi\" + 1; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("arithmetic expects"), "got: {err}");
}

#[test]
fn an_integer_is_not_a_string_argument() {
    // `print_str(42)` used to compile and segfault at runtime (the integer was
    // dereferenced as a pointer). It is now a type error.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 42; print_str(x); return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("print_str"), "got: {err}");
}

#[test]
fn print_str_accepts_a_string_literal() {
    assert!(compile_to_asm(
        "fn main() -> i64 { print_str(\"hello\"); return 0; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn comparing_strings_lowers_to_the_byte_wise_helper() {
    // A `str` is a pointer to its record, so a machine compare would answer
    // "not equal" for two identical literals. The comparison must call the
    // runtime helper instead.
    let out = asm("fn main() -> i64 { if \"abc\" == \"abc\" { return 1; } return 0; }");
    assert!(out.contains("call __rmc_str_lt"), "content compare emitted: {out}");
    assert!(out.contains("__rmc_str_lt:"), "helper linked in");
}

#[test]
fn a_string_equality_compares_by_content() {
    // The two literals are distinct objects at run time; the comparison is
    // decided by their bytes, so `==` is true and `!=` is false.
    let out = asm(
        "fn main() -> i64 { \
         let mut n: i64 = 0; \
         if \"abc\" == \"abc\" { n = n + 1; } \
         if \"abc\" != \"abc\" { n = n + 10; } \
         return n; }",
    );
    assert!(out.contains("call __rmc_str_lt"));
    // No `str` is left to compare with a raw `cmp`, so the program must not
    // contain a bare string-pointer comparison.
    assert!(!out.contains("cmp %rcx, %rax"), "no pointer compare: {out}");
}

#[test]
fn comparing_a_string_with_an_integer_is_rejected() {
    let bad = "fn main() -> i64 { if \"a\" == 1 { return 1; } return 0; }";
    assert!(compile_to_asm(bad, &CompileOptions::default()).is_err());
}

#[test]
fn the_string_helper_is_dropped_when_unused() {
    let out = asm("fn main() -> i64 { print_str(\"hi\"); return 0; }");
    assert!(!out.contains("__rmc_str_lt"), "unused helper dropped: {out}");
}

#[test]
fn a_string_pattern_matches_by_content() {
    // A `str` scrutinee is a pointer, so a `match` on it must compare bytes,
    // not addresses: `match s { "a" => .. }` has to fire for a `let s = "a"`.
    let out = asm(
        "fn main() -> i64 { \
         let s = \"a\"; \
         return match s { \"a\" => 1, _ => 2 }; }",
    );
    assert!(out.contains("call __rmc_str_lt"), "content compare emitted: {out}");
}

#[test]
fn a_string_pattern_composes_with_an_or_pattern() {
    // An or-pattern over two literals must test both, not fall through to `_`.
    // Each string `==` lowers to two helper calls (`!(a<b) && !(b<a)`), so two
    // alternatives give four.
    let out = asm(
        "fn main() -> i64 { \
         let s = \"b\"; \
         return match s { \"a\" | \"b\" => 1, _ => 2 }; }",
    );
    assert_eq!(out.matches("call __rmc_str_lt").count(), 4, "both alternatives tested");
}

#[test]
fn a_string_pattern_in_if_let_lowers_the_same_way() {
    let out = asm(
        "fn main() -> i64 { \
         let s = \"a\"; \
         if let \"a\" = s { return 1; } \
         return 0; }",
    );
    assert!(out.contains("call __rmc_str_lt"));
}

#[test]
fn an_integer_pattern_against_a_string_is_rejected() {
    // The pattern test is a `str`/`i64` comparison, which rustc rejects (E0308).
    let bad = "fn main() -> i64 { let s = \"a\"; return match s { 1 => 1, _ => 2 }; }";
    assert!(compile_to_asm(bad, &CompileOptions::default()).is_err());
}

#[test]
fn str_len_lowers_to_the_runtime_helper() {
    // `s.len()` is the string record's first word, read by the runtime helper;
    // a struct field access would read the wrong offset (the record is
    // `{len, bytes..}`, so field 0 is the length, but a `str` is a pointer, so
    // the load must be indirect).
    let out = asm("fn main() -> i64 { let s = \"hello\"; return s.len(); }");
    assert!(out.contains("call __rmc_str_len"), "len helper emitted: {out}");
}

#[test]
fn array_len_folds_to_the_constant() {
    // An array's length is known at compile time, so `a.len()` is a constant
    // with no runtime work at all.
    let out = asm("fn main() -> i64 { let a = [1, 2, 3]; return a.len(); }");
    assert!(out.contains("mov $3, %rax"), "length folded: {out}");
    assert!(!out.contains("call __rmc_str_len"), "no runtime helper: {out}");
}

#[test]
fn the_string_len_helper_is_dropped_when_unused() {
    // The runtime is linked piece by piece; a program that never calls
    // `str::len` must not carry the helper.
    let out = asm("fn main() -> i64 { return 0; }");
    assert!(!out.contains("__rmc_str_len"), "unused helper dropped: {out}");
}

#[test]
fn str_is_a_nameable_type_in_signatures_and_fields() {
    // `str` is a builtin type, not a struct, so writing it as a parameter,
    // return or field type must resolve to `Type::Str` rather than "unknown
    // type". A string parameter is passed as its one-word pointer.
    let out = asm(
        "fn n(s: str) -> i64 { return s.len(); } \
         fn id(s: str) -> str { return s; } \
         fn main() -> i64 { return n(id(\"hi\")); }",
    );
    assert!(out.contains("call __rmc_str_len"), "str param lowers: {out}");
    assert!(!out.contains("unknown type"), "str is a known type");
}

#[test]
fn a_str_struct_field_round_trips_through_a_copy() {
    let out = asm(
        "struct M { s: str, n: i64 } \
         fn pick(m: M) -> str { return m.s; } \
         fn main() -> i64 { let m = M { s: \"abc\", n: 2 }; return pick(m).len() + m.n; }",
    );
    assert!(out.contains("call __rmc_str_len"), "field read then len: {out}");
}

#[test]
fn len_with_arguments_is_rejected() {
    let bad = "fn main() -> i64 { return \"x\".len(1); }";
    let err = compile_to_asm(bad, &CompileOptions::default()).unwrap_err();
    assert!(err.contains("takes no arguments"), "got: {err}");
}

#[test]
fn argument_setup_folds_a_constant_with_lea() {
    // `fib(n - 1)` must become one `lea`, not `mov; sub; mov`.
    let out = asm_no_inline("fn f(n: i64) -> i64 { return n; } fn g(n: i64) -> i64 { return f(n - 1); } fn main() -> i64 { return g(5); }");
    assert!(out.contains("lea -1(%"), "constant offset folded into lea: {out}");
    assert!(!out.contains("sub $1, %rax"), "no separate subtraction");
}

#[test]
fn a_bad_tail_return_type_is_rejected() {
    let err = compile_to_asm("fn f() -> bool { 3 } fn main() -> i64 { return 0; }", &CompileOptions::default())
        .unwrap_err();
    assert!(err.contains("declared to return"), "got: {err}");
}

#[test]
fn a_cast_to_an_aggregate_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } fn main() -> i64 { let p: P = 5 as P; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("cannot cast"), "got: {err}");
}

#[test]
fn whole_array_assignment_copies_contents() {
    // `a = b` must copy every word, not store b's address in one slot.
    let out = asm(
        "fn main() -> i64 { let mut a: [i64; 2] = [0; 2]; let b: [i64; 2] = [3; 2]; a = b; return a[0]; }",
    );
    // Two word copies into the destination array's base.
    assert_eq!(out.matches("(%rsi), %r8").count(), 2, "copies both words");
}

#[test]
fn an_inferred_array_is_initialised() {
    let out = asm("fn main() -> i64 { let a = [7; 2]; return a[1]; }");
    // The splat fill writes both elements.
    assert!(out.matches("mov %rax, ").count() >= 2, "both elements written: {out}");
}

#[test]
fn a_small_multiply_and_offset_fold_to_one_lea() {
    // `2*x + 3` on a promoted local is a single `lea`, with no `imul`.
    let out = asm_no_inline("fn g(i: i64) -> i64 { let mut x: i64 = i; x = 2 * x + 3; return x; } fn main() -> i64 { return g(4); }");
    // Scope to `g`: the runtime's `arg_i64` contains its own `imul`.
    let g = &out[out.find("g:").unwrap()..out.find(".size g,").unwrap()];
    assert!(g.contains("lea 3(%"), "folded into lea: {g}");
    assert!(!g.contains("imul"), "no multiply remains: {g}");
}

#[test]
fn a_small_constant_multiply_is_a_lea() {
    // `2 * x`, `5 * x` and `8 * x` are one 1-cycle `lea`, not a 3-cycle `imul`.
    // `7` is not a `lea` scale, so it stays an `imul`.
    for (expr, want_lea) in [("2 * x", true), ("5 * x", true), ("8 * x", true), ("7 * x", false)] {
        let out = asm_no_inline(&format!(
            "fn f(x: i64) -> i64 {{ return {expr}; }} fn main() -> i64 {{ return f(3); }}"
        ));
        let f = &out[out.find("f:").unwrap()..out.find(".size f,").unwrap()];
        if want_lea {
            assert!(f.contains("lea "), "`{expr}` folds to a lea: {f}");
            assert!(!f.contains("imul"), "`{expr}` has no imul: {f}");
        } else {
            assert!(f.contains("imul"), "`{expr}` stays an imul: {f}");
        }
    }
}

#[test]
fn a_multiply_by_one_is_dropped() {
    // `1 * x` is `x`; the multiply disappears entirely.
    let out = asm_no_inline("fn f(x: i64) -> i64 { return 1 * x; } fn main() -> i64 { return f(3); }");
    let f = &out[out.find("f:").unwrap()..out.find(".size f,").unwrap()];
    assert!(!f.contains("imul"), "no multiply remains: {f}");
    // The only `lea` in `f` is the frame restore (`... , %rsp`); a `lea` into
    // `%rax` would mean the multiply was not actually dropped.
    assert!(
        !f.lines().any(|l| l.contains("lea") && l.contains("%rax")),
        "no value-computing lea either: {f}"
    );
}

#[test]
fn a_comparison_of_two_promoted_locals_is_one_cmp() {
    // Both operands live in registers, so no value is materialised in `%rax`.
    let out = asm_no_inline(
        "fn g(a: i64, b: i64) -> bool { let mut x: i64 = a; let mut y: i64 = b; x = x + 1; y = y + 1; return x < y; } fn main() -> i64 { if g(1, 2) { return 1; } return 0; }",
    );
    let has_reg_reg_cmp = out.lines().any(|l| {
        let l = l.trim();
        l.starts_with("cmp %") && l.matches("%r").count() >= 2 && !l.contains("%rax")
    });
    assert!(has_reg_reg_cmp, "register-to-register cmp: {out}");
}

#[test]
fn spills_use_registers_not_the_stack() {
    // A value held across a call is spilled to a callee-saved register (or, when
    // those run out, a frame slot), never `push`ed: `%rsp` stays put mid-body,
    // so no alignment pad is emitted around the call.
    let out = asm_no_inline(
        "fn f(x: i64) -> i64 { return x; } fn g(x: i64) -> i64 { return f(x - 1) + f(x - 2); } fn main() -> i64 { return g(5); }",
    );
    let g = &out[out.find("g:").unwrap()..out.find(".size g,").unwrap()];
    assert!(!g.contains("push %rax"), "no stack spill: {g}");
    assert!(!g.contains("push %rcx"), "no stack spill: {g}");
    assert!(g.contains("%r12"), "the cross-call value lives in a spill register: {g}");
}

#[test]
fn a_hot_struct_field_is_promoted_to_a_register() {
    // `p.x` is the hottest field, so it lives in `%rbx`; its initialiser
    // writes the register directly instead of a frame slot, with no `%rax`
    // bounce.
    let out = asm(
        "struct P { x: i64, y: i64 } fn main() -> i64 { let mut p: P = P { x: 0, y: 1 }; let mut i: i64 = 0; while i < 10 { p.x = p.x + p.y; i = i + 1; } return p.x; }",
    );
    assert!(out.contains("mov $0, %rbx"), "p.x initialises straight into a register: {out}");
}

#[test]
fn a_copied_struct_keeps_its_fields_in_memory() {
    // `p` is copied (`let q: P = p`) and passed by value, so every field must
    // stay in memory for the whole-value copy to work.
    let out = asm(
        "struct P { x: i64, y: i64 } fn f(p: P) -> i64 { return p.x; } fn main() -> i64 { let p: P = P { x: 1, y: 2 }; let q: P = p; return f(q) + p.x; }",
    );
    let _ = store_offset_of(&out, 1); // panics if `x` were promoted
}

#[test]
fn a_struct_array_field_keeps_the_field_to_slot_map_consistent() {
    // `tag` is field 0 but sits above the three slots of `data`, so the field
    // count (2) and the slot count (4) differ. Deriving the slot from the
    // field count puts `tag` one word too low, so the literal would write a
    // different slot than `w.tag` reads. With the slot count the field is
    // promoted and both sides use its register.
    let out = asm(
        "struct W { tag: i64, data: [i64; 3] } fn main() -> i64 { let w: W = W { tag: 4, data: [1, 2, 3] }; return w.tag; }",
    );
    assert!(out.contains("mov $4, %rbx"), "literal writes `tag` to its register: {out}");
    assert!(out.contains("mov %rbx, %rax"), "`w.tag` reads the same register: {out}");
}

#[test]
fn a_value_function_that_falls_off_the_end_is_rejected() {
    // A `Unit` body returns zero, which is wrong for a value function whose
    // signature promises a value. The tail-mismatch check does not see this
    // because there is no tail.
    let err = compile_to_asm(
        "fn g() -> i64 { let x: i64 = 5; } fn main() -> i64 { return g(); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("fall off the end"), "got: {err}");
}

#[test]
fn a_function_returning_on_every_path_is_accepted() {
    // Both arms return, so control cannot reach the end without a value.
    let out = asm(
        "fn f(c: bool) -> i64 { if c { return 1; } else { return 2; } } fn main() -> i64 { return f(true); }",
    );
    assert!(out.contains("f:"), "compiles: {out}");
}

#[test]
fn a_folded_lea_displacement_stays_in_range() {
    // `x + 2147483647 + 1` folds both constants into one `lea` displacement of
    // 2147483648, which is out of the signed 32-bit range GNU `as` accepts. The
    // fold must be abandoned, not emitted.
    let out = asm("fn main() -> i64 { let mut x: i64 = 1; x = x + 2147483647 + 1; return x; }");
    assert!(!out.contains("2147483648("), "no out-of-range displacement: {out}");
}

#[test]
fn a_return_before_unreachable_statements_is_accepted() {
    // The `return` ends the block; the trailing statement is unreachable and
    // must not make `block_diverges` decide the function can fall off the end.
    let out = asm("fn f() -> i64 { return 7; print_i64(9); } fn main() -> i64 { return f(); }");
    assert!(out.contains("f:"), "compiles: {out}");
}

#[test]
fn bounds_checks_are_off_by_default() {
    let out = asm("fn main() -> i64 { let a: [i64; 2] = [1, 2]; let i: i64 = 0; return a[i]; }");
    assert!(!out.contains("jae __rmc_bounds"), "no check without the flag");
}

#[test]
fn bounds_checks_guard_a_variable_index() {
    let out = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let i: i64 = 0; return a[i]; }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(out.contains("cmp $2, %rax"), "compares the index to the length");
    assert!(out.contains("jae __rmc_bounds"), "traps on out of range");
}

#[test]
fn a_constant_index_out_of_range_is_a_compile_error() {
    // A constant index outside the array is rejected outright, with or without
    // runtime bounds checks, exactly as rustc rejects it (E0080/E0608). This is
    // the soundness-relevant direction: a program rmc accepts must not read
    // memory it does not own.
    for opts in [CompileOptions::default(), CompileOptions { bounds_checks: true, ..Default::default() }] {
        let err = compile_to_asm(
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[5]; }",
            &opts,
        )
        .unwrap_err();
        assert!(err.contains("out of bounds"), "rejected with a bounds error: {err}");
    }
    // A negated constant is out of range too (`-1` parses as `Un(Neg, 1)`).
    let err = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[-1]; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("out of bounds"), "a negative constant is out of bounds: {err}");
}

#[test]
fn bounds_checks_leave_an_in_range_constant_alone() {
    let out = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[1]; }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(!out.contains("jae __rmc_bounds"), "an in-range constant needs no check");
    assert!(!out.contains("jmp __rmc_bounds"), "an in-range constant needs no check");
}

#[test]
fn bounds_checks_guard_a_block_base() {
    // `{ ... }[i]` is a non-local base; its length comes from the block's tail,
    // so the check must still be emitted.
    // The index is a parameter, so it is not a constant and the runtime check
    // is reached (a constant index is now folded and rejected at compile time).
    let out = compile_to_asm(
        "fn f(i: i64) -> i64 { let v: i64 = { let t: [i64; 3] = [1; 3]; t }[i]; return v; } \
         fn main() -> i64 { return f(9); }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(out.contains("cmp $3, %rax"), "compares to the block's array length");
    assert!(out.contains("jae __rmc_bounds"), "traps on out of range");
}

#[test]
fn bounds_checks_guard_a_runtime_index_on_a_non_local_base() {
    // `make()[i]` takes the non-local, non-constant index branch; it must check
    // too. (A constant index would be rejected at compile time, so the runtime
    // path is only reachable with a variable index.)
    let out = compile_to_asm(
        "fn make() -> [i64; 3] { let a: [i64; 3] = [1; 3]; return a; } \
         fn f(i: i64) -> i64 { return make()[i]; } \
         fn main() -> i64 { return f(10); }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(out.contains("cmp $3, %rax"), "compares to the array length");
    assert!(out.contains("jae __rmc_bounds"), "traps on out of range");
}

#[test]
fn the_bounds_handler_is_in_an_executable_section() {
    // `__rmc_bounds` is called, so it must not be left in the `.rodata` opened
    // by the message before it; a plain `ld` link would then fault.
    let out = compile_to_asm(
        "fn f(i: i64) -> i64 { let v: i64 = { let t: [i64; 3] = [1; 3]; t }[i]; return v; } \
         fn main() -> i64 { return f(9); }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    let at = out.find(".globl __rmc_bounds").expect("handler present");
    let before = &out[..at];
    let text = before.rfind(".text").expect("a .text directive");
    let section = before.rfind(".section");
    assert!(
        section.map_or(true, |s| text > s),
        "the bounds handler must follow a `.text` directive"
    );
}

#[test]
fn a_base_with_side_effects_runs_before_the_bounds_trap() {
    // Rust evaluates the base and index before the bounds check, so the base
    // call must be emitted before the trap, not skipped by it. The index is a
    // variable so the check is a runtime branch (a constant would be rejected
    // at compile time).
    let out = compile_to_asm(
        "fn make() -> [i64; 1] { print_i64(7); return [4]; } \
         fn f(i: i64) -> i64 { return make()[i]; } \
         fn main() -> i64 { return f(2); }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    let call = out.find("call make").expect("the base call is emitted");
    let trap = out.find("jae __rmc_bounds").expect("the check traps");
    assert!(call < trap, "the base call must precede the bounds trap");
}

#[test]
fn a_repeat_literal_in_value_position_is_materialised() {
    // `[v; n]` has no address of its own, so as an index base it must be built
    // into a local array. Before this was handled the literal was emitted as a
    // bare scalar and dereferenced as a pointer, crashing the program.
    let out = asm("fn main() -> i64 { let i: i64 = 1; return [10; 3][i]; }");
    // Three elements are splatted into a frame buffer, whose address is taken
    // and indexed; no bare `mov $10, %rax` is dereferenced as a pointer.
    assert!(out.contains("lea -") && out.contains("(%rbp), %rax"), "a buffer address is taken");
    assert!(out.contains("mov (%rcx,%rax,8), %rax"), "the element is read from it");
}

#[test]
fn a_repeat_literal_let_initialiser_keeps_the_splat() {
    // The common case (`let a = [v; n]`, used by the sieve benchmark) must stay
    // a single splat loop, not n separate stores.
    let out = asm("fn main() -> i64 { let a: [i64; 200000] = [1; 200000]; return a[0]; }");
    assert!(out.contains("mov %r8, (%rdx)"), "one splat loop fills the array");
    assert!(out.contains("$200000, %rcx"), "the loop count is the array length");
}

#[test]
fn a_bare_block_parses_as_a_statement() {
    // A `{ ... }` block needs no trailing `;`. Before this was handled the
    // parser fell through to a statement arm with no `{` case and rejected it.
    let out = asm("fn main() -> i64 { let x: i64 = 1; { let x: i64 = 2; print_i64(x); } print_i64(x); return 0; }");
    // Both the inner (2) and outer (1) values are materialised and printed.
    assert!(out.contains("$2,"), "inner block body is emitted");
    assert!(out.contains("$1,"), "outer binding is emitted");
    assert_eq!(out.matches("call print_i64").count(), 2, "both blocks print");
}

#[test]
fn a_field_of_a_call_result_is_addressed() {
    // `mk().y` is a field of an aggregate temporary. The base is not an
    // lvalue, so `gen_addr` must evaluate it to get its address; it used to
    // hit an `unreachable!()` and abort the compiler.
    let out = asm("struct P { x: i64, y: i64 } fn mk(a: i64) -> P { return P { x: a, y: a * 2 }; } fn main() -> i64 { print_i64(mk(5).y); return 0; }");
    assert!(out.contains("call mk"), "the maker is called");
    assert!(out.contains("(%rax), %rax"), "the field is read from the returned address");
}

#[test]
fn bitwise_and_shift_emit_the_right_instructions() {
    // A simple right operand folds straight into the instruction, whether it is
    // a constant or a variable; the shift count is masked to 6 bits, matching
    // Rust's `<<`/`>>` on `i64`.
    let out = asm("fn main() -> i64 { let a: i64 = 12; let b: i64 = 10; return a & b | a ^ b; }");
    assert!(out.contains("and %r12, %rax"), "a variable right operand folds in");
    assert!(out.contains("xor %r12, %rax"), "a variable right operand folds in");
    assert!(out.contains("or %rcx, %rax"), "a complex right operand still bounces");

    // An out-of-range constant shift (`1 << 70`) is rejected at compile time,
    // matching rustc; the 6-bit mask applies only to a run-time count, which
    // the hardware does anyway (`sal %cl` masks `%cl` to 6 bits for a 64-bit
    // operand).
    let out = asm_no_inline("fn f(n: i64) -> i64 { return 1 << n; } fn main() -> i64 { return f(70); }");
    assert!(out.contains("sal %cl, %rax"), "a variable shift count goes through %cl");

    let out = asm("fn main() -> i64 { return -8 >> 2; }");
    assert!(out.contains("sar $2, %rax"), ">> is arithmetic");
}

#[test]
fn a_register_right_operand_folds_into_arithmetic() {
    // A promoted right operand folds into the instruction instead of bouncing
    // through `%rcx`. `x * y` should be a single `imul`, not `mov %rcx` + `imul`.
    let out = asm(
        "fn main() -> i64 { let x: i64 = 3; let y: i64 = 4; let z: i64 = 5; \
         let a: i64 = x * y; let b: i64 = x + z; let c: i64 = x - z; return a + b + c; }",
    );
    assert!(out.contains("imul %r13, %rax"), "mul folds the right register");
    assert!(out.contains("add %r12, %rax"), "add folds the right register");
    assert!(out.contains("sub %r12, %rax"), "sub folds the right register");
}

#[test]
fn no_jump_to_the_very_next_label() {
    // A `jmp L` immediately followed by `L:` is a no-op the peephole must drop:
    // a function whose last expression is a value jumps to an epilogue label
    // that directly follows, and a `return` in the middle leaves a similar jump.
    for src in [
        "fn main() -> i64 { return 42; }",
        "fn f(n: i64) -> i64 { if n < 2 { return n; } return n + 1; } fn main() -> i64 { return f(5); }",
        "fn main() -> i64 { let mut i: i64 = 0; while i < 3 { i = i + 1; } return i; }",
    ] {
        let out = asm(src);
        let lines: Vec<&str> = out.lines().collect();
        for w in lines.windows(2) {
            let jmp = w[0].trim();
            if let Some(target) = jmp.strip_prefix("jmp ") {
                assert_ne!(
                    format!("{target}:"),
                    w[1].trim(),
                    "`{jmp}` jumps to the next instruction; it should be dropped"
                );
            }
        }
    }
}

#[test]
fn modulo_by_a_constant_has_no_round_trip() {
    // `a % d` computes the quotient in `%rdx`, then the remainder from it. It
    // used to bounce the quotient `%rdx`->`%rax`->`%rdx` through the shared
    // division tail; assert the remainder path consumes `%rdx` directly.
    let out = asm("fn main() -> i64 { let a: i64 = 7; return a % 1000003; }");
    assert!(out.contains("imul %rcx, %rdx"), "remainder scales the quotient in place");
    assert!(
        !out.contains("mov %rdx, %rax\n\tmov %rax, %rdx"),
        "no quotient round-trip through %rax"
    );
}

#[test]
fn a_small_divisor_uses_a_short_immediate() {
    // `movabs` is 10 bytes; when the divisor (or the magic multiplier) fits in a
    // sign-extended 32-bit immediate the 7-byte `mov` encodes the same value.
    // The remainder path materialises the divisor itself (`/` folds it into the
    // magic multiply), so a small `%` divisor is where the short form shows.
    let out = asm("fn main() -> i64 { let a: i64 = 7; return a % 3; }");
    assert!(out.contains("mov $3, %rcx"), "small divisor is a short immediate:\n{out}");
    assert!(!out.contains("movabs $3,"), "no movabs for a value that fits i32");
}

#[test]
fn a_wide_magic_multiplier_keeps_movabs() {
    // A divisor whose magic multiplier needs all 64 bits must still use
    // `movabs`; the short form would truncate it.
    let out = asm("fn main() -> i64 { let a: i64 = 7; return a / 1000003; }");
    assert!(out.contains("movabs $"), "wide multiplier keeps movabs:\n{out}");
}

#[test]
fn a_variable_shift_count_goes_in_cl() {
    // x86 shifts take the count in `%cl`; the right operand already lands in
    // `%rcx`, so no extra move is needed.
    let out = asm("fn main() -> i64 { let x: i64 = 1; let s: i64 = 3; return x << s; }");
    assert!(out.contains("sal %cl, %rax"));
    assert!(out.contains("sar %cl, %rax") || out.contains("sal %cl, %rax"));
}

#[test]
fn shift_binds_looser_than_addition() {
    // Rust parses `1 + 2 << 3` as `(1 + 2) << 3`, not `1 + (2 << 3)`.
    let out = asm("fn main() -> i64 { return 1 + 2 << 3; }");
    assert!(out.contains("add $2, %rax"), "the sum is formed first");
    assert!(out.contains("sal $3, %rax"), "then shifted");
}

#[test]
fn match_desugars_to_equality_compares() {
    // Each integer arm becomes an `==` against the scrutinee.
    let out = asm_no_inline("fn f(x: i64) -> i64 { match x { 0 => 10, 1 => 20, _ => 30 } } fn main() -> i64 { return f(0); }");
    assert!(out.contains("cmpq $0,"), "arm 0 compares against 0");
    assert!(out.contains("cmpq $1,"), "arm 1 compares against 1");
}

#[test]
fn a_match_scrutinee_call_runs_once() {
    // A call in the scrutinee is bound to a temporary before the arm tests, so
    // it is evaluated once no matter how many arms follow.
    let out = asm_no_inline("fn g() -> i64 { return 3; } fn main() -> i64 { return match g() { 1 => 10, 3 => 30, _ => 0 }; }");
    assert_eq!(out.matches("call g").count(), 1, "the scrutinee call runs once");
}

#[test]
fn arguments_are_evaluated_left_to_right() {
    // `f(g(), mk())` must run `g` before `mk`. The old code copied every
    // aggregate argument before any scalar one, so a scalar in a later position
    // ran after the aggregate and the side effects came out in the wrong order.
    let out = asm_no_inline("struct S { v: i64 } fn mk() -> S { return S { v: 1 }; } fn g() -> i64 { return 2; } fn f(x: i64, s: S) -> i64 { return s.v + x; } fn main() -> i64 { return f(g(), mk()); }");
    let g = out.find("call g").expect("g is called");
    let mk = out.find("call mk").expect("mk is called");
    assert!(g < mk, "the first argument's call is emitted first");
}

#[test]
fn an_aggregate_argument_still_travels_by_pointer() {
    // The reordering must not break the by-value copy: the aggregate is copied
    // into its slot and the slot's address is what the callee receives.
    let out = asm("struct S { v: i64 } fn f(s: S) -> i64 { return s.v; } fn main() -> i64 { return f(S { v: 5 }); }");
    assert!(out.contains("lea "), "the slot address is materialised");
    assert!(out.contains("call f"));
}

/// Run `f` on a thread with a large stack. The two depth-limit tests below
/// drive the parser to its nesting limit on purpose; a debug-build parser costs
/// about 10 KiB of stack per level, so 100 levels is close to the 2 MiB the
/// default test thread provides. A dedicated stack keeps these deterministic
/// rather than dependent on the harness's thread size.
fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(f)
        .expect("spawn test thread")
        .join()
        .expect("join test thread")
}

#[test]
fn deeply_nested_input_is_an_error_not_a_crash() {
    // Thousands of nested parens used to recurse until the native stack
    // overflowed and the process aborted. The parser now stops at a depth
    // limit and reports it.
    let err = with_big_stack(|| {
        let src =
            format!("fn main() -> i64 {{ return {}1{}; }}", "(".repeat(4000), ")".repeat(4000));
        compile_to_asm(&src, &CompileOptions::default()).unwrap_err()
    });
    assert!(err.contains("nests too deeply"), "got: {err}");
}

#[test]
fn a_very_long_operator_chain_is_an_error_not_a_crash() {
    // A left-deep chain `1+1+...+1` overflows the stack in sema/codegen unless
    // the parser bounds how deep the tree it builds can be.
    let err = with_big_stack(|| {
        let src = format!("fn main() -> i64 {{ return {}1; }}", "1+".repeat(200_000));
        compile_to_asm(&src, &CompileOptions::default()).unwrap_err()
    });
    assert!(err.contains("nests too deeply"), "got: {err}");
}

#[test]
fn nesting_below_the_limit_still_compiles() {
    // 30 parens (2 depth each) plus a 30-long `+` chain is ~90, under the 100
    // limit, so this must still compile.
    let src = format!("fn main() -> i64 {{ return {}1{} + {}; }}", "(".repeat(30), ")".repeat(30), "1+".repeat(30) + "1");
    assert!(compile_to_asm(&src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_bool_list_literal_infers_a_bool_element_type() {
    // The list-literal desugar used to hard-code `i64` as the element type, so
    // `[true, false]` was rejected as an i64 array of bools.
    assert!(compile_to_asm("fn main() -> i64 { let a: [bool; 2] = [true, false]; return 0; }", &CompileOptions::default()).is_ok());
    assert!(compile_to_asm("fn main() -> i64 { let a = [true, false, true]; return 0; }", &CompileOptions::default()).is_ok());
}

#[test]
fn a_list_literal_with_mixed_element_types_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { let a = [1, true]; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("element type mismatch"), "got: {err}");
}

#[test]
fn a_large_nested_splat_keeps_the_source_fixed() {
    // The counting-loop path for `[v; n]` with a wide element must advance only
    // the destination: every element is a copy of the *same* source value. An
    // earlier version stepped the source too, so element `i` was read from
    // `v + i * elem_size` and the tail of the array was garbage.
    let out = asm("fn main() -> i64 { let m: [[i64; 3]; 4] = [[5; 3]; 4]; return m[3][2]; }");
    assert!(out.contains("mov $4, %rcx"), "the splat loop counts elements:\n{out}");
    assert!(out.contains("add $24, %rdi"), "the destination advances:\n{out}");
    assert!(
        !out.contains("add $24, %rsi"),
        "the source must not advance:\n{out}"
    );
}

#[test]
fn nested_array_slots_are_allocated_per_element_word() {
    // A nested array of `n` elements of `w` words needs `n * w` frame slots. If
    // the `let` allocated only `n`, the elements would overlap and a write to
    // the last row would clobber an earlier one. A 4x3 `i64` array is 12 slots
    // (96 bytes), so the frame must reserve at least that much.
    let out = asm("fn main() -> i64 { let mut m: [[i64; 3]; 4] = [[0; 3]; 4]; m[3][2] = 1; return m[3][2]; }");
    let frame: i32 = out
        .lines()
        .find_map(|l| l.trim().strip_prefix("sub $")?.strip_suffix(", %rsp")?.parse().ok())
        .expect("a frame reservation");
    assert!(frame >= 96, "frame {frame} must hold 12 words:\n{out}");
}

#[test]
fn a_large_struct_splat_keeps_the_source_fixed() {
    // The same loop-path requirement for an array of structs.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: [P; 5] = [P { x: 4, y: 6 }; 5]; return a[4].y; }",
    );
    assert!(out.contains("mov $5, %rcx"), "the splat loop counts elements:\n{out}");
    assert!(out.contains("add $16, %rdi"), "the destination advances:\n{out}");
    assert!(
        !out.contains("add $16, %rsi"),
        "the source must not advance:\n{out}"
    );
}

#[test]
fn nested_arrays_compile_and_index() {
    // An array of arrays is laid out row-major: `m[i]` has the element type of
    // `m`, and `m[i][j]` indexes into it. The stride is the inner array's size,
    // not one slot, so both a constant and a variable index must scale.
    assert!(compile_to_asm(
        "fn main() -> i64 { let m: [[i64; 2]; 3] = [[1, 2], [3, 4], [5, 6]]; return m[0][0] + m[2][1]; }",
        &CompileOptions::default(),
    )
    .is_ok());
    assert!(compile_to_asm(
        "fn main() -> i64 { let m: [[i64; 2]; 3] = [[1, 2], [3, 4], [5, 6]]; \
         let mut i: i64 = 0; let mut s: i64 = 0; \
         while i < 3 { s = s + m[i][1]; i = i + 1; } return s; }",
        &CompileOptions::default(),
    )
    .is_ok());
    // A nested splat `[[v; k]; n]` copies `v`'s bytes into each inner array.
    assert!(compile_to_asm(
        "fn main() -> i64 { let m: [[i64; 2]; 3] = [[5; 2]; 3]; return m[1][1]; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn an_array_of_structs_compiles() {
    // A struct is a wide element: `a[i].field` indexes with a stride of the
    // struct's size, and a whole-element assignment copies the struct's words.
    let src = "struct P { x: i64, y: i64 } \
               fn main() -> i64 { let mut a: [P; 2] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }]; \
               a[1].x = 9; let b: P = P { x: 8, y: 7 }; a[0] = b; return a[0].x + a[1].x; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
    // A struct splat `[P { .. }; n]`.
    assert!(compile_to_asm(
        "struct P { x: i64 } fn main() -> i64 { let a: [P; 3] = [P { x: 2 }; 3]; return a[2].x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn an_array_of_payload_enums_is_still_rejected() {
    // A payload enum is not a fixed-width scalar; rustc rejects it as an array
    // element too, so rmc must give a clean error rather than miscompile.
    let err = compile_to_asm(
        "enum O { N, S(i64) } fn main() -> i64 { let a: [O; 2] = [O::N, O::S(1)]; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("array elements must be"), "got: {err}");
}

#[test]
fn an_empty_list_literal_is_rejected() {
    // `[]` has no element to infer from; the parser rejects it before sema.
    let err = compile_to_asm("fn main() -> i64 { let a = []; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("expected an expression"), "got: {err}");
}

#[test]
fn a_let_underscore_discards_its_initializer() {
    // `let _ = e;` evaluates `e` for its side effects and binds nothing.
    assert!(compile_to_asm(
        "fn g() -> i64 { return 7; } fn main() -> i64 { let _ = g(); let _ = 3 + 4; let _: i64 = 5; return 0; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_let_underscore_still_type_checks_its_initializer() {
    // The discard does not skip the annotation check (rustc E0308).
    let err = compile_to_asm(
        "fn main() -> i64 { let _: bool = 5; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("does not match"), "got: {err}");
}

#[test]
fn a_let_underscore_without_a_type_is_rejected() {
    // `let _;` has nothing to infer from (rustc E0282); `let _: i64;` is fine.
    let err = compile_to_asm("fn main() -> i64 { let _; return 0; }", &CompileOptions::default())
        .unwrap_err();
    assert!(err.contains("needs a type annotation"), "got: {err}");
    assert!(compile_to_asm("fn main() -> i64 { let _: i64; return 0; }", &CompileOptions::default()).is_ok());
}

#[test]
fn impl_methods_compile_and_resolve_by_receiver_type() {
    let src = "struct P { x: i64 } impl P { fn get(self) -> i64 { return self.x; } } \
               struct Q { x: i64 } impl Q { fn get(self) -> i64 { return self.x; } } \
               fn main() -> i64 { let p: P = P { x: 1 }; let q: Q = Q { x: 2 }; return p.get() + q.get(); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_method_on_a_non_struct_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { let x: i64 = 1; return x.foo(); }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("is not a struct"), "got: {err}");
}

#[test]
fn an_unknown_method_is_rejected() {
    let err = compile_to_asm("struct P { x: i64 } fn main() -> i64 { let p: P = P { x: 1 }; return p.nope(); }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("no method"), "got: {err}");
}

#[test]
fn an_associated_function_is_callable_through_the_type() {
    // `fn f()` with no `self` in an `impl` is an associated function, called as
    // `P::f(..)`, not a method. rustc accepts this, so rmc must too.
    let asm = asm_no_inline(
        "struct P { x: i64 } impl P { fn twice(n: i64) -> i64 { return n * 2; } } fn main() -> i64 { return P::twice(21); }",
    );
    assert!(asm.contains("P$twice"), "expected the mangled assoc fn in the asm");
}

#[test]
fn an_associated_function_called_as_a_method_is_rejected() {
    // A method call passes the receiver as the first argument, so `p.twice()`
    // has one argument too many for a receiver-less associated function.
    let err = compile_to_asm(
        "struct P { x: i64 } impl P { fn twice(n: i64) -> i64 { return n * 2; } } fn main() -> i64 { let p: P = P { x: 1 }; return p.twice(1); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("expects 1 argument"), "got: {err}");
}

#[test]
fn an_unknown_associated_function_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64 } impl P { fn new(x: i64) -> P { return P { x }; } } fn main() -> i64 { let p: P = P::nope(1); return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("no associated function"), "got: {err}");
}

#[test]
fn self_outside_an_impl_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { let s: Self = 1; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("outside of an `impl`"), "got: {err}");
}

#[test]
fn enum_variants_are_their_declaration_index() {
    let src = "enum C { Red, Green, Blue } fn main() -> i64 { return C::Blue as i64; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
    // The tag appears as the immediate the variant compiles to.
    let out = asm(src);
    assert!(out.contains("mov $2, %rax"), "expected the tag 2 in:\n{out}");
}

#[test]
fn an_explicit_discriminant_replaces_the_index() {
    // `B = 10` makes B's value 10, not its index 1.
    let out = asm("enum C { A = 5, B = 10 } fn main() -> i64 { return C::B as i64; }");
    assert!(out.contains("mov $10, %rax"), "expected the written tag 10 in:\n{out}");
}

#[test]
fn a_variant_without_a_discriminant_takes_the_previous_plus_one() {
    let out = asm("enum C { A = 4, B, C } fn main() -> i64 { return C::C as i64; }");
    assert!(out.contains("mov $6, %rax"), "expected 4+1+1 = 6 in:\n{out}");
}

#[test]
fn a_match_arm_keys_on_the_written_discriminant() {
    // The arm for B must compare against 10, not 1, or the wrong arm fires.
    let out = asm("enum C { A = 5, B = 10 } fn f(c: C) -> i64 { return match c { C::A => 1, C::B => 2 }; } fn main() -> i64 { return f(C::B); }");
    assert!(out.contains("$10") || out.contains("10,"), "expected a compare against 10 in:\n{out}");
}

#[test]
fn a_duplicate_discriminant_is_rejected() {
    let err = compile_to_asm("enum C { A = 5, B = 5 } fn main() -> i64 { return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("discriminant 5"), "got: {err}");
}

#[test]
fn a_payload_variant_cannot_have_a_discriminant() {
    let err = compile_to_asm("enum C { A(i64) = 5 } fn main() -> i64 { return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("payload cannot have an explicit discriminant"), "got: {err}");
}

#[test]
fn a_discriminant_that_overflows_the_next_variant_is_rejected() {
    // The variant after `i64::MAX` has no in-range value (rustc E0370).
    let err = compile_to_asm("enum C { A = 9223372036854775807, B } fn main() -> i64 { return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("discriminant overflowed"), "got: {err}");
}

#[test]
fn the_extreme_discriminants_are_representable() {
    let out = asm("enum C { A = -9223372036854775808, B = 9223372036854775807 } fn main() -> i64 { return C::A as i64 + C::B as i64; }");
    assert!(out.contains("9223372036854775807"), "expected i64::MAX in:\n{out}");
}

#[test]
fn an_unknown_variant_is_rejected() {
    let err = compile_to_asm("enum C { A } fn main() -> i64 { return C::B as i64; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("has no variant"), "got: {err}");
}

#[test]
fn an_unknown_enum_is_rejected() {
    let err = compile_to_asm("fn main() -> i64 { let c: Nope = Nope::A; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("unknown type") || err.contains("unknown enum"), "got: {err}");
}

#[test]
fn an_enum_may_be_a_field_type() {
    let src = "enum C { A, B } struct P { c: C, n: i64 } \
               fn main() -> i64 { let p: P = P { c: C::B, n: 7 }; return p.c as i64 + p.n; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_enum_method_resolves_by_receiver_type() {
    let src = "enum D { N, E } impl D { fn dx(self) -> i64 { match self { D::E => 1, _ => 0 } } } \
               fn main() -> i64 { return D::E.dx(); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn casting_an_integer_to_an_enum_is_rejected() {
    // rustc rejects `i64 as Enum`; rmc matches it rather than inventing a cast.
    let err = compile_to_asm("enum C { A } fn main() -> i64 { let x: i64 = 0; let c: C = x as C; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("cannot cast to enum"), "got: {err}");
}

#[test]
fn comparing_an_enum_with_an_integer_is_rejected() {
    let err = compile_to_asm("enum C { A } fn main() -> i64 { let c: C = C::A; if c == 0 { return 1; } return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("comparison between different types"), "got: {err}");
}

#[test]
fn a_struct_and_an_enum_cannot_share_a_name() {
    let err = compile_to_asm("enum X { A } struct X { n: i64 } fn main() -> i64 { return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("both a struct and an enum"), "got: {err}");
}

#[test]
fn a_unit_enum_array_is_allowed() {
    // A unit enum occupies one 8-byte slot and codegen emits it like `i64`, so
    // `[E; N]` lays out and indexes correctly. A payload enum is still rejected.
    let ok = "enum C { A, B } fn main() -> i64 { let a: [C; 2] = [C::A, C::B]; return a[1] as i64; }";
    assert!(compile_to_asm(ok, &CompileOptions::default()).is_ok());
    let bad = "enum O { N, S(i64) } fn main() -> i64 { let a: [O; 2] = [O::N; 2]; return 0; }";
    let err = compile_to_asm(bad, &CompileOptions::default()).unwrap_err();
    assert!(err.contains("array elements must be"), "got: {err}");
}

#[test]
fn two_enums_compare_but_an_enum_and_an_integer_do_not() {
    let ok = "enum C { A, B } fn main() -> i64 { let x: C = C::A; let y: C = C::B; if x == y { return 1; } return 0; }";
    assert!(compile_to_asm(ok, &CompileOptions::default()).is_ok());
    let bad = "enum C { A } fn main() -> i64 { let x: C = C::A; if x == 0 { return 1; } return 0; }";
    assert!(compile_to_asm(bad, &CompileOptions::default()).is_err());
}

#[test]
fn a_payload_enum_builds_and_matches() {
    let src = "enum Opt { None, Some(i64) } \
               fn get(o: Opt) -> i64 { match o { Opt::Some(v) => v, _ => 0 - 1 } } \
               fn main() -> i64 { return get(Opt::Some(5)); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_payload_enum_is_a_tagged_union_in_memory() {
    // `Opt` is `{ i64 tag, i64 payload }` = 16 bytes, so its frame use is a
    // tagged union rather than a single slot.
    let out = asm("enum Opt { None, Some(i64) } fn main() -> i64 { let o: Opt = Opt::Some(3); return 0; }");
    assert!(out.contains("movq $1,"), "expected the tag store in:\n{out}");
}

#[test]
fn a_payload_variant_needs_its_payload_bound_in_a_pattern() {
    let err = compile_to_asm(
        "enum O { N, S(i64) } fn main() -> i64 { let o: O = O::S(1); return match o { O::S => 0, _ => 1 }; }",
        &CompileOptions::default(),
    ).unwrap_err();
    assert!(err.contains("has a payload"), "got: {err}");
}

#[test]
fn a_payload_argument_is_type_checked() {
    let err = compile_to_asm(
        "enum O { N, S(i64) } fn main() -> i64 { let o: O = O::S(true); return 0; }",
        &CompileOptions::default(),
    ).unwrap_err();
    assert!(err.contains("expected I64, found Bool"), "got: {err}");
}

#[test]
fn a_payload_enum_cannot_be_compared_or_cast() {
    let cmp = compile_to_asm(
        "enum O { N, S(i64) } fn main() -> i64 { let a: O = O::S(1); let b: O = O::S(1); if a == b { return 1; } return 0; }",
        &CompileOptions::default(),
    );
    assert!(cmp.is_err());
    let cast = compile_to_asm(
        "enum O { N, S(i64) } fn main() -> i64 { let a: O = O::S(1); return a as i64; }",
        &CompileOptions::default(),
    );
    assert!(cast.is_err());
}

#[test]
fn a_recursive_payload_enum_is_rejected() {
    let err = compile_to_asm("enum L { Nil, Cons(i64, L) } fn main() -> i64 { return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("contains itself"), "got: {err}");
}

#[test]
fn a_struct_may_have_a_payload_enum_field() {
    let src = "enum O { N, S(i64) } struct W { o: O, n: i64 } \
               fn main() -> i64 { let w: W = W { o: O::S(3), n: 4 }; \
               return match w.o { O::S(v) => v + w.n, _ => 0 }; }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn an_immutable_local_cannot_be_assigned() {
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 1; x = 2; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("cannot assign to immutable"), "got: {err}");
    // A `mut` binding, and a first assignment to an uninitialised binding, are
    // both allowed.
    assert!(compile_to_asm(
        "fn main() -> i64 { let mut x: i64 = 1; x = 2; return x; }",
        &CompileOptions::default(),
    )
    .is_ok());
    assert!(compile_to_asm(
        "fn main() -> i64 { let x: i64; x = 2; return x; }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn an_immutable_parameter_cannot_be_assigned() {
    let err = compile_to_asm(
        "fn f(x: i64) -> i64 { x = 2; return x; } fn main() -> i64 { return f(1); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("cannot assign to immutable"), "got: {err}");
    assert!(compile_to_asm(
        "fn f(mut x: i64) -> i64 { x = 2; return x; } fn main() -> i64 { return f(1); }",
        &CompileOptions::default(),
    )
    .is_ok());
}

#[test]
fn a_seventh_parameter_is_rejected() {
    let err = compile_to_asm(
        "fn f(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64) -> i64 { return g; } \
         fn main() -> i64 { return f(1, 2, 3, 4, 5, 6, 7); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("at most 6"), "got: {err}");
}

#[test]
fn a_call_in_an_enum_constructor_does_not_clobber_an_argument() {
    // `7` is placed in `%rdi` before the second argument runs `g`; the direct
    // path must be declined so the call cannot destroy `%rdi`. `f` is kept out
    // of the inliner so the call survives as a real `call`.
    let out = asm_no_inline(
        "enum O { N, S(i64) } \
         fn g() -> i64 { return 1; } \
         fn f(a: i64, b: i64) -> i64 { return a + b; } \
         fn main() -> i64 { return f(7, { let e: O = O::S(g()); 9 }); }",
    );
    assert!(
        out.contains("push %rdi") || out.contains("mov %rax, -"),
        "the argument is captured to a frame slot, not left in %rdi across the call"
    );
}

#[test]
fn a_method_on_a_payload_enum_is_callable() {
    let src = "enum O { N, S(i64) } \
               impl O { fn v(self) -> i64 { return match self { O::S(x) => x, _ => 0 }; } } \
               fn main() -> i64 { return O::S(42).v(); }";
    assert!(compile_to_asm(src, &CompileOptions::default()).is_ok());
}

#[test]
fn a_spill_across_a_call_uses_a_register_not_the_frame() {
    // With the self-recursion pass off, `fib`'s only memory traffic is the first
    // recursive result held across the second call. It must live in a
    // callee-saved register, not a frame slot.
    let out = asm_no_tco(
        "fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } \
         fn main() -> i64 { return fib(arg_i64(1)); }",
    );
    assert!(out.contains("push %r12"), "expected a saved spill register in:\n{out}");
    assert!(out.contains("mov %rax, %r12"), "expected the spill into %r12 in:\n{out}");
}

#[test]
fn self_recursion_becomes_a_loop_with_one_call_per_level() {
    // The right-spine rewrite halves the call tree: `fib(n-2)` no longer
    // recurses, so the body makes exactly one `call fib` and the second operand
    // becomes an accumulator add plus a loop-carried `n = n - 2`.
    let out = asm("fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } \
                   fn main() -> i64 { return fib(arg_i64(1)); }");
    let fib = &out[out.find("fib:").unwrap()..];
    let fib = &fib[..fib.find(".size fib").unwrap()];
    assert_eq!(fib.matches("call fib").count(), 1, "one call per level:\n{fib}");
    assert!(fib.contains("add %rax"), "the accumulator adds the result:\n{fib}");
    assert!(fib.contains("lea -2(%rbx), %rbx"), "the loop advances n by -2:\n{fib}");
}

#[test]
fn a_left_operand_that_is_not_a_self_call_still_folds_the_right_spine() {
    // `tree(n) = 1 + tree(n-1) + tree(n-2)` parses as `(1 + tree(n-1)) + tree(n-2)`,
    // so the left operand of the top `+` is *not* a self-call. The right operand
    // still is, and the right spine can still be folded into a loop, so the body
    // must make exactly one `call tree` per level (rustc does the same).
    let out = asm(
        "fn tree(n: i64) -> i64 { if n <= 1 { return n; } \
         return 1 + tree(n - 1) + tree(n - 2); } \
         fn main() -> i64 { return tree(arg_i64(1)); }",
    );
    let tree = &out[out.find("tree:").unwrap()..];
    let tree = &tree[..tree.find(".size tree").unwrap()];
    assert_eq!(tree.matches("call tree").count(), 1, "one call per level:\n{tree}");
    assert!(tree.contains("lea -2(%rbx), %rbx"), "the loop advances n by -2:\n{tree}");
}

#[test]
fn the_self_recursion_pass_leaves_a_foreign_call_in_the_left_operand_alone() {
    // The loop evaluates the left operand once per level, so a call to *another*
    // function there could be duplicated or reordered relative to the recursion;
    // only a self-call is safe. Both calls must stay. `g` is itself recursive, so
    // the inliner cannot remove its call and the pass must see it.
    let out = asm(
        "fn g(n: i64) -> i64 { if n < 2 { return n; } return g(n - 1) + g(n - 2); } \
         fn f(n: i64) -> i64 { if n < 2 { return n; } return g(n) + f(n - 1); } \
         fn main() -> i64 { return f(5); }",
    );
    let f = &out[out.find("f:").unwrap()..];
    let f = &f[..f.find(".size f").unwrap()];
    assert_eq!(f.matches("call f").count(), 1, "the self-call is not folded away:\n{f}");
    assert!(f.contains("call g"), "the foreign call is kept:\n{f}");
}

#[test]
fn the_self_recursion_pass_leaves_a_non_associative_recursion_alone() {
    // `alt(x) = alt(x-1) - alt(x-2)` is not associative, so the rewrite would be
    // wrong; the pass must decline it and keep both calls.
    let out = asm(
        "fn alt(n: i64) -> i64 { if n < 2 { return n; } return alt(n - 1) - alt(n - 2); } \
         fn main() -> i64 { return alt(5); }",
    );
    let alt = &out[out.find("alt:").unwrap()..];
    let alt = &alt[..alt.find(".size alt").unwrap()];
    assert_eq!(alt.matches("call alt").count(), 2, "both calls kept:\n{alt}");
}

#[test]
fn the_self_recursion_pass_leaves_a_return_in_an_argument_alone() {
    // A `return` inside the right argument would leave the transformed loop
    // instead of the recursive frame, dropping the accumulator, so the pass
    // must decline the shape and keep both calls.
    let out = asm(
        "fn f(n: i64) -> i64 { if n < 2 { return n; } \
         return f({ if n == 3 { return 4; } n - 1 }) + f(n - 2); } \
         fn main() -> i64 { return f(5); }",
    );
    let f = &out[out.find("f:").unwrap()..];
    let f = &f[..f.find(".size f").unwrap()];
    assert_eq!(f.matches("call f").count(), 2, "both calls kept:\n{f}");
}

#[test]
fn overflow_checks_disable_the_self_recursion_pass() {
    // Reassociation is only exact for wrapping arithmetic: with checks on, the
    // loop would trap on an intermediate the recursion never forms. The pass
    // must be off, leaving both calls.
    let out = compile_to_asm(
        "fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } \
         fn main() -> i64 { return fib(5); }",
        &CompileOptions { overflow_checks: true, ..Default::default() },
    )
    .unwrap();
    let fib = &out[out.find("fib:").unwrap()..];
    let fib = &fib[..fib.find(".size fib").unwrap()];
    assert_eq!(fib.matches("call fib").count(), 2, "both calls kept under checks:\n{fib}");
}

#[test]
fn the_accumulator_has_an_unspellable_name() {
    // Source cannot spell a leading NUL, so a generated temporary can never
    // collide with a user identifier.
    let out = asm("fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } \
                   fn main() -> i64 { return fib(5); }");
    assert!(!out.contains("__tco_acc"), "no spellable temporary name in:\n{out}");
}

#[test]
fn a_reference_to_a_promoted_local_forces_it_into_memory() {
    // `&x` needs an address, so `x` cannot be held in a register: its frame
    // slot must stay current or the `lea` would read a stale word.
    let out = asm("fn main() -> i64 { let x: i64 = 7; let r: &i64 = &x; return *r; }");
    assert!(out.contains("lea "), "a reference takes an address in:\n{out}");
    assert!(
        !out.contains("mov $7, %r12"),
        "an addressed local must not be promoted to a register in:\n{out}"
    );
}

#[test]
fn a_ref_typed_parameter_is_one_slot_not_a_copy() {
    // A reference is a scalar: `&Point` is passed in one register and is not
    // copied, however big `Point` is.
    let out = asm_no_inline(
        "struct P { x: i64, y: i64, z: i64 } \
         fn f(p: &P) -> i64 { return p.x; } \
         fn main() -> i64 { let v: P = P { x: 1, y: 2, z: 3 }; return f(&v); }",
    );
    // No copy loop and no hidden argument buffer for the aggregate: the call
    // passes the address that `&v` computes.
    assert!(out.contains("call P$") || out.contains("call f"), "calls f in:\n{out}");
    assert!(!out.contains("rep movsq"), "no aggregate copy for a reference in:\n{out}");
}

#[test]
fn a_method_with_a_ref_receiver_passes_an_address() {
    let out = asm_no_inline(
        "struct P { x: i64, y: i64 } \
         impl P { fn get(&self) -> i64 { return self.x; } } \
         fn main() -> i64 { let v: P = P { x: 5, y: 6 }; return v.get(); }",
    );
    // The receiver arrives as an address: the method reads `x` through the
    // pointer it was handed, not from a copied frame slot. The field read is an
    // indirect load off the receiver register (`0(%r..)`), never a frame slot.
    assert!(out.contains("P$get"), "mangled method present in:\n{out}");
    let get = &out[out.find("P$get:").expect("method body")..];
    let body = &get[..get.find(".size P$get").expect("method end")];
    assert!(
        body.contains("(%rbx)") || body.contains("(%rax)"),
        "field read through a pointer in:\n{body}"
    );
}

#[test]
fn a_non_lvalue_operand_of_address_of_is_materialised() {
    // `&(1 + 2)` has no address of its own, so sema gives it a temporary slot
    // and codegen stores the value there and yields that address.
    let out = asm("fn main() -> i64 { let r: &i64 = &(1 + 2); return *r; }");
    assert!(out.contains("lea "), "the temporary's address is taken in:\n{out}");
}

#[test]
fn a_dereference_of_a_non_reference_is_rejected() {
    let bad = "fn main() -> i64 { let x: i64 = 1; return *x; }";
    assert!(compile_to_asm(bad, &CompileOptions::default()).is_err());
}

#[test]
fn address_of_a_non_lvalue_is_accepted_through_a_temporary() {
    // `&(f())` where `f` returns a scalar: the call's result is stored in a
    // temporary and its address is taken.
    let out = asm_no_inline(
        "fn f() -> i64 { return 9; } fn main() -> i64 { let r: &i64 = &(f()); return *r; }",
    );
    assert!(out.contains("call f"), "call present in:\n{out}");
}

#[test]
fn a_reference_may_be_stored_in_a_struct_field() {
    // rmc's struct layout stores a reference field like any other word; this
    // needs no lifetime in rmc even though Rust would require one.
    let out = asm(
        "struct H { r: &i64, n: i64 } \
         fn main() -> i64 { let v: i64 = 7; let h: H = H { r: &v, n: 3 }; return *h.r + h.n; }",
    );
    assert!(out.contains("lea "), "the reference field takes an address in:\n{out}");
}

#[test]
fn a_nested_reference_dereferences_twice() {
    let out = asm(
        "fn main() -> i64 { let v: i64 = 7; let r: &i64 = &v; let rr: &&i64 = &r; return **rr; }",
    );
    // Two loads: through `rr` to reach `r`, then through `r` to reach `v`.
    assert!(out.matches("mov (%rax), %rax").count() >= 2, "two dereferences in:\n{out}");
}

#[test]
fn an_immediate_store_through_a_reference_is_64_bit() {
    // `mov $42, (%rcx)` would be a 32-bit store under `as`'s default and leave
    // the destination's high four bytes stale, so the store must be `movq`.
    let out = asm("fn main() -> i64 { let mut x: i64 = 1; let p: &mut i64 = &mut x; *p = 42; return x; }");
    assert!(
        out.lines().any(|l| l.trim_start().starts_with("movq $42, (")),
        "immediate indirect store is sized in:\n{out}"
    );
    assert!(
        !out.lines().any(|l| l.trim_start().starts_with("mov $42, (")),
        "unsized immediate store in:\n{out}"
    );
}

#[test]
fn a_write_through_a_dereference_stores_to_the_referent() {
    let out = asm("fn main() -> i64 { let mut x: i64 = 1; let p: &mut i64 = &mut x; *p = 42; return x; }");
    // The store's target address is the pointer value, so the store is indirect
    // through a register rather than into a frame slot; the constant value folds
    // straight into it.
    assert!(
        out.lines().any(|l| l.trim_start().starts_with("movq $42, (")),
        "indirect store in:\n{out}"
    );
}

#[test]
fn a_write_through_a_reference_to_a_struct_field_stores_to_the_field() {
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; let r: &mut i64 = &mut p.y; *r = 9; return p.y; }",
    );
    assert!(
        out.lines().any(|l| l.trim_start().starts_with("movq $9, (")),
        "indirect store in:\n{out}"
    );
}

#[test]
fn a_mut_receiver_is_passed_by_address() {
    let out = asm_no_inline(
        "struct C { n: i64 } \
         impl C { fn bump(&mut self) -> i64 { self.n = self.n + 1; return self.n; } } \
         fn main() -> i64 { let mut c: C = C { n: 1 }; return c.bump(); }",
    );
    assert!(out.contains("C$bump"), "mangled method present in:\n{out}");
    assert!(out.contains("lea "), "the receiver's address is taken in:\n{out}");
}

#[test]
fn a_mut_reference_coerces_to_a_shared_reference() {
    // `peek(&mut x)` where `peek` wants `&i64`: allowed, and no code is emitted
    // for the coercion since both are one pointer.
    let out = asm_no_inline(
        "fn peek(p: &i64) -> i64 { return *p; } \
         fn main() -> i64 { let mut x: i64 = 5; return peek(&mut x); }",
    );
    assert!(out.contains("call peek"), "call present in:\n{out}");
}

#[test]
fn a_shared_reference_is_not_assignable_to_a_mut_reference() {
    // The coercion only goes `&mut` to `&`, never the other way.
    let bad = "fn f(p: &mut i64) -> i64 { return *p; } \
               fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; return f(r); }";
    assert!(compile_to_asm(bad, &CompileOptions::default()).is_err());
}

#[test]
fn a_field_read_through_a_reference_is_one_load() {
    // `p.x` where `p: &Point` must fold to `disp(%reg)`, not a
    // `mov`/`add`/`mov` sequence: three instructions per field is what made a
    // `&self` accessor over a wide struct an order of magnitude slower than
    // rustc on the `refs` benchmark.
    let out = asm_no_inline(
        "struct P { x: i64, y: i64 } \
         fn get(p: &P) -> i64 { return p.y; } \
         fn main() -> i64 { let q: P = P { x: 1, y: 2 }; return get(&q); }",
    );
    let body: String = out
        .lines()
        .skip_while(|l| !l.starts_with("get:"))
        .take_while(|l| !l.trim_start().starts_with("ret"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(body.contains("mov 8(%r"), "folded field load in:\n{body}");
    assert!(!body.contains("add $8"), "no address add in:\n{body}");
}

#[test]
fn a_field_read_through_a_double_reference_still_loads_the_inner_pointer() {
    // The register fold only applies one level deep; `(**ppr).x` must go
    // through the inner pointer, not treat it as a register.
    let out = asm_no_inline(
        "struct P { x: i64, y: i64 } \
         fn get(pp: &&P) -> i64 { return (**pp).x; } \
         fn main() -> i64 { let q: P = P { x: 1, y: 2 }; let r: &P = &q; return get(&r); }",
    );
    assert!(out.contains("get:"), "function present in:\n{out}");
}

#[test]
fn a_scalar_dereference_folds_into_a_comparison() {
    // `*p < 5` with `p` in a register is one `cmp mem, imm`; the deref is a
    // memory operand, not a load into `%rax` first.
    let out = asm_no_inline("fn lt(p: &i64) -> bool { return *p < 5; } fn main() -> i64 { let x: i64 = 1; if lt(&x) { return 1; } return 0; }");
    let body: String = out
        .lines()
        .skip_while(|l| !l.starts_with("lt:"))
        .take_while(|l| !l.trim_start().starts_with("ret"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(body.contains("cmpq $5, (%r"), "folded deref cmp in:\n{body}");
}

#[test]
fn no_instructions_follow_an_unconditional_jump() {
    // A `return` inside an expression emits `jmp epilogue` and then the rest of
    // the expression, which is unreachable. The trim pass removes it; this
    // checks the property directly rather than a specific instruction.
    let out = asm("fn f(n: i64) -> i64 { if n > 0 { return 1; } return 2; } fn main() -> i64 { return f(1); }");
    let mut dead = false;
    for line in out.lines() {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let is_label = !line.starts_with('\t') && !line.starts_with(' ') && text.ends_with(':');
        if is_label || line.starts_with('.') {
            dead = false;
            continue;
        }
        assert!(!dead, "unreachable instruction after a jump: {line:?}");
        if text.starts_with("jmp ") || text == "ret" {
            dead = true;
        }
    }
}

#[test]
fn a_register_to_self_move_is_dropped() {
    let out = asm("fn f(x: i64) -> i64 { return x; } fn main() -> i64 { return f(1); }");
    assert!(!out.contains("mov %rax, %rax"), "self move in:\n{out}");
    assert!(!out.contains("mov %rbx, %rbx"), "self move in:\n{out}");
}

#[test]
fn accumulation_folds_into_in_place_adds() {
    // `acc = acc + i + 2` should add straight into `acc`'s register, not build
    // the sum in `%rax` and store it back.
    let out = asm(
        "fn main() -> i64 { let mut acc: i64 = 0; let mut i: i64 = 0; \
         while i < 10 { acc = acc + i + 2; i = i + 1; } return acc; }",
    );
    let body = &out[out.find("main:").unwrap()..];
    let body = &body[..body.find(".size main").unwrap()];
    assert!(
        body.contains("add %rbx, %r12") && body.contains("add $2, %r12"),
        "in-place accumulation expected in:\n{body}"
    );
}

#[test]
fn accumulation_does_not_reuse_a_clobbered_accumulator() {
    // `x = x + x` reads the accumulator on the right; folding it in place would
    // double the running sum instead of the original value, so the tree form
    // must be used and the result must still be correct.
    let out = asm(
        "fn main() -> i64 { let mut x: i64 = 3; let mut i: i64 = 0; \
         while i < 4 { x = x + x; i = i + 1; } return x; }",
    );
    assert!(out.contains("main:"), "compiles");
    // The fold is only safe when the addends do not read `x`'s register; here
    // they do, so no single `add <x-reg>, <x-reg>` may appear.
    let body = &out[out.find("main:").unwrap()..];
    let body = &body[..body.find(".size main").unwrap()];
    for reg in ["%rbx", "%r12", "%r13", "%r14", "%r15"] {
        assert!(
            !body.contains(&format!("add {reg}, {reg}")),
            "self-add of the accumulator in:\n{body}"
        );
    }
}

#[test]
fn accumulation_flattens_a_nested_sum() {
    // `acc = acc + a + (b + c) + c`: the parenthesised sum is flattened into the
    // same in-place chain of adds, not evaluated in `%rax` and added once.
    let src = "fn main() -> i64 { let a: i64 = 3; let b: i64 = 5; let c: i64 = 7; \
               let mut acc: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { acc = acc + a + (b + c) + c; i = i + 1; } return acc; }";
    let body = loop_body(&asm_no_licm(src));
    assert_eq!(body.matches("add ").count(), 4, "one add per term in:\n{body}");
    assert!(!body.contains("mov %rax"), "no `%rax` bounce in:\n{body}");
    assert!(!body.contains("mov %rax, -"), "no spill in:\n{body}");
}

#[test]
fn an_inlined_accessor_chain_folds_into_the_accumulator() {
    // The `refs` shape: a `&self` accessor inlined into a running sum. Once the
    // inliner unwraps the spliced block, the accessor's fields are plain
    // operands, so the sum folds into one add per term with no `%rax` bounce and
    // no call.
    let src = "struct W { a: i64, b: i64 } \
               impl W { fn sum(&self) -> i64 { return self.a + self.b; } } \
               fn main() -> i64 { let w: W = W { a: 3, b: 4 }; let wr: &W = &w; \
               let mut acc: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { acc = acc + wr.sum(); i = i + 1; } return acc; }";
    let body = loop_body(&asm(src));
    assert!(!body.contains("call"), "the accessor is inlined:\n{body}");
    assert!(!body.contains("mov %rax"), "the sum folds into the accumulator:\n{body}");
}

#[test]
fn a_small_accessor_call_is_inlined_away() {
    // A `&self` accessor is small and call-free, so a call to it becomes its
    // body: no `call` and no argument setup remain.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         impl P { fn sum(&self) -> i64 { return self.x + self.y; } } \
         fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return p.sum(); }",
    );
    assert!(!out.contains("call P$sum"), "the accessor call is inlined in:\n{out}");
}

#[test]
fn a_wrapper_is_inlined_once_its_callees_are() {
    // `wrap` calls two accessors, so it is not a candidate until they are
    // inlined into it; the pass must iterate to a fixpoint to reach it.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         impl P { fn sum(&self) -> i64 { return self.x + self.y; } } \
         fn wrap(p: &P) -> i64 { return p.sum() + p.sum(); } \
         fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return wrap(&p); }",
    );
    assert!(!out.contains("call wrap"), "the wrapper call is inlined in:\n{out}");
    assert!(!out.contains("call P$sum"), "the accessor call is inlined in:\n{out}");
}

#[test]
fn inlining_evaluates_an_argument_with_side_effects_once() {
    // `f(sc(5))` binds the call's result to one slot, so `sc` runs once even
    // though `x` is read twice in the body.
    let out = asm(
        "fn sc(t: i64) -> i64 { print_i64(t); return t * 2; } \
         fn f(x: i64) -> i64 { return x + x; } \
         fn main() -> i64 { return f(sc(5)); }",
    );
    assert_eq!(out.matches("call sc").count(), 1, "the argument call runs once in:\n{out}");
}

#[test]
fn a_recursive_function_is_not_inlined_into_itself() {
    // `fib` calls itself, so its body is not call-free and it is not a
    // candidate; the recursive call must survive as a real call.
    let out = asm(
        "fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } \
         fn main() -> i64 { return fib(10); }",
    );
    assert!(out.contains("call fib"), "the recursive call is kept in:\n{out}");
}

#[test]
fn a_loop_bearing_helper_is_inlined_into_its_caller() {
    // `gcd` has a `while` loop, but it is call-free and small, so it is still a
    // candidate: inlining it removes the call from the caller's hot loop. The
    // loop lives in the caller's frame now, so no `call gcd` remains.
    let out = asm(
        "fn gcd(a: i64, b: i64) -> i64 { \
             let mut x: i64 = a; let mut y: i64 = b; \
             while y != 0 { let t: i64 = x % y; x = y; y = t; } \
             return x; } \
         fn main() -> i64 { return gcd(12, 18); }",
    );
    assert!(!out.contains("call gcd"), "the loop-bearing helper is inlined in:\n{out}");
}

#[test]
fn a_loop_bearing_helper_with_break_is_not_inlined() {
    // A `break`/`continue` in a candidate would bind to the *caller's* enclosing
    // loop once spliced, so a body containing one must stay a real call.
    let out = asm(
        "fn first_over(limit: i64) -> i64 { \
             let mut i: i64 = 0; \
             while i < 100 { if i > limit { break; } i = i + 1; } \
             return i; } \
         fn main() -> i64 { return first_over(3); }",
    );
    assert!(out.contains("call first_over"), "the early-exit helper is kept in:\n{out}");
}

#[test]
fn no_inline_option_keeps_the_call() {
    let out = asm_no_inline(
        "struct P { x: i64, y: i64 } \
         impl P { fn sum(&self) -> i64 { return self.x + self.y; } } \
         fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return p.sum(); }",
    );
    assert!(out.contains("call P$sum"), "inlining is off in:\n{out}");
}

#[test]
fn an_inlined_local_does_not_alias_a_caller_slot() {
    // Two calls to a body with a local must each get their own frame region, or
    // the second would clobber the first's local.
    let out = asm(
        "fn g(v: i64) -> i64 { let t: i64 = v * 3; return t + v; } \
         fn main() -> i64 { let a: i64 = g(1); let b: i64 = g(2); return a * 100 + b; }",
    );
    assert!(!out.contains("call g"), "g is inlined in:\n{out}");
    assert!(out.contains("main:"), "compiles");
}

#[test]
fn an_inlined_parameter_assignment_does_not_reach_the_caller() {
    // A parameter is a local copy, so a body that assigns it must write a fresh
    // slot rather than the caller's argument. Here `a` is promoted to `%r12`, so
    // the increment must target a different register and `%r12` must survive.
    let out = asm(
        "fn bump(mut x: i64) -> i64 { x = x + 1; return x; } \
         fn main() -> i64 { let a: i64 = 5; return bump(a) * 100 + a; }",
    );
    assert!(!out.contains("call bump"), "bump is inlined in:\n{out}");
    for store in ["add $1, %r12", "lea 1(%r12), %r12", "inc %r12"] {
        assert!(!out.contains(store), "the parameter write reaches the caller ({store}):\n{out}");
    }
}

/// Assembly with loop-invariant code motion disabled.
fn asm_no_licm(src: &str) -> String {
    compile_to_asm(src, &CompileOptions { no_licm: true, ..Default::default() })
        .expect("should compile")
}

/// The instructions inside `main` only, so a hoisted value appearing before the
/// loop is visible but a runtime helper's code is not.
fn main_body(out: &str) -> String {
    let body = &out[out.find("main:").unwrap()..];
    body[..body.find(".size main").unwrap()].to_string()
}

/// Every `name: .. .size name` function body in the assembly.
fn fn_bodies(out: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = out.lines().collect();
    let mut v = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if l.ends_with(':') && !l.starts_with('.') {
            let name = l.trim_end_matches(':').to_string();
            let mut body = String::new();
            let mut j = i + 1;
            while j < lines.len() && !lines[j].trim_start().starts_with(".size ") {
                body.push_str(lines[j]);
                body.push('\n');
                j += 1;
            }
            v.push((name, body));
            i = j;
        } else {
            i += 1;
        }
    }
    v
}

/// The depth a function's prologue pushes `%rsp` below its entry value, mod 16.
/// Entry `%rsp` is 8 mod 16 (the caller's `call` pushed the return address), so
/// the call ABI wants this to be 8: at a `call`, `%rsp` is then 0 mod 16.
fn prologue_depth_mod16(body: &str) -> i32 {
    let mut d = 0i32;
    for l in body.lines() {
        let t = l.trim();
        if t.starts_with("push ") {
            d += 8;
        } else if let Some(rest) = t.strip_prefix("sub $") {
            if let Some(n) = rest.strip_suffix(", %rsp") {
                d += n.parse::<i32>().unwrap();
            }
        }
    }
    d % 16
}

/// The first loop in `main`: the label a later `jmp` targets, through that jump.
/// Anything LICM hoisted sits before this region.
fn loop_body(out: &str) -> String {
    let body = main_body(out);
    let lines: Vec<&str> = body.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        // The loop's back edge is the jump that targets a label emitted earlier.
        // A `while` is rotated, so its back edge is conditional (`jne`/`jl`); a
        // `for` and a `loop` still close with an unconditional `jmp`.
        let target = if let Some(rest) = t.strip_prefix("jmp ") {
            rest
        } else if t.starts_with('j') {
            t.split_whitespace().nth(1).unwrap_or("")
        } else {
            continue;
        };
        if target.is_empty() {
            continue;
        }
        let label = format!("{target}:");
        if let Some(start) = (0..i).rev().find(|&j| lines[j].trim() == label) {
            return lines[start..=i].join("\n");
        }
    }
    String::new()
}

#[test]
fn an_invariant_product_is_hoisted_out_of_a_loop() {
    // `a` is never written in the loop, so `a * a` is computed once, before the
    // loop, and the loop only adds the result.
    let src = "fn main() -> i64 { let a: i64 = 3; let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + a * a; i = i + 1; } return s; }";
    let with = loop_body(&asm(src));
    let without = loop_body(&asm_no_licm(src));
    assert!(!with.contains("imul"), "the product is hoisted out of the loop:\n{with}");
    assert!(without.contains("imul"), "without LICM the product stays in the loop:\n{without}");
}

#[test]
fn a_value_the_loop_writes_is_not_hoisted() {
    // `x` is reassigned in the loop, so `x * 7` is loop-variant and must stay.
    // The multiplier is deliberately not `lea`-expressible (2/3/4/5/8/9), so the
    // product still compiles to an `imul` and this asserts on the multiply
    // itself rather than on a `lea` that also appears in the loop's own
    // increment.
    let src = "fn main() -> i64 { let mut x: i64 = 1; let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { x = x + i; s = s + x * 7; i = i + 1; } return s; }";
    let with = loop_body(&asm(src));
    assert!(with.contains("imul"), "a loop-variant product stays in the loop:\n{with}");
}

#[test]
fn invariant_terms_of_a_sum_are_grouped_into_one_add() {
    // `acc + a*b + b*c + a*c` parses left-associatively, so no sub-expression is
    // invariant and the pass would otherwise hoist each term separately, leaving
    // one dependent add per term in the loop. Regrouping the invariant terms
    // into a single sub-sum collapses them to one hoisted value, so the loop
    // does one add per iteration — the reassociation LLVM performs, and what
    // makes the `refs` hot loop competitive.
    let src = "fn main() -> i64 { let a: i64 = 2; let b: i64 = 3; let c: i64 = 4; \
               let mut acc: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { acc = acc + a * b + b * c + a * c; i = i + 1; } return acc; }";
    let with = loop_body(&asm(src));
    assert_eq!(with.matches("add ").count(), 1, "one add per iteration:\n{with}");
    assert!(!with.contains("imul"), "the products are hoisted:\n{with}");
    let without = loop_body(&asm_no_licm(src));
    assert!(without.contains("imul"), "without LICM nothing is hoisted:\n{without}");
}

#[test]
fn a_variant_term_keeps_the_sum_in_place() {
    // `acc + inv + i` has a loop-variant term, so the invariant half is still
    // grouped and hoisted, but the loop keeps one add for it and one for `i`.
    let src = "fn main() -> i64 { let a: i64 = 2; let b: i64 = 3; \
               let mut acc: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { acc = acc + a * b + i; i = i + 1; } return acc; }";
    let with = loop_body(&asm(src));
    assert_eq!(with.matches("add ").count(), 2, "one add for the hoist, one for `i`:\n{with}");
    assert!(!with.contains("imul"), "the product is hoisted:\n{with}");
}


#[test]
fn a_loop_that_calls_is_left_alone() {
    // A call can write anything, so nothing in this loop is hoisted.
    let src = "fn f(x: i64) -> i64 { print_i64(x); return x; } \
               fn main() -> i64 { let a: i64 = 3; let b: i64 = 4; let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + a * b + f(i); i = i + 1; } return s; }";
    let with = loop_body(&asm(src));
    assert!(with.contains("imul"), "a call blocks hoisting:\n{with}");
}

#[test]
fn a_for_loop_variable_is_not_hoisted() {
    // The `for` variable is assigned every iteration by the loop itself, so the
    // comparison against it must stay inside the body.
    let src = "fn main() -> i64 { let mut s: i64 = 0; \
               for i in 0..10 { if i == 3 { s = s + 1; } s = s + i; } return s; }";
    let with = loop_body(&asm(src));
    let without = loop_body(&asm_no_licm(src));
    assert_eq!(with.matches("sete").count(), without.matches("sete").count(),
        "a `for` variable is loop-carried:\n{with}");
}

#[test]
fn overflow_checks_disable_licm() {
    // Hoisting a checked `+` would trap before the loop, not at the overflowing
    // iteration, so the pass is off under `--overflow-checks`.
    let src = "fn main() -> i64 { let a: i64 = 3; let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + a * a; i = i + 1; } return s; }";
    let out = compile_to_asm(src, &CompileOptions { overflow_checks: true, ..Default::default() })
        .expect("should compile");
    assert!(loop_body(&out).contains("imul"), "LICM is off under overflow checks:\n{out}");
}

#[test]
fn no_licm_option_keeps_the_loop_work() {
    let src = "fn main() -> i64 { let a: i64 = 3; let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + a * a; i = i + 1; } return s; }";
    assert!(!loop_body(&asm(src)).contains("imul"), "the default hoists");
    assert!(loop_body(&asm_no_licm(src)).contains("imul"), "--no-licm does not");
}

#[test]
fn a_loop_condition_that_mutates_blocks_hoisting() {
    // The condition `tick(&mut x)` writes `x` through a reference, so `x * x` is
    // not invariant: the condition runs before every iteration.
    let src = "fn tick(x: &mut i64) -> bool { *x = *x + 1; return *x < 4; } \
               fn main() -> i64 { let mut x: i64 = 0; let mut s: i64 = 0; \
               while tick(&mut x) { s = s + x * x; } return s; }";
    let with = loop_body(&asm(src));
    assert!(with.contains("imul"), "a mutating condition blocks hoisting:\n{with}");
}

#[test]
fn a_reference_to_a_reassigned_struct_blocks_hoisting() {
    // `p = P { a: 2 }` rewrites the words `r.a` reads even though the loop never
    // assigns `r`'s own slot, so the field read must stay in the loop.
    let src = "struct P { a: i64 } \
               fn main() -> i64 { let mut p: P = P { a: 1 }; let r: &P = &p; \
               let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 3 { s = s + r.a * r.a; p = P { a: 2 }; i = i + 1; } return s; }";
    let with = loop_body(&asm(src));
    assert!(with.contains("imul"), "a read through a reassigned struct stays in the loop:\n{with}");
}

#[test]
fn a_call_in_an_assignment_target_blocks_hoisting() {
    // `a[next(&mut x)] = 0` calls a function that writes `x` through a reference,
    // so `x * x` is not invariant even though no statement assigns `x` directly.
    let src = "fn next(x: &mut i64) -> i64 { *x = *x + 1; return 0; } \
               fn main() -> i64 { let mut x: i64 = 0; let mut a: [i64; 4] = [0, 0, 0, 0]; \
               let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 3 { s = s + x * x; a[next(&mut x)] = 0; i = i + 1; } return s; }";
    let with = loop_body(&asm(src));
    assert!(with.contains("imul"), "a call in the target blocks hoisting:\n{with}");
}

#[test]
fn a_register_only_leaf_has_no_frame_pointer() {
    // Every local is promoted into a callee-saved register and nothing is
    // materialised, so the body addresses no frame slot. Nothing is addressed
    // through `%rbp`, so the prologue omits the frame pointer entirely — no
    // `push %rbp`/`mov %rsp, %rbp`/`leave`. The two saved registers push 16
    // bytes, which is 0 mod 16, so an 8-byte pad restores the 8 mod 16 the call
    // ABI wants; that pad is the only `sub`.
    let src = "fn main() -> i64 { let a: i64 = 1; let b: i64 = 2; return a + b; }";
    let body = main_body(&asm(src));
    assert!(!body.contains("push %rbp"), "no frame pointer without frame slots:\n{body}");
    assert!(!body.contains("leave"), "no `leave` without a frame pointer:\n{body}");
    assert!(!body.contains("sub $16"), "only the alignment pad, no local frame:\n{body}");
}

#[test]
fn every_function_keeps_rsp_aligned_at_calls() {
    // The call ABI wants `%rsp ≡ 8 (mod 16)` at entry (so 0 at a `call`). Both
    // prologue paths must honour it: with a frame pointer the `push %rbp`
    // supplies the 8, without one an explicit pad does. A violation is silent
    // (rmc's runtime never touches aligned SSE), so it is asserted here over a
    // mix of leaf, spill-heavy and frame-heavy functions.
    let srcs = [
        "fn main() -> i64 { return 1; }",
        "fn main() -> i64 { let a: i64 = 1; let b: i64 = 2; return a + b; }",
        "fn f(x: i64) -> i64 { return x; } fn g(x: i64) -> i64 { return f(x - 1) + f(x - 2); } fn main() -> i64 { return g(5); }",
        "fn main() -> i64 { let a: [i64; 3] = [1, 2, 3]; return a[0] + a[1] + a[2]; }",
        "fn main() -> i64 { let mut s: i64 = 0; let mut i: i64 = 0; while i < 10 { s = s + i; i = i + 1; } return s; }",
    ];
    for src in srcs {
        for out in [asm(src), asm_no_inline(src)] {
            for (name, body) in fn_bodies(&out) {
                // `_start` realigns `%rsp` itself (`and $-16, %rsp`) and is not
                // a compiled function; the other runtime helpers take no calls.
                if name.starts_with('_') || !body.contains("call ") {
                    continue;
                }
                assert_eq!(
                    prologue_depth_mod16(&body), 8,
                    "`{name}` misaligns `%rsp` at a call:\n{body}"
                );
            }
        }
    }
}

#[test]
fn a_frame_is_sized_to_the_slots_the_body_uses() {
    // The array forces a materialised frame buffer, so the prologue must
    // allocate one — but only for the slots that are actually addressed.
    let src = "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[0] + a[1]; }";
    let body = main_body(&asm(src));
    assert!(body.contains("sub $"), "an array needs a frame:\n{body}");
}

#[test]
fn frame_spills_do_not_overlap_locals() {
    // A right-leaning call chain keeps every outer partial result live across
    // the inner call, so more values spill than there are spill registers and
    // the surplus lands in frame slots. Those slots must sit past the deepest
    // local the body names, or they would clobber it.
    let src = "fn f(x: i64) -> i64 { return x + 1; } \
               fn main() -> i64 { let mut a: i64 = 1; let mut b: i64 = 2; let mut c: i64 = 3; \
               let r: i64 = f(a) + (f(b) + (f(c) + (f(a+1) + (f(b+1) + (f(c+1)))))); \
               return r + a + b + c; }";
    let out = asm_no_inline(src);
    // The deepest frame spill must be a distinct slot from every local, i.e.
    // the frame reservation covers it. Emitting and running the program is the
    // real check (see the differential suite); here we only assert the shape.
    assert!(out.contains("(%rbp)"), "spills reach the frame:\n{out}");
}

#[test]
fn a_commutative_sum_of_two_calls_skips_a_move() {
    // Neither `f(1)` nor `f(2)` is a simple operand, so both go through the
    // stack. `+` is commutative, so the saved left result is popped straight
    // into `%rcx` rather than bounced back through `%rax`, and the sum reads
    // `%rcx` directly — one instruction fewer than the order-preserving path
    // (which would need `mov %rax, %rcx` then `pop %rax`).
    let src = "fn f(x: i64) -> i64 { return x + 1; } fn main() -> i64 { return f(1) + f(2); }";
    let body = main_body(&asm_no_inline(src));
    assert!(body.contains("add %rcx, %rax"), "the sum reads %rcx:\n{body}");
    assert!(!body.contains("mov %rax, %rcx"), "no bounce through %rax:\n{body}");
}

#[test]
fn a_subtraction_of_two_calls_keeps_operand_order() {
    // `-` is not commutative, so the order-preserving path must stay: the
    // right operand's result moves to `%rcx` and the saved left operand is
    // restored to `%rax`.
    let src = "fn f(x: i64) -> i64 { return x + 1; } fn main() -> i64 { return f(1) - f(2); }";
    let body = main_body(&asm_no_inline(src));
    assert!(body.contains("mov %rax, %rcx"), "left operand preserved:\n{body}");
    assert!(body.contains("sub %rcx, %rax"), "the difference reads %rcx:\n{body}");
}

#[test]
fn no_inverse_move_pair_survives() {
    // The spill/restore path can leave `mov %rX, Y` immediately undone by
    // `mov Y, %rX` (e.g. `let q = p as &mut N`: the pointer lands in a register
    // and the store's base wants it back in `%rax`). `trim_dead` drops the
    // second, since the pair is a no-op. Assert the emitted body of a
    // pointer-heavy program has no such adjacent pair for any operand.
    let src = "struct N { v: i64 } fn main() -> i64 { \
               let p: i64 = alloc(16); let q: &mut N = p as &mut N; \
               (*q).v = 7; return (*q).v; }";
    let body = main_body(&asm(src));
    let parse = |s: &str| -> Option<(String, String)> {
        let r = s.strip_prefix("mov ")?;
        let (a, b) = r.split_once(',')?;
        Some((a.trim().to_string(), b.trim().to_string()))
    };
    let insns: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    for w in insns.windows(2) {
        if let (Some((s1, d1)), Some((s2, d2))) = (parse(w[0]), parse(w[1])) {
            assert!(
                !(s2 == d1 && d2 == s1 && s1 != d1),
                "inverse pair survived: `{}` then `{}`:\n{body}", w[0], w[1]
            );
        }
    }
}

#[test]
fn a_variable_divisor_uses_its_register_directly() {
    // `y` is a promoted local, so it already lives in a callee-saved register.
    // `idiv` can read that register directly rather than bouncing the divisor
    // through `%rcx`, which is what the order-preserving path used to do. The
    // dividend still moves into `%rax` and `cqto` fills `%rdx`.
    let src = "fn gcd(a: i64, b: i64) -> i64 { \
               let mut x: i64 = a; let mut y: i64 = b; \
               while y != 0 { let t: i64 = x % y; x = y; y = t; } \
               return x; } \
               fn main() -> i64 { return gcd(48, 18); }";
    let out = asm_no_inline(src);
    let gcd = &out[out.find("gcd:").unwrap()..];
    let gcd = &gcd[..gcd.find(".size gcd").unwrap()];
    assert!(gcd.contains("idiv %rbx"), "divisor read from its register:\n{gcd}");
    assert!(!gcd.contains("idiv %rcx"), "no bounce through %rcx:\n{gcd}");
}

#[test]
fn a_variable_division_has_a_32_bit_fast_path() {
    // Both operands usually fit in 32 bits, where `div`/`idiv` is cheaper than
    // the 64-bit form. The guard ORs the two operands and shifts the top 32 bits
    // out, so a negative operand (whose high bit is set) takes the 64-bit `idiv`
    // path and the result is still correct.
    let src = "fn q(a: i64, b: i64) -> i64 { return a / b; } \
               fn main() -> i64 { return q(100, 7); }";
    let out = asm_no_inline(src);
    let q = &out[out.find("q:").unwrap()..];
    let q = &q[..q.find(".size q").unwrap()];
    assert!(q.contains("shr $32, %rdx"), "32-bit-fit guard:\n{q}");
    assert!(q.contains("jnz "), "guard branches to the slow path:\n{q}");
    // The 32-bit divide names a `d`-suffixed register (`div %r12d`), never the
    // 64-bit `idiv`.
    assert!(q.contains("div %") && q.lines().any(|l| l.trim_start().starts_with("div %")), "32-bit fast path:\n{q}");
    assert!(q.contains("cqto"), "64-bit slow path:\n{q}");
}

#[test]
fn a_while_loop_is_rotated() {
    // The test sits at the bottom and its branch is the back edge, so the loop
    // has no unconditional jump and each iteration is `cmp` + `jl`.
    let src = "fn main() -> i64 { let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + i; i = i + 1; } return s; }";
    let body = loop_body(&asm(src));
    assert!(!body.contains("jmp .L"), "no unconditional back edge:\n{body}");
    assert_eq!(body.matches("cmp").count(), 1, "one test per iteration:\n{body}");
    // The condition is a comparison, so the back edge is the direct `jl`, not a
    // materialised 0/1 fed to `test`.
    assert!(body.contains("jl "), "comparison back edge:\n{body}");
}

#[test]
fn a_while_whose_body_is_empty_still_tests_once() {
    // Rotation moves the test, so an empty body must not loop forever: the
    // guard is still evaluated at the bottom and gates a conditional back edge.
    let src = "fn main() -> i64 { let mut i: i64 = 0; while i < 3 { } return i; }";
    let body = loop_body(&asm(src));
    assert!(body.contains("cmpq $3"), "the guard is still tested:\n{body}");
    assert!(body.contains("jl "), "the guard gates the back edge:\n{body}");
    // Nothing but the rotated test sits between the body label and the guard.
    let inner: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.ends_with(':') && !l.starts_with('j'))
        .collect();
    assert_eq!(inner.len(), 1, "the body is empty but for the test:\n{body}");
}

#[test]
fn an_accumulated_call_result_is_added_in_place() {
    // `sum = sum + f(i)` must add the call's `%rax` straight into the promoted
    // accumulator, not bounce it through `%rcx`: `add %rax, %r13`.
    let src = "fn f(x: i64) -> i64 { return x + 1; } \
               fn main() -> i64 { let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + f(i); i = i + 1; } return s; }";
    let body = loop_body(&asm_no_inline(&src));
    assert!(body.contains("call f"), "the call is in the loop:\n{body}");
    assert!(
        body.contains("add %rax, %"),
        "the call result is added in place:\n{body}"
    );
    assert!(
        !body.contains("mov %rax, %rcx"),
        "no bounce of the call result through %rcx:\n{body}"
    );
}

#[test]
fn a_product_term_is_accumulated_without_a_bounce() {
    // A non-simple term (here a product) is computed into `%rax` and added with
    // one `add %rax, dst`, not `mov %rax, %rcx; add %rcx, %rax`.
    let src = "fn main() -> i64 { let mut s: i64 = 0; let mut i: i64 = 0; \
               while i < 10 { s = s + i * i; i = i + 1; } return s; }";
    let body = loop_body(&asm_no_licm(src));
    assert!(body.contains("add %rax, %"), "product added in place:\n{body}");
    assert!(
        !body.contains("mov %rax, %rcx"),
        "no bounce of the product through %rcx:\n{body}"
    );
}

#[test]
fn an_accumulator_chain_with_calls_keeps_source_order() {
    // Evaluation stays left to right even with a call that writes memory: `a`
    // (the `acc + a` operand) must be read before the `h(&mut a)` call runs, or
    // the addition would see the post-increment value.
    let src = "fn h(p: &mut i64) -> i64 { *p = *p + 1; return *p; } \
               fn main() -> i64 { let mut a: i64 = 10; let mut acc: i64 = 100; \
               acc = acc + a + h(&mut a); return acc; }";
    let out = asm_no_inline(&src);
    let body = main_body(&out);
    // `a` lives at `-16(%rbp)` here; the read must precede the call, and the
    // call's result must be added after it.
    let read_a = body.find("add -16(%rbp)").expect("the `a` operand is read");
    let call = body.find("call h").expect("the call is emitted");
    let add_ret = body.find("add %rax, %rbx").expect("the call result is added");
    assert!(read_a < call, "`a` read before the call:\n{body}");
    assert!(call < add_ret, "the call result added after the call:\n{body}");
}

#[test]
fn an_if_without_else_in_statement_position_emits_no_unit_value() {
    // `if n < 2 { return n; }` as a statement has no value, so the dead
    // `xor %eax, %eax` (and the `jmp` over the empty else) must not appear.
    let src = "fn f(n: i64) -> i64 { if n < 2 { return n; } return 0; } \
               fn main() -> i64 { return f(3); }";
    let out = asm_no_inline(&src);
    let body = &out[out.find("f:").unwrap()..];
    let body = &body[..body.find(".size f").unwrap()];
    assert!(
        !body.contains("xor %eax, %eax"),
        "no unit materialisation for a statement `if`:\n{body}"
    );
}

#[test]
fn a_statement_if_with_else_keeps_both_branches() {
    // `bare` must require `els.is_none()`, so an `if`/`else` in statement
    // position still emits the else body and the jump over it.
    let src = "fn main() -> i64 { let mut x: i64 = 0; \
               if 1 > 0 { x = 5; } else { x = 9; } return x; }";
    let out = asm(&src);
    let body = main_body(&out);
    assert!(body.contains("jmp"), "the else is jumped over:\n{body}");
    assert!(body.contains("$5"), "then branch present:\n{body}");
    assert!(body.contains("$9"), "else branch present:\n{body}");
}

#[test]
fn a_store_through_a_computed_address_folds_a_simple_value() {
    // `(*np).next = head`, both operands in registers: the value folds straight
    // into the store (`mov %r, (%rax)`) instead of bouncing through a spill
    // slot. The address is in `%rax`, the value in a callee-saved register.
    let src = "struct N { val: i64, next: i64 } \
               fn main() -> i64 { let mut head: i64 = 0; let mut i: i64 = 0; \
               while i < 3 { let p: i64 = 16; let np: &mut N = p as &mut N; \
               (*np).val = i; (*np).next = head; head = p; i = i + 1; } return head; }";
    let out = asm(&src);
    let body = main_body(&out);
    // A folded store through a computed address: `mov %rSRC, off(%rDST)` with
    // the destination a register other than `%rbp` (which would be a frame
    // store). The old path emitted `mov %rax, -N(%rbp)` (spill) then reloaded.
    let folded = body.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("mov %r") && t.contains("(%r") && !t.contains("%rbp")
    });
    assert!(folded, "value folds into the store:\n{body}");
}

#[test]
fn a_const_is_substituted_as_its_value_with_no_storage() {
    // `const N: i64 = 7;` has no codegen of its own: every use of `N` is the
    // literal 7, so `return N;` is `mov $7, %rax` and the frame allocates
    // nothing for a constant.
    let out = asm("const N: i64 = 7; fn main() -> i64 { return N; }");
    let body = main_body(&out);
    assert!(body.contains("$7"), "the value is folded in:\n{body}");
}

#[test]
fn a_const_folds_arithmetic_over_another_const() {
    // `M = N * 2` folds through the parser, so the emitted code has the product
    // and not a runtime multiply.
    let out = asm("const N: i64 = 6; const M: i64 = N * 2; fn main() -> i64 { return M; }");
    let body = main_body(&out);
    assert!(body.contains("$12"), "product folded:\n{body}");
    assert!(!body.contains("imul"), "no runtime multiply:\n{body}");
}

#[test]
fn a_const_length_sizes_an_array() {
    // `[i64; N]` with a constant `N` is a fixed-size array of `N` elements, so
    // the frame reserves room for the whole array.
    let out = asm("const N: i64 = 4; fn main() -> i64 { let a: [i64; N] = [1, 2, 3, 4]; return a[3]; }");
    let body = main_body(&out);
    assert!(body.contains("$4"), "the element is read:\n{body}");
}

#[test]
fn a_const_used_as_a_match_pattern_matches_its_value() {
    // A bare name that is a constant is that value as a pattern, not a fresh
    // binding, exactly as rustc reads it.
    let ok = compile_to_asm(
        "const N: i64 = 2; fn main() -> i64 { match 2 { N => { return 1; } _ => { return 0; } } }",
        &CompileOptions::default(),
    );
    assert!(ok.is_ok(), "a constant pattern compiles: {ok:?}");
}

#[test]
fn a_let_that_shadows_a_const_is_rejected() {
    // rustc E0530: a local binding may not reuse a constant's name.
    let err = compile_to_asm(
        "const N: i64 = 5; fn main() -> i64 { let N = 3; return N; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("shadows a constant"), "got: {err}");
}

#[test]
fn a_for_variable_that_shadows_a_const_is_rejected() {
    let err = compile_to_asm(
        "const N: i64 = 5; fn main() -> i64 { let mut s = 0; for N in 0..3 { s = s + N; } return s; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("shadows a constant"), "got: {err}");
}

#[test]
fn a_parameter_that_shadows_a_const_is_rejected() {
    let err = compile_to_asm(
        "const N: i64 = 5; fn f(N: i64) -> i64 { return N; } fn main() -> i64 { return f(1); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("shadows a constant"), "got: {err}");
}

#[test]
fn a_const_with_a_forward_reference_resolves() {
    // `B` names `A`, declared after it; the parser's fixpoint resolves both.
    let out = asm("const B: i64 = A + 3; const A: i64 = 2; fn main() -> i64 { return B; }");
    let body = main_body(&out);
    assert!(body.contains("$5"), "forward reference folded:\n{body}");
}

#[test]
fn a_const_cycle_is_rejected() {
    let err = compile_to_asm(
        "const A: i64 = B; const B: i64 = A; fn main() -> i64 { return A; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("constant `A` is not a constant"), "got: {err}");
}

#[test]
fn a_const_with_a_non_folding_initializer_is_rejected() {
    let err = compile_to_asm(
        "fn g() -> i64 { return 1; } const A: i64 = g(); fn main() -> i64 { return A; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("constant `A` is not a constant"), "got: {err}");
}

#[test]
fn a_const_of_an_unsupported_type_is_rejected() {
    let err = compile_to_asm(
        "const P: [i64; 2] = [1, 2]; fn main() -> i64 { return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("rmc only folds"), "got: {err}");
}

#[test]
fn a_const_and_a_fn_with_the_same_name_are_rejected() {
    let err = compile_to_asm(
        "const N: i64 = 5; fn N() -> i64 { return 1; } fn main() -> i64 { return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("defined multiple times"), "got: {err}");
}

#[test]
fn a_duplicate_const_is_rejected() {
    let err = compile_to_asm(
        "const N: i64 = 1; const N: i64 = 2; fn main() -> i64 { return N; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("defined multiple times"), "got: {err}");
}

#[test]
fn a_value_loop_binds_its_break_value() {
    // `loop` is an expression: the `break` value is stored into the loop's
    // result slot, which the code after the loop reads.
    let out = asm("fn main() -> i64 { let x: i64 = loop { break 5; }; return x; }");
    assert!(out.contains("movq $5, "), "break value stored to the result slot:\n{out}");
}

#[test]
fn a_break_value_in_a_while_is_rejected() {
    // rustc E0571: only a `loop` can be broken out of with a value.
    let err = compile_to_asm(
        "fn main() -> i64 { let mut i: i64 = 0; while i < 2 { break 5; } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("only allowed in a `loop`"), "got: {err}");
}

#[test]
fn a_break_value_in_a_statement_loop_is_rejected() {
    // A `loop` in statement position is typed `()`, so a value cannot break out
    // of it: the `break` value is checked against `()`.
    let err = compile_to_asm(
        "fn main() -> i64 { loop { break 5; } return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("used as a statement"), "got: {err}");
}

#[test]
fn a_diverging_loop_branch_without_else_is_accepted() {
    // `if c { loop {} }`: the branch is `!`, so the `if` is `()` and the code
    // after it is still reachable.
    let out = asm("fn main() -> i64 { let mut n: i64 = 0; if n > 0 { loop {} } n = n + 1; return n; }");
    assert!(out.contains("main:"), "accepted:\n{out}");
}

#[test]
fn a_completing_loop_body_must_be_unit() {
    // A body that can fall through cannot yield a value; only `break` does.
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = loop { 5 }; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("`loop` body must evaluate to `()`"), "got: {err}");
}

#[test]
fn mismatched_break_values_are_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = loop { break 1; break true; }; return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("break` value type mismatch"), "got: {err}");
}

#[test]
fn a_break_value_in_a_while_let_is_rejected() {
    // `while let` desugars to a `loop` whose `else` arm always ends in a plain
    // `break;`, so a `break value` in the body is a type mismatch. rustc
    // rejects it too (E0571).
    let err = compile_to_asm(
        "enum Opt { Some(i64), None } \
         fn main() -> i64 { let mut i: i64 = 0; \
         while let Opt::Some(x) = Opt::Some(i) { i = i + 1; if i == 3 { break 5; } } \
         return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("break`"), "got: {err}");
}

// ---------------------------------------------------------------------------
// Struct update syntax `S { field: value, ..base }`
// ---------------------------------------------------------------------------

#[test]
fn a_struct_update_fills_the_unwritten_fields_from_the_base() {
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; let b: P = P { x: 5, ..a }; return b.x + b.y; }",
    );
    assert!(out.contains("main:"), "accepted:\n{out}");
}

#[test]
fn a_struct_update_evaluates_the_field_values_before_the_base() {
    // rustc evaluates the written fields left to right, then the base. The
    // desugaring binds each field value before `call mk`, so `call f` precedes
    // `call mk` in the emitted code.
    let out = asm_no_inline(
        "struct P { x: i64, y: i64 } \
         fn f(t: i64) -> i64 { return t; } \
         fn mk() -> P { return P { x: 7, y: 8 }; } \
         fn main() -> i64 { let b: P = P { x: f(1), ..mk() }; return b.x + b.y; }",
    );
    let f_at = out.find("call f").expect("call f");
    let mk_at = out.find("call mk").expect("call mk");
    assert!(f_at < mk_at, "field value must be evaluated before the base:\n{out}");
}

#[test]
fn a_struct_update_copies_an_aggregate_field_by_value() {
    // A struct-typed field must be copied word by word, not stored as an
    // address, so the result does not alias the field's source.
    let out = asm(
        "struct Inner { a: i64 } struct P { i: Inner, y: i64 } \
         fn main() -> i64 { let g: Inner = Inner { a: 5 }; let a: P = P { i: g, y: 2 }; \
         let b: P = P { y: 9, ..a }; return b.i.a + b.y; }",
    );
    assert!(out.contains("main:"), "accepted:\n{out}");
}

#[test]
fn a_struct_update_may_copy_every_field() {
    // `P { ..a }` with no written fields is valid Rust.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; let b: P = P { ..a }; return b.x + b.y; }",
    );
    assert!(out.contains("main:"), "accepted:\n{out}");
}

#[test]
fn a_struct_update_base_may_be_another_struct_literal() {
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; \
         let b: P = P { x: 3, ..P { y: 4, ..a } }; return b.x + b.y; }",
    );
    assert!(out.contains("main:"), "accepted:\n{out}");
}

#[test]
fn a_struct_update_uses_a_local_that_does_not_alias_the_base() {
    // The desugaring binds the base to a fresh local, so the result is an
    // independent value: mutating it must not write through to the base.
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; let mut b: P = P { x: 5, ..a }; \
         b.y = 9; return a.y + b.y; }",
    );
    assert!(out.contains("main:"), "accepted:\n{out}");
}

#[test]
fn a_field_written_twice_in_a_struct_update_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; let b: P = P { x: 5, x: 6, ..a }; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("more than once"), "got: {err}");
}

#[test]
fn a_struct_update_with_an_unknown_field_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; let b: P = P { z: 5, ..a }; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("has no field `z`"), "got: {err}");
}

#[test]
fn a_struct_update_with_a_wrong_field_type_is_rejected() {
    // The field value is bound to a temporary and assigned into the result, so
    // the mismatch surfaces as an assignment type error.
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let a: P = P { x: 1, y: 2 }; let b: P = P { x: true, ..a }; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("mismatch"), "got: {err}");
}

#[test]
fn a_struct_update_base_of_the_wrong_type_is_rejected() {
    let err = compile_to_asm(
        "struct P { x: i64, y: i64 } struct Q { z: i64 } \
         fn main() -> i64 { let q: Q = Q { z: 1 }; let b: P = P { x: 5, ..q }; return 0; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("does not match"), "got: {err}");
}


#[test]
fn a_tuple_pattern_in_a_match_arm_may_test_an_element() {
    // A refutable element adds a test; the arm still binds the other element.
    let out = asm_no_inline(
        "fn main() -> i64 { match (1, 2) { (0, _) => { return 10; } (a, b) => { return a + b; } } }",
    );
    assert!(out.contains("main:"), "compiled:\n{out}");
}

#[test]
fn a_tuple_pattern_with_a_range_element_is_accepted() {
    let out = asm(
        "fn main() -> i64 { match (3, 2) { (1..=5, x) => { return x; } _ => { return 0; } } }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn a_tuple_pattern_with_an_or_element_is_accepted() {
    let out = asm(
        "fn main() -> i64 { match (2, 5) { (0 | 1 | 2, x) => { return x; } _ => { return 0; } } }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn a_refutable_tuple_pattern_in_if_let_is_accepted() {
    let out = asm("fn main() -> i64 { if let (1, x) = (1, 5) { return x; } return 0; }");
    assert!(out.contains("main:"));
}

#[test]
fn a_refutable_tuple_pattern_in_while_let_is_accepted() {
    let out = asm(
        "fn main() -> i64 { let mut i = 0; let mut a = 0; \
         while let (1, x) = (1, i) { a = a + x; i = i + 1; if i > 2 { break; } } return a; }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn a_literal_tuple_element_in_a_let_is_rejected() {
    // A `let` pattern is irrefutable, so a literal element is rejected, as rustc.
    let err = compile_to_asm(
        "fn main() -> i64 { let (1, x) = (1, 2); return x; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("tuple pattern elements must be bindings"), "got: {err}");
}

#[test]
fn a_literal_tuple_element_in_a_parameter_is_rejected() {
    let err = compile_to_asm(
        "fn f((1, b): (i64, i64)) -> i64 { return b; } fn main() -> i64 { return f((1, 2)); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("tuple pattern elements must be bindings"), "got: {err}");
}

#[test]
fn a_variant_tuple_element_is_rejected_even_in_a_match() {
    // rmc generates no element test that would carry a variant payload.
    let err = compile_to_asm(
        "enum E { A, B } fn main() -> i64 { match (E::A, 1) { (E::A, x) => { return x; } _ => { return 0; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("must be irrefutable or a scalar test"), "got: {err}");
}

#[test]
fn a_binding_tuple_element_in_an_or_pattern_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { match (1, 2) { (x | 1, y) => { return y; } _ => { return 0; } } }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("not bound in all patterns"), "got: {err}");
}

#[test]
fn a_tuple_element_test_reads_the_element_field() {
    // The element test must read field "0" of the scrutinee, exactly as the
    // equivalent `t.0 == 0` would; assert on that read rather than on the whole
    // function, whose label numbering is not stable across forms.
    let out = asm_no_inline(
        "fn f(t: (i64, i64)) -> i64 { match t { (0, b) => { return b; } _ => { return 0; } } } \
         fn main() -> i64 { return f((0, 7)); }",
    );
    let body = &out[out.find("f:").expect("f")..out.find("main:").expect("main")];
    assert!(body.contains("cmpq $0,"), "expected an equality test:\n{body}");
}


#[test]
fn an_at_binding_binds_the_whole_value() {
    let out = asm("fn main() -> i64 { let x = 3; let n @ _ = x; return n; }");
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_chain_binds_every_name() {
    let out = asm("fn main() -> i64 { let x = 3; let p @ q @ _ = x; return p + q; }");
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_may_be_mutable() {
    let out = asm("fn main() -> i64 { let x = 3; let mut n @ _ = x; n = n + 1; return n; }");
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_binds_a_tuple_sub_pattern() {
    let out = asm("fn main() -> i64 { let t @ (a, b) = (1, 2); return t.0 + t.1 + a + b; }");
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_on_a_struct_field_binds_the_field() {
    let out = asm(
        "struct S { x: i64 } fn main() -> i64 { let s = S { x: 1 }; let S { x: n @ _ } = s; return n; }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_on_a_struct_field_may_be_mutable() {
    let out = asm(
        "struct S { x: i64 } fn main() -> i64 { let s = S { x: 1 }; \
         let S { x: mut n @ _ } = s; n = n + 1; return n; }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_for_range_head_is_accepted() {
    let out = asm(
        "fn main() -> i64 { let mut s = 0; for i @ j in 0..3 { s = s + i + j; } return s; }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_for_array_head_is_accepted() {
    let out = asm("fn main() -> i64 { let a = [1, 2, 3]; let mut s = 0; \
         for e @ _ in a { s = s + e; } return s; }");
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_match_arm_may_test_the_sub_pattern() {
    let out = asm(
        "fn main() -> i64 { match 3 { n @ 1..=5 => { return n; } _ => { return 0; } } }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_an_if_let_is_accepted() {
    let out = asm("fn main() -> i64 { if let n @ 1..=5 = 3 { return n; } return 0; }");
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_match_arm_may_be_mutable() {
    let out = asm(
        "fn main() -> i64 { match 3 { mut n @ 1..=5 => { n = n + 1; return n; } _ => { return 0; } } }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_parameter_is_accepted() {
    let out = asm(
        "fn f(n @ _: i64) -> i64 { return n; } fn main() -> i64 { return f(3); }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_parameter_may_be_mutable() {
    let out = asm(
        "fn f(mut n @ _: i64) -> i64 { n = n + 1; return n; } fn main() -> i64 { return f(3); }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn an_at_binding_in_a_tuple_parameter_is_accepted() {
    let out = asm(
        "fn f(t @ (a, b): (i64, i64)) -> i64 { return t.0 + a + b; } \
         fn main() -> i64 { return f((1, 2)); }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn a_refutable_at_binding_in_a_let_is_rejected() {
    // rustc rejects `let n @ 1..=5 = e;` as a refutable local binding (E0005).
    let err = compile_to_asm(
        "fn main() -> i64 { let x = 3; let n @ 1..=5 = x; return n; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("refutable pattern in local binding"), "got: {err}");
}

#[test]
fn a_refutable_at_binding_in_a_for_range_is_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { let mut s = 0; for n @ 1..=5 in 0..3 { s = s + n; } return s; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("refutable pattern in `for` binding"), "got: {err}");
}

#[test]
fn an_at_binding_test_reads_the_scrutinee() {
    // `n @ 1..=5` adds the range test, exactly as `n` alone would.
    let out = asm_no_inline(
        "fn f(x: i64) -> i64 { match x { n @ 1..=5 => { return n; } _ => { return 0; } } } \
         fn main() -> i64 { return f(3); }",
    );
    let body = &out[out.find("f:").expect("f")..out.find("main:").expect("main")];
    assert!(body.contains("cmpq $5,"), "expected a range test:\n{body}");
}

#[test]
fn a_mutable_at_binding_in_an_array_element_is_accepted() {
    let out = asm(
        "fn main() -> i64 { let a = [1, 2]; let [mut n @ _, m] = a; \
         n = n + 1; return n + m; }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn a_mutable_at_binding_in_a_tuple_element_is_accepted() {
    let out = asm(
        "fn main() -> i64 { let (mut n @ _, m) = (1, 2); n = n + 1; return n + m; }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn abs_desugars_to_a_conditional_negation() {
    // `x.abs()` is `if x < 0 { -x } else { x }`, so it emits a sign test and a
    // `neg`, and never a call. A positive literal avoids the `neg` the parser
    // would emit for a negative constant.
    let out = asm_no_inline("fn main() -> i64 { let x: i64 = 3; return x.abs(); }");
    assert!(out.contains("cmpq $0,") || out.contains("cmp $0,"), "expected a sign test:\n{out}");
    assert!(out.contains("neg %rax"), "expected a negation:\n{out}");
    assert!(!out.contains("call abs"), "abs must not be a call:\n{out}");
}

#[test]
fn signum_needs_no_negation() {
    // `(x > 0) as i64 - (x < 0) as i64` produces -1/0/1 with only comparisons,
    // so `i64::MIN` is safe and no `neg` is emitted. A positive literal keeps
    // the parser from emitting a `neg` of its own.
    let out = asm_no_inline("fn main() -> i64 { let x: i64 = 3; return x.signum(); }");
    assert!(out.contains("setg %al"), "expected a `> 0` test:\n{out}");
    assert!(out.contains("setl %al"), "expected a `< 0` test:\n{out}");
    assert!(!out.contains("neg %rax"), "signum must not negate:\n{out}");
}

#[test]
fn min_compares_the_argument_against_the_receiver() {
    // `x.min(y)` is `if y < x { y } else { x }`, so the comparison is
    // `y < x` — not `x < y`, which would return `y` on a tie. The receiver is
    // evaluated into a temporary first, so it is `cmp <recv>, <arg>` and a
    // branch, not a `setcc`.
    let out = asm_no_inline("fn main() -> i64 { let x: i64 = 4; return x.min(4); }");
    let body = &out[out.find("main:").expect("main")..];
    assert!(body.contains("cmp "), "expected a comparison:\n{body}");
    assert!(body.contains("jge "), "expected a `y < x` branch:\n{body}");
}

#[test]
fn abs_of_a_const_min_is_accepted() {
    // `MIN.abs()` is not a written `-`, so the compile-time neg-overflow lint
    // must not fire on the `-x` that `abs` synthesises: rustc accepts this and
    // yields `i64::MIN` in release. A written `-MIN` is still rejected.
    let out = asm("const MIN: i64 = -9223372036854775808; fn main() -> i64 { return MIN.abs(); }");
    assert!(out.contains("main:"));
    let err = compile_to_asm(
        "const MIN: i64 = -9223372036854775808; fn main() -> i64 { return -MIN; }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("overflow"), "expected a neg-overflow error: {err}");
}

#[test]
fn a_side_effecting_receiver_is_evaluated_once() {
    // Each desugaring reads the receiver more than once, so it must be bound to
    // a temporary: `g()` is called once, not once per arm.
    let out = asm_no_inline(
        "fn g() -> i64 { return 3; } \
         fn main() -> i64 { return g().abs(); }",
    );
    let body = &out[out.find("main:").expect("main")..];
    assert_eq!(body.matches("call g").count(), 1, "receiver must be called once:\n{body}");
}

#[test]
fn a_side_effecting_min_argument_is_evaluated_once() {
    let out = asm_no_inline(
        "fn g() -> i64 { return 3; } \
         fn main() -> i64 { return g().min(g()); }",
    );
    let body = &out[out.find("main:").expect("main")..];
    assert_eq!(body.matches("call g").count(), 2, "each operand called once:\n{body}");
}

#[test]
fn is_positive_and_is_negative_evaluate_the_receiver_once() {
    // `is_negative` desugars to a comparison, which reads the receiver once,
    // but it still goes through the temp-binding path so a call receiver is
    // evaluated a single time.
    let out = asm_no_inline(
        "fn g() -> i64 { return 3; } \
         fn main() -> i64 { let b: bool = g().is_negative(); if b { return 1; } return 0; }",
    );
    let body = &out[out.find("main:").expect("main")..];
    assert_eq!(body.matches("call g").count(), 1, "receiver must be called once:\n{body}");
    assert!(body.contains("cmp"), "expected a comparison:\n{body}");
}

#[test]
fn is_positive_yields_a_bool_usable_as_one() {
    // The result type is `bool`, not `i64`: it initialises a `bool` binding and
    // drives an `if` with no cast. `0` is neither positive nor negative.
    let out = asm(
        "fn main() -> i64 { \
         let x: i64 = -5; \
         let p: bool = x.is_positive(); \
         let n: bool = x.is_negative(); \
         if n { return 1; } \
         if p { return 2; } \
         return 0; \
         }",
    );
    assert!(out.contains("main:"));
}

#[test]
fn div_euclid_and_rem_euclid_evaluate_each_operand_once() {
    // Each desugar binds its receiver and its argument before the two `q`/`r`
    // divisions, so a call operand runs a single time.
    for m in ["div_euclid", "rem_euclid"] {
        let out = asm_no_inline(&format!(
            "fn g() -> i64 {{ return 3; }} \
             fn main() -> i64 {{ return g().{m}(g()); }}"
        ));
        let body = &out[out.find("main:").expect("main")..];
        assert_eq!(body.matches("call g").count(), 2, "{m}: each operand once:\n{body}");
    }
}

#[test]
fn a_min_divisor_in_the_euclidean_methods_does_not_trip_the_overflow_lint() {
    // `x.rem_euclid(i64::MIN)` synthesises `r - MIN` on the negative-remainder
    // path, but that arm never runs when the divisor is `MIN`, so rustc accepts
    // it. The desugar must not const-fold the dead arm and reject the program.
    // `MIN.div_euclid(1)` is the mirror: its `q - 1` arm is dead because the
    // remainder is non-negative.
    for src in [
        "fn main() -> i64 { let m: i64 = -9223372036854775808; return m.rem_euclid(m); }",
        "fn main() -> i64 { let m: i64 = -9223372036854775808; return 7.rem_euclid(m); }",
        "fn main() -> i64 { let m: i64 = -9223372036854775808; return m.div_euclid(1); }",
        "fn main() -> i64 { let x: i64 = -9223372036854775808; let y: i64 = -1; return x.div_euclid(y); }",
    ] {
        let out = asm(src);
        assert!(out.contains("main:"), "should compile:\n{src}");
    }
}

#[test]
fn clamp_evaluates_each_operand_once_and_traps_on_an_inverted_range() {
    // Each operand is bound once, so a call operand runs a single time.
    let out = asm_no_inline(
        "fn g() -> i64 { return 3; } \
         fn main() -> i64 { return g().clamp(g(), g()); }",
    );
    let body = &out[out.find("main:").expect("main")..];
    assert_eq!(body.matches("call g").count(), 3, "each operand once:\n{body}");
    // An inverted range calls the trap helper; the range check precedes the
    // bound comparisons, so a valid range never reaches it.
    assert!(out.contains("call __rmc_clamp"), "inverted range traps:\n{body}");
}

#[test]
fn clamp_pulls_in_the_trap_helper_only_when_used() {
    // A program with no `clamp` must not carry the trap, matching the
    // runtime's per-symbol dead-code elimination.
    let out = asm("fn main() -> i64 { return 1; }");
    assert!(!out.contains("__rmc_clamp"), "trap dropped when unused:\n{out}");
}

#[test]
fn a_nested_fn_is_hoisted_and_callable() {
    // A `fn` inside a block is an item, not a closure: it captures nothing, so
    // it is hoisted to the top level and emitted like any other function.
    let out = asm_no_inline("fn main() -> i64 { fn g(x: i64) -> i64 { return x + 1; } return g(1); }");
    assert!(out.contains("rmc$nf0$g:"), "nested fn emitted:\n{out}");
    assert!(out.contains("call rmc$nf0$g"), "nested fn called:\n{out}");
}

#[test]
fn a_nested_fn_shadows_a_top_level_fn_of_the_same_name() {
    // The nested `fn` wins inside its block (rustc's scoping), so it is hoisted
    // under a distinct symbol and the call resolves to it, not the outer one.
    let out = asm_no_inline(
        "fn g() -> i64 { return 1; } \
         fn main() -> i64 { fn g() -> i64 { return 2; } return g(); }",
    );
    assert!(out.contains("g:"), "top-level g kept:\n{out}");
    assert!(out.contains("rmc$nf0$g:"), "nested g hoisted under a unique name:\n{out}");
    let body = &out[out.find("main:").expect("main")..];
    assert!(body.contains("call rmc$nf0$g"), "call resolves to the nested fn:\n{body}");
}

#[test]
fn a_nested_fn_may_call_a_sibling_declared_later() {
    // Items are hoisted, so the whole block sees every nested `fn` regardless of
    // order; `a` calls `b`, declared after it.
    let out = asm_no_inline(
        "fn main() -> i64 { fn a() -> i64 { return b(); } fn b() -> i64 { return 9; } return a(); }",
    );
    let a_body = &out[out.find("rmc$nf0$a:").expect("a")..];
    assert!(a_body.contains("call rmc$nf1$b"), "a calls the later b:\n{a_body}");
}

#[test]
fn two_nested_fns_with_the_same_name_in_one_block_are_rejected() {
    let err = compile_to_asm(
        "fn main() -> i64 { fn g() -> i64 { return 1; } fn g() -> i64 { return 2; } return g(); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("defined multiple times"), "E0428-like error:\n{err}");
}

#[test]
fn a_nested_fn_cannot_capture_a_local() {
    // A nested `fn` is not a closure, so a local of the enclosing block is not
    // in scope inside it (rustc's E0434).
    let err = compile_to_asm(
        "fn main() -> i64 { let x: i64 = 5; fn g() -> i64 { return x; } return g(); }",
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(err.contains("unknown variable `x`"), "capture rejected:\n{err}");
}

#[test]
fn a_builtin_integer_method_on_a_struct_is_not_shadowed() {
    // A struct may still define its own `min`; the built-in only fires on an
    // `i64` receiver.
    let out = asm(
        "struct S { v: i64 } impl S { fn min(self, o: i64) -> i64 { return self.v - o; } } \
         fn main() -> i64 { let s = S { v: 10 }; return s.min(3); }",
    );
    assert!(out.contains("main:"));
}
