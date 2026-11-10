#!/usr/bin/env python3
"""Differential test suite for the MIR path with `std` available.

`tests/mir/` is freestanding (`#![no_std]`, one runtime) and `tests/run.py` is
the `.rms` path. Neither exercises `println!`: the first has no `std`, and the
second's front end has no macro. This suite closes that gap.

Each case in `tests/std/` is a plain `std` program. It is compiled two ways and
both binaries must print identical stdout and exit with the same status:

  * reference -- `rustc -O`, i.e. the full LLVM pipeline with libstd.
  * rmc       -- `rmc case.rs`, which runs `rustc --emit=mir -O` and lowers the
    MIR through rmc's own back end.

Only the `println!`/`print!`/`eprintln!` shapes the MIR path models belong
here: an all-constant format string (rustc folds it to `Arguments::from_str`)
or a format string whose placeholders take runtime arguments (rustc builds an
`Arguments` with a `&[Argument]` array).

Usage: python3 tests/std/run.py [-v]
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(os.path.dirname(ROOT))
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
OUT = "/tmp/rmcstdtests"

# argv vectors per case. A case with no entry runs once with no arguments.
ARGS = {
    "hello": [],
    "prints": [],
    "eprint": [],
    "escapes": [],
    "many": [],
}


def run(binary, argv):
    p = subprocess.run([binary] + argv, capture_output=True)
    return p.returncode, p.stdout, p.stderr


def main():
    verbose = "-v" in sys.argv[1:]
    os.makedirs(OUT, exist_ok=True)
    names = sorted(
        f[:-3] for f in os.listdir(ROOT)
        if f.endswith(".rs")
    )
    if not names:
        print("no cases in tests/std/")
        return 1

    failures = 0
    for name in names:
        src = os.path.join(ROOT, f"{name}.rs")
        ref = os.path.join(OUT, f"{name}.rust")
        got = os.path.join(OUT, f"{name}.rmc")

        p = subprocess.run(["rustc", "-O", src, "-o", ref], capture_output=True)
        if p.returncode != 0:
            print(f"FAIL {name}: rustc failed:\n{p.stderr.decode(errors='replace')}")
            failures += 1
            continue

        p = subprocess.run([RMC, "--rustc-frontend", src, "-o", got], capture_output=True)
        if p.returncode != 0:
            print(f"FAIL {name}: rmc failed:\n{p.stderr.decode(errors='replace')}")
            failures += 1
            continue

        for argv in (ARGS.get(name) or [[]]):
            want = run(ref, argv)
            have = run(got, argv)
            if want != have:
                print(f"FAIL {name} {argv}: rustc={want!r} rmc={have!r}")
                failures += 1
                break
        else:
            if verbose:
                print(f"ok   {name}")

    if failures:
        print(f"\n{failures} case(s) failed")
        return 1
    print(f"\n{len(names)}/{len(names)} cases matched")
    return 0


if __name__ == "__main__":
    sys.exit(main())
