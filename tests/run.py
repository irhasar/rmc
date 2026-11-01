#!/usr/bin/env python3
"""Differential test suite: every case is compiled by nrc and by rustc and the
two programs must print identical output and exit with the same status.

Each case lives in `tests/cases/` as a pair `NAME.nrs` (nrc) and `NAME.rs`
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
NRC = os.path.join(PROJ, "compiler", "target", "release", "nrc")
CASES = os.path.join(ROOT, "cases")
OUT = "/tmp/nrctests"
ARGS_FILE = os.path.join(ROOT, "args.json")


def load_args():
    if os.path.exists(ARGS_FILE):
        with open(ARGS_FILE) as f:
            return json.load(f)
    return {}


def compile_and_run(name, argv):
    """Compile both twins and run them with `argv`.

    Returns `(nrc_result, rust_result)` where each result is
    `("ok", exit_status, stdout_bytes)` or `("error", message)`. Both are always
    present so the caller can unpack the pair unconditionally.
    """
    nrs = os.path.join(CASES, f"{name}.nrs")
    rs = os.path.join(CASES, f"{name}.rs")
    nrc_bin = os.path.join(OUT, f"{name}.nrc")
    rust_bin = os.path.join(OUT, f"{name}.rust")

    p = subprocess.run([NRC, nrs, "-o", nrc_bin], capture_output=True)
    if p.returncode != 0:
        err = ("error", f"nrc failed:\n{p.stderr.decode(errors='replace')}")
        return err, ("error", "not run")
    p = subprocess.run(["rustc", "-O", rs, "-o", rust_bin], capture_output=True)
    if p.returncode != 0:
        err = ("error", f"rustc failed:\n{p.stderr.decode(errors='replace')}")
        return ("error", "not run"), err

    def run(binary):
        p = subprocess.run([binary] + argv, capture_output=True)
        return ("ok", p.returncode, p.stdout)

    return run(nrc_bin), run(rust_bin)


def main():
    verbose = "-v" in sys.argv
    os.makedirs(OUT, exist_ok=True)
    args = load_args()

    names = sorted(f[:-4] for f in os.listdir(CASES) if f.endswith(".nrs"))
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
            nrc_res, rust_res = compile_and_run(name, argv)
            for which, res in (("nrc", nrc_res), ("rustc", rust_res)):
                if res[0] == "error":
                    print(f"FAIL {name} {argv}: {which}: {res[1]}")
                    failures += 1
                    break
            else:
                _, nrc_status, nrc_out = nrc_res
                _, rust_status, rust_out = rust_res
                if nrc_out != rust_out or nrc_status != rust_status:
                    failures += 1
                    print(f"FAIL {name} {argv}")
                    print(f"  nrc   exit={nrc_status} out={nrc_out!r}")
                    print(f"  rustc exit={rust_status} out={rust_out!r}")
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
