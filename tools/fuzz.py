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
    expr  random integer expressions over three variables, with calls,
          guarded division, comparisons, string comparisons and `.len()` on
          strings and arrays
    stmt  loops, conditionals, match, arrays and aggregate calls, with
          side-effecting argument order and bounds-checked indexing
    opt   a self-recursive function, an invariant loop and inlined helpers,
          to exercise TCO, LICM and inlining
    arith overflowing arithmetic near i64::MAX, to exercise the overflow
          trap and the strength reductions that must be off under it
    nest  nested aggregates (struct-in-struct, struct-with-array, an enum
          whose payload is a struct), including a deep copy and a nested-path
          write, which `stmt` never emits
    pat   `match` expressions: a multi-variant enum, a variant with two
          payload fields, a binding pattern, a negative literal, a nested
          `match`, string literal patterns, and a non-trivial side-effecting
          scrutinee (the temporary `desugar_match` binds it to, which must be
          evaluated once)
    order side-effecting arguments, to pin evaluation order
    shift `<<`/`>>` over the full count range (negative, >= 64, boundary)
    cast  expressions on the `as` / unary precedence boundary (`*r as i64`,
          `-x as i64`, `!x as i64`, `x * y as i64`), where a wrong `as` level
          changes the value
    assoc associated functions (`Type::name(..)`) and methods on the same
          struct and enum, so a wrongly passed receiver or a mis-resolved
          constructor changes the result

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


def _inproc_text(path):
    """The RX LOAD segment's file bytes, i.e. the in-process assembled `.text`."""
    import struct
    data = open(path, "rb").read()
    phoff = struct.unpack_from("<Q", data, 0x20)[0]
    phentsize = struct.unpack_from("<H", data, 0x36)[0]
    phnum = struct.unpack_from("<H", data, 0x38)[0]
    for i in range(phnum):
        o = phoff + i * phentsize
        p_type, p_flags = struct.unpack_from("<II", data, o)
        p_offset = struct.unpack_from("<Q", data, o + 8)[0]
        p_filesz = struct.unpack_from("<Q", data, o + 32)[0]
        if p_type == 1 and (p_flags & 1):
            start = phoff + phnum * phentsize
            return data[start:p_offset + p_filesz]
    return None


def _binutils_text(path, scratch):
    subprocess.run(["objcopy", "-O", "binary", "--only-section=.text", path, scratch],
                   capture_output=True)
    try:
        return open(scratch, "rb").read()
    except OSError:
        return None


def _disasm(raw, scratch):
    """Normalised instruction list: branch targets folded, alignment nops and
    immediates reduced to placeholders, so only the instruction *stream* is
    compared. This is what catches an operand-size or alignment bug, which a
    behaviour comparison cannot see."""
    import re
    open(scratch, "wb").write(raw)
    out = run(["objdump", "-D", "-b", "binary", "-m", "i386:x86-64", scratch])
    ins = []
    for line in out.stdout.decode(errors="replace").splitlines():
        m = re.match(r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{2} )+\s*(.*)$", line)
        if not m:
            continue
        t = m.group(1).strip()
        t = re.sub(r"\$-?0x[0-9a-f]+", "$I", t)
        t = re.sub(r"\$-?[0-9]+", "$I", t)
        t = re.sub(r"0x[0-9a-f]+", "A", t)
        t = re.sub(r"\s+", " ", t)
        if not t:
            continue
        if "nop" in t or t == "xchg %ax,%ax" or t.startswith("data16"):
            continue
        ins.append(t)
    return ins


def encoding_divergence(rms, flags, tmp):
    """Compile both ways and compare the decoded `.text`.

    Returns `None` when either build fails (nothing to compare), `True` when the
    instruction streams agree, or a `(index, in_process, as)` tuple on a
    divergence."""
    direct = f"{tmp}/enc.direct"
    binut = f"{tmp}/enc.binutils"
    if run([RMC] + flags + [rms, "-o", direct]).returncode != 0:
        return None
    if run([RMC, "--binutils"] + flags + [rms, "-o", binut]).returncode != 0:
        return None
    raw = _inproc_text(direct)
    bu = _binutils_text(binut, f"{tmp}/enc.text")
    if raw is None or bu is None:
        return None
    a = _disasm(raw, f"{tmp}/enc.a.bin")
    b = _disasm(bu, f"{tmp}/enc.b.bin")
    n = min(len(a), len(b))
    if a[:n] != b[:n]:
        k = next((i for i in range(n) if a[i] != b[i]), n)
        return (k, a[k] if k < len(a) else "<end>", b[k] if k < len(b) else "<end>")
    return True


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, **kw)


# ---------------------------------------------------------------- expr mode

EXPR_OPS = ["+", "-", "*", "&", "|", "^", "<<", ">>"]
CMP_OPS = ["<", ">", "<=", ">=", "==", "!="]


def gen_expr(rng, depth):
    if depth >= 3 or rng.random() < 0.35:
        return rng.choice(["a", "b", "c", str(rng.randint(-30, 30))])
    r = rng.random()
    if r < 0.12:
        return f"h({gen_expr(rng, depth + 1)})"
    if r < 0.18:
        return f"(-{gen_expr(rng, depth + 1)})"
    if r < 0.24:
        # `!` is bitwise not on an integer. The operand goes through `gen_i64`
        # so it is typed `i64` in both compilers (an unconstrained literal would
        # default to `i32` in rustc and disagree with rmc's i64-only `!`).
        return f"(!{gen_i64(rng, depth + 1)})"
    op = rng.choice(EXPR_OPS)
    if op in ("<<", ">>"):
        left = gen_i64(rng, depth + 1)
        # Bound the count to 0..63 so it is a valid `i64` shift. Otherwise a
        # constant local (or one rustc folds through) can be negative or >= 64,
        # which rustc denies; rmc now folds constant locals too and would reject
        # the program, which is correct but not a differential signal.
        right = f"({gen_i64(rng, depth + 1)} & 63)"
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
    right = gen_i64(rng, depth + 1)
    if op in ("<<", ">>"):
        # Keep the count in 0..63, a valid `i64` shift; see the note in
        # `gen_expr`.
        right = f"({right} & 63)"
    return f"({gen_i64(rng, depth + 1)} {op} {right})"


def gen_cmp(rng, depth):
    """A comparison used as a value: `(x op y) as i64`. rustc infers `i32` for
    an unsuffixed literal, so both sides go through `gen_expr` (whose leaves are
    variables and calls), and the `as i64` makes the result type agree. This is
    the only generator that feeds `emit_cmp`, whose operand-class dispatch
    (immediate/immediate, immediate/memory, memory/immediate, memory/memory) is
    otherwise reached only by loop conditions."""
    op = rng.choice(CMP_OPS)
    return f"(({gen_expr(rng, depth + 1)}) {op} ({gen_expr(rng, depth + 1)})) as i64"


def gen_str_lit(rng):
    """A random `str` literal over a two-letter alphabet. The small alphabet is
    deliberate: it makes equal literals and long shared prefixes common, which
    is where a pointer comparison instead of a content comparison shows up."""
    n = rng.randint(0, 5)
    return '"' + "".join(rng.choice("ab") for _ in range(n)) + '"'


def gen_strcmp(rng):
    """A string comparison, `(lit op lit)`. A `str` is a pointer to its record,
    so rmc must lower this to a byte-wise helper; comparing the pointers would
    answer "not equal" for two identical literals. Both sides are literals so
    the two compilers agree on the type."""
    op = rng.choice(["==", "!=", "<", "<=", ">", ">="])
    return f"({gen_str_lit(rng)} {op} {gen_str_lit(rng)})"


def gen_expr_program(rng):
    a, b, c = (rng.randint(-1000, 1000) for _ in range(3))
    exprs = []
    for _ in range(rng.randint(1, 5)):
        r = rng.random()
        if r < 0.4:
            e = gen_expr(rng, 0)
        elif r < 0.7:
            e = gen_guarded_div(rng, 0)
        else:
            e = gen_cmp(rng, 0)
        exprs.append(e)
    # `i64::MIN` has to be a typed binding: in an untyped position rustc infers
    # `i32` for the literal and rejects it, while rmc's only integer type is
    # `i64`, so comparing the two there would be a false divergence.
    min_exprs = [
        "m",
        "(m + 1)",
        "(m / 2)",
        "(!m)",
        "((m < 0) as i64)",
        "((m == -9223372036854775808) as i64)",
    ]
    if rng.random() < 0.4:
        exprs.append(rng.choice(min_exprs))
    rms = [
        "fn h(x: i64) -> i64 { return (x * 3 + 1) % 97 - 48; }",
        "fn main() -> i64 {",
        f"    let a: i64 = {a};",
        f"    let b: i64 = {b};",
        f"    let c: i64 = {c};",
        "    let m: i64 = -9223372036854775808;",
    ]
    rs = [
        "fn h(x: i64) -> i64 { (x * 3 + 1) % 97 - 48 }",
        "fn main() {",
        f"    let a: i64 = {a};",
        f"    let b: i64 = {b};",
        f"    let c: i64 = {c};",
        "    let m: i64 = -9223372036854775808;",
    ]
    for e in exprs:
        rms.append(f"    print_i64({e});")
        rs.append(f'    println!("{{}}", {e});')
    # String comparisons: printed as `bool`, which both compilers render as
    # `0`/`1` through the `pb` twin. A pointer compare would diverge here.
    for _ in range(rng.randint(1, 4)):
        s = gen_strcmp(rng)
        rms.append(f"    print_bool({s});")
        rs.append(f'    println!("{{}}", ({s}) as i64);')
    # Lengths. `"lit".len()` reads the string record's length word, so it must be
    # an indirect load through the pointer, not a struct field; an array's length
    # is a compile-time constant. rustc's `len()` is `usize`, so the twin casts.
    for lit in (gen_str_lit(rng) for _ in range(rng.randint(1, 3))):
        rms.append(f"    print_i64({lit}.len());")
        rs.append(f'    println!("{{}}", {lit}.len() as i64);')
    sv = gen_str_lit(rng)
    n = rng.randint(1, 6)
    rms.append(f"    let sv = {sv};")
    rms.append("    print_i64(sv.len());")
    rs.append(f"    let sv = {sv};")
    rs.append('    println!("{}", sv.len() as i64);')
    rms.append(f"    let arr: [i64; {n}] = [0; {n}];")
    rms.append("    print_i64(arr.len());")
    rs.append(f"    let arr: [i64; {n}] = [0; {n}];")
    rs.append('    println!("{}", arr.len() as i64);')
    rms += ["    return 0;", "}"]
    rs += ["}"]
    return "\n".join(rms), "\n".join(rs)


def gen_cast_program(rng):
    """Expressions that sit on the `as` / unary precedence boundary. `as` binds
    looser than `* - ! &`, so `*r as i64` is `(*r) as i64` and `-a as i64` is
    `(-a) as i64`, but tighter than `* / %`, so `a * b as i64` is `a * (b as
    i64)`. Every shape below changes value if `as` is parsed at the wrong level,
    which is exactly the bug this mode pins: rmc used to parse `as` in
    `parse_postfix`, binding it tighter than unary, and rejected `*r as i64`."""
    a, b, c = (rng.randint(-1000, 1000) for _ in range(3))
    bl = rng.choice(["true", "false"])
    i64_leaves = ["a", "b", "c", "*r", "(a - b)", "(-a)", "(!b)", "h(a)"]
    bool_leaves = ["bl", "(a < b)", "(a == c)", "(!bl)"]

    def X():
        return rng.choice(i64_leaves)

    def B():
        return rng.choice(bool_leaves)

    tmpl = [
        lambda: "(*r as i64)",
        lambda: f"(-{X()} as i64)",
        lambda: f"(!{X()} as i64)",
        lambda: "(-*r as i64)",
        lambda: "(!*r as i64)",
        lambda: f"(-{X()} * {X()})",
        lambda: f"({X()} * -{X()})",
        lambda: f"({X()} as i64 * {X()})",
        lambda: f"(-{X()} as i64 * {X()})",
        lambda: f"(!{X()} as i64 + {X()})",
        lambda: f"({B()} as i64 * {X()})",
        lambda: f"(!{B()} as i64 + {X()})",
        lambda: f"({X()} - {X()} as i64)",
        lambda: f"(*r as i64 - {X()})",
        lambda: f"(!({X()} as i64))",
        lambda: f"(-({X()} as i64))",
        lambda: f"({X()} + -{X()} as i64)",
        lambda: f"(a as i64 as i64 + {X()})",
        lambda: f"({B()} as i64 as i64 - {X()})",
    ]
    exprs = [rng.choice(tmpl)() for _ in range(rng.randint(4, 12))]
    rms = [
        "fn h(x: i64) -> i64 { return (x * 3 + 1) % 97 - 48; }",
        "fn main() -> i64 {",
        f"    let a: i64 = {a};",
        f"    let b: i64 = {b};",
        f"    let c: i64 = {c};",
        f"    let bl: bool = {bl};",
        "    let r: &i64 = &a;",
    ]
    rs = [
        "fn h(x: i64) -> i64 { (x * 3 + 1) % 97 - 48 }",
        "fn main() {",
        f"    let a: i64 = {a};",
        f"    let b: i64 = {b};",
        f"    let c: i64 = {c};",
        f"    let bl: bool = {bl};",
        "    let r: &i64 = &a;",
    ]
    for e in exprs:
        rms.append(f"    print_i64({e});")
        rs.append(f'    println!("{{}}", {e});')
    rms += ["    return 0;", "}"]
    rs += ["}"]
    return "\n".join(rms), "\n".join(rs)


def gen_assoc_program(rng):
    """Associated functions (`Type::name(..)`) and methods on the same types.
    Random constants are baked into constructor calls, and each value is read
    back through a method, so a receiver wrongly passed to an associated
    function, or an associated function wrongly resolved, changes the result."""
    a, b, k = (rng.randint(-500, 500) for _ in range(3))
    calls = []
    for _ in range(rng.randint(3, 8)):
        x, y, m = (rng.randint(-50, 50) for _ in range(3))
        calls.append((x, y, m))
    rms = [
        "struct S { a: i64, b: i64 }",
        "impl S {",
        "    fn new(a: i64, b: i64) -> S { return S { a, b }; }",
        "    fn zero() -> S { return S { a: 0, b: 0 }; }",
        "    fn scale_of(v: i64) -> i64 { return v * 3; }",
        "    fn sum(self) -> i64 { return self.a + self.b; }",
        "    fn bump(self, d: i64) -> S { return S::new(self.a + d, self.b + d); }",
        "}",
        "enum E { Off, On }",
        "impl E {",
        "    fn from(n: i64) -> E { if n % 2 == 0 { return E::Off; } return E::On; }",
        "    fn tag(self) -> i64 { return self as i64; }",
        "}",
        "fn main() -> i64 {",
        f"    let s: S = S::new({a}, {b});",
        "    print_i64(s.sum());",
        "    print_i64(S::zero().sum());",
        f"    print_i64(S::scale_of({k}));",
        f"    print_i64(s.bump({k}).sum());",
        f"    print_i64(S::new({k}, {k}).bump({a}).sum());",
        "    print_i64(E::from(0).tag());",
        "    print_i64(E::from(1).tag());",
    ]
    rs = [
        "struct S { a: i64, b: i64 }",
        "impl S {",
        "    fn new(a: i64, b: i64) -> S { S { a, b } }",
        "    fn zero() -> S { S { a: 0, b: 0 } }",
        "    fn scale_of(v: i64) -> i64 { v * 3 }",
        "    fn sum(&self) -> i64 { self.a + self.b }",
        "    fn bump(&self, d: i64) -> S { S::new(self.a + d, self.b + d) }",
        "}",
        "enum E { Off, On }",
        "impl E {",
        "    fn from(n: i64) -> E { if n % 2 == 0 { E::Off } else { E::On } }",
        "    fn tag(self) -> i64 { self as i64 }",
        "}",
        "fn main() {",
        f"    let s = S::new({a}, {b});",
        '    println!("{}", s.sum());',
        '    println!("{}", S::zero().sum());',
        f'    println!("{{}}", S::scale_of({k}));',
        f'    println!("{{}}", s.bump({k}).sum());',
        f'    println!("{{}}", S::new({k}, {k}).bump({a}).sum());',
        '    println!("{}", E::from(0).tag());',
        '    println!("{}", E::from(1).tag());',
    ]
    for (x, y, m) in calls:
        rms.append(f"    print_i64(S::new({x}, {y}).bump({m}).sum() + E::from({m}).tag());")
        rs.append(f'    println!("{{}}", S::new({x}, {y}).bump({m}).sum() + E::from({m}).tag());')
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
    rs.append("        " + inner.replace("acc = acc +", "acc +=", 1)
               .replace("arr[i % 4]", "arr[(i % 4) as usize]"))
    rms.append("        acc = acc + match i % 3 { 0 => 5, 1 => 7, _ => 11 };")
    rs.append("        acc += match i % 3 { 0 => 5, 1 => 7, _ => 11 };")
    rms.append("        arr[i % 4] = arr[i % 4] + i;")
    rs.append("        arr[(i % 4) as usize] += i;")
    rms.append("        i = i + 1;")
    rs.append("        i += 1;")
    rms += ["    }"]
    rs += ["    }"]
    # `while let` over a payload enum: the scrutinee is a variable here (a
    # call scrutinee is covered in `pat` mode), and the loop stops when the
    # value becomes `None`. A `continue` before the `break` check keeps the
    # re-test path live.
    rms.append("    let mut wl: i64 = 0;")
    rs.append("    let mut wl: i64 = 0;")
    rms.append("    let mut wo: Opt = Opt::Some(2);")
    rs.append("    let mut wo: Opt = Opt::Some(2);")
    rms.append("    while let Opt::Some(v) = wo { wl = wl + v; if wl > 100 { wo = Opt::None; continue; } if wl > 6 { wo = Opt::None; } }")
    rs.append("    while let Opt::Some(v) = wo { wl += v; if wl > 100 { wo = Opt::None; continue; } if wl > 6 { wo = Opt::None; } }")
    rms.append("    acc = acc + wl * 7 + nodiv(i);")
    rs.append("    acc += wl * 7 + nodiv(i);")
    # A bare `loop` whose `break` is the last thing in the block, so the `;` is
    # omitted (rustc allows it because `break` diverges).
    rms.append("    let mut lc: i64 = 0;")
    rs.append("    let mut lc: i64 = 0;")
    rms.append("    loop { lc = lc + 1; if lc >= 3 { break } }")
    rs.append("    loop { lc += 1; if lc >= 3 { break } }")
    rms.append("    acc = acc + lc * 11;")
    rs.append("    acc += lc * 11;")
    # A `for` range over i64 variables, exclusive or inclusive. The bounds are
    # named `i64` locals so Rust does not default the literals to `i32`; that
    # also exercises a non-literal bound and `..=`'s strict exit test.
    fa, fb = rng.randint(-3, 3), rng.randint(-3, 3)
    op = rng.choice(["..", "..="])
    rms.append(f"    let fa: i64 = {fa};")
    rs.append(f"    let fa: i64 = {fa};")
    rms.append(f"    let fb: i64 = {fb};")
    rs.append(f"    let fb: i64 = {fb};")
    rms.append(f"    for fi in fa{op}fb {{ acc = acc + fi * 3; }}")
    rs.append(f"    for fi in fa{op}fb {{ acc += fi * 3; }}")
    # `for x in ARRAY`: the array is iterated by value, and the head is a local
    # (not a literal) so a stale read shows up. A `continue` keeps the increment
    # path live, and a wildcard counts the elements.
    rms.append("    let farr: [i64; 3] = [1, 2, 3];")
    rs.append("    let farr: [i64; 3] = [1, 2, 3];")
    rms.append("    for fv in farr { acc = acc + fv * 5; }")
    rs.append("    for fv in farr { acc += fv * 5; }")
    rms.append("    let mut fc: i64 = 0;")
    rs.append("    let mut fc: i64 = 0;")
    rms.append("    for fv in farr { if fv == 2 { continue; } fc = fc + fv; }")
    rs.append("    for fv in farr { if fv == 2 { continue; } fc += fv; }")
    rms.append("    for _ in farr { fc = fc + 1; }")
    rs.append("    for _ in farr { fc += 1; }")
    rms.append("    acc = acc + fc * 17;")
    rs.append("    acc += fc * 17;")
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
    # `let _ = e;` discards `e`: a side-effecting call must still run once and a
    # discarded aggregate must not corrupt the frame. Annotated and unannotated
    # forms both appear.
    rms.append("    let _ = sc(5);")
    rs.append("    let _ = sc(5);")
    rms.append("    let _: i64 = sc(6);")
    rs.append("    let _: i64 = sc(6);")
    rms.append("    let _ = mk(7);")
    rs.append("    let _ = mk(7);")
    rms.append("    let _ = [1, 2, 3];")
    rs.append("    let _ = [1, 2, 3];")
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
    rms.insert(0, "fn nodiv(c: i64) -> i64 { if c > 0 { 1 } else { return 2 } }")
    rms.insert(0, "fn h(x: i64) -> i64 { return (x * 3 + 1) % 97; }")
    # `rs[0]` is the struct's `#[derive]`, so the helper has to go *before* it:
    # inserting at 1 puts the attribute on `fn h`, which rustc rejects.
    rs.insert(0, "fn nodiv(c: i64) -> i64 { if c > 0 { 1 } else { return 2 } }")
    rs.insert(0, "fn h(x: i64) -> i64 { (x * 3 + 1) % 97 }")
    return "\n".join(rms), "\n".join(rs)


# ---------------------------------------------------------------- opt mode

def gen_arith_program(rng):
    """A program whose arithmetic actually overflows, so `--overflow-checks`
    traps and the strength reductions that must be disabled under it are
    exercised. `expr`/`stmt`/`opt` use small values, so no generated program
    ever traps: the trap path was reached only by the single `ovf` corpus case,
    and the `lea`/accumulate folds (`gen_expr_into_reg`, which have no `jo` and
    are skipped under `--overflow-checks`) were never checked for it.

    Values sit near `i64::MAX`, and every operator can overflow. The program is
    run both ways: with checks off, where rmc and rustc must wrap identically,
    and with checks on against a `-C overflow-checks=on` rustc, where both must
    trap at the same point. A missing `jo` after a folded `add`, or a fold that
    wrongly survives the check, shows up as a divergence.

    Literals stay in `0..=i64::MAX`: rmc parses the magnitude before the sign,
    so `-9223372036854775808` is rejected as an out-of-range literal.
    """
    MAX = 9223372036854775807
    big = rng.choice([MAX, MAX - 1, MAX - 2, MAX - rng.randint(3, 999)])
    # `x + c`/`x - c` are one `lea`; `x * k`/`x * k + c` are the scaled form;
    # `x + x` is the doubled form. Each must fall back to `add`/`imul` + `jo`
    # under `--overflow-checks`.
    shape = rng.choice([
        "x + 1", "x - 1", "x + 7 - 3", "x + x", "x * 2", "x * 3", "x * 3 + 1",
        "x * 9 - 2", "x * 2 + 5", "x + x + 3",
    ])
    # A loop-carried accumulator: `a` is `dst`, so `gen_accumulate` flattens the
    # chain into a run of `add`s on `a`'s register, which has no `jo`.
    step = rng.choice(["a + 1 + 1", "a + i + 1", "a + i * 2 + 3", "a + a", "a + i + i"])
    n0 = rng.randint(1, 4)
    rms = [
        f"fn f(x: i64) -> i64 {{ return {shape}; }}",
        "fn acc(n: i64) -> i64 {",
        "    let mut a: i64 = 9223372036854775807;",
        "    let mut i: i64 = 0;",
        f"    while i < n {{ a = {step}; i = i + 1; }}",
        "    return a;",
        "}",
        "fn main() -> i64 {",
        f"    print_i64(f({big}));",
        f"    print_i64(acc({n0}));",
        "    return 0;",
        "}",
    ]
    rs = [
        f"fn f(x: i64) -> i64 {{ {shape} }}",
        "fn acc(n: i64) -> i64 {",
        "    let mut a: i64 = 9223372036854775807;",
        "    let mut i: i64 = 0;",
        f"    while i < n {{ a = {step}; i = i + 1; }}",
        "    a",
        "}",
        "fn main() {",
        f'    println!("{{}}", f({big}));',
        f'    println!("{{}}", acc({n0}));',
        "}",
    ]
    return "\n".join(rms) + "\n", "\n".join(rs) + "\n"


def gen_shift_program(rng):
    """A program whose shift counts come from `argv`, so they are opaque to the
    compiler and can be out of range (negative or >= 64). The `expr`/`stmt`
    generators mask every shift count with `& 63` to keep it a valid `i64` shift
    (see `gen_expr`), which means the shift-overflow trap was never exercised:
    rmc used to mask a runtime count with the hardware's 6 bits and produce a
    wrong value where rustc panics. The count here is a parameter, so it is
    neither constant-folded by rmc nor rejected at compile time by rustc.

    The same argument is fed to `<<`, `>>`, and the compound `<<=`/`>>=`, which
    all reach the same code path, plus a shift inside an array index so the
    compound-store form is covered too.
    """
    shape = rng.choice([
        "y << n", "y >> n", "(y << n) + (y >> n)", "(y + 1) << n",
        "(y * 3) >> n", "y << (n - 1)", "y >> (n + 1)",
    ])
    cshape = rng.choice(["y <<= n", "y >>= n", "y <<= n + 1", "y >>= n - 1"])
    ashape = cshape.replace("y", "a[0]", 1)
    rms = [
        "fn main() -> i64 {",
        "    let n: i64 = arg_i64(1);",
        "    let mut y: i64 = arg_i64(2);",
        f"    print_i64({shape});",
        "    let mut a: [i64; 3] = [1, 2, 3];",
        "    a[0] = y;",
        f"    {ashape};",
        "    print_i64(a[0]);",
        f"    {cshape};",
        "    print_i64(y);",
        "    return 0;",
        "}",
    ]
    rs = [
        "fn main() {",
        "    let args: Vec<i64> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();",
        "    let n: i64 = args[0];",
        "    let mut y: i64 = args[1];",
        f'    println!("{{}}", {shape});',
        "    let mut a: [i64; 3] = [1, 2, 3];",
        "    a[0] = y;",
        f"    {ashape};",
        '    println!("{}", a[0]);',
        f"    {cshape};",
        '    println!("{}", y);',
        "}",
    ]
    return "\n".join(rms) + "\n", "\n".join(rs) + "\n"


def gen_nest_program(rng):
    """A program built from nested aggregates: a struct whose fields include
    another struct and an array, and an enum whose payload is a struct. `stmt`
    mode only ever gives a struct plain `i64` fields, so the field-offset
    arithmetic for a nested path (`o.mid.f`, `o.arr[i]`) and the deep copy of an
    aggregate local were never generated. A wrong nested offset reads the wrong
    field; a shallow copy aliases the source, so mutating the copy would change
    the original. Both show up as a wrong final value.

    Every value descends from a call (`seed`), not a literal, so no local is
    constant-folded and the nested paths stay live.
    """
    inner = [f"i{j}" for j in range(rng.randint(2, 3))]
    na = rng.randint(2, 4)
    idecl = ", ".join(f"{f}: i64" for f in inner)
    inner_mk = ", ".join(f"{f}: n + {j + 1}" for j, f in enumerate(inner))
    arr_mk = ", ".join(f"n + {j}" for j in range(na))
    inner_lit = ", ".join(f"{f}: base + {j + 1}" for j, f in enumerate(inner))
    inner_lit2 = ", ".join(f"{f}: base + {j + 5}" for j, f in enumerate(inner))
    sum_terms = ["o.s"] + [f"o.mid.{f}" for f in inner] + [f"o.arr[{j}]" for j in range(na)]
    wval = " + ".join(
        f"v.{f} * {10 ** (len(inner) - 1 - j)}" for j, f in enumerate(inner)
    )
    m1, m2 = rng.randint(10, 30), rng.randint(50, 99)
    base_k = rng.randint(1, 6)
    f0 = inner[0]

    # A 2D array `[[i64; c2]; r2]` and an array of structs `[Inner; ns]`, so the
    # stride-aware indexing and the aggregate element copy are exercised. The
    # element stride is the element's size in slots, not one: a wrong stride
    # reads a neighbouring element and shows up as a wrong final value. Every
    # struct element gets distinct field values for the same reason.
    r2, c2 = rng.randint(2, 3), rng.randint(2, 3)
    ns = rng.randint(2, 3)
    grid_lit = ", ".join(
        "[" + ", ".join(f"base + {r * c2 + c}" for c in range(c2)) + "]"
        for r in range(r2)
    )
    st_lit = ", ".join(
        "Inner { "
        + ", ".join(f"{f}: base + {r * 10 + j + 1}" for j, f in enumerate(inner))
        + " }"
        for r in range(ns)
    )

    rms = [
        "fn seed(x: i64) -> i64 { return x * 7 + 1; }",
        f"struct Inner {{ {idecl} }}",
        f"struct Outer {{ s: i64, mid: Inner, arr: [i64; {na}] }}",
        "enum W { Z, V(Inner) }",
        "struct Wrap { w: W, t: i64 }",
        f"fn mk(n: i64) -> Outer {{ return Outer {{ s: n, mid: Inner {{ {inner_mk} }}, arr: [{arr_mk}] }}; }}",
        f"fn sum(o: Outer) -> i64 {{ return {' + '.join(sum_terms)}; }}",
        f"fn wval(w: W) -> i64 {{ return match w {{ W::Z => 0, W::V(v) => {wval} }}; }}",
        f"fn wwrap(p: Wrap) -> i64 {{ return match p.w {{ W::Z => p.t, W::V(v) => p.t + v.{f0} }}; }}",
        f"fn bump(o: &mut Outer) {{ o.mid.{f0} = o.mid.{f0} + 5; o.s = o.s + 1; }}",
        "fn main() -> i64 {",
        f"    let base: i64 = seed({base_k});",
        "    let mut acc: i64 = 0;",
        "    let o0: Outer = mk(base);",
        f"    acc = acc + sum(o0) + o0.mid.{inner[1]} + o0.arr[{na - 1}];",
        "    let mut o1: Outer = o0;",
        f"    o1.mid.{f0} = o1.mid.{f0} + {m1};",
        f"    o1.arr[0] = o1.arr[0] + {m2};",
        f"    acc = acc + o1.mid.{f0} + o0.mid.{f0} + o1.arr[0] + o0.arr[0];",
        "    let mut k: i64 = 0;",
        f"    while k < {na} {{ acc = acc + o0.arr[k]; k = k + 1; }}",
        f"    let w0: W = W::V(Inner {{ {inner_lit} }});",
        "    let w1: W = W::Z;",
        "    acc = acc + wval(w0) + wval(w1);",
        "    let wr: Wrap = Wrap { w: w0, t: 3 };",
        "    let wz: Wrap = Wrap { w: w1, t: 4 };",
        "    acc = acc + wwrap(wr) + wwrap(wz);",
        "    let mut o2: Outer = mk(base + 1);",
        "    bump(&mut o2);",
        f"    acc = acc + o2.mid.{f0} + o2.s;",
        # Nested arrays and arrays of structs: literal, splat, variable-index
        # reads, element write, whole-element copy.
        f"    let g: [[i64; {c2}]; {r2}] = [{grid_lit}];",
        "    let mut gi: i64 = 0;",
        f"    while gi < {r2} {{ let mut gj: i64 = 0; while gj < {c2} {{ acc = acc + g[gi][gj]; gj = gj + 1; }} gi = gi + 1; }}",
        f"    acc = acc + g[0][0] + g[{r2 - 1}][{c2 - 1}];",
        f"    let mut gm: [[i64; {c2}]; {r2}] = [[3; {c2}]; {r2}];",
        f"    gm[{r2 - 1}][0] = acc;",
        f"    acc = acc + gm[0][0] + gm[{r2 - 1}][0] + gm[{r2 - 1}][{c2 - 1}];",
        f"    let mut sa: [Inner; {ns}] = [{st_lit}];",
        f"    sa[{ns - 1}].{f0} = sa[{ns - 1}].{f0} + 3;",
        f"    let sc: [Inner; 2] = [Inner {{ {inner_lit2} }}; 2];",
        f"    sa[0] = sc[1];",
        "    let mut si: i64 = 0;",
        f"    while si < {ns} {{ acc = acc + sa[si].{f0} + sa[si].{inner[1]}; si = si + 1; }}",
        f"    acc = acc + sc[0].{f0} + sc[1].{inner[1]};",
        "    print_i64(acc);",
        "    return 0;",
        "}",
    ]
    rs = [
        "fn seed(x: i64) -> i64 { x * 7 + 1 }",
        "#[derive(Copy, Clone)]",
        f"struct Inner {{ {idecl} }}",
        "#[derive(Copy, Clone)]",
        f"struct Outer {{ s: i64, mid: Inner, arr: [i64; {na}] }}",
        "#[derive(Copy, Clone)]",
        "enum W { Z, V(Inner) }",
        "#[derive(Copy, Clone)]",
        "struct Wrap { w: W, t: i64 }",
        f"fn mk(n: i64) -> Outer {{ Outer {{ s: n, mid: Inner {{ {inner_mk} }}, arr: [{arr_mk}] }} }}",
        f"fn sum(o: Outer) -> i64 {{ {' + '.join(sum_terms)} }}",
        f"fn wval(w: W) -> i64 {{ match w {{ W::Z => 0, W::V(v) => {wval} }} }}",
        f"fn wwrap(p: Wrap) -> i64 {{ match p.w {{ W::Z => p.t, W::V(v) => p.t + v.{f0} }} }}",
        f"fn bump(o: &mut Outer) {{ o.mid.{f0} += 5; o.s += 1; }}",
        "fn main() {",
        f"    let base: i64 = seed({base_k});",
        "    let mut acc: i64 = 0;",
        "    let o0: Outer = mk(base);",
        f"    acc += sum(o0) + o0.mid.{inner[1]} + o0.arr[{na - 1}];",
        "    let mut o1: Outer = o0;",
        f"    o1.mid.{f0} += {m1};",
        f"    o1.arr[0] += {m2};",
        f"    acc += o1.mid.{f0} + o0.mid.{f0} + o1.arr[0] + o0.arr[0];",
        "    let mut k: i64 = 0;",
        f"    while k < {na} {{ acc += o0.arr[k as usize]; k += 1; }}",
        f"    let w0: W = W::V(Inner {{ {inner_lit} }});",
        "    let w1: W = W::Z;",
        "    acc += wval(w0) + wval(w1);",
        "    let wr: Wrap = Wrap { w: w0, t: 3 };",
        "    let wz: Wrap = Wrap { w: w1, t: 4 };",
        "    acc += wwrap(wr) + wwrap(wz);",
        "    let mut o2: Outer = mk(base + 1);",
        "    bump(&mut o2);",
        f"    acc += o2.mid.{f0} + o2.s;",
        f"    let g: [[i64; {c2}]; {r2}] = [{grid_lit}];",
        "    let mut gi: i64 = 0;",
        f"    while gi < {r2} {{ let mut gj: i64 = 0; while gj < {c2} {{ acc += g[gi as usize][gj as usize]; gj += 1; }} gi += 1; }}",
        f"    acc += g[0][0] + g[{r2 - 1}][{c2 - 1}];",
        f"    let mut gm: [[i64; {c2}]; {r2}] = [[3; {c2}]; {r2}];",
        f"    gm[{r2 - 1}][0] = acc;",
        f"    acc += gm[0][0] + gm[{r2 - 1}][0] + gm[{r2 - 1}][{c2 - 1}];",
        f"    let mut sa: [Inner; {ns}] = [{st_lit}];",
        f"    sa[{ns - 1}].{f0} += 3;",
        f"    let sc: [Inner; 2] = [Inner {{ {inner_lit2} }}; 2];",
        "    sa[0] = sc[1];",
        "    let mut si: i64 = 0;",
        f"    while si < {ns} {{ acc += sa[si as usize].{f0} + sa[si as usize].{inner[1]}; si += 1; }}",
        f"    acc += sc[0].{f0} + sc[1].{inner[1]};",
        '    println!("{}", acc);',
        "}",
    ]
    return "\n".join(rms) + "\n", "\n".join(rs) + "\n"


def gen_pat_program(rng):
    """A program whose control flow is a set of `match` expressions, so the
    desugaring and the arm codegen are exercised. `stmt` mode only matches a
    single-payload enum with a wildcard and an integer with a wildcard, so a
    multi-variant enum, a variant with two payload fields, a binding pattern, a
    negative literal, a nested `match`, a `bool` match covering both values,
    and — the path that matters most — a non-trivial scrutinee (a call, a binary
    operator, an index or a field, which `desugar_match` binds to a temporary)
    were never generated. That
    temporary must be evaluated exactly once: re-evaluating it per arm repeats
    a side effect, and a stale one reads the wrong value.
    """
    k = rng.randint(1, 9)
    v = rng.randint(2, 9)
    neg = rng.choice([-1, -7, -20])
    ncolors = rng.randint(2, 4)
    bound = rng.randint(1, 6)
    a0, a1 = rng.randint(0, 9), rng.randint(0, 9)

    colors = [f"C{j}" for j in range(ncolors)]
    color_def = ", ".join(colors)
    color_arms = ", ".join(f"Color::{c} => {j + 1}" for j, c in enumerate(colors))
    color_pick = ", ".join(f"{j} => Color::{c}" for j, c in enumerate(colors))
    # An or-pattern arm naming a group of variants: `C0 | C1 => 1`. Splitting the
    # variants into two groups covers every variant, so the match is exhaustive
    # and usable as a value, which exercises or-pattern exhaustiveness.
    half = max(1, ncolors // 2)
    or_arms = " | ".join(f"Color::{c}" for c in colors[:half]) + " => 1, " \
        + " | ".join(f"Color::{c}" for c in colors[half:]) + " => 2"
    # A non-empty integer range, half-open and inclusive.
    rlo = rng.randint(-5, 2)
    rhi = rlo + rng.randint(1, 5)
    # Two string literals from the shared two-letter alphabet, so collisions
    # and shared prefixes are common: a pointer compare and a content compare
    # then disagree on which arm fires.
    s0, s1 = gen_str_lit(rng), gen_str_lit(rng)

    rms = [
        "fn side(k: i64) -> i64 { print_i64(k); return k * 3 + 1; }",
        f"fn classify(x: i64) -> i64 {{ return match x {{ {neg} => 10, 0 => 20, 1 => 30, _ => 40 }}; }}",
        "fn bindit(x: i64) -> i64 { return match x { y => y * 2 + 1 }; }",
        f"fn bscr(x: i64) -> i64 {{ return match x + {k} {{ {v} => 100, _ => 0 }}; }}",
        f"fn cscr(k: i64) -> i64 {{ return match side(k) {{ {v} => 5, _ => 7 }}; }}",
        f"fn mscr(k: i64) -> i64 {{ return match side(k) {{ {v} => 11, {v + 1} => 22, {v + 2} => 33, _ => 44 }}; }}",
        f"fn iscr(a: [i64; 4], i: i64) -> i64 {{ return match a[i] {{ {v} => 70, _ => 0 }}; }}",
        "fn nested(x: i64) -> i64 { return match x { 0 => match x + 5 { 5 => 100, _ => 101 }, _ => match x { 2 => 200, _ => 201 } }; }",
        "fn bmatch(b: bool) -> i64 { return match b { true => 7, false => 9 }; }",
        f"enum Color {{ {color_def} }}",
        f"fn color_code(c: Color) -> i64 {{ return match c {{ {color_arms} }}; }}",
        "enum Shape { Dot, Line(i64), Rect(i64, i64) }",
        "fn area(s: Shape) -> i64 { return match s { Shape::Dot => 0, Shape::Line(w) => w, Shape::Rect(w, h) => w * h }; }",
        "struct S { a: i64, b: i64 }",
        f"fn fscr(s: S) -> i64 {{ return match s.b {{ {v} => 20, _ => 0 }}; }}",
        f"fn iflet_int(x: i64) -> i64 {{ if let {v} = x {{ 50 }} else {{ 60 }} }}",
        "fn iflet_bind(x: i64) -> i64 { if let y = x { y + 1 } else { 0 } }",
        f"fn iflet_side(k: i64) -> i64 {{ if let {v} = side(k) {{ 5 }} else {{ 7 }} }}",
        f"fn iflet_color(c: Color) -> i64 {{ if let Color::{colors[0]} = c {{ 1 }} else {{ 2 }} }}",
        "fn iflet_rect(s: Shape) -> i64 { if let Shape::Rect(w, h) = s { w * h } else { 0 } }",
        f"fn iflet_chain(x: i64) -> i64 {{ if let {v} = x {{ 10 }} else if let {v + 1} = x {{ 20 }} else {{ 30 }} }}",
        f"fn orpat(x: i64) -> i64 {{ return match x {{ {v} | {v + 1} | {v + 2} => 77, _ => 0 }}; }}",
        f"fn orpat_color(c: Color) -> i64 {{ return match c {{ {or_arms} }}; }}",
        f"fn rangepat(x: i64) -> i64 {{ return match x {{ {rlo}..={rhi} => 88, _ => 0 }}; }}",
        f"fn range_excl(x: i64) -> i64 {{ return match x {{ {rlo}..{rhi} => 99, _ => 0 }}; }}",
        f"fn orrange(x: i64) -> i64 {{ return match x {{ {rlo}..={rhi} => 1, {v} | {v + 5} => 2, _ => 0 }}; }}",
        f"fn oriflet(x: i64) -> i64 {{ if let {v} | {v + 1} = x {{ 5 }} else {{ 6 }} }}",
        f"fn rangeiflet(x: i64) -> i64 {{ if let {rlo}..={rhi} = x {{ 7 }} else {{ 8 }} }}",
        f"fn strmatch(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; return match s {{ {s0} => 1, {s1} => 2, _ => 3 }}; }}",
        f"fn stror(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; return match s {{ {s0} | {s1} => 1, _ => 2 }}; }}",
        f"fn striflet(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; if let {s0} = s {{ 5 }} else {{ 6 }} }}",
        f"fn strbind(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; return match s {{ {s0} => 1, other => if other == {s1} {{ 2 }} else {{ 3 }} }}; }}",
        f"fn armassign(x: i64) -> i64 {{ let mut a: i64 = 100; match x % 3 {{ 0 => a = a + 1, 1 => a += 10, _ => a = a - 5 }} return a; }}",
        f"fn gguard(x: i64) -> i64 {{ return match x {{ n if n > {v} => 100, n if n > {v} - 3 => 200, _ => 0 }}; }}",
        f"fn gbind(x: i64) -> i64 {{ return match x {{ n if n % 2 == 1 => n * 3, _ => 0 }}; }}",
        f"fn gcolor(c: Color) -> i64 {{ return match c {{ Color::{colors[0]} if c as i64 == 0 => 1, _ => 2 }}; }}",
        "fn gshape(s: Shape) -> i64 { return match s { Shape::Line(w) if w > 3 => w, Shape::Line(w) => 0 - w, _ => 0 }; }",
        f"fn gorpat(x: i64) -> i64 {{ return match x {{ {v} | {v + 1} if x > {v} => 1, _ => 0 }}; }}",
        f"fn grange(x: i64) -> i64 {{ return match x {{ n if n > 0 && n < {v + 5} => 1, _ => 0 }}; }}",
        "fn gbool(b: bool) -> i64 { return match b { true if b => 1, _ => 0 }; }",
        f"fn gwild(x: i64) -> i64 {{ return match x {{ _ if x == {v} => 9, _ => 8 }}; }}",
        "fn main() -> i64 {",
        "    let mut acc: i64 = 0;",
        f"    let arr: [i64; 4] = [{a0}, {a1}, {v}, {k}];",
        "    let mut i: i64 = 0;",
        f"    while i < {bound} {{",
        f"        let c: Color = match i % {ncolors} {{ {color_pick}, _ => Color::{colors[0]} }};",
        "        acc = acc + color_code(c) * 1000;",
        "        let s: Shape = match i % 3 { 0 => Shape::Dot, 1 => Shape::Line(i + 2), _ => Shape::Rect(i + 1, i + 2) };",
        "        acc = acc + area(s);",
        "        acc = acc + classify(i) + bindit(i);",
        "        acc = acc + bscr(i) + cscr(i) + mscr(i) + iscr(arr, i % 4) + fscr(S { a: i, b: i % 5 });",
        "        acc = acc + nested(i % 3);",
        "        acc = acc + bmatch(i % 2 == 0);",
        f"        acc = acc + bmatch(side(i) > {v});",
        "        acc = acc + iflet_int(i) + iflet_bind(i) + iflet_side(i) + iflet_chain(i);",
        "        acc = acc + orpat(i) + orpat_color(c) + rangepat(i) + range_excl(i);",
        "        acc = acc + orrange(i) + oriflet(i) + rangeiflet(i);",
        "        acc = acc + iflet_color(c) + iflet_rect(s);",
        "        acc = acc + strmatch(i % 2) + stror(i % 2) + striflet(i % 2) + strbind(i % 2);",
        "        acc = acc + armassign(i);",
        "        acc = acc + gguard(i) + gbind(i) + gorpat(i) + grange(i);",
        "        acc = acc + gcolor(c) + gshape(s) + gbool(i % 2 == 0) + gwild(i);",
        "        i = i + 1;",
        "    }",
        "    print_i64(acc);",
        "    return 0;",
        "}",
    ]
    rs = [
        'fn side(k: i64) -> i64 { println!("{}", k); k * 3 + 1 }',
        f"fn classify(x: i64) -> i64 {{ match x {{ {neg} => 10, 0 => 20, 1 => 30, _ => 40 }} }}",
        "fn bindit(x: i64) -> i64 { match x { y => y * 2 + 1 } }",
        f"fn bscr(x: i64) -> i64 {{ match x + {k} {{ {v} => 100, _ => 0 }} }}",
        f"fn cscr(k: i64) -> i64 {{ match side(k) {{ {v} => 5, _ => 7 }} }}",
        f"fn mscr(k: i64) -> i64 {{ match side(k) {{ {v} => 11, {v + 1} => 22, {v + 2} => 33, _ => 44 }} }}",
        f"fn iscr(a: [i64; 4], i: i64) -> i64 {{ match a[i as usize] {{ {v} => 70, _ => 0 }} }}",
        "fn nested(x: i64) -> i64 { match x { 0 => match x + 5 { 5 => 100, _ => 101 }, _ => match x { 2 => 200, _ => 201 } } }",
        "fn bmatch(b: bool) -> i64 { match b { true => 7, false => 9 } }",
        "#[derive(Copy, Clone)]",
        f"enum Color {{ {color_def} }}",
        f"fn color_code(c: Color) -> i64 {{ match c {{ {color_arms} }} }}",
        "#[derive(Copy, Clone)]",
        "enum Shape { Dot, Line(i64), Rect(i64, i64) }",
        "fn area(s: Shape) -> i64 { match s { Shape::Dot => 0, Shape::Line(w) => w, Shape::Rect(w, h) => w * h } }",
        "#[derive(Copy, Clone)]",
        "struct S { a: i64, b: i64 }",
        f"fn fscr(s: S) -> i64 {{ match s.b {{ {v} => 20, _ => 0 }} }}",
        f"fn iflet_int(x: i64) -> i64 {{ if let {v} = x {{ 50 }} else {{ 60 }} }}",
        "fn iflet_bind(x: i64) -> i64 { if let y = x { y + 1 } else { 0 } }",
        f"fn iflet_side(k: i64) -> i64 {{ if let {v} = side(k) {{ 5 }} else {{ 7 }} }}",
        f"fn iflet_color(c: Color) -> i64 {{ if let Color::{colors[0]} = c {{ 1 }} else {{ 2 }} }}",
        "fn iflet_rect(s: Shape) -> i64 { if let Shape::Rect(w, h) = s { w * h } else { 0 } }",
        f"fn iflet_chain(x: i64) -> i64 {{ if let {v} = x {{ 10 }} else if let {v + 1} = x {{ 20 }} else {{ 30 }} }}",
        f"fn orpat(x: i64) -> i64 {{ match x {{ {v} | {v + 1} | {v + 2} => 77, _ => 0 }} }}",
        f"fn orpat_color(c: Color) -> i64 {{ match c {{ {or_arms} }} }}",
        f"fn rangepat(x: i64) -> i64 {{ match x {{ {rlo}..={rhi} => 88, _ => 0 }} }}",
        f"fn range_excl(x: i64) -> i64 {{ match x {{ {rlo}..{rhi} => 99, _ => 0 }} }}",
        f"fn orrange(x: i64) -> i64 {{ match x {{ {rlo}..={rhi} => 1, {v} | {v + 5} => 2, _ => 0 }} }}",
        f"fn oriflet(x: i64) -> i64 {{ if let {v} | {v + 1} = x {{ 5 }} else {{ 6 }} }}",
        f"fn rangeiflet(x: i64) -> i64 {{ if let {rlo}..={rhi} = x {{ 7 }} else {{ 8 }} }}",
        f"fn strmatch(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; match s {{ {s0} => 1, {s1} => 2, _ => 3 }} }}",
        f"fn stror(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; match s {{ {s0} | {s1} => 1, _ => 2 }} }}",
        f"fn striflet(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; if let {s0} = s {{ 5 }} else {{ 6 }} }}",
        f"fn strbind(sel: i64) -> i64 {{ let s = if sel == 0 {{ {s0} }} else {{ {s1} }}; match s {{ {s0} => 1, other => if other == {s1} {{ 2 }} else {{ 3 }} }} }}",
        f"fn armassign(x: i64) -> i64 {{ let mut a: i64 = 100; match x % 3 {{ 0 => a = a + 1, 1 => a += 10, _ => a = a - 5 }} return a; }}",
        f"fn gguard(x: i64) -> i64 {{ match x {{ n if n > {v} => 100, n if n > {v} - 3 => 200, _ => 0 }} }}",
        f"fn gbind(x: i64) -> i64 {{ match x {{ n if n % 2 == 1 => n * 3, _ => 0 }} }}",
        f"fn gcolor(c: Color) -> i64 {{ match c {{ Color::{colors[0]} if c as i64 == 0 => 1, _ => 2 }} }}",
        "fn gshape(s: Shape) -> i64 { match s { Shape::Line(w) if w > 3 => w, Shape::Line(w) => 0 - w, _ => 0 } }",
        f"fn gorpat(x: i64) -> i64 {{ match x {{ {v} | {v + 1} if x > {v} => 1, _ => 0 }} }}",
        f"fn grange(x: i64) -> i64 {{ match x {{ n if n > 0 && n < {v + 5} => 1, _ => 0 }} }}",
        "fn gbool(b: bool) -> i64 { match b { true if b => 1, _ => 0 } }",
        f"fn gwild(x: i64) -> i64 {{ match x {{ _ if x == {v} => 9, _ => 8 }} }}",
        "fn main() {",
        "    let mut acc: i64 = 0;",
        f"    let arr: [i64; 4] = [{a0}, {a1}, {v}, {k}];",
        "    let mut i: i64 = 0;",
        f"    while i < {bound} {{",
        f"        let c: Color = match i % {ncolors} {{ {color_pick}, _ => Color::{colors[0]} }};",
        "        acc += color_code(c) * 1000;",
        "        let s: Shape = match i % 3 { 0 => Shape::Dot, 1 => Shape::Line(i + 2), _ => Shape::Rect(i + 1, i + 2) };",
        "        acc += area(s);",
        "        acc += classify(i) + bindit(i);",
        "        acc += bscr(i) + cscr(i) + mscr(i) + iscr(arr, i % 4) + fscr(S { a: i, b: i % 5 });",
        "        acc += nested(i % 3);",
        "        acc += bmatch(i % 2 == 0);",
        f"        acc += bmatch(side(i) > {v});",
        "        acc += iflet_int(i) + iflet_bind(i) + iflet_side(i) + iflet_chain(i);",
        "        acc += orpat(i) + orpat_color(c) + rangepat(i) + range_excl(i);",
        "        acc += orrange(i) + oriflet(i) + rangeiflet(i);",
        "        acc += iflet_color(c) + iflet_rect(s);",
        "        acc += strmatch(i % 2) + stror(i % 2) + striflet(i % 2) + strbind(i % 2);",
        "        acc += armassign(i);",
        "        acc += gguard(i) + gbind(i) + gorpat(i) + grange(i);",
        "        acc += gcolor(c) + gshape(s) + gbool(i % 2 == 0) + gwild(i);",
        "        i += 1;",
        "    }",
        '    println!("{}", acc);',
        "}",
    ]
    return "\n".join(rms) + "\n", "\n".join(rs) + "\n"


def gen_opt_program(rng):
    """A program built to exercise the three optimiser passes with randomised
    shapes, so the multi-config comparison has something to differ on.

    `expr`/`stmt` mode already calls small helpers (so inlining is reached), but
    neither emits a *self-recursive* function (TCO) or a loop with an invariant
    subexpression (LICM). This mode emits all three:

    * `rec` is in the right-spine shape `f(x) = f(g(x)) op f(h(x))` with a base
      case, which is exactly what TCO rewrites; `g`, `h`, `op`, the guard and
      the condition are randomised.
    * `licmfn` has a loop whose body reads a subexpression that does not depend
      on the loop counter, plus a counter-dependent term and a local the loop
      writes, so a wrong hoist (or a missing one that breaks the value) shows up.
    * helpers `id`/`bump`/`dbl` are called with trivial and non-trivial
      arguments, the inlining hazards `inlmulti` pins.

    Termination is guaranteed: `rec`'s arguments strictly decrease by 1 or 2 and
    the base case catches every value at or below the guard, so a small call
    argument bounds the work.
    """
    # --- TCO: self-recursive right-spine function ---
    cond = rng.choice(["n < 1", "n < 2", "n <= 1"])
    guard = rng.choice(["0", "1", "n", "2"])
    op = rng.choice(["+", "*"])
    g = rng.choice(["n - 1", "n - 2"])
    h = rng.choice(["n - 1", "n - 2"])
    n0 = rng.randint(1, 14)

    # --- LICM: loop with an invariant subexpression ---
    bound = rng.randint(1, 6)
    inv = rng.choice([
        "(a + b)", "(a * b)", "((a + b) * (a - b))", "(a << 2)",
        "((a + b) & 255)", "(a / (b % 97 + 100))", "(a % (b % 97 + 100))",
    ])
    a0, b0 = rng.randint(-40, 40), rng.randint(-40, 40)

    # --- inlining: helpers with a mutating param and a reused param ---
    extra = rng.choice([
        "print_i64(bump(id(3)) + dbl(a0));",
        "print_i64(id(rec(6)) + dbl(id(5)));",
        "print_i64(bump(a0) + id(b0));",
        "print_i64(licmfn(a0, b0) + dbl(rec(5)));",
    ])

    rms = [
        f"fn rec(n: i64) -> i64 {{ if {cond} {{ return {guard}; }} return rec({g}) {op} rec({h}); }}",
        "fn id(x: i64) -> i64 { return x; }",
        "fn bump(mut x: i64) -> i64 { x = x + 1; return x + x; }",
        "fn dbl(x: i64) -> i64 { return x + x; }",
        "fn licmfn(a: i64, b: i64) -> i64 {",
        "    let mut i: i64 = 0;",
        "    let mut t: i64 = 0;",
        "    let mut w: i64 = a;",
        "    while i < %d {" % bound,
        "        t = t + %s;" % inv,
        "        t = t + (i * 3);",
        "        t = t + w;",
        "        w = w + 1;",
        "        i = i + 1;",
        "    }",
        "    return t;",
        "}",
        "fn main() -> i64 {",
        "    let a0: i64 = %d;" % a0,
        "    let b0: i64 = %d;" % b0,
        "    print_i64(rec(%d));" % n0,
        "    print_i64(licmfn(a0, b0));",
        "    " + extra,
        "    return 0;",
        "}",
    ]
    rs = [
        f"fn rec(n: i64) -> i64 {{ if {cond} {{ return {guard}; }} rec({g}) {op} rec({h}) }}",
        "fn id(x: i64) -> i64 { x }",
        "fn bump(mut x: i64) -> i64 { x = x + 1; x + x }",
        "fn dbl(x: i64) -> i64 { x + x }",
        "fn licmfn(a: i64, b: i64) -> i64 {",
        "    let mut i: i64 = 0;",
        "    let mut t: i64 = 0;",
        "    let mut w: i64 = a;",
        "    while i < %d {" % bound,
        "        t = t + %s;" % inv,
        "        t = t + (i * 3);",
        "        t = t + w;",
        "        w = w + 1;",
        "        i = i + 1;",
        "    }",
        "    t",
        "}",
        "fn main() {",
        "    let a0: i64 = %d;" % a0,
        "    let b0: i64 = %d;" % b0,
        '    println!("{}", rec(%d));' % n0,
        '    println!("{}", licmfn(a0, b0));',
        '    println!("{}", ' + extra[len("print_i64("):-2] + ");",
        "}",
    ]
    return "\n".join(rms), "\n".join(rs)


# ------------------------------------------------------------- order mode

def gen_order_program(rng):
    """Assignment evaluation order. `stmt` mode always assigns to a plain
    `arr[i % 4]` with a side-effect-free value, so the ordering of the assignee
    place against the assigned value was never exercised. rustc evaluates the
    value first and the place second (the one expression whose operands are
    ordered right-to-left), and a divergence is only observable when *both*
    sides have effects, so every statement here gives the index and the value
    their own `li` calls. The array is printed at the end so a wrong order or a
    lost store shows up.
    """
    rms = ["fn li(t: i64) -> i64 { print_i64(t); return t; }"]
    rs = ['fn li(t: i64) -> i64 { println!("{}", t); t }']
    init = ", ".join(str(rng.randint(0, 9)) for _ in range(8))
    rms += ["fn main() -> i64 {", f"    let mut a: [i64; 8] = [{init}];"]
    rs += ["fn main() {", f"    let mut a: [i64; 8] = [{init}];"]
    # `=` and every compound operator: a compound assignment desugars to a read,
    # a combine and a write, so its operand order is its own path.
    ops = ["=", "+=", "-=", "*=", "&=", "|=", "^=", "<<=", ">>="]
    for _ in range(rng.randint(2, 5)):
        idx = rng.randint(0, 7)
        op = rng.choice(ops)
        shape = rng.choice(["v", "v+v", "v*v"])
        a, b = rng.randint(0, 9), rng.randint(0, 9)
        val = {"v": f"li({a})", "v+v": f"li({a}) + li({b})", "v*v": f"li({a}) * li({b})"}[shape]
        rms.append(f"    a[li({idx})] {op} {val};")
        rs.append(f"    a[li({idx}) as usize] {op} {val};")
    # A plain assignment whose target index is a variable but whose value calls:
    # the place has no effect, so only the value's order against nothing matters.
    rms.append("    let mut i: i64 = 3;")
    rs.append("    let mut i: i64 = 3;")
    rms.append("    a[i] = li(41);")
    rs.append("    a[i as usize] = li(41);")
    # Assignments through a `&mut` array and a `&mut` scalar: these take the
    # generic store-through-address path rather than the array-local one.
    rms.append("    let r: &mut [i64; 8] = &mut a;")
    rs.append("    let r = &mut a;")
    for _ in range(rng.randint(1, 2)):
        idx, val = rng.randint(0, 7), rng.randint(0, 9)
        rms.append(f"    r[li({idx})] = li({val});")
        rs.append(f"    r[li({idx}) as usize] = li({val});")
    rms.append("    let mut x: i64 = 0;")
    rs.append("    let mut x: i64 = 0;")
    rms.append("    let p: &mut i64 = &mut x;")
    rs.append("    let p = &mut x;")
    rms.append("    *p = li(13);")
    rs.append("    *p = li(13);")
    rms.append("    a[i] = a[i] + x;")
    rs.append("    a[i as usize] += x;")
    # A side-effecting value with a side-effecting index in one expression.
    rms.append("    a[li(i + 1)] = li(a[li(0)]);")
    rs.append("    a[li(i + 1) as usize] = li(a[li(0) as usize]);")
    rms.append("    let mut acc: i64 = 0;")
    rs.append("    let mut acc: i64 = 0;")
    for k in range(8):
        rms.append(f"    acc = acc + a[{k}] * {k + 1};")
        rs.append(f"    acc += a[{k}] * {k + 1};")
    rms.append("    print_i64(acc);")
    rs.append('    println!("{}", acc);')
    rms.append("    return 0;")
    rs.append("}")
    rms.append("}")
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
    # Per-process temp dir: two fuzz runs sharing one would overwrite each
    # other's p.s/p.rms and report a bogus assembler failure.
    tmp = f"/tmp/rmc-fuzz-{os.getpid()}"
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


# -------------------------------------------------------------- reject mode

def reject_corpus():
    """Deterministic programs that `rustc` rejects. `rmc` must reject each one
    too, with a clean error (exit 1), never accept it and never crash.

    This targets the soundness class the differential fuzzers cannot see: a
    program `rmc` wrongly accepts is never run against the rustc reference,
    because the reference does not compile. The exhaustiveness cases pin the
    `match`-boundary handling: a wildcard arm whose body is itself a `match`
    must not borrow the outer match's exhaustiveness."""
    def both(body):
        rms = (
            "enum E { A, B }\n"
            "fn main() -> i64 {\n"
            "    let a: E = E::B;\n"
            "    let b: E = E::A;\n"
            f"    let t: i64 = {body};\n"
            "    print_i64(t);\n"
            "    return 0;\n"
            "}\n"
        )
        rs = (
            "enum E { A, B }\n"
            "fn main() {\n"
            "    let a: E = E::B;\n"
            "    let b: E = E::A;\n"
            f"    let t: i64 = {body};\n"
            '    println!("{}", t);\n'
            "}\n"
        )
        return rms, rs

    def stmt(body, enum="enum E { A, B }\n", decl="let e: E = E::A;", rs_decl="let e = E::A;", rs_enum="enum E { A, B }\n"):
        """A `match` used as a statement: rmc must reject a non-exhaustive one
        just as rustc does, even though the fall-through to `()` is harmless."""
        rms = (
            f"{enum}"
            "fn main() -> i64 {\n"
            f"    {decl}\n"
            f"    {body}\n"
            "    return 0;\n"
            "}\n"
        )
        rs = (
            f"{rs_enum}"
            "fn main() {\n"
            f"    {rs_decl}\n"
            f"    {body}\n"
            "}\n"
        )
        return rms, rs

    return {
        # The outer match names A and wildcards the rest; the missing B test is
        # only inside the nested match, so the outer chain is not exhaustive.
        "nested_wildcard_other": both("match a { E::A => 1, _ => match b { E::B => 2 } }"),
        # Same shape on the same value: after desugaring this is structurally
        # identical to `match a { E::A => 1, E::B => 2 }`, so only the match
        # boundary keeps the two apart.
        "nested_wildcard_same": both("match a { E::A => 1, _ => match a { E::B => 2 } }"),
        # A plain non-exhaustive match in value position.
        "missing_variant_value": both("match a { E::A => 1 }"),
        # Non-exhaustive matches in *statement* position. The arms are unit
        # typed, so the fall-through to `()` is harmless and rmc used to accept
        # them; rustc still rejects each with E0004.
        "stmt_enum_partial": stmt("match e { E::A => { print_i64(1); } }",
                                  rs_decl="let e = E::A;"),
        "stmt_enum_one_short": stmt(
            "match e { E::A => { print_i64(1); } E::B => { print_i64(2); } }",
            enum="enum E { A, B, C }\n", decl="let e: E = E::A;",
            rs_enum="enum E { A, B, C }\n", rs_decl="let e = E::A;"),
        "stmt_int_partial": stmt("match x { 0 => { print_i64(1); } }",
                                 enum="", decl="let x: i64 = 5;",
                                 rs_enum="", rs_decl="let x = 5;"),
        # A non-exhaustive match nested in a statement's block (inside an `if`
        # and a loop body) must be rejected too.
        "stmt_in_if": stmt("if true { match e { E::A => { print_i64(1); } } }",
                           rs_decl="let e = E::A;"),
        "stmt_in_loop": stmt(
            "let mut i: i64 = 0; while i < 1 { match e { E::A => { print_i64(1); } } i = i + 1; }",
            rs_decl="let e = E::A;"),
        # A guarded arm covers nothing for exhaustiveness — the guard may be
        # false, so `E::A if ..` does not cover `A` (rustc's E0004). A match
        # that would be exhaustive without the guard is non-exhaustive with it.
        "guard_variant_gap": both("match a { E::A if a as i64 == 0 => 1, E::B => 2 }"),
        # A *guarded binding* arm covers nothing either, even though a bare
        # binding would: `e if ..` is not a catch-all.
        "guard_binding_not_catchall": both("match a { e if e as i64 == 0 => 1 }"),
        # Same, in statement position: the arms are unit-typed, so only the
        # exhaustiveness walk rejects it.
        "stmt_guard_gap": stmt("match e { E::A if 1 == 1 => { print_i64(1); } }"),
        # Assignment to an immutable binding. rmc used to accept these and let
        # codegen write the slot; rustc rejects each with E0384.
        "assign_immutable_let": (
            "fn main() -> i64 { let x: i64 = 1; x = 2; return x; }\n",
            "fn main() { let x: i64 = 1; x = 2; println!(\"{}\", x); }\n",
        ),
        "assign_immutable_param": (
            "fn f(x: i64) -> i64 { x = x + 1; return x; }\n"
            "fn main() -> i64 { return f(1); }\n",
            "fn f(x: i64) -> i64 { x = x + 1; x }\n"
            "fn main() { println!(\"{}\", f(1)); }\n",
        ),
        "assign_immutable_compound": (
            "fn main() -> i64 { let x: i64 = 1; x += 1; return x; }\n",
            "fn main() { let x: i64 = 1; x += 1; println!(\"{}\", x); }\n",
        ),
        # A read of a binding that was declared but never initialized (E0381).
        "use_uninit_scalar": (
            "fn main() -> i64 { let x: i64; return x; }\n",
            "fn main() { let x: i64; println!(\"{}\", x); }\n",
        ),
        # An uninitialized array whose elements are written in a loop and then
        # read: rustc rejects it (it cannot see that the loop covers every
        # element), so rmc must too.
        "use_uninit_array": (
            "fn main() -> i64 {\n"
            "    let mut a: [i64; 6];\n"
            "    let mut i: i64 = 0;\n"
            "    while i < 6 { a[i] = i + 1; i = i + 1; }\n"
            "    return a[0];\n"
            "}\n",
            "fn main() {\n"
            "    let mut a: [i64; 6];\n"
            "    let mut i: usize = 0;\n"
            "    while i < 6 { a[i] = i as i64 + 1; i += 1; }\n"
            '    println!("{}", a[0]);\n'
            "}\n",
        ),
        # A constant index outside the array (E0080/E0608). rmc used to emit a
        # trap only under --bounds-checks and otherwise read adjacent memory.
        "const_index_positive_oob": (
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[5]; }\n",
            "fn main() { let a: [i64; 2] = [1, 2]; println!(\"{}\", a[5]); }\n",
        ),
        "const_index_negative_oob": (
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[-1]; }\n",
            "fn main() { let a: [i64; 2] = [1, 2]; println!(\"{}\", a[-1]); }\n",
        ),
        # The magnitude `2^63` is only valid as the operand of a `-` that makes
        # it `i64::MIN`. Standing alone (E0080) or as the operand of a second
        # negation (E0080/E0608 overflow) rustc rejects it, and so must rmc.
        "i64_min_magnitude_alone": (
            "fn main() -> i64 { return 9223372036854775808; }\n",
            "fn main() { let x: i64 = 9223372036854775808; println!(\"{}\", x); }\n",
        ),
        "negate_i64_min": (
            "fn main() -> i64 { return - -9223372036854775808; }\n",
            "fn main() { let x: i64 = - -9223372036854775808; println!(\"{}\", x); }\n",
        ),
        # A string literal is a `&str`, not an integer. Adding it to an integer
        # (E0369) and passing it where an `i64` is expected must be rejected;
        # rmc used to treat the string's address as an `i64`.
        "add_int_to_str": (
            "fn main() -> i64 { let s: i64 = \"hi\"; let x: i64 = s + 1; return x; }\n",
            "fn main() { let s = \"hi\"; let x = s + 1; println!(\"{}\", x); }\n",
        ),
        "print_str_int": (
            "fn main() -> i64 { let x: i64 = 42; print_str(x); return 0; }\n",
            "fn main() { let s: &str = 42; println!(\"{}\", s); }\n",
        ),
        # Comparing a `str` with an integer. rmc used to compare the string's
        # address against the integer, which compiled to nonsense; rustc rejects
        # it (E0308). The two are different types, so both must refuse.
        "str_vs_int": (
            "fn main() -> i64 { if \"a\" == 1 { return 1; } return 0; }\n",
            "fn main() { if \"a\" == 1 { } }\n",
        ),
        # Writing through a place whose root binding is not `mut`. The first
        # mutability check only looked at a bare `Var` target, so these all
        # compiled. rustc rejects each (E0594/E0596).
        "field_assign_immutable": (
            "struct P { x: i64, y: i64 }\n"
            "fn main() -> i64 { let p: P = P { x: 1, y: 2 }; p.x = 2; return p.x; }\n",
            "struct P { x: i64, y: i64 }\n"
            "fn main() -> i64 { let p = P { x: 1, y: 2 }; p.x = 2; p.x }\n",
        ),
        "elem_assign_immutable": (
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; a[0] = 5; return a[0]; }\n",
            "fn main() -> i64 { let a = [1i64, 2]; a[0] = 5; a[0] }\n",
        ),
        "compound_field_immutable": (
            "struct P { x: i64, y: i64 }\n"
            "fn main() -> i64 { let p: P = P { x: 1, y: 2 }; p.x += 1; return p.x; }\n",
            "struct P { x: i64, y: i64 }\n"
            "fn main() -> i64 { let p = P { x: 1, y: 2 }; p.x += 1; p.x }\n",
        ),
        "borrow_immutable_as_mut": (
            "fn main() -> i64 { let x: i64 = 1; let r: &mut i64 = &mut x; return 0; }\n",
            "fn main() { let x = 1i64; let r = &mut x; let _ = r; }\n",
        ),
        "write_through_shared_ref": (
            "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; *r = 5; return 0; }\n",
            "fn main() { let x = 1i64; let r = &x; *r = 5; }\n",
        ),
        "shared_ref_field_write": (
            "struct P { x: i64, y: i64 }\n"
            "fn main() -> i64 { let p: P = P { x: 1, y: 2 }; let r: &P = &p; r.x = 5; return 0; }\n",
            "struct P { x: i64, y: i64 }\n"
            "fn main() -> i64 { let p = P { x: 1, y: 2 }; let r = &p; r.x = 5; }\n",
        ),
        "partial_init_then_read": (
            "fn main() -> i64 { let a: [i64; 2]; a[0] = 1; return a[0]; }\n",
            "fn main() -> i64 { let a: [i64; 2]; a[0] = 1; a[0] }\n",
        ),
        # Duplicate parameter names (E0415) and a `main` that takes arguments
        # (E0580); rmc used to bind the duplicates to one slot and accept any
        # `main`.
        "duplicate_parameter": (
            "fn f(x: i64, x: i64) -> i64 { return x; }\n"
            "fn main() -> i64 { return f(1, 2); }\n",
            "fn f(x: i64, x: i64) -> i64 { x }\nfn main() { let _ = f(1, 2); }\n",
        ),
        "main_with_parameter": (
            "fn main(x: i64) -> i64 { return x; }\n",
            "fn main(x: i64) { let _ = x; }\n",
        ),
        # Constant arithmetic that rustc denies: division by zero, overflow and
        # an out-of-range shift all trap, so rmc rejects the constant case.
        "const_div_by_zero": (
            "fn main() -> i64 { let x: i64 = 1 / 0; return x; }\n",
            "fn main() { let x = 1i64 / 0; let _ = x; }\n",
        ),
        "const_overflow_add": (
            "fn main() -> i64 { let x: i64 = 9223372036854775807 + 1; return x; }\n",
            "fn main() { let x = 9223372036854775807i64 + 1; let _ = x; }\n",
        ),
        "const_shift_out_of_range": (
            "fn main() -> i64 { return 1 << 64; }\n",
            "fn main() { let x = 1i64 << 64; let _ = x; }\n",
        ),
        # A constant index written as an expression must fold like a literal, or
        # an out-of-range one reads past the array (E0080 in rustc).
        "const_index_expr_oob": (
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; return a[1 + 4]; }\n",
            "fn main() -> i64 { let a = [1i64, 2]; a[1 + 4] }\n",
        ),
        # A constant *local* used as an index must fold too. rustc's constant
        # propagation knows `i`'s value, so it denies the out-of-range index;
        # rmc used to read past the array (the memory-safety gap).
        "const_local_index_oob": (
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let i: i64 = -1; return a[i]; }\n",
            "fn main() { let a = [1i64, 2]; let i: i64 = -1; println!(\"{}\", a[i as usize]); }\n",
        ),
        "const_local_index_expr_oob": (
            "fn main() -> i64 { let a: [i64; 2] = [1, 2]; let i: i64 = 1 - 3; return a[i]; }\n",
            "fn main() { let a = [1i64, 2]; let i: i64 = 1 - 3; println!(\"{}\", a[i as usize]); }\n",
        ),
        # A constant local that overflows or divides by zero is folded like a
        # literal (E0080 in rustc).
        "const_local_overflow": (
            "fn main() -> i64 { let x: i64 = 9223372036854775807; let y: i64 = x + 1; return y; }\n",
            "fn main() { let x = 9223372036854775807i64; let y = x + 1; let _ = y; }\n",
        ),
        "const_local_div_zero": (
            "fn main() -> i64 { let d: i64 = 0; let y: i64 = 1 / d; return y; }\n",
            "fn main() { let d = 0i64; let y = 1i64 / d; let _ = y; }\n",
        ),
        # A reference may only be cast to a reference of the same type, may not
        # gain mutability, and may not become a scalar (E0605/E0606). rmc's
        # `i64 as &T` raw pointer cast is a documented extension and stays.
        "cast_ref_to_scalar": (
            "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; return r as i64; }\n",
            "fn main() { let x = 1i64; let r = &x; let _ = r as i64; }\n",
        ),
        "cast_shared_ref_to_mut": (
            "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; "
            "let s: &mut i64 = r as &mut i64; return 0; }\n",
            "fn main() { let x = 1i64; let r = &x; let s = r as &mut i64; let _ = s; }\n",
        ),
        "cast_ref_to_other_ref": (
            "fn main() -> i64 { let x: i64 = 1; let r: &i64 = &x; "
            "let s: &bool = r as &bool; return 0; }\n",
            "fn main() { let x = 1i64; let r = &x; let s = r as &bool; let _ = s; }\n",
        ),
        "cast_bool_to_ref": (
            "fn main() -> i64 { let b: bool = true; let r: &i64 = b as &i64; return 0; }\n",
            "fn main() { let b = true; let r = b as &i64; let _ = r; }\n",
        ),
        # `if let` is sugar for `match`, but unlike `match` it need not be
        # exhaustive. What it may *not* do is have a non-unit `then` with no
        # `else` (E0308): the fall-through `_` arm is `()`, so the branch types
        # differ. rmc must reject it too.
        "iflet_nonunit_without_else": (
            "fn main() -> i64 { let o: i64 = 3; if let 3 = o { 1 } return 0; }\n",
            "fn main() { let o = 3i64; if let 3 = o { 1 } }\n",
        ),
        # An *irrefutable* pattern makes the fall-through `_` arm dead, but rustc
        # still type-checks it: a non-unit `then` with a `()` fallback is
        # rejected. The `if true` wrapper in `irrefutable_arm` is what carries
        # this check; dropping the dead arm (as for `match`) would accept it.
        "iflet_irrefutable_nonunit_without_else": (
            "fn main() -> i64 { let o: i64 = 3; if let y = o { 1 } return 0; }\n",
            "fn main() { let o = 3i64; if let y = o { 1 } }\n",
        ),
    }


def run_reject():
    # Per-process temp dir: two fuzz runs sharing one would overwrite each
    # other's p.s/p.rms and report a bogus assembler failure.
    tmp = f"/tmp/rmc-fuzz-{os.getpid()}"
    os.makedirs(tmp, exist_ok=True)
    failures = 0
    corpus = reject_corpus()
    for name, (rms, rs) in corpus.items():
        open(f"{tmp}/{name}.rms", "w").write(rms)
        open(f"{tmp}/{name}.rs", "w").write(rs)
        # The reference must reject it, or the corpus entry proves nothing.
        ref = run([RUSTC, "-A", "warnings", "--edition", "2021", f"{tmp}/{name}.rs",
                   "-o", f"{tmp}/{name}.ref"])
        if ref.returncode == 0:
            print(f"[reject] {name}: rustc ACCEPTED the program; corpus is wrong")
            failures += 1
            continue
        p = run([RMC, f"{tmp}/{name}.rms", "-o", f"{tmp}/{name}.bin"])
        if p.returncode < 0 or b"panicked" in p.stderr or b"overflowed its stack" in p.stderr:
            print(f"[reject] {name}: CRASH (exit {p.returncode})")
            failures += 1
        elif p.returncode == 0:
            print(f"[reject] {name}: rmc ACCEPTED a program rustc rejects\n{rms}")
            failures += 1
        elif p.returncode != 1:
            print(f"[reject] {name}: unexpected exit {p.returncode}")
            failures += 1
    if failures == 0:
        print(f"reject: all {len(corpus)} rustc-rejected programs are rejected cleanly")
    return 1 if failures else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["expr", "stmt", "opt", "arith", "nest", "pat", "order", "shift", "cast", "assoc", "robust", "reject"], default="expr")
    ap.add_argument("--n", type=int, default=200)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--keep", default=None, help="write failing programs here")
    ap.add_argument("--bounds", action="store_true",
                    help="also compile with --bounds-checks, to exercise the trap codegen")
    ap.add_argument("--encoding", action="store_true",
                    help="also compare the in-process .text against the as+ld "
                         ".text instruction-for-instruction (catches encoding "
                         "drift that a behaviour comparison cannot see)")
    ap.add_argument("--overflow", action="store_true",
                    help="also compile with --overflow-checks, against a rustc built "
                         "with -C overflow-checks=on")
    args = ap.parse_args()

    if not os.path.exists(RMC):
        print(f"rmc not found at {RMC}; build with `cargo build --release`", file=sys.stderr)
        return 2
    if args.mode == "robust":
        return run_robust()
    if args.mode == "reject":
        if not rustc_available():
            print("rustc not found; set RUSTC or install a toolchain", file=sys.stderr)
            return 2
        return run_reject()
    if not rustc_available():
        print("rustc not found; set RUSTC or install a toolchain", file=sys.stderr)
        return 2
    # `arith` mode generates overflowing arithmetic, so the `--overflow-checks`
    # configs are always wanted: the whole point is to compare the trap.
    if args.mode in ("arith", "shift"):
        args.overflow = True

    # Per-process temp dir: two fuzz runs sharing one would overwrite each
    # other's p.s/p.rms and report a bogus assembler failure.
    tmp = f"/tmp/rmc-fuzz-{os.getpid()}"
    os.makedirs(tmp, exist_ok=True)
    gen = {"expr": gen_expr_program, "stmt": gen_stmt_program,
           "opt": gen_opt_program, "arith": gen_arith_program,
           "nest": gen_nest_program, "pat": gen_pat_program,
           "order": gen_order_program, "shift": gen_shift_program,
           "cast": gen_cast_program, "assoc": gen_assoc_program}[args.mode]
    rng = random.Random(args.seed)
    # The optimiser passes are off in the rustc reference only in the corpus
    # suite; a fuzz program is otherwise only checked with every pass on, so a
    # bug that a single pass introduces (a mis-inlined assignment, a bad hoist)
    # is invisible here. Recompile each program with each pass off, like the
    # corpus matrix. `all-off` is the plainest code path.
    # Each config is (label, rmc flags, rustc flags). The rustc flags select the
    # reference build: the default `rustc -O` wraps on overflow, so an
    # `--overflow-checks` run must be compared against `-C overflow-checks=on`.
    configs = [
        ("", [], []),
        ("no-inline", ["--no-inline"], []),
        ("no-licm", ["--no-licm"], []),
        ("no-tco", ["--no-tco"], []),
        ("all-off", ["--no-inline", "--no-licm", "--no-tco"], []),
    ]
    # `--bounds-checks` turns on the trap path, which the default configs never
    # reach: every generated index is in bounds (see the generators), so no
    # program should trap and the outputs must still match rustc. A spurious
    # trap, or a miscompile the check codegen introduces, shows up here.
    if args.bounds:
        configs += [
            ("bounds", ["--bounds-checks"], []),
            ("bounds-all-off", ["--bounds-checks", "--no-inline", "--no-licm", "--no-tco"], []),
        ]
    # `--overflow-checks` traps on `+`/`-`/`*` overflow at runtime; the reference
    # must overflow-check too or the two disagree by design. `<<`/`>>` are not
    # value-checked by either, so a shifted value that overflows wraps in both.
    if args.overflow:
        ovf = ["-C", "overflow-checks=on"]
        configs += [
            ("ovf", ["--overflow-checks"], ovf),
            ("ovf-all-off", ["--overflow-checks", "--no-inline", "--no-licm", "--no-tco"], ovf),
        ]
    failures = 0
    for t in range(args.n):
        rms, rs = gen(rng)
        open(f"{tmp}/p.rms", "w").write(rms)
        open(f"{tmp}/p.rs", "w").write(rs)
        # Shift mode reads its counts from argv; every other mode ignores
        # arguments. `count` is deliberately drawn from the full i64 range so
        # out-of-range shifts (negative, >= 64) are exercised.
        argv = []
        if args.mode == "shift":
            argv = [str(rng.choice([rng.randint(-4, 4), rng.randint(60, 68),
                                    rng.randint(0, 63), -1, 64, 63, 0])),
                    str(rng.randint(-1000, 1000))]
        # One rustc reference per distinct flag set, built once and cached.
        refs = {}
        skip = False
        for rf in {tuple(cf) for (_, _, cf) in configs}:
            rbin = f"{tmp}/q{len(refs)}.bin"
            c = run([RUSTC, "-O", "-A", "warnings", "--edition", "2021",
                     *rf, f"{tmp}/p.rs", "-o", rbin])
            if c.returncode != 0:
                skip = True  # not a valid rust reference program
                break
            refs[rf] = (rbin, run([rbin] + argv))
        if skip:
            continue
        bad = None
        for label, flags, rf in configs:
            c = run([RMC] + flags + [f"{tmp}/p.rms", "-o", f"{tmp}/p.bin"])
            if c.returncode != 0:
                # rmc rejects the program. If rustc rejects it too the generator
                # produced an ill-formed program (a constant shift or overflow
                # that rmc now folds), so skip it; only an rmc-only rejection is
                # a bug.
                r = run([RUSTC, "-O", "-A", "warnings", "--edition", "2021", f"{tmp}/p.rs", "-o", f"{tmp}/q.bin"])
                if r.returncode != 0:
                    bad = None
                    break
                bad = f"rmc[{label}] failed to compile:\n{c.stderr.decode(errors='replace')}"
                break
            o1 = run([f"{tmp}/p.bin"] + argv)
            o2 = refs[tuple(rf)][1]
            if (o1.returncode, o1.stdout) != (o2.returncode, o2.stdout):
                bad = (f"rmc[{label}] argv={argv} exit={o1.returncode} stdout={o1.stdout!r}\n"
                       f"  rustc    exit={o2.returncode} stdout={o2.stdout!r}")
                break
            if args.encoding:
                enc = encoding_divergence(f"{tmp}/p.rms", flags, tmp)
                if enc is not None and enc is not True:
                    k, ip, asm = enc
                    bad = (f"rmc[{label}] in-process vs as differ at instruction {k}\n"
                           f"  in-process = {ip}\n  as+ld      = {asm}")
                    break
        if bad:
            print(f"[{args.mode} seed={args.seed} iter={t}] DIVERGENCE\n{rms}\n  {bad}")
            if args.keep:
                os.makedirs(args.keep, exist_ok=True)
                open(f"{args.keep}/fuzz-{args.mode}-{args.seed}-{t}.rms", "w").write(rms)
            failures += 1
            break
    if failures == 0:
        print(f"{args.n} {args.mode} programs x {len(configs)} configs, seed {args.seed}: all match rustc")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
