#!/usr/bin/env python3
"""Compare four pipelines on the same source: the true `-Zcodegen-backend`,
the printed-MIR prototype, LLVM, and rustc's Cranelift back end.

  * ``cb``   -- ``rustc +nightly -Zcodegen-backend=libr... .so src -o bin``.
                One rustc process; rmc lowers the MIR in-process and writes
                the ELF itself. No MIR text, no second process, no `ld`.
  * ``mir``  -- ``rustc --emit=mir -O -Zno-codegen`` then ``rmcmir``. The
                prototype: same back end, but the interface is printed MIR.
  * ``llvm`` -- ``rustc --emit=obj -O -C lto=fat`` then ``ld -s``. The
                reference the whole experiment is against.
  * ``cl``   -- ``rustc -Zcodegen-backend=cranelift --emit=obj -O`` then
                ``ld -s``. rustc's other non-LLVM back end, and the closest
                like-for-like rival: it has no LTO, so that one flag is the
                only difference from the ``llvm`` row.

All four use the *same* nightly front end, so a compile difference is the
back end and the interface, not the parser. ``cb`` and ``mir`` link the same
freestanding runtime; ``llvm`` and ``cl`` link it too (plus trap stubs for the
panic entry points rmc lowers to its own traps), so run-time numbers compare
the same program.

Fairness rules, the same as ``run_fair.py``: every back end must produce the
same stdout and exit status, and every stage is sampled round-robin so thermal
drift cannot favour whichever ran first.

Usage: python3 bench/run_codegen.py [program ...]
"""
import os
import shutil
import subprocess
import sys
import time

os.environ.setdefault("RUSTC_BOOTSTRAP", "1")

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
RMCMIR = os.path.join(PROJ, "compiler", "target", "release", "examples", "rmcmir")
CBWRAP = os.path.join(PROJ, "backend", "rmc-rustc")
CBSO = os.path.join(PROJ, "backend", "target", "release",
                    "librmc_codegen_backend.so")
OUT = "/tmp/rmccodegen"

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats", "refs",
            "heap", "callrec", "forloops", "light", "heavy"]
ARGS = {
    "hello": [], "fib": ["32"], "mandel": ["800"], "sieve": ["200000"],
    "mix": ["2000000"], "stats": ["2000000"], "refs": ["5000000"],
    "heap": ["200000"], "callrec": ["30"], "forloops": ["2000000"],
    "light": ["2000000"], "heavy": ["1000000000"],
}

RUSTC = ["rustc", "+nightly"]
RUSTC_MIR = RUSTC + ["--emit=mir", "-O", "-Zno-codegen", "-C", "panic=abort",
                     "--crate-type=bin", "--edition", "2021", "-Awarnings"]
RUSTC_OBJ = RUSTC + ["--emit=obj", "-O", "-C", "opt-level=3", "-C", "lto=fat",
                     "-C", "codegen-units=1", "-C", "panic=abort",
                     "--edition", "2021", "-Awarnings"]
# `cb` drives the backend dylib through rustc directly, the same way the LLVM
# row invokes rustc: one process, no shell wrapper. The dylib carries an rpath
# to the toolchain's `lib`, so `-Zcodegen-backend` needs no environment fixup.
CB = RUSTC + ["-Zcodegen-backend=" + CBSO, "--edition", "2021",
              "-C", "panic=abort", "-C", "opt-level=3",
              "--crate-type=bin", "-Awarnings"]
# rustc's Cranelift back end: the other non-LLVM codegen path in the toolchain,
# and the closest like-for-like rival to rmc. Cranelift does not implement LTO
# (`-C lto=fat` is rejected outright), so this row keeps every other setting the
# same but omits LTO; that gap is a property of the back end, not a choice made
# here. It emits an object that is linked the same way the LLVM row's is.
CL = RUSTC + ["-Zcodegen-backend=cranelift", "--emit=obj", "-O",
              "-C", "opt-level=3", "-C", "codegen-units=1", "-C", "panic=abort",
              "--edition", "2021", "-Awarnings"]

NOISE_BAND = 0.02

# A whole program can finish inside the sampler's resolution. `refs` is folded
# to a closed form by every optimising back end, so its run is fork+exec, and
# `hello` is barely longer; comparing wall/CPU there measures process start-up,
# not the code under test. Runs below this floor are reported as unmeasured
# rather than as a ratio, so noise cannot show up as a fabricated loss.
RUN_FLOOR = 1e-3

PANIC_SYMS = [
    "core::panicking::panic_bounds_check",
    "core::panicking::panic_const::panic_const_div_by_zero",
    "core::panicking::panic_const::panic_const_rem_by_zero",
    "core::panicking::panic_const::panic_const_rem_overflow",
    "core::panicking::panic_const::panic_const_div_overflow",
    "core::panicking::panic_const::panic_const_add_overflow",
    "core::panicking::panic_const::panic_const_mul_overflow",
    "core::panicking::panic_const::panic_const_sub_overflow",
    "core::panicking::panic_const::panic_const_neg_overflow",
    "core::panicking::panic_const::panic_const_shl_overflow",
    "core::panicking::panic_const::panic_const_shr_overflow",
]


def _quiet_actions():
    """Redirect a spawned child's stdout/stderr to `/dev/null`.

    The measured programs print their result, and the RSS pass runs each one
    repeatedly; without this the benchmark report is buried in that output.
    """
    flags = os.O_WRONLY
    return [(os.POSIX_SPAWN_OPEN, 1, os.devnull, flags, 0),
            (os.POSIX_SPAWN_OPEN, 2, os.devnull, flags, 0)]


def peak_rss_kb(pid):
    """The child's own high-water RSS from `/proc`, in kilobytes.

    `wait4`'s `ru_maxrss` cannot be used here: `posix_spawn` reaches `exec` via
    `vfork`, so the child briefly shares this process's address space and its
    `ru_maxrss` carries the parent's resident size as a floor. Reading `VmHWM`
    after `posix_spawn` has returned (that is, after the child has `exec`d)
    reports the program's own peak, which is what the comparison needs.
    """
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def measure_stages(stages, reps):
    """Wall, CPU and peak RSS per stage, sampled round-robin."""
    names = [n for n, _ in stages]
    walls = {n: [] for n in names}
    cpus = {n: [] for n in names}
    rss = {n: 0 for n in names}
    for _ in range(reps):
        for n, argv in stages:
            exe = shutil.which(argv[0]) or argv[0]
            t0 = time.perf_counter()
            pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ,
                                 file_actions=_quiet_actions())
            _, status, ru = os.wait4(pid, 0)
            walls[n].append(time.perf_counter() - t0)
            cpus[n].append(ru.ru_utime + ru.ru_stime)
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
    for _ in range(max(3, reps)):
        for n, argv in stages:
            exe = shutil.which(argv[0]) or argv[0]
            pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ,
                                 file_actions=_quiet_actions())
            peak = 0
            while True:
                hwm = peak_rss_kb(pid)
                if hwm > peak:
                    peak = hwm
                done, status, _ = os.wait4(pid, os.WNOHANG)
                if done:
                    break
                os.sched_yield()
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
            if peak > rss[n]:
                rss[n] = peak
    return {n: dict(wall=min(walls[n]), cpu=min(cpus[n]), rss=rss[n])
            for n in names}


def _v0_mangled(name, crate_hash):
    """Mangle a `core::panicking` item the way nightly's v0 scheme does.

    Only the shape the toolchain actually emits is needed: `_RNv` + nested
    (`Nt`) module segments + `C<crate_hash>` + the item path. This is the
    inverse of the `nm -u` output, and keeps the stub table valid even though
    the standard library's crate hash changes between toolchain builds.
    """
    parts = name.split("::")
    segs = "".join(f"{len(p)}{p}" for p in parts[:-1])
    last = parts[-1]
    return f"_RNvNtC{crate_hash}{segs}{len(last)}{last}"


def probe_panic_syms():
    """Panic symbols the current toolchain references, mangled as it emits them.

    `panic_bounds_check` and the `panic_const_*` shims are mangled with the
    standard library's crate disambiguator, which is not stable across
    toolchain builds, so a hardcoded spelling silently stops matching. A probe
    crate that hits every panic path is compiled with the reference flags; the
    v0 spellings are reconstructed from the disambiguator read back from one
    known symbol.
    """
    src = os.path.join(OUT, "probe_panic.rs")
    obj = os.path.join(OUT, "probe_panic.o")
    with open(src, "w") as f:
        f.write(
            "use std::hint::black_box;\n"
            "#[inline(never)] fn o() -> i64 "
            "{ black_box(std::env::args().count() as i64) }\n"
            "#[no_mangle] pub fn p_bounds(i: usize) -> i64 "
            "{ let a = [1i64, 2, 3]; a[i] }\n"
            "#[no_mangle] pub fn p_div(a: i64, b: i64) -> i64 { a / b }\n"
            "#[no_mangle] pub fn p_rem(a: i64, b: i64) -> i64 { a % b }\n"
            "#[no_mangle] pub fn p_add(a: i64, b: i64) -> i64 { a + b }\n"
            "#[no_mangle] pub fn p_sub(a: i64, b: i64) -> i64 { a - b }\n"
            "#[no_mangle] pub fn p_mul(a: i64, b: i64) -> i64 { a * b }\n"
            "#[no_mangle] pub fn p_neg(a: i64) -> i64 { -a }\n"
            "#[no_mangle] pub fn p_shl(a: i64, b: u32) -> i64 { a << b }\n"
            "#[no_mangle] pub fn p_shr(a: i64, b: u32) -> i64 { a >> b }\n"
            "fn main() {\n"
            "    let x = o();\n"
            "    let _ = (p_bounds(x as usize), p_div(x, x), p_rem(x, x),\n"
            "             p_add(x, x), p_sub(x, x), p_mul(x, x), p_neg(x),\n"
            "             p_shl(x, x as u32), p_shr(x, x as u32));\n"
            "}\n")
    r = subprocess.run(RUSTC_OBJ + [src, "-o", obj], capture_output=True)
    if r.returncode != 0:
        return []
    out = subprocess.run(["nm", "-u", obj], capture_output=True, text=True)
    real = [l.split()[-1] for l in out.stdout.splitlines() if "panicking" in l]
    m = next((s for s in real if s.endswith("18panic_bounds_check")), None)
    if m is None:
        return real
    # `_RNvNtC` + hash + `4core9panicking18panic_bounds_check`
    crate_hash = m[len("_RNvNtC"):m.index("4core9panicking")]
    syms = list(real)
    for name in PANIC_SYMS:
        syms.append(_v0_mangled(name, crate_hash))
    return syms


def build_runtime():
    asm = subprocess.run([RMC, "--emit-runtime-asm"], capture_output=True,
                         check=True).stdout
    asm_path = os.path.join(OUT, "runtime.s")
    with open(asm_path, "wb") as f:
        f.write(asm)
    obj = os.path.join(OUT, "runtime.o")
    subprocess.run(["as", "--64", "-o", obj, asm_path], check=True)
    stubs = ["\t.text", "\t.globl __rmc_trap_stub", "__rmc_trap_stub:",
             "\tmov $60, %rax", "\tmov $101, %rdi", "\tsyscall"]
    # rmc's own back end and the printed-MIR prototype call the panic shims by
    # their unmangled Rust paths; rustc's LLVM objects reference the v0-mangled
    # spellings. Both are stubbed so every pipeline links.
    names = list(dict.fromkeys(PANIC_SYMS + probe_panic_syms()))
    for s in names:
        stubs += [f'\t.globl "{s}"', f'"{s}":', "\tjmp __rmc_trap_stub"]
    stub_s = os.path.join(OUT, "panic_stubs.s")
    with open(stub_s, "w") as f:
        f.write("\n".join(stubs) + "\n")
    stub_o = os.path.join(OUT, "panic_stubs.o")
    subprocess.run(["as", "--64", "-o", stub_o, stub_s], check=True)
    return obj, stub_o


def pipeline_stages(name, runtime_obj, stub_obj):
    src = os.path.join(ROOT, "mir", f"{name}.rs")
    mir = os.path.join(OUT, f"{name}.mir")
    cb_bin = os.path.join(OUT, f"{name}.cb.bin")
    mir_bin = os.path.join(OUT, f"{name}.mir.bin")
    obj = os.path.join(OUT, f"{name}.llvm.o")
    llvm_bin = os.path.join(OUT, f"{name}.llvm.bin")
    cl_obj = os.path.join(OUT, f"{name}.cl.o")
    cl_bin = os.path.join(OUT, f"{name}.cl.bin")
    stages = [
        # Single process: rustc drives rmc's backend, which writes the ELF.
        ("cb", CB + [src, "-o", cb_bin]),
        ("mir.front", RUSTC_MIR + [src, "-o", mir]),
        ("mir.back", [RMCMIR, mir, "-o", mir_bin]),
        ("llvm.front", RUSTC_OBJ + [src, "-o", obj]),
        ("llvm.back", ["ld", "-s", "-o", llvm_bin, "-e", "_start", obj,
                       runtime_obj, stub_obj]),
        ("cl.front", CL + [src, "-o", cl_obj]),
        ("cl.back", ["ld", "-s", "-o", cl_bin, "-e", "_start", cl_obj,
                     runtime_obj, stub_obj]),
    ]
    return stages, cb_bin, mir_bin, llvm_bin, cl_bin


def measure_program(name, runtime_obj, stub_obj, reps=9):
    stages, cb_bin, mir_bin, llvm_bin, cl_bin = pipeline_stages(
        name, runtime_obj, stub_obj)
    r = measure_stages(stages, reps)
    return {
        "cb": {"wall": r["cb"]["wall"], "cpu": r["cb"]["cpu"],
               "rss": r["cb"]["rss"], "size": os.path.getsize(cb_bin)},
        "mir": {"wall": r["mir.front"]["wall"] + r["mir.back"]["wall"],
                "cpu": r["mir.front"]["cpu"] + r["mir.back"]["cpu"],
                "rss": max(r["mir.front"]["rss"], r["mir.back"]["rss"]),
                "size": os.path.getsize(mir_bin)},
        "llvm": {"wall": r["llvm.front"]["wall"] + r["llvm.back"]["wall"],
                 "cpu": r["llvm.front"]["cpu"] + r["llvm.back"]["cpu"],
                 "rss": max(r["llvm.front"]["rss"], r["llvm.back"]["rss"]),
                 "size": os.path.getsize(llvm_bin)},
        "cl": {"wall": r["cl.front"]["wall"] + r["cl.back"]["wall"],
               "cpu": r["cl.front"]["cpu"] + r["cl.back"]["cpu"],
               "rss": max(r["cl.front"]["rss"], r["cl.back"]["rss"]),
               "size": os.path.getsize(cl_bin)},
    }


def verify_outputs(bins, args):
    first = next(iter(bins))
    ref = subprocess.run([bins[first]] + list(args), capture_output=True)
    for name, path in bins.items():
        p = subprocess.run([path] + list(args), capture_output=True)
        if p.returncode != ref.returncode or p.stdout != ref.stdout:
            raise SystemExit(
                f"output mismatch for {args!r}: {name} exited {p.returncode} "
                f"with {p.stdout[:60]!r}, {first} exited {ref.returncode} with "
                f"{ref.stdout[:60]!r}; the back ends disagree, so the run-time "
                f"comparison would be invalid")
    return ref.stdout


def measure_interleaved(bins, args, reps):
    names = list(bins)
    walls = {n: [] for n in names}
    cpus = {n: [] for n in names}
    for _ in range(reps):
        for n in names:
            exe = shutil.which(bins[n]) or bins[n]
            t0 = time.perf_counter()
            pid = os.posix_spawn(exe, [exe] + list(args), os.environ,
                                 file_actions=_quiet_actions())
            _, status, ru = os.wait4(pid, 0)
            walls[n].append(time.perf_counter() - t0)
            cpus[n].append(ru.ru_utime + ru.ru_stime)
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
    rss = {n: 0 for n in names}
    for _ in range(max(3, reps)):
        for n in names:
            exe = shutil.which(bins[n]) or bins[n]
            pid = os.posix_spawn(exe, [exe] + list(args), os.environ,
                                 file_actions=_quiet_actions())
            peak = 0
            while True:
                hwm = peak_rss_kb(pid)
                if hwm > peak:
                    peak = hwm
                done, status, _ = os.wait4(pid, os.WNOHANG)
                if done:
                    break
                os.sched_yield()
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
            if peak > rss[n]:
                rss[n] = peak
    return {n: dict(wall=min(walls[n]), cpu=min(cpus[n]), rss=rss[n])
            for n in names}


def speedup(res, kind, a, base):
    """`base / kind` for one axis, or `None` when the ratio is meaningless."""
    def val(k):
        r = res[k]
        if a == "size":
            return r["size"]
        if a.startswith("run_"):
            return r["run"][a[4:]]
        return r[a]

    b = val(base)
    # A run shorter than the sampler's resolution can finish between two
    # `/proc` reads and come back as 0, and a run near it measures process
    # start-up rather than the code. Reporting either as a ratio would be a
    # fabricated win or loss, so leave it unmeasured.
    if a in ("run_wall", "run_cpu", "run_rss"):
        if b < RUN_FLOOR or val(kind) < RUN_FLOOR:
            return None
    return b / val(kind) if val(kind) else None


def report_vs(rows, base, label, axes, flag_losses=False, kind="cb"):
    """Print the `kind`-vs-`base` table and per-axis means; return all-win."""
    print(f"{kind} vs {label}  (>1 means the first is better; +/- "
          f"{NOISE_BAND:.2f} counts as a tie)")
    print(f"{'program':<9} " + " ".join(f"{a:>9}" for a in axes))
    means = {a: [] for a in axes}
    all_ok = True
    for p, res in rows:
        cells = []
        ok = True
        for a in axes:
            v = speedup(res, kind, a, base)
            means[a].append(v)
            cells.append("        -" if v is None else f"{v:9.2f}")
            if v is not None and v < 1.0 - NOISE_BAND:
                ok = False
        all_ok = all_ok and ok
        suffix = "" if (ok or not flag_losses) else "  <-- LOSS"
        print(f"{p:<9} " + " ".join(cells) + suffix)
    print()
    print(f"mean speedup per axis ({kind} vs {base})")
    for a in axes:
        vs = [v for v in means[a] if v is not None]
        if not vs:
            print(f"  {a:<9}       -  (unmeasured)")
            continue
        m = sum(vs) / len(vs)
        verdict = "win" if m >= 1.0 + NOISE_BAND else (
            "tie" if m >= 1.0 - NOISE_BAND else "loss")
        print(f"  {a:<9} {m:6.3f}  ({verdict})")
    print()
    return all_ok


def main():
    progs = sys.argv[1:] or PROGRAMS
    if not os.path.exists(CBWRAP) or not os.path.exists(CBSO):
        raise SystemExit("backend not built; run backend/build.sh first")
    os.makedirs(OUT, exist_ok=True)
    runtime_obj, stub_obj = build_runtime()
    rows = []
    for p in progs:
        res = measure_program(p, runtime_obj, stub_obj)
        bins = {k: os.path.join(OUT, f"{p}.{k}.bin")
                for k in ("cb", "mir", "llvm", "cl")}
        verify_outputs(bins, ARGS[p])
        runs = measure_interleaved(bins, ARGS[p], 50)
        for k in ("cb", "mir", "llvm", "cl"):
            res[k]["run"] = runs[k]
        rows.append((p, res))

    axes = ("wall", "cpu", "rss", "size", "run_wall", "run_cpu", "run_rss")
    hdr = (f"{'program':<9} {'be':<5} {'wall_ms':>9} {'cpu_ms':>9} "
           f"{'rss_kb':>9} {'size_b':>9} {'run_ms':>9} {'run_cpu':>9} "
           f"{'run_rss':>9}")
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for k in ("cb", "mir", "llvm", "cl"):
            r = res[k]
            print(f"{p:<9} {k:<5} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {r['run']['wall']*1000:9.2f} "
                  f"{r['run']['cpu']*1000:9.2f} {r['run']['rss']:9.0f}")

    print()
    all_ok = report_vs(rows, "llvm", "LLVM", axes, flag_losses=True)
    report_vs(rows, "mir", "printed-MIR prototype (`mir`)", axes)
    report_vs(rows, "cl", "rustc Cranelift (`cl`)", axes)

    print()
    print("ALL CB WINS" if all_ok else "some axes lost (cb vs llvm)")


if __name__ == "__main__":
    sys.exit(main())
