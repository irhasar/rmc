#!/usr/bin/env python3
"""Differential fuzzer for rmc.

Generates random programs in the rmc subset, compiles each with both rmc and
rustc -O, runs both, and compares stdout plus exit status. A mismatch prints the
program and exits non-zero. Seeds are reproducible, so a reported failure can be
re-run with `--seed`.

Usage:
    tools/fuzz.py --mode expr --n 500 --seed 1
    tools/fuzz.py --mode stmt --n 500 --seed 1
    tools/fuzz.py --mode expr --n 200 --seed 7 --keep /tmp/fail

Modes:
    expr  random integer expressions over three variables, with calls and
          guarded division
    stmt  loops, conditionals, match, arrays and aggregate calls, with
          side-effecting argument order and bounds-checked indexing

The reference is `rustc -O`, so a mismatch is a real semantic divergence.
"""
import argparse
import os
import random
import shutil
import subprocess
import sys

RMC = os.environ.get("RMC", os.path.join(os.path.dirname(__file__), "..", "compiler", "target", "release", "rmc"))
RUSTC = os.environ.get("RUSTC", "rustc")


def rustc_available():
    return shutil.which(RUSTC) is not None


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, **kw)


# ---------------------------------------------------------------- expr mode

EXPR_OPS = ["+", "-", "*", "&", "|", "^", "<<", ">>"]


def gen_expr(rng, depth):
    if depth >= 3 or rng.random() < 0.35:
        return rng.choice(["a", "b", "c", str(rng.randint(-30, 30))])
    r = rng.random()
    if r < 0.12:
        return f"h({gen_expr(rng, depth + 1)})"
    if r < 0.18:
        return f"(-{gen_expr(rng, depth + 1)})"
    op = rng.choice(EXPR_OPS)
    if op in ("<<", ">>"):
        left = gen_i64(rng, depth + 1)
        right = gen_i64(rng, depth + 1)
    else:
        left = gen_expr(rng, depth + 1)
        right = gen_expr(rng, depth + 1)
    return f"({left} {op} {right})"


def gen_guarded_div(rng, depth):
    # A divisor of the form `(X % 97 + 100)` is always in 4..=196, so division
    # never traps and both compilers agree on the value.
    num = gen_expr(rng, depth + 1)
    den = f"({gen_expr(rng, depth + 1)} % 97 + 100)"
    op = rng.choice(["/", "%"])
    return f"({num} {op} {den})"


def gen_i64(rng, depth):
    """An expression that is *typed* `i64` in both rmc and rustc: every leaf is
    a variable or a call, so no integer literal is left unconstrained. This
    matters for shifts: a shift amount is otherwise unconstrained, and rustc
    defaults such a literal to `i32`, whose shift count is masked to 5 bits,
    while rmc is i64-only and masks to 6. Mixing the two would report a false
    divergence, so shift operands go through here."""
    if depth >= 3 or rng.random() < 0.5:
        return rng.choice(["a", "b", "c", f"h({gen_expr(rng, depth + 1)})"])
    op = rng.choice(EXPR_OPS)
    return f"({gen_i64(rng, depth + 1)} {op} {gen_i64(rng, depth + 1)})"


def gen_expr_program(rng):
    a, b, c = (rng.randint(-1000, 1000) for _ in range(3))
    exprs = []
    for _ in range(rng.randint(1, 5)):
        e = gen_expr(rng, 0) if rng.random() < 0.5 else gen_guarded_div(rng, 0)
        exprs.append(e)
    rms = [
        "fn h(x: i64) -> i64 { return (x * 3 + 1) % 97 - 48; }",
        "fn main() -> i64 {",
        f"    let a: i64 = {a};",
        f"    let b: i64 = {b};",
        f"    let c: i64 = {c};",
    ]
    rs = [
        "fn h(x: i64) -> i64 { (x * 3 + 1) % 97 - 48 }",
        "fn main() {",
        f"    let a: i64 = {a};",
        f"    let b: i64 = {b};",
        f"    let c: i64 = {c};",
    ]
    for e in exprs:
        rms.append(f"    print_i64({e});")
        rs.append(f'    println!("{{}}", {e});')
    rms += ["    return 0;", "}"]
    rs += ["}"]
    return "\n".join(rms), "\n".join(rs)


# ---------------------------------------------------------------- stmt mode

def gen_stmt_program(rng):
    nf = rng.randint(1, 3)
    fields = [f"f{i}" for i in range(nf)]
    fdecl = ", ".join(f"{n}: i64" for n in fields)
    lit = ", ".join(f"{n}: {rng.randint(-9, 9)}" for n in fields)
    rms = [f"struct S {{ {fdecl} }}"]
    rs = ["#[derive(Copy, Clone)]", f"struct S {{ {fdecl} }}"]
    # A unit enum, matched and cast, to exercise tag resolution and enum
    # patterns alongside the struct paths.
    rms.append("enum E { P, Q, R }")
    rs.append("#[derive(Copy, Clone)]")
    rs.append("enum E { P, Q, R }")
    rms.append("fn tag(e: E) -> i64 { match e { E::P => 0, E::Q => 1, _ => 2 } }")
    rs.append("fn tag(e: E) -> i64 { match e { E::P => 0, E::Q => 1, _ => 2 } }")
    # A payload enum: a tagged union built, passed, and destructured.
    rms.append("enum Opt { None, Some(i64) }")
    rs.append("#[derive(Copy, Clone)]")
    rs.append("enum Opt { None, Some(i64) }")
    rms.append("fn unwrap(o: Opt, d: i64) -> i64 { match o { Opt::Some(v) => v, _ => d } }")
    rs.append("fn unwrap(o: Opt, d: i64) -> i64 { match o { Opt::Some(v) => v, _ => d } }")
    # An exhaustive enum `match` with no `_` arm, used as a value: the desugared
    # `if` chain has no final `else` and must still type-check.
    rms.append("fn exval(e: E) -> i64 { return match e { E::P => 1, E::Q => 2, E::R => 3 }; }")
    rs.append("fn exval(e: E) -> i64 { match e { E::P => 1, E::Q => 2, E::R => 3 } }")
    # Methods on a payload enum, by value and by reference.
    rms.append("impl Opt { fn or_else(self, d: i64) -> i64 { return match self { Opt::Some(v) => v, Opt::None => d }; } fn is_some(&self) -> bool { return match *self { Opt::Some(v) => true, Opt::None => false }; } }")
    rs.append("impl Opt { fn or_else(self, d: i64) -> i64 { match self { Opt::Some(v) => v, Opt::None => d } } fn is_some(&self) -> bool { match *self { Opt::Some(v) => true, Opt::None => false } } }")
    rms.append(f"fn mk(t: i64) -> S {{ print_i64(t); return S {{ {lit} }}; }}")
    rs.append(f'fn mk(t: i64) -> S {{ println!("{{}}", t); S {{ {lit} }} }}')
    rms.append("fn sc(t: i64) -> i64 { print_i64(t); return t * 2; }")
    rs.append('fn sc(t: i64) -> i64 { println!("{}", t); t * 2 }')
    # a two-argument function mixing kinds, to exercise evaluation order
    kinds = [rng.choice(["scalar", "agg"]) for _ in range(2)]
    params, callargs = [], []
    for i, k in enumerate(kinds):
        if k == "scalar":
            params.append(f"a{i}: i64")
            callargs.append(f"sc({rng.randint(1, 9)})")
        else:
            params.append(f"s{i}: S")
            callargs.append(f"mk({rng.randint(1, 9)})")
    body = " + ".join(
        (f"a{i}" if kinds[i] == "scalar" else f"s{i}.{fields[0]}") for i in range(2)
    )
    rms.append(f"fn mix({', '.join(params)}) -> i64 {{ return {body}; }}")
    rs.append(f"fn mix({', '.join(params)}) -> i64 {{ {body} }}")
    # References: a `&self` method, a `&S` parameter and a `&i64` parameter, all
    # reached through `&`/`*` and auto-deref.
    rms.append(f"impl S {{ fn first(&self) -> i64 {{ return self.{fields[0]}; }} }}")
    rs.append(f"impl S {{ fn first(&self) -> i64 {{ self.{fields[0]} }} }}")
    rms.append(f"fn sref(s: &S) -> i64 {{ return s.{fields[0]}; }}")
    rs.append(f"fn sref(s: &S) -> i64 {{ s.{fields[0]} }}")
    rms.append("fn vref(p: &i64) -> i64 { return *p + 1; }")
    rs.append("fn vref(p: &i64) -> i64 { *p + 1 }")
    # A `&mut` parameter and a `&mut self` method: both write through the
    # reference, so a wrong address shows up as a wrong final value.
    rms.append("fn bumpv(p: &mut i64) -> i64 { *p = *p + 1; return *p; }")
    rs.append("fn bumpv(p: &mut i64) -> i64 { *p += 1; *p }")
    rms.append("impl S { fn bump(&mut self) -> i64 { self." + fields[0] + " = self." + fields[0] + " + 2; return self." + fields[0] + "; } }")
    rs.append("impl S { fn bump(&mut self) -> i64 { self." + fields[0] + " += 2; self." + fields[0] + " } }")

    bound = rng.randint(1, 8)
    rms += ["fn main() -> i64 {", "    let mut acc: i64 = 0;",
            "    let mut arr: [i64; 4] = [1, 2, 3, 4];", "    let mut i: i64 = 0;"]
    rs += ["fn main() {", "    let mut acc: i64 = 0;",
           "    let mut arr: [i64; 4] = [1, 2, 3, 4];", "    let mut i: i64 = 0;"]
    rms.append(f"    while i < {bound} {{")
    rs.append(f"    while i < {bound} {{")
    inner = rng.choice([
        "acc = acc + h(i) + arr[i % 4];",
        "acc = acc + (i << 2) + (i & 3);",
        "acc = acc + (i ^ (i >> 1));",
    ])
    rms.append("        " + inner)
    rs.append("        " + inner.replace("acc = acc +", "acc +=", 1))
    rms.append("        acc = acc + match i % 3 { 0 => 5, 1 => 7, _ => 11 };")
    rs.append("        acc += match i % 3 { 0 => 5, 1 => 7, _ => 11 };")
    rms.append("        arr[i % 4] = arr[i % 4] + i;")
    rs.append("        arr[i % 4] += i;")
    rms.append("        i = i + 1;")
    rs.append("        i += 1;")
    rms += ["    }"]
    rs += ["    }"]
    # A bool array built from a list literal, then counted. Guards the
    # list-literal element-type inference and bool indexing.
    rms.append("    let flags: [bool; 4] = [true, false, i < 1, true];")
    rs.append("    let flags: [bool; 4] = [true, false, i < 1, true];")
    rms.append("    let mut f: i64 = 0;")
    rs.append("    let mut f: i64 = 0;")
    rms.append("    let mut k: i64 = 0;")
    rs.append("    let mut k: i64 = 0;")
    rms.append("    while k < 4 { if flags[k] { f = f + 1; } k = k + 1; }")
    rs.append("    while k < 4 { if flags[k as usize] { f += 1; } k += 1; }")
    rms.append("    acc = acc + f * 1000;")
    rs.append("    acc += f * 1000;")
    # An enum value chosen by the loop counter, matched and cast.
    rms.append("    let e: E = match i % 3 { 0 => E::P, 1 => E::Q, _ => E::R };")
    rs.append("    let e = match i % 3 { 0 => E::P, 1 => E::Q, _ => E::R };")
    rms.append("    acc = acc + tag(e) * 100 + (e as i64);")
    rs.append("    acc += tag(e) * 100 + (e as i64);")
    # An exhaustive enum `match` with no `_` arm, used as a value.
    rms.append("    acc = acc + exval(e);")
    rs.append("    acc += exval(e);")
    # An array of unit enums, indexed and matched.
    rms.append("    let es: [E; 3] = [E::P, E::Q, E::R];")
    rs.append("    let es: [E; 3] = [E::P, E::Q, E::R];")
    rms.append("    acc = acc + (es[k % 3] as i64) + exval(es[(k + 1) % 3]);")
    rs.append("    acc += (es[(k % 3) as usize] as i64) + exval(es[((k + 1) % 3) as usize]);")
    # Build a payload variant, pass it to a function, and destructure it.
    rms.append("    let opt: Opt = match i % 2 { 0 => Opt::Some(i + 1), _ => Opt::None };")
    rs.append("    let opt = match i % 2 { 0 => Opt::Some(i + 1), _ => Opt::None };")
    # Methods on the payload enum, by value and by reference.
    rms.append("    acc = acc + opt.or_else(77);")
    rs.append("    acc += opt.or_else(77);")
    rms.append("    if opt.is_some() { acc = acc + 5; }")
    rs.append("    if opt.is_some() { acc += 5; }")
    rms.append("    acc = acc + unwrap(opt, 1000);")
    rs.append("    acc += unwrap(opt, 1000);")
    rms.append(f"    acc = acc + mix({', '.join(callargs)});")
    rs.append(f"    acc += mix({', '.join(callargs)});")
    # Build a struct, take references to it and to a scalar, and read back
    # through `&self`, `&S` and `&i64`.
    rms.append(f"    let s0: S = S {{ {lit} }};")
    rs.append(f"    let s0 = S {{ {lit} }};")
    rms.append("    let sr: &S = &s0;")
    rs.append("    let sr: &S = &s0;")
    rms.append("    let vr: &i64 = &acc;")
    rs.append("    let vr: &i64 = &acc;")
    rms.append("    acc = acc + sr.first() + sref(sr) + sref(&s0) + vref(vr) + vref(&acc);")
    rs.append("    acc += sr.first() + sref(sr) + sref(&s0) + vref(vr) + vref(&acc);")
    rms.append("    let mut s1: S = S { " + lit + " };")
    rs.append("    let mut s1: S = S { " + lit + " };")
    rms.append("    let mut a1: i64 = 3;")
    rs.append("    let mut a1: i64 = 3;")
    rms.append("    acc = acc + s1.bump() + bumpv(&mut a1) + bumpv(&mut a1) + a1 + s1.first();")
    rs.append("    acc += s1.bump() + bumpv(&mut a1) + bumpv(&mut a1) + a1 + s1.first();")
    rms.append("    print_i64(acc);")
    rs.append('    println!("{}", acc);')
    rms += ["    return 0;", "}"]
    rs += ["}"]
    rms.insert(0, "fn h(x: i64) -> i64 { return (x * 3 + 1) % 97; }")
    rs.insert(1, "fn h(x: i64) -> i64 { (x * 3 + 1) % 97 }")
    return "\n".join(rms), "\n".join(rs)


# -------------------------------------------------------------- robust mode

def robust_corpus():
    """Deterministic adversarial inputs. None of them is a valid program, but
    each must produce a clean error (exit 0 or 1), never a panic or a signal.
    A deep-nesting or long-chain input that overflows the native stack shows up
    here as a SIGABRT, which is the crash a fuzzer is for."""
    rng = random.Random(0)
    cases = {
        "random_bytes": bytes(rng.randrange(256) for _ in range(200)),
        "valid_prefix_garbage": b"fn main() -> i64 { " + bytes(rng.randrange(256) for _ in range(200)),
        "deep_paren": b"fn main() -> i64 { return " + b"(" * 5000 + b"1" + b")" * 5000 + b"; }",
        "deep_block": b"fn main() -> i64 { " + b"{ " * 5000 + b"return 0; " + b"}" * 5000 + b" }",
        "deep_type": b"fn main() -> i64 { let x: " + b"(" * 5000 + b"i64" + b")" * 5000 + b" = 0; return 0; }",
        "deep_neg": b"fn main() -> i64 { return " + b"-" * 200000 + b"1; }",
        "deep_not": b"fn main() -> i64 { let mut x: bool = true; x = " + b"!" * 200000 + b"true; return 0; }",
        "long_add_chain": b"fn main() -> i64 { return " + b"1+" * 200000 + b"1; }",
        "long_cmp_chain": b"fn main() -> i64 { return " + b"1==" * 100000 + b"1; }",
        "long_field_chain": b"struct S { a: i64 } fn main() -> i64 { let s: S = S { a: 1 }; return s" + b".a" * 100000 + b"; }",
        "nested_if_stmt": b"fn main() -> i64 { " + b"if true { " * 3000 + b"return 0; " + b"}" * 3000 + b" return 1; }",
        "nested_while": b"fn main() -> i64 { " + b"while true { " * 3000 + b"break; " + b"}" * 3000 + b" return 0; }",
        "nested_call": b"fn f(x: i64) -> i64 { return x; } fn main() -> i64 { return " + b"f(" * 3000 + b"1" + b")" * 3000 + b"; }",
        "huge_int": b"fn main() -> i64 { return " + b"9" * 4000 + b"; }",
        "unterminated_string": b'fn main() -> i64 { print_str("abc); return 0; }',
        "null_bytes": b"fn main() -> i64 { return 0; }\x00\x00",
        "truncated": b"fn ma",
        "empty": b"",
        "lone_keyword": b"struct",
        "unbalanced_brace": b"fn main() -> i64 { return 0;",
        "deep_deref": b"fn main() -> i64 { let x: i64 = 0; return " + b"*" * 100000 + b"x; }",
        "deep_ref_type": b"fn main() -> i64 { let x: " + b"&" * 5000 + b"i64 = x; return 0; }",
        "deep_addr_of": b"fn main() -> i64 { let x: i64 = 0; return " + b"&" * 100000 + b"x; }",
    }
    return cases


def run_robust():
    tmp = "/tmp/rmc-fuzz"
    os.makedirs(tmp, exist_ok=True)
    failures = 0
    for name, data in robust_corpus().items():
        path = f"{tmp}/{name}.rms"
        open(path, "wb").write(data)
        p = run([RMC, path, "-o", f"{tmp}/o.bin"])
        # A signal (negative return code) or a panic message is a crash.
        crashed = p.returncode < 0 or b"panicked" in p.stderr or b"overflowed its stack" in p.stderr
        if crashed:
            print(f"[robust] {name}: CRASH (exit {p.returncode}) {p.stderr.decode(errors='replace')[:200]}")
            failures += 1
        elif p.returncode not in (0, 1):
            print(f"[robust] {name}: unexpected exit {p.returncode}")
            failures += 1
    if failures == 0:
        print(f"robust: all {len(robust_corpus())} adversarial inputs error cleanly")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["expr", "stmt", "robust"], default="expr")
    ap.add_argument("--n", type=int, default=200)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--keep", default=None, help="write failing programs here")
    args = ap.parse_args()

    if not os.path.exists(RMC):
        print(f"rmc not found at {RMC}; build with `cargo build --release`", file=sys.stderr)
        return 2
    if args.mode == "robust":
        return run_robust()
    if not rustc_available():
        print("rustc not found; set RUSTC or install a toolchain", file=sys.stderr)
        return 2

    tmp = "/tmp/rmc-fuzz"
    os.makedirs(tmp, exist_ok=True)
    gen = gen_expr_program if args.mode == "expr" else gen_stmt_program
    rng = random.Random(args.seed)
    failures = 0
    for t in range(args.n):
        rms, rs = gen(rng)
        open(f"{tmp}/p.rms", "w").write(rms)
        open(f"{tmp}/p.rs", "w").write(rs)
        c = run([RMC, f"{tmp}/p.rms", "-o", f"{tmp}/p.bin"])
        if c.returncode != 0:
            print(f"[{args.mode} seed={args.seed} iter={t}] rmc failed to compile:\n{rms}\n{c.stderr.decode(errors='replace')}")
            failures += 1
            break
        c = run([RUSTC, "-O", "-A", "warnings", "--edition", "2021", f"{tmp}/p.rs", "-o", f"{tmp}/q.bin"])
        if c.returncode != 0:
            continue  # not a valid rust reference program; skip
        o1 = run([f"{tmp}/p.bin"])
        o2 = run([f"{tmp}/q.bin"])
        if (o1.returncode, o1.stdout) != (o2.returncode, o2.stdout):
            print(f"[{args.mode} seed={args.seed} iter={t}] DIVERGENCE\n{rms}\n"
                  f"  rmc   exit={o1.returncode} stdout={o1.stdout!r}\n"
                  f"  rustc exit={o2.returncode} stdout={o2.stdout!r}")
            if args.keep:
                os.makedirs(args.keep, exist_ok=True)
                open(f"{args.keep}/fuzz-{args.mode}-{args.seed}-{t}.rms", "w").write(rms)
            failures += 1
            break
    if failures == 0:
        print(f"{args.n} {args.mode} programs, seed {args.seed}: all match rustc")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
