#!/usr/bin/env python3
"""Survey how the rustc-MIR path handles realistic Rust input.

The differential suites only feed the MIR back end programs it is known to
support. A user typing arbitrary Rust gets whatever falls out, and "whatever
falls out" must be a clean `rmc: error: ...` with a non-zero exit — never a
panic, an `unreachable!`, an index-out-of-bounds, or a stack overflow. This
tool compiles a corpus of realistic snippets and classifies the result:

  ok        compiled; the binary ran (exit status recorded)
  clean     rmc reported an error and exited non-zero without a panic
  CRASH     signal, Rust panic message, or stack overflow

Only `CRASH` is a failure. `clean` is the acceptable answer for a feature the
back end does not implement: the point is the diagnostic, not the coverage.

Usage:
  python3 tools/mir_survey.py              # classify every corpus entry
  python3 tools/mir_survey.py --keep DIR   # write the crashing inputs to DIR
"""

import argparse
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RMC = os.path.join(ROOT, "compiler", "target", "release", "rmc")

# Each entry is (name, source). They are ordinary programs a user might write,
# not adversarial inputs: the corpus is biased toward the shapes that appear in
# tutorials, kata solutions and small tools, because that is what a compiler's
# first real users type.
CORPUS = {
    "arith": "fn main() { let a = 6; let b = 7; println!(\"{}\", a * b); }",
    "arith_unused": "fn main() { let a = 6 * 7; }",
    "hello_literal": 'fn main() { println!("hello, world"); }',
    "hello_fmt": 'fn main() { println!("n = {}", 42); }',
    "hello_var": 'fn main() { let n = 42; println!("n = {}", n); }',
    "print_macro": 'fn main() { print!("a"); print!("b"); }',
    "eprintln": 'fn main() { eprintln!("oops"); }',
    "fmt_width": 'fn main() { println!("{:>5}", 1); }',
    "fmt_hex": 'fn main() { println!("{:#x}", 255); }',
    "fmt_named": 'fn main() { println!("{n}", n = 3); }',
    "string_len": 'fn main() { let s = "abc"; println!("{}", s.len()); }',
    "string_concat": 'fn main() { let s = format!("{}-{}", 1, 2); println!("{}", s); }',
    "string_push": 'fn main() { let mut s = String::new(); s.push_str("hi"); println!("{}", s); }',
    "vec_basic": "fn main() { let mut v = Vec::new(); v.push(1); v.push(2); println!(\"{}\", v.len()); }",
    "vec_macro": "fn main() { let v = vec![1, 2, 3]; println!(\"{}\", v[1]); }",
    "vec_iter": "fn main() { let v = vec![1, 2, 3]; let s: i64 = v.iter().sum(); println!(\"{}\", s); }",
    "hashmap": "use std::collections::HashMap;\nfn main() { let mut m = HashMap::new(); m.insert(1, 2); println!(\"{:?}\", m); }",
    "box": "fn main() { let b = Box::new(5); println!(\"{}\", b); }",
    "rc": "use std::rc::Rc;\nfn main() { let r = Rc::new(3); println!(\"{}\", r); }",
    "option": "fn main() { let o: Option<i64> = Some(5); match o { Some(x) => println!(\"{}\", x), None => println!(\"none\") } }",
    "result": "fn f() -> Result<i64, String> { Ok(1) }\nfn main() { if let Ok(x) = f() { println!(\"{}\", x); } }",
    "option_unwrap": "fn main() { let o = Some(5); println!(\"{}\", o.unwrap()); }",
    "string_slice": 'fn main() { let s = "hello"; println!("{}", &s[1..3]); }',
    "char": "fn main() { let c = 'x'; println!(\"{}\", c); }",
    "float": "fn main() { let x = 1.5; let y = x * 2.0; println!(\"{}\", y); }",
    "float_fmt": 'fn main() { println!("{:.2}", 3.14159); }',
    "cast_float": "fn main() { let x = 2.9; println!(\"{}\", x as i64); }",
    "i32_overflow": "fn main() { let a: i32 = 100_000; let b: i32 = 100_000; println!(\"{}\", a * b); }",
    "u8_wrap": "fn main() { let a: u8 = 250; let b = a.wrapping_add(10); println!(\"{}\", b); }",
    "i128": "fn main() { let a: i128 = 1; println!(\"{}\", a); }",
    "usize_index": "fn main() { let v = [1, 2, 3]; let i: usize = 1; println!(\"{}\", v[i]); }",
    "tuple": "fn main() { let t = (1, 2); println!(\"{}\", t.0); }",
    "nested_tuple": "fn main() { let t = ((1, 2), 3); println!(\"{}\", (t.0).1); }",
    "struct": "struct P { x: i64, y: i64 }\nfn main() { let p = P { x: 1, y: 2 }; println!(\"{}\", p.x); }",
    "struct_method": "struct P { x: i64 }\nimpl P { fn get(&self) -> i64 { self.x } }\nfn main() { let p = P { x: 9 }; println!(\"{}\", p.get()); }",
    "trait": "trait S { fn a(&self) -> i64; }\nstruct Q;\nimpl S for Q { fn a(&self) -> i64 { 3 } }\nfn main() { println!(\"{}\", Q.a()); }",
    "generic": "fn id<T>(x: T) -> T { x }\nfn main() { println!(\"{}\", id(5)); }",
    "closure": "fn main() { let f = |x: i64| x + 1; println!(\"{}\", f(2)); }",
    "closure_capture": "fn main() { let n = 3; let f = |x: i64| x + n; println!(\"{}\", f(2)); }",
    "iterator_chain": "fn main() { let s: i64 = (1..10).filter(|x| x % 2 == 0).sum(); println!(\"{}\", s); }",
    "range_fold": "fn main() { let s = (1..=5).fold(0, |a, b| a + b); println!(\"{}\", s); }",
    "enum_payload": "enum E { A, B(i64) }\nfn main() { let e = E::B(7); match e { E::A => {}, E::B(n) => println!(\"{}\", n) } }",
    "enum_generic": "enum E<T> { A(T) }\nfn main() { let e = E::A(1); match e { E::A(x) => println!(\"{}\", x) } }",
    "recursion": "fn f(n: i64) -> i64 { if n <= 1 { 1 } else { n * f(n - 1) } }\nfn main() { println!(\"{}\", f(5)); }",
    "slice_param": "fn s(x: &[i64]) -> i64 { x.len() as i64 }\nfn main() { println!(\"{}\", s(&[1, 2, 3])); }",
    "str_param": 'fn f(s: &str) -> usize { s.len() }\nfn main() { println!("{}", f("abc")); }',
    "mut_ref": "fn inc(x: &mut i64) { *x += 1; }\nfn main() { let mut a = 1; inc(&mut a); println!(\"{}\", a); }",
    "static_item": "static X: i64 = 7;\nfn main() { println!(\"{}\", X); }",
    "const_item": "const X: i64 = 7;\nfn main() { println!(\"{}\", X); }",
    "const_fn": "const fn f() -> i64 { 3 }\nfn main() { println!(\"{}\", f()); }",
    "array_init": "fn main() { let a = [0; 8]; println!(\"{}\", a[3]); }",
    "array_2d": "fn main() { let a = [[1, 2], [3, 4]]; println!(\"{}\", a[1][0]); }",
    "struct_update": "struct P { x: i64, y: i64 }\nfn main() { let a = P { x: 1, y: 2 }; let b = P { x: 3, ..a }; println!(\"{}\", b.y); }",
    "match_guard": 'fn main() { let n = 5; let s = match n { x if x > 3 => "big", _ => "small" }; println!("{}", s); }',
    "match_or": "fn main() { let n = 2; match n { 1 | 2 => println!(\"low\"), _ => println!(\"hi\") } }",
    "loop_break": "fn main() { let mut i = 0; loop { i += 1; if i > 3 { break; } } println!(\"{}\", i); }",
    "while_let": "fn main() { let mut o = Some(1); while let Some(x) = o { println!(\"{}\", x); o = None; } }",
    "for_range": "fn main() { let mut s = 0; for i in 0..5 { s += i; } println!(\"{}\", s); }",
    "for_rev": "fn main() { for i in (0..3).rev() { println!(\"{}\", i); } }",
    "panic_explicit": 'fn main() { panic!("boom"); }',
    "assert": "fn main() { assert!(1 == 1); }",
    "assert_eq": "fn main() { assert_eq!(1, 1); }",
    "todo_macro": "fn main() { todo!(); }",
    "unreachable_macro": "fn main() { unreachable!(); }",
    "dbg_macro": "fn main() { dbg!(1 + 1); }",
    "vec_index_oob": "fn main() { let v = vec![1]; println!(\"{}\", v[5]); }",
    "div_zero": "fn main() { let a = 1; let b = 0; println!(\"{}\", a / b); }",
    "shift_wide": "fn main() { let a: i64 = 1; println!(\"{}\", a << 40); }",
    "negative_mod": "fn main() { println!(\"{}\", -7 % 3); }",
    "char_compare": "fn main() { let c = 'a'; if c == 'a' { println!(\"yes\"); } }",
    "string_compare": 'fn main() { if "a" == "a" { println!("yes"); } }',
    "string_eq_var": 'fn main() { let a = "x"; if a == "x" { println!("yes"); } }',
    "vec_of_struct": "struct P { x: i64 }\nfn main() { let v = vec![P { x: 1 }]; println!(\"{}\", v[0].x); }",
    "generic_struct": "struct W<T> { v: T }\nfn main() { let w = W { v: 1 }; println!(\"{}\", w.v); }",
    "impl_display": 'use std::fmt;\nstruct P;\nimpl fmt::Display for P { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "P") } }\nfn main() { println!("{}", P); }',
    "derive_debug": '#[derive(Debug)]\nstruct P { x: i64 }\nfn main() { println!("{:?}", P { x: 1 }); }',
    "mod_use": "mod m { pub fn f() -> i64 { 1 } }\nfn main() { println!(\"{}\", m::f()); }",
    "use_std_fs": 'use std::fs;\nfn main() { let _ = fs::read_to_string("nope"); }',
    "process_args": "fn main() { let a: Vec<String> = std::env::args().collect(); println!(\"{}\", a.len()); }",
    "thread": "use std::thread;\nfn main() { let h = thread::spawn(|| 1); let _ = h.join(); }",
    "unsafe_ptr": "fn main() { let x = 5; let p = &x as *const i64; unsafe { println!(\"{}\", *p); } }",
    "raw_ptr_arith": "fn main() { let a = [1, 2, 3]; let p = a.as_ptr(); unsafe { println!(\"{}\", *p.add(1)); } }",
    "transmute": "fn main() { let x: i64 = unsafe { std::mem::transmute(1u64) }; println!(\"{}\", x); }",
    "size_of": "fn main() { println!(\"{}\", std::mem::size_of::<i64>()); }",
    "mem_swap": "fn main() { let mut a = 1; let mut b = 2; std::mem::swap(&mut a, &mut b); println!(\"{}\", a); }",
    "sort_vec": "fn main() { let mut v = vec![3, 1, 2]; v.sort(); println!(\"{:?}\", v); }",
    "to_string": "fn main() { let s = 5.to_string(); println!(\"{}\", s); }",
    "parse_int": 'fn main() { let n: i64 = "42".parse().unwrap(); println!("{}", n); }',
    "iter_map": "fn main() { let v: Vec<i64> = (1..4).map(|x| x * 2).collect(); println!(\"{}\", v[0]); }",
    "option_map": "fn main() { let o = Some(2).map(|x| x + 1); println!(\"{:?}\", o); }",
    "match_string": 'fn main() { let s = "b"; match s { "a" => println!("a"), _ => println!("other") } }',
    "nested_fn": "fn main() { fn inner() -> i64 { 3 } println!(\"{}\", inner()); }",
    "empty_main": "fn main() {}",
    "main_return": "fn main() -> i64 { 0 }",
    "unit_fn": "fn f() {}\nfn main() { f(); }",
    "many_args": "fn f(a: i64, b: i64, c: i64, d: i64, e: i64, g: i64, h: i64) -> i64 { a + b + c + d + e + g + h }\nfn main() { println!(\"{}\", f(1, 2, 3, 4, 5, 6, 7)); }",
    "array_return": "fn f() -> [i64; 3] { [1, 2, 3] }\nfn main() { println!(\"{}\", f()[1]); }",
    "struct_return": "struct P { x: i64, y: i64 }\nfn f() -> P { P { x: 1, y: 2 } }\nfn main() { println!(\"{}\", f().y); }",
    "closure_as_arg": "fn apply(f: impl Fn(i64) -> i64, x: i64) -> i64 { f(x) }\nfn main() { println!(\"{}\", apply(|x| x + 1, 2)); }",
    "dyn_trait": "trait S { fn a(&self) -> i64; }\nfn f(s: &dyn S) -> i64 { s.a() }\nstruct Q;\nimpl S for Q { fn a(&self) -> i64 { 1 } }\nfn main() { println!(\"{}\", f(&Q)); }",
    "impl_trait_ret": "fn f() -> impl Fn(i64) -> i64 { |x| x }\nfn main() { println!(\"{}\", f()(1)); }",
    "async_fn": "async fn f() -> i64 { 1 }\nfn main() { let _ = f(); }",
    "macro_rules": "macro_rules! twice { ($e:expr) => { $e + $e } }\nfn main() { println!(\"{}\", twice!(3)); }",
    "derive_clone": "#[derive(Clone, Copy)]\nstruct P { x: i64 }\nfn main() { let a = P { x: 1 }; let b = a; println!(\"{}\", b.x); }",
    "drop_impl": "struct P;\nimpl Drop for P { fn drop(&mut self) {} }\nfn main() { let _p = P; }",
    "iter_enumerate": "fn main() { for (i, c) in \"ab\".chars().enumerate() { println!(\"{} {}\", i, c); } }",
    "while_loop": "fn main() { let mut i = 0; while i < 3 { i += 1; } println!(\"{}\", i); }",
    "nested_loops": "fn main() { let mut s = 0; for i in 0..3 { for j in 0..3 { s += i * j; } } println!(\"{}\", s); }",
    "bool_ops": "fn main() { let a = true; let b = false; println!(\"{}\", a && !b || b); }",
    "bit_ops": "fn main() { let a: i64 = 0b1010; println!(\"{}\", a & 0b0110 | a ^ 1); }",
    "char_to_u8": "fn main() { let c = 'A'; println!(\"{}\", c as u8); }",
    "wrapping_all": "fn main() { let a: i32 = i32::MAX; println!(\"{}\", a.wrapping_add(1)); }",
    "checked_add": "fn main() { let a: i64 = 5; println!(\"{:?}\", a.checked_add(1)); }",
    "abs_sign": "fn main() { let a: i64 = -5; println!(\"{}\", a.abs()); }",
    "min_max": "fn main() { println!(\"{}\", 3.min(5)); }",
    "pow": "fn main() { println!(\"{}\", 2i64.pow(10)); }",
    "gcd_loop": "fn gcd(mut a: i64, mut b: i64) -> i64 { while b != 0 { let t = b; b = a % b; a = t; } a }\nfn main() { println!(\"{}\", gcd(48, 18)); }",
    "string_builder": 'fn main() { let mut s = String::from("a"); s.push(\'b\'); println!("{}", s); }',
    "slice_iter": "fn main() { let a = [1, 2, 3]; let mut s = 0; for x in a.iter() { s += x; } println!(\"{}\", s); }",
    "ref_pattern": "fn main() { let a = [1, 2]; let [x, y] = a; println!(\"{} {}\", x, y); }",
    "nested_match": "fn main() { let t = (1, Some(2)); match t { (a, Some(b)) => println!(\"{} {}\", a, b), _ => {} } }",
}


def classify(name, src, tmp, keep):
    path = os.path.join(tmp, name + ".rs")
    with open(path, "w") as f:
        f.write(src + "\n")
    exe = os.path.join(tmp, name + ".bin")
    try:
        p = subprocess.run([RMC, path, "-o", exe], capture_output=True, timeout=60)
    except subprocess.TimeoutExpired:
        return "CRASH", "timeout (hang)"
    err = p.stderr.decode(errors="replace")
    if p.returncode < 0:
        return "CRASH", f"signal {-p.returncode}"
    if "panicked" in err or "overflowed its stack" in err:
        first = next((l for l in err.splitlines() if "panic" in l or "stack" in l), err)
        return "CRASH", first.strip()
    if p.returncode == 0:
        run = subprocess.run([exe], capture_output=True, timeout=60)
        status = "ok" if run.returncode == 0 else f"ok (exit {run.returncode})"
        return status, ""
    first = next((l for l in err.splitlines() if l.startswith("rmc:")), err.splitlines()[0] if err else "?")
    return "clean", first.strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", default=None, help="write crashing inputs here")
    args = ap.parse_args()

    if not os.path.exists(RMC):
        print(f"build {RMC} first", file=sys.stderr)
        return 2

    counts = {}
    crashes = []
    with tempfile.TemporaryDirectory() as tmp:
        for name, src in CORPUS.items():
            kind, detail = classify(name, src, tmp, args.keep)
            counts[kind] = counts.get(kind, 0) + 1
            if kind == "CRASH":
                crashes.append((name, detail))
                print(f"CRASH {name}: {detail}")
            elif kind.startswith("ok"):
                print(f"  ok  {name}  [{kind}]")
            else:
                print(f" cln  {name}: {detail}")

    print()
    for k in sorted(counts):
        print(f"{counts[k]:4d}  {k}")
    if crashes and args.keep:
        os.makedirs(args.keep, exist_ok=True)
        for name, _ in crashes:
            with open(os.path.join(args.keep, name + ".rs"), "w") as f:
                f.write(CORPUS[name] + "\n")
        print(f"\ncrashing inputs written to {args.keep}")
    return 1 if crashes else 0


if __name__ == "__main__":
    sys.exit(main())
