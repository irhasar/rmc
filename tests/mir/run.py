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
    "regshare": [["5", "3"], ["-4", "9"], ["0", "0"]],
    "bigindex": [["2"], ["-1"], ["6"]],
    "cmpkinds": [["3"], ["9"], ["-5"], ["0"]],
    "looprange": [["9", "5"], ["0", "7"], ["40", "-3"], ["2", "0"], ["30", "11"]],
    "scalarstruct": [["3"], ["-11"], ["0"], ["40"]],
    "selects": [["3"], ["4"], ["-7"], ["9"], ["0"], ["6"]],
    "argshuffle": [["3", "5"], ["-8", "21"], ["0", "0"], ["13", "-2"]],
    "mulconst": [["7"], ["-1"], ["922337203685477581"], ["-4611686018427387904"], ["255"], ["0"]],
    "ranges": [["0"], ["7"], ["-95"], ["60"], ["-616"], ["300"]],
    "stalefacts": [["3", "5", "0"], ["3", "1005", "0"], ["3", "5", "1"], ["3", "1005", "1"]],
    "cmp": [["-5"], ["0"], ["7"]],
    "forloop": [["6"], ["0"]],
    "calls": [["10", "20", "30"]],
    "recursion": [["15"]],
    "aggret": [["9"]],
    "sretrec": [["4"], ["0"]],
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
    "dropagg": [["1", "2", "3", "4", "5", "6", "7", "8"]],
    "modname": [["11", "22", "33"]],
    "lowerenum": [["1", "0"], ["1", "42"], ["0", "0"]],
    "indexdrop": [["1", "0"], ["1", "1"], ["2", "1"], ["3", "0"]],
    "genstruct": [["7"], ["0"], ["42"]],
    "modgen": [["7"], ["0"], ["42"]],
    "modrelpath": [["1", "2", "3", "4"], ["9", "8", "7", "6"]],
    # ovfcheck runs with overflow checks on: the first vector is in range, the
    # second overflows the add and must exit 101 on both binaries.
    "ovfcheck": [["2", "3"], ["9223372036854775807", "1"]],
    # A constant on the left of `-`: small, negative, zero and extreme operands,
    # and a loop count (the second argument) of zero, one and several.
    "constleft": [["6", "0"], ["-6", "1"], ["0", "5"], ["9223372036854775807", "3"],
                  ["-9223372036854775808", "2"]],
    # With overflow checks: `5 - i64::MIN` overflows and must exit 101 on both
    # sides; `5 - 6` must not.
    "constleftovf": [["6"], ["-9223372036854775808"], ["-9223372036854775807"]],
    # srread reads its loop bounds from argv, so run zero, one and several
    # iterations to catch the off-by-one the strength-reduce guard prevents.
    "srread": [["0"], ["1"], ["2"], ["7"], ["64"]],
    # The counted-loop closure: zero, one, negative and large trip counts for
    # each of the three bounds, so the zero-trip guard and the fold are both hit.
    "loopcount": [["0", "0", "0"], ["1", "1", "1"], ["5", "-3", "9"], ["1000", "7", "-1"]],
    "loopneg": [["0"], ["1"], ["6"], ["-4"]],
    # `[v; n]` with byte, word and multi-word elements, read at runtime indices.
    "repeatelem": [["10", "0"], ["-3", "5"], ["7", "3"]],
    # Realistic workloads, at small sizes. Each one found a back-end bug: a
    # constant-left `-` (nqueens), an `&mut [T; N]` parameter written after
    # inlining (fannkuch, matmul), a memory-to-memory `cmp` (sort), duplicate
    # local labels across functions (vm), and an 8-byte `[0u8; N]` splat
    # (textparse).
    "real_vm": [["200"], ["1"]],
    "real_sort": [["2"]],
    "real_hashmap": [["20000"]],
    "real_bfs": [["2"]],
    "real_life": [["30"]],
    "real_nqueens": [["8"], ["1"]],
    "real_fannkuch": [["7"]],
    "real_matmul": [["2"]],
    "real_textparse": [["3"]],
    "real_particles": [["100"]],
    # float accumulates `n` half-steps; run the empty and non-empty loops.
    "float": [["0"], ["1"], ["5"], ["9"]],
    # resenum builds a `Result` at runtime: n > 0 takes `Ok`, n <= 0 takes
    # `Err`, so both the payload read and the failure arm are exercised.
    "resenum": [["5"], ["0"]],
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


def overflow_checks(name):
    """A `NAME.overflow` marker turns on overflow checks on both sides."""
    return os.path.exists(os.path.join(ROOT, f"{name}.overflow"))


def undefined_symbols(obj):
    """The symbols `obj` references but does not define.

    `nm` reports them with a `U` type on the standard three-column line and a
    blank type on the single-column form, so match both: the name is the last
    field of a `U` line, or the whole line when the type is blank.
    """
    p = subprocess.run(["nm", "-u", obj], capture_output=True)
    if p.returncode != 0:
        sys.exit(f"nm failed on {obj}:\n{p.stderr.decode(errors='replace')}")
    names = []
    for line in p.stdout.decode(errors="replace").splitlines():
        parts = line.split()
        if not parts:
            continue
        if len(parts) >= 2 and parts[-2] == "U":
            names.append(parts[-1])
        elif len(parts) == 1:
            names.append(parts[0])
    return names


def overflow_stubs(obj):
    """Stubs for the panic symbols the reference object leaves undefined.

    With overflow checks on, the reference calls the *mangled*
    `core::panicking::panic_const::*_overflow`, whose name embeds the hash of
    the particular libcore build and therefore changes with every toolchain.
    Hard-coding the spelling (as this harness once did) makes the reference
    link fail on any other nightly. Read the names off the object instead, and
    define each as a jump to `__rmc_overflow` in the shared runtime, so the
    stub traps exactly the way rmc's own checked arithmetic does regardless of
    toolchain. The symbol set is not assumed: whatever `nm -u` reports is what
    gets stubbed (`panic_bounds_check` and the other panic entries included).
    Skip `arg_i64`/`print_i64` -- those come from the shared runtime.
    """
    stubs = os.path.join(OUT, f"overflow_stubs_{os.path.basename(obj)}.o")
    src = os.path.join(OUT, f"overflow_stubs_{os.path.basename(obj)}.s")
    names = [n for n in undefined_symbols(obj)
             if "panic" in n or "overflow" in n]
    with open(src, "w") as f:
        f.write(".text\n")
        for n in names:
            f.write(f'.globl "{n}"\n.type "{n}", @function\n"{n}":\n  jmp __rmc_overflow\n')
    rc, _, err = run(["cc", "-c", src, "-o", stubs])
    if rc != 0:
        sys.exit(f"overflow stub assembly failed:\n{err.decode(errors='replace')}")
    return stubs


def build_reference(name, rt):
    """rustc -> object -> static link against the shared runtime."""
    src = os.path.join(ROOT, f"{name}.rs")
    obj = os.path.join(OUT, f"{name}.ref.o")
    ref = os.path.join(OUT, f"{name}.ref")
    rc, _, err = run([
        "rustc", "-O", "-C", "opt-level=3", "-C", "panic=abort",
        "--crate-type=bin", "--edition", "2021",
        "-C", "relocation-model=static", "--emit=obj",
        "-C", "overflow-checks=on" if overflow_checks(name) else "overflow-checks=off",
        src, "-o", obj,
    ])
    if rc != 0:
        return None, f"rustc failed:\n{err.decode(errors='replace')}"
    # Every case, not only the `.overflow` ones: a runtime index keeps a
    # `panic_bounds_check` call in the reference even with overflow checks
    # off, and its mangled name is toolchain-specific, so it is stubbed from
    # the object's own `nm -u` like the overflow entries.
    link = ["cc", "-static", "-nostdlib", "-no-pie", obj, rt, overflow_stubs(obj)]
    link += ["-o", ref]
    rc, _, err = run(link)
    if rc != 0:
        return None, f"cc failed:\n{err.decode(errors='replace')}"
    return ref, None


def build_rmc(name, binutils=False):
    """Build with rmc. `binutils` assembles with GNU `as` and links with `ld`
    instead of the in-process encoder: the two must accept the same assembly
    and produce binaries that behave identically. Label collisions and
    width-ambiguous instructions (`cmp $k, mem`) each made them disagree."""
    src = os.path.join(ROOT, f"{name}.rs")
    mir = os.path.join(OUT, f"{name}.{'bu' if binutils else 'mir'}.bin")
    argv = [RMC, "--rustc-frontend", src, "-o", mir]
    if overflow_checks(name):
        argv.insert(2, "--overflow-checks")
    if binutils:
        argv.insert(2, "--binutils")
    rc, _, err = run(argv)
    if rc != 0:
        return None, f"rmc{' --binutils' if binutils else ''} failed:\n{err.decode(errors='replace')}"
    if binutils and b"Warning" in err:
        return None, f"`as` warned about the generated assembly:\n{err.decode(errors='replace')}"
    return mir, None


REJECT = os.path.join(ROOT, "reject")


def run_reject_cases(verbose):
    """Cases in `reject/` that rustc accepts but rmc must refuse *cleanly*.

    The value here is the failure mode, not the success: an unsupported
    construct must exit with status 1 and a message that names the feature, not
    a panic (status 101) or a stray `undefined symbol` from the linker. Each
    case is checked on both sides - the reference must build and run, so the
    program is real, and rmc must fail with status 1.
    """
    if not os.path.isdir(REJECT):
        return 0, 0
    rt = runtime_asm()
    failures = 0
    cases = sorted(f[:-3] for f in os.listdir(REJECT) if f.endswith(".rs"))
    for name in cases:
        src = os.path.join(REJECT, f"{name}.rs")
        obj = os.path.join(OUT, f"{name}.rej.o")
        rc, _, err = run([
            "rustc", "-O", "-C", "panic=abort", "--crate-type=bin",
            "--edition", "2021", "-C", "relocation-model=static", "--emit=obj",
            src, "-o", obj,
        ])
        if rc != 0:
            print(f"FAIL reject/{name}: rustc could not build the reference:\n"
                  f"{err.decode(errors='replace')}")
            failures += 1
            continue
        # rmc must refuse with a clean, named error. Status 1 (not 101) is the
        # contract; a 101 would be a panic, which is the regression this guards.
        out_bin = os.path.join(OUT, f"{name}.rej.bin")
        rc, _, err = run([RMC, "--rustc-frontend", src, "-o", out_bin])
        msg = err.decode(errors="replace")
        if rc == 0:
            print(f"FAIL reject/{name}: rmc accepted a construct it cannot compile")
            failures += 1
        elif rc != 1:
            print(f"FAIL reject/{name}: rmc exited {rc} (want a clean 1, not a "
                  f"panic/abort):\n{msg}")
            failures += 1
        elif "note:" not in msg:
            print(f"FAIL reject/{name}: the error names no feature (no `note:`):\n{msg}")
            failures += 1
        elif verbose:
            print(f"ok   reject/{name}")
    return len(cases), failures


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
        if not err:
            bu, err = build_rmc(name, binutils=True)
        if err:
            print(f"FAIL {name}: {err}")
            failures += 1
            continue
        vectors = ARGS.get(name, [[]])
        bad = False
        for argv in vectors:
            r_rc, r_out, _ = run([ref] + argv)
            m_rc, m_out, _ = run([mir] + argv)
            b_rc, b_out, _ = run([bu] + argv)
            if r_rc != m_rc or r_out != m_out:
                print(f"FAIL {name} {argv}: rustc rc={r_rc} out={r_out!r} | "
                      f"rmc rc={m_rc} out={m_out!r}")
                bad = True
                failures += 1
                break
            if (b_rc, b_out) != (m_rc, m_out):
                print(f"FAIL {name} {argv}: in-process rc={m_rc} out={m_out!r} | "
                      f"--binutils rc={b_rc} out={b_out!r}")
                bad = True
                failures += 1
                break
        if not bad:
            if verbose:
                print(f"ok   {name}")

    # `reject/` cases: rustc accepts, rmc must refuse cleanly. Counted
    # separately so the "N cases matched" line stays about the positive suite.
    n_reject, reject_failures = run_reject_cases(verbose)
    failures += reject_failures

    total = len(cases)
    print(f"\n{total - (failures - reject_failures) - skipped}/{total} cases matched"
          + (f" ({failures - reject_failures} failed, {skipped} skipped)"
             if (failures - reject_failures) or skipped else ""))
    if n_reject:
        print(f"{n_reject - reject_failures}/{n_reject} reject cases refused cleanly"
              + (f" ({reject_failures} failed)" if reject_failures else ""))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
