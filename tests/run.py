#!/usr/bin/env python3
"""Differential test suite: every case is compiled by rmc and by rustc and the
two programs must print identical output and exit with the same status.

Each case lives in `tests/cases/` as a pair `NAME.rms` (rmc) and `NAME.rs`
(Rust). The two files are written to behave identically, so any divergence in
stdout or exit code is a compiler bug. `tests/args.json` optionally maps a case
name to a list of argv vectors; each vector is one run, and the case passes
only if every run matches. This is what catches bugs like a folded comparison
silently inverting an inequality.

Each case is also recompiled with the optimiser passes turned off in turn
(`--no-inline`, `--no-licm`, `--no-tco`, and all three), because a bug in a pass
can hide behind another pass or only appear on the unoptimised path. The Rust
twin is built once and reused across the five configurations.

Usage: python3 tests/run.py [-v]
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
CASES = os.path.join(ROOT, "cases")
OUT = "/tmp/rmctests"
ARGS_FILE = os.path.join(ROOT, "args.json")


def load_args():
    if os.path.exists(ARGS_FILE):
        with open(ARGS_FILE) as f:
            return json.load(f)
    return {}


def extra_flags(name):
    """Per-case compiler flags. A `NAME.overflow` marker turns on overflow
    checks for both compilers, so the case can be diffed like any other."""
    if os.path.exists(os.path.join(CASES, f"{name}.overflow")):
        return ["--overflow-checks"], ["-C", "overflow-checks=on"]
    if os.path.exists(os.path.join(CASES, f"{name}.bounds")):
        # rustc checks array bounds by default, so only rmc needs the flag.
        return ["--bounds-checks"], []
    return [], []


def compile_rust(name):
    """Compile the Rust twin once; the optimizer matrix reuses it.

    The Rust binary does not depend on which rmc passes are on, so compiling it
    per combo would be 5x the rustc cost for no extra coverage.
    """
    rs = os.path.join(CASES, f"{name}.rs")
    rust_bin = os.path.join(OUT, f"{name}.rust")
    _, rustc_flags = extra_flags(name)
    p = subprocess.run(["rustc", "-O"] + rustc_flags + [rs, "-o", rust_bin],
                       capture_output=True)
    if p.returncode != 0:
        return None, f"rustc failed:\n{p.stderr.decode(errors='replace')}"
    return rust_bin, None


def compile_rmc(name, optimizer_flags):
    rms = os.path.join(CASES, f"{name}.rms")
    rmc_bin = os.path.join(OUT, f"{name}.rmc")
    rmc_flags, _ = extra_flags(name)
    p = subprocess.run([RMC] + rmc_flags + list(optimizer_flags) + [rms, "-o", rmc_bin],
                       capture_output=True)
    if p.returncode != 0:
        return None, f"rmc failed:\n{p.stderr.decode(errors='replace')}"
    return rmc_bin, None


def run_both(rmc_bin, rust_bin, argv):
    """Run both binaries with `argv` and return `(rmc, rust)` results, each
    `("ok", exit_status, stdout_bytes)`."""
    def run(binary):
        p = subprocess.run([binary] + argv, capture_output=True)
        return ("ok", p.returncode, p.stdout)

    return run(rmc_bin), run(rust_bin)


def check_bounds_traps(combos):
    """Assemble and link out-of-range programs with a plain `ld` (no `-n`),
    which maps sections by permission. Every one must report the error and exit
    101, not fault. This catches a bounds handler emitted into non-executable
    `.rodata`, and index paths that skip the check (a block or call base, a
    struct-field array, a computed or negative index, and a store).

    Each case is run under every optimiser configuration in `combos`, so a pass
    cannot hide a check: a hoist or inline that drops the `cmp`/`jae` would let
    the program read or write out of range and exit without the message.
    """
    # Each case is (source, required stdout prefix). The prefix checks that a
    # base with a side effect runs before the trap: Rust evaluates the base and
    # index before the bounds check, so `make()[2]` must print before it exits.
    # A constant index out of range is rejected at compile time, so every
    # runtime trap uses a variable index (the value is known but not a literal).
    # The index is a parameter, so it is not a constant local: rmc would fold
    # `let i = 9` (matching rustc, which rejects it), leaving the runtime path
    # unreachable. A parameter value is only known at run time.
    cases = {
        "block": ("fn f(i: i64) -> i64 { let v: i64 = { let t: [i64; 3] = [10, 20, 30]; t }[i]; return v; } fn main() -> i64 { return f(9); }", b""),
        "call": ("fn make() -> [i64; 3] { let a: [i64; 3] = [10, 20, 30]; return a; } fn f(i: i64) -> i64 { return make()[i]; } fn main() -> i64 { return f(10); }", b""),
        "base_effect": ("fn make() -> [i64; 1] { print_i64(7); return [4]; } fn f(i: i64) -> i64 { return make()[i]; } fn main() -> i64 { return f(2); }", b"7\n"),
        "local": ("fn f(i: i64) -> i64 { let a: [i64; 3] = [1, 2, 3]; return a[i]; } fn main() -> i64 { return f(3); }", b""),
        "negative": ("fn f(i: i64) -> i64 { let a: [i64; 3] = [1, 2, 3]; return a[i]; } fn main() -> i64 { return f(-1); }", b""),
        "computed": ("fn f(i: i64) -> i64 { let a: [i64; 3] = [1, 2, 3]; return a[i + 1]; } fn main() -> i64 { return f(3); }", b""),
        "field": ("struct S { n: i64, a: [i64; 3] } fn f(i: i64) -> i64 { let s: S = S { n: 0, a: [4, 5, 6] }; return s.a[i]; } fn main() -> i64 { return f(5); }", b""),
        "store": ("fn f(i: i64) -> i64 { let mut a: [i64; 3] = [1, 2, 3]; a[i] = 9; return a[0]; } fn main() -> i64 { return f(4); }", b""),
    }
    failures = 0
    for name, (src, want_stdout) in cases.items():
        rms = os.path.join(OUT, f"trap_{name}.rms")
        with open(rms, "w") as f:
            f.write(src)
        for label, opt_flags in combos:
            asm = subprocess.run([RMC, "--bounds-checks"] + list(opt_flags)
                                 + ["--emit-asm", rms], capture_output=True)
            if asm.returncode != 0:
                print(f"FAIL trap {name}{label}: rmc failed:\n{asm.stderr.decode(errors='replace')}")
                failures += 1
                continue
            s_path = os.path.join(OUT, f"trap_{name}.s")
            o_path = os.path.join(OUT, f"trap_{name}.o")
            exe = os.path.join(OUT, f"trap_{name}.exe")
            with open(s_path, "wb") as f:
                f.write(asm.stdout)
            p = subprocess.run(["as", s_path, "-o", o_path], capture_output=True)
            if p.returncode != 0:
                print(f"FAIL trap {name}{label}: as failed:\n{p.stderr.decode(errors='replace')}")
                failures += 1
                continue
            # A plain link honours section permissions, unlike the CLI's `-n`.
            p = subprocess.run(["ld", "-o", exe, o_path], capture_output=True)
            if p.returncode != 0:
                print(f"FAIL trap {name}{label}: ld failed:\n{p.stderr.decode(errors='replace')}")
                failures += 1
                continue
            p = subprocess.run([exe], capture_output=True)
            if p.returncode != 101 or b"index out of bounds" not in p.stderr:
                print(f"FAIL trap {name}{label}: exit={p.returncode} stderr={p.stderr!r} (want 101 + message)")
                failures += 1
            elif not p.stdout.startswith(want_stdout):
                print(f"FAIL trap {name}{label}: stdout={p.stdout!r} (want prefix {want_stdout!r}; base must run first)")
                failures += 1
    if failures == 0:
        print(f"all {len(cases)} bounds traps exit 101 under a plain `ld` link, "
              f"in {len(combos)} optimiser configurations")
    return failures


def check_stripped():
    """Size behaviour of both link paths.

    The default in-process link writes no symbol table and no section-header
    table, so it is the smallest artifact and needs no strip step. The
    `--binutils` fallback links with `ld`, which keeps the symbol table by
    default; there `--no-strip` keeps it and `--min-size` drops the section
    headers as well. Pin the ordering and that every artifact runs.
    """
    rms = os.path.join(OUT, "strip_hello.rms")
    with open(rms, "w") as f:
        f.write("fn main() -> i64 { print_i64(1); return 0; }\n")

    default = os.path.join(OUT, "strip_default.bin")
    binstrip = os.path.join(OUT, "strip_binutils.bin")
    binkeep = os.path.join(OUT, "strip_binutils_kept.bin")
    binmin = os.path.join(OUT, "strip_binutils_min.bin")

    builds = [
        (default, [RMC, rms, "-o", default]),
        (binstrip, [RMC, "--binutils", rms, "-o", binstrip]),
        (binkeep, [RMC, "--binutils", "--no-strip", rms, "-o", binkeep]),
        (binmin, [RMC, "--binutils", "--min-size", rms, "-o", binmin]),
    ]
    for path, cmd in builds:
        p = subprocess.run(cmd, capture_output=True)
        if p.returncode != 0:
            print(f"FAIL strip: {cmd[1:-2]} failed:\n{p.stderr.decode(errors='replace')}")
            return 1
        p = subprocess.run([path], capture_output=True)
        if p.returncode != 0 or p.stdout != b"1\n":
            print(f"FAIL strip: {path} misbehaved: rc={p.returncode} out={p.stdout!r}")
            return 1

    # The default link carries no section-header table at all.
    p = subprocess.run(["readelf", "-SW", default], capture_output=True)
    if p.returncode != 0 or b"There are no sections" not in p.stdout:
        print("FAIL strip: default binary has a section-header table")
        return 1

    # `--binutils` strips symbols by default; `--no-strip` keeps them.
    p = subprocess.run(["readelf", "-sW", binstrip], capture_output=True)
    if p.returncode != 0:
        print("FAIL strip: readelf failed on the binutils binary")
        return 1
    if b"main" in p.stdout:
        print("FAIL strip: symbol table present in the binutils default binary")
        return 1
    p = subprocess.run(["readelf", "-sW", binkeep], capture_output=True)
    if p.returncode != 0 or b"main" not in p.stdout:
        print("FAIL strip: --no-strip dropped the symbol table")
        return 1

    # Size ordering: direct < binutils --min-size < binutils stripped < kept.
    order = [default, binmin, binstrip, binkeep]
    sizes = [os.path.getsize(x) for x in order]
    if any(a >= b for a, b in zip(sizes, sizes[1:])):
        print(f"FAIL strip: artifact sizes not increasing {sizes}")
        return 1

    # An absolute bound. A one-line program is a few hundred bytes; a 4 KB
    # regression (e.g. a section that should be in the RX segment getting
    # page-aligned into its own) would not change the ordering above but would
    # blow this bound.
    if sizes[0] >= 1024:
        print(f"FAIL strip: default artifact is {sizes[0]} bytes (want < 1024)")
        return 1
    return 0


def check_link_paths_agree():
    """The in-process assembler and the `as`+`ld` path must agree.

    The default link no longer shells out, so the two paths can drift. Compile
    a handful of cases that between them exercise every encoding the in-process
    assembler implements (immediates, byte moves, sign extension, RIP-relative
    data, traps) and require identical stdout and exit status from both.
    """
    cases = ["arith", "div", "io", "utf8", "structs", "match", "bounds", "heap"]
    failures = 0
    for name in cases:
        rms = os.path.join(CASES, f"{name}.rms")
        if not os.path.exists(rms):
            continue
        direct = os.path.join(OUT, f"paths_{name}.direct")
        binut = os.path.join(OUT, f"paths_{name}.binutils")
        p = subprocess.run([RMC, rms, "-o", direct], capture_output=True)
        q = subprocess.run([RMC, "--binutils", rms, "-o", binut], capture_output=True)
        if p.returncode != 0 or q.returncode != 0:
            print(f"FAIL paths {name}: compile failed "
                  f"(direct={p.returncode} binutils={q.returncode})")
            failures += 1
            continue
        for argv in load_args().get(name, [[]]):
            a = subprocess.run([direct] + argv, capture_output=True)
            b = subprocess.run([binut] + argv, capture_output=True)
            if (a.returncode, a.stdout) != (b.returncode, b.stdout):
                print(f"FAIL paths {name} {argv}: direct={a.returncode}/{a.stdout[:40]!r} "
                      f"binutils={b.returncode}/{b.stdout[:40]!r}")
                failures += 1
                break
    if failures == 0:
        print("in-process link and the `as`+`ld` path produce identical behaviour")
    return failures


def disassemble_text(path, raw_text):
    """Normalised instruction list for a `.text` blob, or a linked binary.

    Branch and call targets are folded to a placeholder (the two paths lay code
    out differently) and alignment `nop`s are dropped (GNU `as` emits the
    multi-byte forms, the in-process encoder the one-byte form). What is left is
    the instruction stream, which must be identical.
    """
    import re
    if raw_text:
        blob = os.path.join(OUT, "_enc_raw.bin")
        with open(blob, "wb") as f:
            f.write(raw_text)
        cmd = ["objdump", "-D", "-b", "binary", "-m", "i386:x86-64", blob]
    else:
        cmd = ["objdump", "-d", path]
    p = subprocess.run(cmd, capture_output=True)
    if p.returncode != 0:
        return None
    ins = []
    for line in p.stdout.decode(errors="replace").splitlines():
        m = re.match(r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{2} )+\s*(.*)$", line)
        if not m:
            continue
        text = m.group(1).strip()
        # `objdump -b binary` prints immediates in hex, ELF mode in decimal, so
        # fold immediates first, then remaining addresses.
        text = re.sub(r"\$-?0x[0-9a-f]+", "$I", text)
        text = re.sub(r"\$-?[0-9]+", "$I", text)
        text = re.sub(r"0x[0-9a-f]+", "A", text)
        text = re.sub(r"\s+", " ", text)
        if not text:
            # A long multi-byte `nop` wraps: objdump prints its trailing bytes
            # on a continuation line with no mnemonic.
            continue
        # Alignment padding: the encoder emits single-byte `nop`s, `as` the
        # multi-byte forms that objdump renders as `nopw`/`nopl`/`data16 ...`
        # or the `xchg %ax,%ax` idiom. Drop them all; they carry no behaviour.
        if "nop" in text or text in ("xchg %ax,%ax",) or text.startswith("data16"):
            continue
        ins.append(text)
    return ins


def in_process_text(path):
    """The RX LOAD segment's file bytes, i.e. the assembled `.text`."""
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


def check_encoding_matches_as():
    """The in-process encoder must agree with `as` instruction-for-instruction.

    `check_link_paths_agree` compares behaviour, which cannot see an encoding
    that is merely slower: `enc_div` set REX.W on `div %ebx`, turning the 32-bit
    divide into the 64-bit one, and `.p2align 4` aligned to 4 bytes instead of
    16. Both produced identical stdout and exit status while costing 14% at run
    time on a divide-bound loop. This compares the decoded instruction streams,
    so an operand-size or alignment regression fails the suite.
    """
    names = sorted(f[:-4] for f in os.listdir(CASES) if f.endswith(".rms"))
    # Encoding is chosen by the codegen, so the optimiser passes can change which
    # instruction forms appear. Check the default and the plainest path.
    combos = [("", []), (" [no-inline]", ["--no-inline"])]
    failures = 0
    for name in names:
        rms = os.path.join(CASES, f"{name}.rms")
        for label, flags in combos:
            tag = name + label.replace(" ", "_").replace("[", "").replace("]", "")
            direct = os.path.join(OUT, f"enc_{tag}.direct")
            binut = os.path.join(OUT, f"enc_{tag}.binutils")
            p = subprocess.run([RMC, rms, "-o", direct] + flags, capture_output=True)
            q = subprocess.run([RMC, "--binutils", rms, "-o", binut] + flags,
                               capture_output=True)
            if p.returncode != 0 or q.returncode != 0:
                print(f"FAIL encoding {name}{label}: compile failed")
                failures += 1
                continue
            raw = in_process_text(direct)
            # dump the as+ld `.text` with objcopy for the same treatment
            blob = os.path.join(OUT, f"_enc_{tag}.text")
            r = subprocess.run(["objcopy", "-O", "binary", "--only-section=.text",
                                binut, blob], capture_output=True)
            if raw is None or r.returncode != 0:
                print(f"FAIL encoding {name}{label}: could not extract .text")
                failures += 1
                continue
            a = disassemble_text(direct, raw)
            b = disassemble_text(binut, open(blob, "rb").read())
            if a is None or b is None:
                print(f"FAIL encoding {name}{label}: objdump failed")
                failures += 1
                continue
            # The in-process RX segment packs `.rodata` after `.text`, so the
            # disassembly of its tail is string data, not code. Compare the
            # common prefix; a genuine encoding difference shows up mid-stream,
            # where the instruction counts still line up.
            n = min(len(a), len(b))
            if a[:n] != b[:n]:
                k = next((i for i in range(n) if a[i] != b[i]), n)
                print(f"FAIL encoding {name}{label}: in-process vs as differ at "
                      f"instruction {k}")
                print(f"    in-process = {a[k] if k < len(a) else '<end>'}")
                print(f"    as+ld      = {b[k] if k < len(b) else '<end>'}")
                failures += 1
    if failures == 0:
        print("in-process encoder matches `as` instruction-for-instruction")
    return failures


def check_no_rwx():
    """The linked executable must have no writable-and-executable LOAD segment.

    The CLI used to link with `ld -n`, which packs every section into one
    read+write+execute segment: a W^X violation, and an invitation to exploit if
    the program ever wrote to a page the kernel would then execute. A plain
    `readelf -l` on a compiled `hello` catches a regression to `-n` (or any
    other linker flag that reintroduces a RWX segment).
    """
    rms = os.path.join(OUT, "rwx_hello.rms")
    with open(rms, "w") as f:
        f.write("fn main() -> i64 { print_i64(1); return 0; }\n")
    exe = os.path.join(OUT, "rwx_hello.bin")
    p = subprocess.run([RMC, rms, "-o", exe], capture_output=True)
    if p.returncode != 0:
        print(f"FAIL rwx: rmc failed:\n{p.stderr.decode(errors='replace')}")
        return 1
    p = subprocess.run(["readelf", "-lW", exe], capture_output=True)
    if p.returncode != 0:
        print("FAIL rwx: readelf failed")
        return 1
    for line in p.stdout.decode().splitlines():
        line = line.strip()
        if line.startswith("LOAD"):
            # Columns: type offset vaddr paddr filesz memsz flg align
            flg = line.split()[6]
            if "W" in flg and "E" in flg:
                print(f"FAIL rwx: writable+executable LOAD segment: {line}")
                return 1
    return 0


def main():
    verbose = "-v" in sys.argv
    os.makedirs(OUT, exist_ok=True)
    args = load_args()

    names = sorted(f[:-4] for f in os.listdir(CASES) if f.endswith(".rms"))
    if not names:
        print("no test cases found", file=sys.stderr)
        return 1

    # The optimiser passes are off by default in the Rust twin (rustc runs them
    # all), so a bug that only shows with one pass disabled would otherwise go
    # unnoticed. Each combo recompiles the case with rmc and reruns both
    # binaries; the Rust twin is built once. `all-off` is the strongest check -
    # every pass off at once is the compiler's plainest code path.
    combos = [
        ("", []),
        (" [no-inline]", ["--no-inline"]),
        (" [no-licm]", ["--no-licm"]),
        (" [no-tco]", ["--no-tco"]),
        (" [all-off]", ["--no-inline", "--no-licm", "--no-tco"]),
    ]

    failures = 0
    for name in names:
        if not os.path.exists(os.path.join(CASES, f"{name}.rs")):
            print(f"FAIL {name}: missing Rust twin ({name}.rs)")
            failures += 1
            continue

        rust_bin, err = compile_rust(name)
        if err:
            print(f"FAIL {name}: {err}")
            failures += 1
            continue

        for label, opt_flags in combos:
            rmc_bin, err = compile_rmc(name, opt_flags)
            if err:
                print(f"FAIL {name}{label}: {err}")
                failures += 1
                continue
            for argv in args.get(name, [[]]):
                rmc_res, rust_res = run_both(rmc_bin, rust_bin, argv)
                _, rmc_status, rmc_out = rmc_res
                _, rust_status, rust_out = rust_res
                if rmc_out != rust_out or rmc_status != rust_status:
                    failures += 1
                    print(f"FAIL {name}{label} {argv}")
                    print(f"  exit rmc={rmc_status} rustc={rust_status}")
                    rmc_lines = rmc_out.split(b"\n")
                    rust_lines = rust_out.split(b"\n")
                    for i in range(max(len(rmc_lines), len(rust_lines))):
                        a = rmc_lines[i] if i < len(rmc_lines) else b"<missing>"
                        b = rust_lines[i] if i < len(rust_lines) else b"<missing>"
                        if a != b:
                            print(f"  first difference at output line {i + 1}:")
                            print(f"    rmc   = {a!r}")
                            print(f"    rustc = {b!r}")
                            break
                elif verbose:
                    print(f"ok   {name}{label} {argv}")

    failures += check_bounds_traps(combos)
    stripped = check_stripped()
    failures += stripped
    if stripped == 0:
        print("default link is smallest; --binutils strip/--min-size behave")
    rwx = check_no_rwx()
    failures += rwx
    if rwx == 0:
        print("linked binary has no writable+executable LOAD segment")
    failures += check_link_paths_agree()
    failures += check_encoding_matches_as()

    total = len(names)
    if failures == 0:
        print(f"all {total} cases match rustc in {len(combos)} optimiser configurations")
    else:
        print(f"{failures} failing run(s) across {total} cases")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
