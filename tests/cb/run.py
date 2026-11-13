#!/usr/bin/env python3
"""Differential test suite for the real `-Zcodegen-backend` path.

`tests/std/run.py` drives rmc's *printed-MIR prototype* (`rmc --rustc-frontend`),
which runs `rustc --emit=mir` in a separate process. The production pipeline is
the codegen backend: rustc links `librmc_codegen_backend.so` and hands it the
MIR directly. The two render the same MIR differently -- `--emit=mir` resolves
the *visible* path (`std::io::_print`, `PhantomData`), the backend route uses
`with_no_visible_paths!` and prints the *defining* one (`std::io::stdio::_print`,
`core::marker::PhantomData`). A front end keyed to the visible spelling works
under the prototype and miscompiles (or fails) under the backend, so the
prototype suite cannot stand in for this one.

Each case in `tests/cb/` is a plain `std` program compiled two ways:

  * reference -- `rustc -O -C panic=abort` (the full LLVM pipeline with libstd).
  * rmc       -- `rustc +nightly -Zcodegen-backend=<...>.so -C opt-level=3`.

Both binaries must print identical stdout and either both succeed or both
abort; a panic's exact status is not compared, because rmc's `panic!` sink exits
101 where libstd raises SIGABRT, and stderr is not compared, because the sink
writes a fixed line rather than reproducing libstd's message (deliberate
simplifications).

Needs the nightly toolchain with the `rustc-dev` component, and the backend
dylib built by `backend/build.sh`. When either is missing the suite reports it
and exits 0, so a stable-only checkout (and CI's `test` job) is not blocked;
CI's dedicated `cb` job installs the prerequisites and runs it for real.

Usage: python3 tests/cb/run.py [-v]
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(os.path.dirname(ROOT))
SO = os.path.join(PROJ, "backend", "target", "release", "librmc_codegen_backend.so")
OUT = "/tmp/rmccbtests"

# argv vectors per case. A case with no entry runs once with no arguments.
ARGS = {}

CB = ["rustc", "+nightly", "-Zcodegen-backend=" + SO, "--edition", "2021",
      "-C", "opt-level=3", "-C", "panic=abort", "--crate-type=bin"]
REF = ["rustc", "-O", "-C", "panic=abort", "--edition", "2021", "--crate-type=bin"]


def run(binary, argv):
    p = subprocess.run([binary] + argv, capture_output=True)
    return p.returncode, p.stdout, p.stderr


def have_prereqs():
    if not os.path.exists(SO):
        print(f"skip: {SO} not built (run backend/build.sh)")
        return False
    p = subprocess.run(["rustc", "+nightly", "--print", "sysroot"],
                       capture_output=True)
    if p.returncode != 0:
        print("skip: nightly toolchain not installed")
        return False
    sysroot = p.stdout.decode().strip()
    libdir = os.path.join(sysroot, "lib")
    have_driver = os.path.isdir(libdir) and any(
        f.startswith("librustc_driver") and f.endswith(".so")
        for f in os.listdir(libdir)
    )
    if not have_driver:
        print("skip: nightly `rustc-dev` component not installed")
        return False
    return True


def main():
    verbose = "-v" in sys.argv[1:]
    if not have_prereqs():
        return 0
    os.makedirs(OUT, exist_ok=True)

    names = sorted(f[:-3] for f in os.listdir(ROOT) if f.endswith(".rs"))
    if not names:
        print("no cases in tests/cb/")
        return 1

    failures = 0
    for name in names:
        src = os.path.join(ROOT, f"{name}.rs")
        ref = os.path.join(OUT, f"{name}.rust")
        got = os.path.join(OUT, f"{name}.rmc")

        p = subprocess.run(REF + [src, "-o", ref], capture_output=True)
        if p.returncode != 0:
            print(f"FAIL {name}: rustc failed:\n{p.stderr.decode(errors='replace')}")
            failures += 1
            continue

        p = subprocess.run(CB + [src, "-o", got], capture_output=True)
        if p.returncode != 0:
            print(f"FAIL {name}: rmc backend failed:\n{p.stderr.decode(errors='replace')}")
            failures += 1
            continue

        for argv in (ARGS.get(name) or [[]]):
            want = run(ref, argv)
            have = run(got, argv)
            # Success is compared exactly (stdout + status); a non-zero exit is
            # only compared for "did it fail", since rmc's abort exits 101 and
            # libstd's raises SIGABRT.
            ok_want, ok_have = want[0] == 0, have[0] == 0
            mismatch = ok_want != ok_have or (ok_want and want[1] != have[1])
            if mismatch:
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
