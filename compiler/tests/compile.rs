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
        &CompileOptions { overflow_checks: true },
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
