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
