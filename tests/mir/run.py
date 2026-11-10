#!/usr/bin/env python3
"""Differential test suite for the rustc-MIR front end + rmc back end.

Each case in `tests/mir/` is a freestanding `#![no_std]` program (it includes
`prelude.rs`). The same source is compiled two ways:

  * reference -- `rustc --emit=obj`, linked against the *same* freestanding
    runtime rmc uses (`rmc --emit-runtime-asm`), with `cc -static -nostdlib`.
  * rmc       -- `rmc case.rs`, which runs `rustc --emit=mir` and lowers the MIR
    through rmc's own back end.

The two binaries share one runtime, one entry point and one syscall layer, so
the only variable is the code generator. They must print identical stdout and
exit with the same status. This is the suite that actually exercises the MIR
path; `tests/run.py` covers the AST path.

Usage: python3 tests/mir/run.py [-v]
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(os.path.dirname(ROOT))
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
OUT = "/tmp/rmcmirtests"

# argv vectors per case. Each vector is one run; the case passes only if every
# run matches. A case with no entry runs once with no arguments.
ARGS = {
    "cmp": [["-5"], ["0"], ["7"]],
    "forloop": [["6"], ["0"]],
    "calls": [["10", "20", "30"]],
    "recursion": [["15"]],
    "aggret": [["9"]],
    "bitops": [["42"]],
    "loopsum": [["1000"], ["0"]],
    "heap": [["1", "77"]],
    "matchint": [["4"], ["1"]],
    "divmod": [["7"], ["0"]],
    "boolops": [["1", "0"], ["0", "0"], ["5", "5"]],
    "drop": [["5"], ["0"]],
    "argorder": [["3"]],
    "aggarg": [["3"]],
    "regpressure": [["3"]],
}


def run(argv, cwd=None):
    p = subprocess.run(argv, capture_output=True, cwd=cwd)
    return p.returncode, p.stdout, p.stderr


def runtime_asm():
    os.makedirs(OUT, exist_ok=True)
    rt = os.path.join(OUT, "runtime.s")
    if not os.path.exists(rt):
        rc, out, err = run([RMC, "--emit-runtime-asm"])
        if rc != 0:
            sys.exit(f"rmc --emit-runtime-asm failed:\n{err.decode(errors='replace')}")
        with open(rt, "wb") as f:
            f.write(out)
    return rt


def build_reference(name, rt):
    """rustc -> object -> static link against the shared runtime."""
    src = os.path.join(ROOT, f"{name}.rs")
    obj = os.path.join(OUT, f"{name}.ref.o")
    ref = os.path.join(OUT, f"{name}.ref")
    rc, _, err = run([
        "rustc", "-O", "-C", "opt-level=3", "-C", "panic=abort",
        "--crate-type=bin", "--edition", "2021",
        "-C", "relocation-model=static", "--emit=obj", src, "-o", obj,
    ])
    if rc != 0:
        return None, f"rustc failed:\n{err.decode(errors='replace')}"
    rc, _, err = run([
        "cc", "-static", "-nostdlib", "-no-pie", obj, rt, "-o", ref,
    ])
    if rc != 0:
        return None, f"cc failed:\n{err.decode(errors='replace')}"
    return ref, None


def build_rmc(name):
    src = os.path.join(ROOT, f"{name}.rs")
    mir = os.path.join(OUT, f"{name}.mir.bin")
    rc, _, err = run([RMC, "--rustc-frontend", src, "-o", mir])
    if rc != 0:
        return None, f"rmc failed:\n{err.decode(errors='replace')}"
    return mir, None


def main():
    verbose = "-v" in sys.argv
    os.makedirs(OUT, exist_ok=True)
    rt = runtime_asm()
    cases = sorted(
        f[:-3] for f in os.listdir(ROOT)
        if f.endswith(".rs") and f != "prelude.rs"
    )
    if not cases:
        sys.exit("no cases found")

    failures = 0
    skipped = 0
    for name in cases:
        ref, err = build_reference(name, rt)
        if err:
            print(f"FAIL {name}: reference build: {err}")
            failures += 1
            continue
        mir, err = build_rmc(name)
        if err:
            print(f"FAIL {name}: {err}")
            failures += 1
            continue
        vectors = ARGS.get(name, [[]])
        bad = False
        for argv in vectors:
            r_rc, r_out, _ = run([ref] + argv)
            m_rc, m_out, _ = run([mir] + argv)
            if r_rc != m_rc or r_out != m_out:
                print(f"FAIL {name} {argv}: rustc rc={r_rc} out={r_out!r} | "
                      f"rmc rc={m_rc} out={m_out!r}")
                bad = True
                failures += 1
                break
        if not bad:
            if verbose:
                print(f"ok   {name}")

    total = len(cases)
    print(f"\n{total - failures - skipped}/{total} cases matched"
          + (f" ({failures} failed, {skipped} skipped)" if failures or skipped else ""))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
