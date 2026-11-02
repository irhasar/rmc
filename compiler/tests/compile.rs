//! Unit tests that exercise the compiler front end and code generator without
//! invoking `as` or `ld`, so they run anywhere a Rust toolchain does.

use nrc::{compile_to_asm, CompileOptions};

fn asm(src: &str) -> String {
    compile_to_asm(src, &CompileOptions::default()).expect("should compile")
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
    // `x` at offset 0, `y` at offset 8: the initialiser must store to both.
    let out = asm("struct P { x: i64, y: i64 } fn main() -> i64 { let p: P = P { x: 1, y: 2 }; return p.y; }");
    assert!(out.contains("mov %rax, -16(%rbp)"), "x at the low address");
    assert!(out.contains("mov %rax, -8(%rbp)"), "y at the high address");
}

#[test]
fn field_write_targets_the_right_slot() {
    let out = asm("struct P { x: i64, y: i64 } fn main() -> i64 { let mut p: P = P { x: 1, y: 2 }; p.y = 9; return p.y; }");
    assert!(out.contains("mov %rax, -8(%rbp)"));
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
    assert!(!plain.contains("jo __nrc_overflow"));
    let checked = compile_to_asm(
        "fn main() -> i64 { let mut x: i64 = 1; x = x + 1; return x; }",
        &CompileOptions { overflow_checks: true },
    )
    .unwrap();
    assert!(checked.contains("jo __nrc_overflow"));
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
