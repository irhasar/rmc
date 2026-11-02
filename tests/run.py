#!/usr/bin/env python3
"""Differential test suite: every case is compiled by rmc and by rustc and the
two programs must print identical output and exit with the same status.

Each case lives in `tests/cases/` as a pair `NAME.rms` (rmc) and `NAME.rs`
(Rust). The two files are written to behave identically, so any divergence in
stdout or exit code is a compiler bug. `tests/args.json` optionally maps a case
name to a list of argv vectors; each vector is one run, and the case passes
only if every run matches. This is what catches bugs like a folded comparison
silently inverting an inequality.

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


def compile_and_run(name, argv):
    """Compile both twins and run them with `argv`.

    Returns `(rmc_result, rust_result)` where each result is
    `("ok", exit_status, stdout_bytes)` or `("error", message)`. Both are always
    present so the caller can unpack the pair unconditionally.
    """
    rms = os.path.join(CASES, f"{name}.rms")
    rs = os.path.join(CASES, f"{name}.rs")
    rmc_bin = os.path.join(OUT, f"{name}.rmc")
    rust_bin = os.path.join(OUT, f"{name}.rust")

    rmc_flags, rustc_flags = extra_flags(name)
    p = subprocess.run([RMC] + rmc_flags + [rms, "-o", rmc_bin], capture_output=True)
    if p.returncode != 0:
        err = ("error", f"rmc failed:\n{p.stderr.decode(errors='replace')}")
        return err, ("error", "not run")
    p = subprocess.run(["rustc", "-O"] + rustc_flags + [rs, "-o", rust_bin], capture_output=True)
    if p.returncode != 0:
        err = ("error", f"rustc failed:\n{p.stderr.decode(errors='replace')}")
        return ("error", "not run"), err

    def run(binary):
        p = subprocess.run([binary] + argv, capture_output=True)
        return ("ok", p.returncode, p.stdout)

    return run(rmc_bin), run(rust_bin)


def main():
    verbose = "-v" in sys.argv
    os.makedirs(OUT, exist_ok=True)
    args = load_args()

    names = sorted(f[:-4] for f in os.listdir(CASES) if f.endswith(".rms"))
    if not names:
        print("no test cases found", file=sys.stderr)
        return 1

    failures = 0
    for name in names:
        if not os.path.exists(os.path.join(CASES, f"{name}.rs")):
            print(f"FAIL {name}: missing Rust twin ({name}.rs)")
            failures += 1
            continue

        for argv in args.get(name, [[]]):
            rmc_res, rust_res = compile_and_run(name, argv)
            for which, res in (("rmc", rmc_res), ("rustc", rust_res)):
                if res[0] == "error" and res[1] != "not run":
                    print(f"FAIL {name} {argv}: {which}: {res[1]}")
                    failures += 1
                    break
                if res[0] == "error":
                    print(f"FAIL {name} {argv}: {which} did not run ({rmc_res if which == 'rustc' else rust_res})")
                    failures += 1
                    break
            else:
                _, rmc_status, rmc_out = rmc_res
                _, rust_status, rust_out = rust_res
                if rmc_out != rust_out or rmc_status != rust_status:
                    failures += 1
                    print(f"FAIL {name} {argv}")
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
                    print(f"ok   {name} {argv}")

    total = len(names)
    if failures == 0:
        print(f"all {total} cases match rustc")
    else:
        print(f"{failures} failing run(s) across {total} cases")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
