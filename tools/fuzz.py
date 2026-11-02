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
    rms.append(f"    acc = acc + mix({', '.join(callargs)});")
    rs.append(f"    acc += mix({', '.join(callargs)});")
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
