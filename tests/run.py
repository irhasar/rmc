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
    rwx = check_no_rwx()
    failures += rwx
    if rwx == 0:
        print("linked binary has no writable+executable LOAD segment")

    total = len(names)
    if failures == 0:
        print(f"all {total} cases match rustc in {len(combos)} optimiser configurations")
    else:
        print(f"{failures} failing run(s) across {total} cases")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
