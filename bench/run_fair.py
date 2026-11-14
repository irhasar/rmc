#!/usr/bin/env python3
"""Fair back-end benchmark: one source, two back ends.

The `run_mir.py` harness compares rmc's MIR back end against a *different*
LLVM program: the LLVM side is built from the `bench/rust/*.rs` `std` sources
while the MIR side is built from the `bench/mir/*.rs` freestanding twins, so
the two are not the same input and the run-time numbers are not comparable.

This harness removes that variable. Both back ends compile the **same**
`bench/mir/*.rs` source and link the **same** freestanding runtime
(`rmc --emit-runtime-asm`, assembled with `as`):

  * ``mir``  -- ``rustc --emit=mir -O`` then ``rmcmir`` (rmc's in-process
                assembler/linker), the whole point of the experiment;
  * ``llvm`` -- ``rustc --emit=obj -O -C lto=fat -C codegen-units=1`` then
                ``ld`` with rmc's runtime, i.e. LLVM's best codegen for the
                identical program.

Both link against the same `print_i64`/`arg_i64`/`alloc` runtime and the same
panic stubs (both trap with exit 101), so a run-time difference is the code
generator, not the standard library.

Measured per pipeline: compile wall time, compiler CPU, compiler peak RSS,
artifact size, and the produced binary's run-time wall/CPU/RSS.

Two properties keep the verdict defensible:

  * correctness gate -- before any run-time number is reported, both binaries
    are run once and must agree on stdout and exit status (see
    `verify_outputs`); a back end is only "faster" on a workload the other also
    computes correctly;
  * interleaved sampling -- the four compile stages (`measure_stages`) and the
    two produced binaries (`measure_interleaved`) are each timed round-robin in
    one pass, so CPU frequency and thermal drift cannot favour whichever
    pipeline ran first.
"""
import os
import subprocess
import sys
import time
import shutil


def pinned_channel():
    """The toolchain channel from `backend/rust-toolchain.toml`.

    `run_codegen.py`'s `cb` pipeline links one specific nightly's
    `rustc_private`, and this harness compares against it, so every rustc here
    must be that nightly rather than whatever `+nightly` points at. Read the pin
    (the single source of truth) instead of repeating the date.
    """
    pin = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                       "backend", "rust-toolchain.toml")
    with open(pin) as f:
        for line in f:
            key, _, val = line.partition("=")
            if key.strip() == "channel":
                return val.strip().strip('"')
    raise SystemExit(f"no `channel` in {pin}")

# `rmc` drives rustc with `-Zno-codegen` so the front end stops after the MIR
# optimisations instead of preparing codegen it will never use. The flag is
# nightly-only, so rmc sets `RUSTC_BOOTSTRAP=1` on the child rustc (see
# `front_end_mir` in compiler/src/main.rs); the harness mirrors that so the
# `mir` column measures the pipeline rmc actually runs.
os.environ.setdefault("RUSTC_BOOTSTRAP", "1")

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
RMCMIR = os.path.join(PROJ, "compiler", "target", "release", "examples", "rmcmir")
OUT = "/tmp/rmcfair"

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats", "refs",
            "heap", "callrec", "forloops", "light", "heavy"]
ARGS = {
    "hello": [],
    "fib": ["32"],
    "mandel": ["800"],
    "sieve": ["200000"],
    "mix": ["2000000"],
    "stats": ["2000000"],
    "refs": ["5000000"],
    "heap": ["200000"],
    "callrec": ["30"],
    "forloops": ["2000000"],
    "light": ["2000000"],
    "heavy": ["1000000000"],
}

RUSTC = ["rustc", "+" + pinned_channel()]
RUSTC_MIR = RUSTC + ["--emit=mir", "-O", "-Zno-codegen", "-C", "panic=abort",
                     "--crate-type=bin", "--edition", "2021"]
RUSTC_OBJ = RUSTC + ["--emit=obj", "-O", "-C", "opt-level=3", "-C", "lto=fat",
                     "-C", "codegen-units=1", "-C", "panic=abort",
                     "--edition", "2021"]

# Speedup ratios inside +/- this band are a statistical tie, not a win or a
# loss. The run axes take the minimum of 50 interleaved samples, but two runs of
# the *same* binary still land within ~1% of each other on this host (measured
# spread 0.8-1.3% for `mix`/`heap`), so a ratio like 0.99 carries no signal.
# 1.02 keeps the verdict from calling that tie a loss.
NOISE_BAND = 0.02

# Panic entry points the LLVM object may reference; the `mir` back end lowers
# the same checks to its own trap. Both paths exit 101, so the runtime
# comparison stays apples to apples. The names are *not* listed here: the
# mangled spelling embeds the hash of one libcore build, so it is read off the
# object under test instead (see `panic_stubs`), which keeps the harness working
# on any toolchain.
PANIC_MARKERS = ("panic", "overflow")


def undefined_symbols(obj):
    """The symbols `obj` references but does not define (see `nm -u`)."""
    p = subprocess.run(["nm", "-u", obj], capture_output=True)
    if p.returncode != 0:
        raise SystemExit(f"nm failed on {obj}:\n{p.stderr.decode(errors='replace')}")
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


def peak_rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def measure_stages(stages, reps):
    """Measure a pipeline's stages round-robin.

    `stages` is a list of ``(name, argv)``. Timing every stage of one pipeline
    to completion before starting the other lets CPU frequency drift land on
    the pipeline that ran first; interleaving the stages spreads it evenly.
    Each stage keeps its own wall/CPU/RSS, so the per-stage split stays
    available. Wall/CPU come from a clean `wait4` pass; peak RSS is sampled in
    its own pass (a busy-wait during the timed pass would perturb the very
    interval being measured, and a sub-millisecond stage can exit between two
    reads, so the sampler retries).
    """
    names = [n for n, _ in stages]
    walls = {n: [] for n in names}
    cpus = {n: [] for n in names}
    rss = {n: 0 for n in names}
    for _ in range(reps):
        for n, argv in stages:
            exe = shutil.which(argv[0]) or argv[0]
            t0 = time.perf_counter()
            pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ,
                                 file_actions=[])
            done, status, ru = os.wait4(pid, 0)
            walls[n].append(time.perf_counter() - t0)
            cpus[n].append(ru.ru_utime + ru.ru_stime)
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
    for _ in range(max(3, reps)):
        for n, argv in stages:
            exe = shutil.which(argv[0]) or argv[0]
            pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ,
                                 file_actions=[])
            peak = 0
            while True:
                hwm = peak_rss_kb(pid)
                if hwm > peak:
                    peak = hwm
                done, status, ru = os.wait4(pid, os.WNOHANG)
                if done:
                    break
                os.sched_yield()
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
            if peak > rss[n]:
                rss[n] = peak
    return {n: dict(wall=min(walls[n]), cpu=min(cpus[n]), rss=rss[n])
            for n in names}


def build_runtime():
    asm = subprocess.run([RMC, "--emit-runtime-asm"], capture_output=True,
                         check=True).stdout
    asm_path = os.path.join(OUT, "runtime.s")
    with open(asm_path, "wb") as f:
        f.write(asm)
    obj = os.path.join(OUT, "runtime.o")
    subprocess.run(["as", "--64", "-o", obj, asm_path], check=True)

    # Discover the panic symbols from the programs themselves rather than
    # hard-coding them: `lto=fat` mangles every entry point with the hash of
    # one particular libcore build, so a literal list breaks the link on any
    # other toolchain. Compile each program once (untimed, before sampling) and
    # union the undefined `panic`/`overflow` names, so the stub set adapts to
    # whatever codegen actually emits.
    names = set()
    for name in PROGRAMS:
        src = os.path.join(ROOT, "mir", f"{name}.rs")
        probe = os.path.join(OUT, f"{name}.probe.o")
        p = subprocess.run(RUSTC_OBJ + [src, "-o", probe], capture_output=True)
        if p.returncode != 0:
            raise SystemExit(f"probe compile failed for {name}:\n"
                             f"{p.stderr.decode(errors='replace')}")
        names.update(n for n in undefined_symbols(probe)
                     if any(m in n for m in PANIC_MARKERS))
    if not names:
        raise SystemExit(
            "no panic/overflow symbols found in any program; the stub set "
            "cannot be built, so the LLVM link would fail")

    stubs = ["\t.text", "\t.globl __rmc_trap_stub", "__rmc_trap_stub:",
             "\tmov $60, %rax", "\tmov $101, %rdi", "\tsyscall"]
    for s in sorted(names):
        stubs += [f'\t.globl "{s}"', f'"{s}":', "\tjmp __rmc_trap_stub"]
    stub_s = os.path.join(OUT, "panic_stubs.s")
    with open(stub_s, "w") as f:
        f.write("\n".join(stubs) + "\n")
    stub_o = os.path.join(OUT, "panic_stubs.o")
    subprocess.run(["as", "--64", "-o", stub_o, stub_s], check=True)
    return obj, stub_o


def pipeline_stages(name, runtime_obj, stub_obj):
    """The four compile stages of one program, both pipelines.

    `mir` is rustc's MIR front end then rmc's back end; `llvm` is rustc's LLVM
    back end then `ld`. Listing them together lets `measure_stages` interleave
    all four, so neither pipeline is systematically measured on a colder or
    hotter core than the other.
    """
    src = os.path.join(ROOT, "mir", f"{name}.rs")
    mir = os.path.join(OUT, f"{name}.mir")
    mir_bin = os.path.join(OUT, f"{name}.mir.bin")
    obj = os.path.join(OUT, f"{name}.llvm.o")
    llvm_bin = os.path.join(OUT, f"{name}.llvm.bin")
    stages = [
        ("mir.front", RUSTC_MIR + [src, "-o", mir]),
        ("mir.back", [RMCMIR, mir, "-o", mir_bin]),
        ("llvm.front", RUSTC_OBJ + [src, "-o", obj]),
        # `ld -s` strips the LLVM link's symbol table. rmc's own linker emits
        # no section headers at all, so its binary is symbol-free already;
        # leaving LLVM's symtab in would credit rmc with ~5.7 kB it did not
        # save. `run.py` strips rustc for the same reason.
        ("llvm.back", ["ld", "-s", "-o", llvm_bin, "-e", "_start", obj,
                       runtime_obj, stub_obj]),
    ]
    return stages, mir_bin, llvm_bin


def measure_program(name, runtime_obj, stub_obj, reps=9):
    stages, mir_bin, llvm_bin = pipeline_stages(name, runtime_obj, stub_obj)
    r = measure_stages(stages, reps)
    mf, mb = r["mir.front"], r["mir.back"]
    lf, lb = r["llvm.front"], r["llvm.back"]
    return {
        "mir": {
            "wall": mf["wall"] + mb["wall"], "cpu": mf["cpu"] + mb["cpu"],
            "rss": max(mf["rss"], mb["rss"]),
            "front_wall": mf["wall"], "back_wall": mb["wall"],
            "front_cpu": mf["cpu"], "back_cpu": mb["cpu"],
            "front_rss": mf["rss"], "back_rss": mb["rss"],
            "size": os.path.getsize(mir_bin),
        },
        "llvm": {
            "wall": lf["wall"] + lb["wall"], "cpu": lf["cpu"] + lb["cpu"],
            # Peak of the whole pipeline, `ld` included, matching the `mir`
            # column (which counts `rmcmir`): the link step allocates its own
            # RSS on both sides, so leaving it out would flatter LLVM.
            "rss": max(lf["rss"], lb["rss"]),
            "front_wall": lf["wall"], "back_wall": lb["wall"],
            "front_cpu": lf["cpu"], "back_cpu": lb["cpu"],
            "front_rss": lf["rss"], "back_rss": lb["rss"],
            "size": os.path.getsize(llvm_bin),
        },
    }


def verify_outputs(bins, args):
    """Require both back ends to produce the same program.

    Run each produced binary once and compare stdout and exit status. If they
    differ, the two pipelines are not compiling the same program and every
    run-time number below is meaningless, so abort instead of printing a
    verdict. This is the fairness precondition: a back end is only faster on a
    workload the other back end also computes correctly.
    """
    first_name = next(iter(bins))
    ref = subprocess.run([bins[first_name]] + list(args), capture_output=True)
    for name, path in bins.items():
        p = subprocess.run([path] + list(args), capture_output=True)
        if p.returncode != ref.returncode or p.stdout != ref.stdout:
            raise SystemExit(
                f"output mismatch for {args!r}: {name} exited "
                f"{p.returncode} with {p.stdout!r}, {first_name} exited "
                f"{ref.returncode} with {ref.stdout!r}; the back ends disagree, "
                f"so the run-time comparison would be invalid")
    return ref.stdout


def measure_interleaved(bins, args, reps):
    """Round-robin runtime sampling of several binaries.

    Measuring one pipeline to completion and then the next lets CPU frequency
    and thermal drift land on whichever ran first, which can decide a
    near-parity case by itself. Interleaving spreads any drift across the
    pipelines, the same way `run_mir.py`'s `measure_runs` does. Wall/CPU come
    from `wait4` in a clean pass; peak RSS is sampled in its own pass, matching
    how `measure` treats the compile steps.
    """
    names = list(bins)
    walls = {n: [] for n in names}
    cpus = {n: [] for n in names}
    for _ in range(reps):
        for n in names:
            exe = shutil.which(bins[n]) or bins[n]
            t0 = time.perf_counter()
            pid = os.posix_spawn(exe, [exe] + list(args), os.environ,
                                 file_actions=[])
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
                                 file_actions=[])
            peak = 0
            while True:
                hwm = peak_rss_kb(pid)
                if hwm > peak:
                    peak = hwm
                done, status, ru = os.wait4(pid, os.WNOHANG)
                if done:
                    break
                os.sched_yield()
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
            if peak > rss[n]:
                rss[n] = peak
    return {n: dict(wall=min(walls[n]), cpu=min(cpus[n]), rss=rss[n])
            for n in names}


def main():
    os.makedirs(OUT, exist_ok=True)
    runtime_obj, stub_obj = build_runtime()
    rows = []
    for p in PROGRAMS:
        res = measure_program(p, runtime_obj, stub_obj)
        bins = {kind: os.path.join(OUT, f"{p}.{kind}.bin")
                for kind in ("mir", "llvm")}
        verify_outputs(bins, ARGS[p])
        runs = measure_interleaved(bins, ARGS[p], 50)
        for kind in ("mir", "llvm"):
            res[kind]["run"] = runs[kind]
        rows.append((p, res))

    axes = ("wall", "cpu", "rss", "size", "run_wall", "run_cpu", "run_rss")
    hdr = (f"{'program':<9} {'be':<5} {'wall_ms':>9} {'cpu_ms':>9} "
           f"{'rss_kb':>9} {'size_b':>9} {'run_ms':>9} {'run_cpu':>9} "
           f"{'run_rss':>9}")
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for kind in ("mir", "llvm"):
            r = res[kind]
            print(f"{p:<9} {kind:<5} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {r['run']['wall']*1000:9.2f} "
                  f"{r['run']['cpu']*1000:9.2f} {r['run']['rss']:9.0f}")

    def speedups(res):
        """mir/llvm ratio per axis; >1 means mir is better. None = unmeasured."""
        m, l = res["mir"], res["llvm"]
        out = {}
        for a in axes:
            if a == "size":
                mv, lv = m["size"], l["size"]
            elif a.startswith("run_"):
                mv, lv = m["run"][a[4:]], l["run"][a[4:]]
            else:
                mv, lv = m[a], l[a]
            out[a] = lv / mv if mv else None
        return out

    print()
    print("speedup mir vs llvm (>1 means mir is better; +/- "
          f"{NOISE_BAND:.2f} counts as a tie)")
    print(f"{'program':<9} " + " ".join(f"{a:>9}" for a in axes))
    all_ok = True
    for p, res in rows:
        sp = speedups(res)
        ok = True
        for a in axes:
            r = sp[a]
            if r is not None and r < 1.0 - NOISE_BAND:
                ok = False
        all_ok = all_ok and ok
        mark = "" if ok else "  <-- LOSS"
        cells = " ".join("      n/a" if sp[a] is None else f"{sp[a]:9.2f}"
                         for a in axes)
        print(f"{p:<9} {cells}{mark}")

    # The two compile-RSS and run-RSS axes are aggregates whose per-program
    # verdicts are meaningful, but the *headline* claim is "no axis loses
    # systematically". Report the mean speedup per axis so a single program's
    # tie cannot hide a real regression and one program's loss cannot be
    # generalised.
    print()
    print("mean speedup per axis")
    for a in axes:
        vals = [speedups(res)[a] for _, res in rows]
        vals = [v for v in vals if v is not None]
        mean = sum(vals) / len(vals)
        verdict = "win" if mean >= 1.0 + NOISE_BAND else (
            "tie" if mean >= 1.0 - NOISE_BAND else "loss")
        print(f"  {a:<9} {mean:6.3f}  ({verdict})")

    # The one axis the whole-pipeline view calls a tie is compile RSS, and the
    # split shows why: rustc's front end dominates both peaks, and `--emit=mir`
    # allocates ~3-4 MB more than `--emit=obj` *inside the same rustc*, while
    # rmc's own back end (`rmcmir`) peaks below `ld`. The front end is rustc in
    # both pipelines, so the back-end-only view isolates the stage this
    # experiment actually swaps.
    print()
    print("back-end-only comparison (the stage rmc replaces: rmcmir vs ld)")
    print(f"{'program':<9} {'be_wall':>8} {'be_cpu':>7} {'be_rss':>7} "
          f"{'be_size':>8}   (llvm/rmc, >1 means rmc better)")
    be_wall, be_cpu, be_rss = [], [], []
    for p, res in rows:
        m, l = res["mir"], res["llvm"]
        rw = l["back_wall"] / m["back_wall"] if m["back_wall"] else None
        rc = l["back_cpu"] / m["back_cpu"] if m["back_cpu"] else None
        rr = l["back_rss"] / m["back_rss"] if m["back_rss"] else None
        be_wall.append(rw); be_cpu.append(rc); be_rss.append(rr)
        print(f"{p:<9} {rw:>8.2f} {rc:>7.2f} {rr:>7.2f} "
              f"{res['llvm']['size']/res['mir']['size']:>8.2f}")
    def _mean(vs):
        vs = [v for v in vs if v is not None]
        return sum(vs) / len(vs) if vs else 0.0
    print(f"{'mean':<9} {_mean(be_wall):>8.2f} {_mean(be_cpu):>7.2f} "
          f"{_mean(be_rss):>7.2f}")

    print()
    print("ALL MIR WINS" if all_ok else "some axes lost")


if __name__ == "__main__":
    sys.exit(main())
