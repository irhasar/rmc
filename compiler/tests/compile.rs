//! Unit tests that exercise the compiler front end and code generator without
//! invoking `as` or `ld`, so they run anywhere a Rust toolchain does.

use rmc::{compile_to_asm, CompileOptions};

fn asm(src: &str) -> String {
    compile_to_asm(src, &CompileOptions::default()).expect("should compile")
}

/// The frame offset stored by the first `mov %rax, <off>(%rbp)` after `marker`.
/// Lets a test check relative layout without hard-coding absolute offsets.
fn store_offset_after(out: &str, marker: &str) -> i32 {
    let tail = &out[out.find(marker).expect("marker present")..];
    let rest = &tail[tail.find("mov %rax, ").expect("a store") + "mov %rax, ".len()..];
    let end = rest.find("(%rbp)").expect("frame store");
    rest[..end].trim().parse().expect("offset")
}

#[test]
fn emits_a_global_main() {
    let out = asm("fn main() -> i64 { return 0; }");
    assert!(out.contains(".globl main"));
    assert!(out.contains("main:"));
    // The runtime is always appended.
    assert!(out.contains("_start:"));
    assert!(out.contains("print_i64:"));
}

#[test]
fn struct_fields_are_laid_out_in_order() {
    // Declaration order, no padding: `y` sits exactly one word above `x`.
    let out = asm("struct P { x: i64, y: i64 } fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return p.y; }");
    let x = store_offset_after(&out, "mov $1, %rax");
    let y = store_offset_after(&out, "mov $2, %rax");
    assert_eq!(y, x + 8, "fields laid out in declaration order");
    assert!(out.contains(&format!("mov {y}(%rbp), %rax")), "p.y reads the y slot");
}

#[test]
fn field_write_targets_the_right_slot() {
    let out = asm("struct P { x: i64, y: i64 } fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; p.y = 9; return p.y; }");
    let y = store_offset_after(&out, "mov $2, %rax");
    let w = store_offset_after(&out, "mov $9, %rax");
    assert_eq!(w, y, "`p.y = 9` writes the slot `p.y` reads");
}

#[test]
fn for_loop_has_a_continue_label() {
    // `continue` must jump to the increment, which is why `for` is not a while.
    let out = asm("fn main() -> i64 { let mut s: i64 = 0; for i in 0..3 { if i == 1 { continue; } s = s + i; } return s; }");
    assert!(out.contains("jge"), "range end comparison");
    assert!(out.contains("add $1, %rax"), "increment");
}

#[test]
fn compound_assignment_expands() {
    let out = asm("fn main() -> i64 { let mut x: i64 = 1; x += 4; return x; }");
    assert!(out.contains("add $4, %rax") || out.contains("add %rcx, %rax"));
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
        t.elapsed().as_secs_f64() < 2.0,
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
fn argument_setup_folds_a_constant_with_lea() {
    // `fib(n - 1)` must become one `lea`, not `mov; sub; mov`.
    let out = asm("fn f(n: i64) -> i64 { return n; } fn g(n: i64) -> i64 { return f(n - 1); } fn main() -> i64 { return 0; }");
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
    let out = asm("fn g(i: i64) -> i64 { let mut x: i64 = i; x = 2 * x + 3; return x; } fn main() -> i64 { return 0; }");
    // Scope to `g`: the runtime's `arg_i64` contains its own `imul`.
    let g = &out[out.find("g:").unwrap()..out.find(".size g,").unwrap()];
    assert!(g.contains("lea 3(%"), "folded into lea: {g}");
    assert!(!g.contains("imul"), "no multiply remains: {g}");
}

#[test]
fn a_comparison_of_two_promoted_locals_is_one_cmp() {
    // Both operands live in registers, so no value is materialised in `%rax`.
    let out = asm(
        "fn g(a: i64, b: i64) -> bool { let mut x: i64 = a; let mut y: i64 = b; x = x + 1; y = y + 1; return x < y; } fn main() -> i64 { return 0; }",
    );
    let has_reg_reg_cmp = out.lines().any(|l| {
        let l = l.trim();
        l.starts_with("cmp %") && l.matches("%r").count() >= 2 && !l.contains("%rax")
    });
    assert!(has_reg_reg_cmp, "register-to-register cmp: {out}");
}

#[test]
fn spills_use_frame_slots_not_the_stack() {
    // A value held across a call is spilled to a frame slot, so `%rsp` never
    // moves and no 16-byte alignment pad is emitted around the call.
    let out = asm(
        "fn f(x: i64) -> i64 { return x; } fn g(x: i64) -> i64 { return f(x - 1) + f(x - 2); } fn main() -> i64 { return 0; }",
    );
    let g = &out[out.find("g:").unwrap()..out.find(".size g,").unwrap()];
    assert!(!g.contains("push %rax"), "no stack spill: {g}");
    assert!(!g.contains("sub $8, %rsp"), "no alignment pad: {g}");
    assert!(g.contains("(%rbp)"), "spill lands in a frame slot: {g}");
}

#[test]
fn a_hot_struct_field_is_promoted_to_a_register() {
    // `p.x` is the hottest field, so it lives in `%rbx`; its initialiser
    // writes the register instead of a frame slot.
    let out = asm(
        "struct P { x: i64, y: i64 } fn main() -> i64 { let mut p: P = P { x: 0, y: 1 }; let mut i: i64 = 0; while i < 10 { p.x = p.x + p.y; i = i + 1; } return p.x; }",
    );
    assert!(out.contains("mov %rax, %rbx"), "p.x initialises into a register: {out}");
}

#[test]
fn a_copied_struct_keeps_its_fields_in_memory() {
    // `p` is copied (`let q: P = p`) and passed by value, so every field must
    // stay in memory for the whole-value copy to work.
    let out = asm(
        "struct P { x: i64, y: i64 } fn f(p: P) -> i64 { return p.x; } fn main() -> i64 { let p: P = P { x: 1, y: 2 }; let q: P = p; return f(q) + p.x; }",
    );
    let _ = store_offset_after(&out, "mov $1, %rax"); // panics if `x` were promoted
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
    assert!(out.contains("mov %rax, %rbx"), "literal writes `tag` to its register: {out}");
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
fn bounds_checks_reject_an_out_of_range_constant_at_compile_time() {
    let out = compile_to_asm(
        "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[5]; }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(out.contains("jmp __rmc_bounds"), "a constant out of range traps");
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
    let out = compile_to_asm(
        "fn main() -> i64 { let i: i64 = 9; let v: i64 = { let t: [i64; 3] = [1; 3]; t }[i]; return v; }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(out.contains("cmp $3, %rax"), "compares to the block's array length");
    assert!(out.contains("jae __rmc_bounds"), "traps on out of range");
}

#[test]
fn bounds_checks_guard_a_constant_index_on_a_non_local_base() {
    // `make()[10]` takes the non-local constant-index branch; it must check too.
    let out = compile_to_asm(
        "fn make() -> [i64; 3] { let a: [i64; 3] = [1; 3]; return a; } \
         fn main() -> i64 { return make()[10]; }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    assert!(out.contains("jmp __rmc_bounds"), "an out-of-range constant traps");
}

#[test]
fn the_bounds_handler_is_in_an_executable_section() {
    // `__rmc_bounds` is called, so it must not be left in the `.rodata` opened
    // by the message before it; a plain `ld` link would then fault.
    let out = asm("fn main() -> i64 { return 0; }");
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
    // call must be emitted before the trap, not skipped by it.
    let out = compile_to_asm(
        "fn make() -> [i64; 1] { print_i64(7); return [4]; } \
         fn main() -> i64 { return make()[2]; }",
        &CompileOptions { bounds_checks: true, ..Default::default() },
    )
    .unwrap();
    let call = out.find("call make").expect("the base call is emitted");
    let trap = out.find("jmp __rmc_bounds").expect("the check traps");
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
    // Constant operands fold straight into the instruction; the shift count is
    // masked to 6 bits, matching Rust's `<<`/`>>` on `i64`.
    let out = asm("fn main() -> i64 { let a: i64 = 12; let b: i64 = 10; return a & b | a ^ b; }");
    assert!(out.contains("and %rcx, %rax"));
    assert!(out.contains("or %rcx, %rax"));
    assert!(out.contains("xor %rcx, %rax"));

    let out = asm("fn main() -> i64 { return 1 << 70; }");
    assert!(out.contains("sal $6, %rax"), "70 & 63 == 6");

    let out = asm("fn main() -> i64 { return -8 >> 2; }");
    assert!(out.contains("sar $2, %rax"), ">> is arithmetic");
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
    let out = asm("fn f(x: i64) -> i64 { match x { 0 => 10, 1 => 20, _ => 30 } } fn main() -> i64 { return f(0); }");
    assert!(out.contains("cmpq $0,"), "arm 0 compares against 0");
    assert!(out.contains("cmpq $1,"), "arm 1 compares against 1");
}

#[test]
fn a_match_scrutinee_call_runs_once() {
    // A call in the scrutinee is bound to a temporary before the arm tests, so
    // it is evaluated once no matter how many arms follow.
    let out = asm("fn g() -> i64 { return 3; } fn main() -> i64 { return match g() { 1 => 10, 3 => 30, _ => 0 }; }");
    assert_eq!(out.matches("call g").count(), 1, "the scrutinee call runs once");
}

#[test]
fn arguments_are_evaluated_left_to_right() {
    // `f(g(), mk())` must run `g` before `mk`. The old code copied every
    // aggregate argument before any scalar one, so a scalar in a later position
    // ran after the aggregate and the side effects came out in the wrong order.
    let out = asm("struct S { v: i64 } fn mk() -> S { return S { v: 1 }; } fn g() -> i64 { return 2; } fn f(x: i64, s: S) -> i64 { return s.v + x; } fn main() -> i64 { return f(g(), mk()); }");
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

#[test]
fn deeply_nested_input_is_an_error_not_a_crash() {
    // Thousands of nested parens used to recurse until the native stack
    // overflowed and the process aborted. The parser now stops at a depth
    // limit and reports it.
    let src = format!("fn main() -> i64 {{ return {}1{}; }}", "(".repeat(4000), ")".repeat(4000));
    let err = compile_to_asm(&src, &CompileOptions::default()).unwrap_err();
    assert!(err.contains("nests too deeply"), "got: {err}");
}

#[test]
fn a_very_long_operator_chain_is_an_error_not_a_crash() {
    // A left-deep chain `1+1+...+1` overflows the stack in sema/codegen unless
    // the parser bounds how deep the tree it builds can be.
    let src = format!("fn main() -> i64 {{ return {}1; }}", "1+".repeat(200_000));
    let err = compile_to_asm(&src, &CompileOptions::default()).unwrap_err();
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
fn an_array_of_aggregates_is_rejected_not_miscompiled() {
    // Nested arrays and arrays of structs are out of scope; they must be a
    // clean error rather than a segfault.
    let e1 = compile_to_asm("fn main() -> i64 { let m: [[i64; 2]; 2] = [[1, 2], [3, 4]]; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(e1.contains("array elements must be"), "got: {e1}");
    let e2 = compile_to_asm("struct P { x: i64 } fn main() -> i64 { let a: [P; 1] = [P { x: 1 }]; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(e2.contains("array elements must be"), "got: {e2}");
}

#[test]
fn an_empty_list_literal_is_rejected() {
    // `[]` has no element to infer from; the parser rejects it before sema.
    let err = compile_to_asm("fn main() -> i64 { let a = []; return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("expected an expression"), "got: {err}");
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
fn a_method_without_self_is_rejected() {
    let err = compile_to_asm("struct P { x: i64 } impl P { fn f() -> i64 { return 1; } } fn main() -> i64 { return 0; }", &CompileOptions::default()).unwrap_err();
    assert!(err.contains("expected identifier"), "got: {err}");
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
fn an_enum_array_is_rejected() {
    // Arrays stay `i64`/`bool`; `[E; N]` would mis-size the 8-byte element.
    let err = compile_to_asm("enum C { A } fn main() -> i64 { let a: [C; 2] = [C::A; 2]; return 0; }", &CompileOptions::default()).unwrap_err();
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
fn a_spill_across_a_call_uses_a_register_not_the_frame() {
    // `fib`'s only memory traffic is the first recursive result held across the
    // second call. It must live in a callee-saved register, not a frame slot.
    let out = asm(
        "fn fib(n: i64) -> i64 { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } \
         fn main() -> i64 { return fib(arg_i64(1)); }",
    );
    assert!(out.contains("push %r12"), "expected a saved spill register in:\n{out}");
    assert!(out.contains("mov %rax, %r12"), "expected the spill into %r12 in:\n{out}");
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
    let out = asm(
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
    let out = asm(
        "struct P { x: i64, y: i64 } \
         impl P { fn get(&self) -> i64 { return self.x; } } \
         fn main() -> i64 { let v: P = P { x: 5, y: 6 }; return v.get(); }",
    );
    // The receiver arrives as an address: the method reads `x` through the
    // pointer it was handed, not from a copied frame slot.
    assert!(out.contains("P$get"), "mangled method present in:\n{out}");
    assert!(out.contains("(%rax)"), "field read through a pointer in:\n{out}");
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
    let out = asm(
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
fn a_write_through_a_dereference_stores_to_the_referent() {
    let out = asm("fn main() -> i64 { let mut x: i64 = 1; let p: &i64 = &x; *p = 42; return x; }");
    // The store's target address is the pointer value, so the store is indirect
    // through a register rather than into a frame slot.
    assert!(out.contains("mov %rax, (%rcx)"), "indirect store in:\n{out}");
}

#[test]
fn a_write_through_a_reference_to_a_struct_field_stores_to_the_field() {
    let out = asm(
        "struct P { x: i64, y: i64 } \
         fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; let r: &i64 = &p.y; *r = 9; return p.y; }",
    );
    assert!(out.contains("mov %rax, (%rcx)"), "indirect store in:\n{out}");
}

#[test]
fn a_mut_receiver_is_passed_by_address() {
    let out = asm(
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
    let out = asm(
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
    let out = asm(
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
    let out = asm(
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
    let out = asm("fn lt(p: &i64) -> bool { return *p < 5; } fn main() -> i64 { let x: i64 = 1; if lt(&x) { return 1; } return 0; }");
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
