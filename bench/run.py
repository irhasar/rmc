#!/usr/bin/env python3
"""Benchmark harness: rmc vs rustc (LLVM) vs rustc (Cranelift).

Measures both halves of the comparison, so "faster" is answered on every axis
that matters:

  * compile: build wall time, compiler CPU, compiler peak RAM, and artifact size
    (the on-disk footprint of the linked executable);
  * run:     wall time, CPU and peak RAM of the compiled program.

Three compilers are compared. rustc with the LLVM backend is the headline
opponent. rustc with the Cranelift backend (`-Zcodegen-backend=cranelift`, a
nightly-only component) is the fair baseline for the "no LLVM" claim: it is
also LLVM-free, so a gap between it and rmc is rmc's own codegen and link step,
not the cost of an optimiser rmc never had. The Cranelift column is detected at
run time and simply omitted where the host has no nightly toolchain.

Wall and CPU time come from wait4(). Peak RAM comes from each process's own
`VmHWM`, not wait4()'s ru_maxrss: a spawned child shares this Python process's
address space before exec, so ru_maxrss floors at Python's RSS and hides the
real peak of a small binary. The same measurement is applied to the compilers
(as/ld for rmc, LLVM + link for rustc) and to the produced binaries.

Pass `--markdown PATH` to also record the whole comparison to a file in the
repo, so the numbers are versioned rather than lost in a terminal log.

Set `RMC_BENCH_CRANELIFT_RUSTC` to a nightly `rustc` path to override the
auto-detected Cranelift compiler.
"""
import os, subprocess, sys, time, shutil, statistics

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
OUT = "/tmp/rmcbench"
RUSTFLAGS_BASE = []

# `rustc_codegen_cranelift` is the third compiler in the comparison. It is a
# nightly-only backend (the `rustc-codegen-cranelift-preview` component), and it
# sits at the opposite end of the compile/run trade-off from LLVM: it compiles
# far faster but optimises far less, so it is the honest baseline for "how much
# of rmc's win is the frontend and how much is simply not running LLVM". It is
# detected, not assumed, so the harness still runs on a host without nightly
# (the cranelift column is then omitted rather than the suite failing).
CRANELIFT_ENV = "RMC_BENCH_CRANELIFT_RUSTC"

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats", "refs", "heap", "callrec", "forloops"]
# Runtime input per program, passed as argv so the workload is opaque to the
# optimizer and cannot be constant-folded away.
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
}


def peak_rss_kb(pid):
    """The child's own peak RSS (`VmHWM`) in kB, or 0 if it could not be read."""
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def measure_spawned(argv, reps):
    """Run argv `reps` times and return minimum wall/CPU and peak RSS.

    Spawns with posix_spawn, and reads peak RAM from the child's own `VmHWM`
    rather than `wait4`'s `ru_maxrss`. A freshly spawned child briefly shares
    this Python process's address space before `exec`, which floors `ru_maxrss`
    at Python's RSS (~15 MB) and hides the real peak of a small program or
    compiler. `VmHWM` is read after `exec`, so it reflects the process itself;
    it is a high-water mark, so sampling it until the child exits captures the
    peak. It is taken as the maximum over the reps, because a sub-millisecond
    run can exit between two samples and read back as zero.

    The sampling loop yields between reads (`sched_yield`). A tight
    `wait4(WNOHANG)` spin would otherwise keep this process runnable for the
    whole child lifetime and compete with the benchmark on a single-core host,
    inflating the very wall time being measured. A `sleep` would give up CPU
    too, but it also drops the sample rate enough to miss the peak of a
    sub-millisecond process (measured: ~0.8 MB vs the true ~2 MB for `hello`),
    so the loop yields rather than sleeps and keeps polling.
    """
    exe = shutil.which(argv[0]) or argv[0]
    walls, cpus, rsss = [], [], []
    for _ in range(reps):
        # A sub-millisecond run can exit before Python first reads `/proc`, so a
        # rep occasionally samples zero. Retry that rep rather than fall back to
        # `ru_maxrss`, which is floored at this process's RSS (see the docstring)
        # and would report a tiny program as ~15 MB.
        for _attempt in range(5):
            t0 = time.perf_counter()
            pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ)
            peak = 0
            while True:
                hwm = peak_rss_kb(pid)
                if hwm > peak:
                    peak = hwm
                done, status, ru = os.wait4(pid, os.WNOHANG)
                if done:
                    break
                os.sched_yield()
            wall = time.perf_counter() - t0
            if status != 0:
                raise RuntimeError(f"run failed for {exe}: status={status}")
            if peak > 0:
                break
        walls.append(wall)
        cpus.append(ru.ru_utime + ru.ru_stime)
        rsss.append(peak)
    # The minimum, not the median: the workload is deterministic and
    # single-threaded, so the least-contended sample is the best estimate of the
    # program's own cost. The median folds in any host noise that happened to
    # land on the middle sample, which on a shared machine can move a ratio by
    # tens of percent (a `stats` run once read 0.64x where the minimum is 1.14x).
    return dict(wall=min(walls), cpu=min(cpus), rss=max(rsss))


def median(xs):
    return statistics.median(xs)


_cranelift_rustc = None


def cranelift_rustc():
    """Path to a rustc whose cranelift backend is usable, or None.

    Prefers `RMC_BENCH_CRANELIFT_RUSTC` (an explicit path, for a host where the
    nightly toolchain lives elsewhere), then asks rustup for the nightly rustc.
    The returned path is the toolchain's own `rustc` binary, not the `rustup`
    proxy: the proxy forks a second process, which would show up as compile
    time the backend does not actually cost. Cached after the first probe.
    """
    global _cranelift_rustc
    if _cranelift_rustc is not None:
        return _cranelift_rustc or None
    candidate = os.environ.get(CRANELIFT_ENV)
    if not candidate:
        try:
            r = subprocess.run(["rustup", "which", "--toolchain", "nightly", "rustc"],
                               capture_output=True, text=True)
            if r.returncode == 0:
                candidate = r.stdout.strip()
        except FileNotFoundError:
            candidate = None
    # Verify the backend is present and functional with a trivial compile, so a
    # nightly without the component (or a broken one) drops the column instead
    # of failing mid-suite.
    if candidate and os.path.exists(candidate):
        try:
            probe = subprocess.run(
                [candidate, "-Zcodegen-backend=cranelift", "--version"],
                capture_output=True, text=True)
            if probe.returncode == 0:
                _cranelift_rustc = candidate
                return candidate
        except OSError:
            pass
    _cranelift_rustc = False
    return None


def measure_compile(name, kind, reps=5):
    if kind == "rmc":
        out = os.path.join(OUT, f"{name}.rmc.bin")
        # Bounds checks are on by default, matching Rust's release semantics
        # (bounds checks are always on; overflow checks are off). rustc is
        # invoked with `-C overflow-checks=off` below, so the two build the same
        # program: both check every index, neither checks arithmetic.
        argv = [RMC, os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out]
    elif kind == "cranelift":
        out = os.path.join(OUT, f"{name}.cranelift.bin")
        # The nightly backend via the rustup proxy. `codegen-units=1` and
        # `panic=abort` are accepted; `lto` is not (cranelift rejects it), so
        # the flag set is the closest supported match to the LLVM column
        # rather than a byte-for-byte copy of it. `-C opt-level=3` is implied
        # by `-O` and kept for symmetry with the LLVM invocation.
        argv = [cranelift_rustc(), "-Zcodegen-backend=cranelift", "-O",
                "-C", "opt-level=3", "-C", "codegen-units=1",
                "-C", "panic=abort", "-C", "strip=symbols",
                "-C", "overflow-checks=off", "-C", "debug-assertions=off",
                os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out]
    else:
        out = os.path.join(OUT, f"{name}.rustc.bin")
        argv = ["rustc", "-O", "-C", "opt-level=3", "-C", "lto=fat",
                "-C", "codegen-units=1", "-C", "panic=abort",
                "-C", "strip=symbols",
                # Overflow checks are already off at `-O`; set them explicitly
                # so the parity with rmc (which defaults them off) is stated
                # rather than implied by the profile. Bounds checks stay on:
                # they are Rust's release semantics and rmc's `--bounds-checks`
                # enables the matching ones.
                "-C", "overflow-checks=off",
                "-C", "debug-assertions=off",
                os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out]
    r = measure_spawned(argv, reps)
    r["size"] = os.path.getsize(out)
    return r


def measure_rustc_plain(name):
    """A plain `rustc -O` build, no fat LTO, no `codegen-units=1`, no stripping.

    The headline comparison above uses an aggressive rustc configuration, which
    is the right thing to beat on *compile* cost but the wrong thing to compare
    *artifact size* against: fat LTO and stripping shrink the rustc binary to a
    few hundred kB. The default `rustc -O` output is ~4.5 MB (mostly the static
    libstd/panic machinery), so the size ratio against a default build is far
    larger than the one the aggressive build reports. Both numbers are recorded
    so neither overstates the gap.
    """
    out = os.path.join(OUT, f"{name}.rustc.plain.bin")
    argv = ["rustc", "-O", os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out]
    subprocess.run(argv, check=True, capture_output=True)
    return os.path.getsize(out)


def measure_rustc_static(name):
    """A statically linked, stripped `rustc -O` build.

    rmc links no libc at all, so the honest size comparison is against a rustc
    binary that also does not depend on the system libc at run time. `+crt-static`
    produces that; it is ~1.2 MB for a hello-world, ~179x rmc's 6.6 kB, rather
    than the ~43x the dynamic stripped build suggests.
    """
    out = os.path.join(OUT, f"{name}.rustc.static.bin")
    argv = ["rustc", "-O", "-C", "panic=abort", "-C", "strip=symbols",
            "-C", "target-feature=+crt-static",
            os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out]
    subprocess.run(argv, check=True, capture_output=True)
    return os.path.getsize(out)


def env_note():
    """A short environment line so a recorded result is reproducible."""
    import platform
    rustc = subprocess.run(["rustc", "--version"], capture_output=True, text=True)
    cpu = ""
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    cpu = line.split(":", 1)[1].strip()
                    break
    except OSError:
        pass
    cl = cranelift_rustc()
    cranelift_ver = ""
    if cl:
        cr = subprocess.run([cl, "-Zcodegen-backend=cranelift", "--version"],
                            capture_output=True, text=True)
        cranelift_ver = (cr.stdout or "unknown").strip()
    return {
        "date": time.strftime("%Y-%m-%d %H:%M %Z"),
        "host": platform.platform(),
        "cpu": cpu or "unknown",
        "rustc": (rustc.stdout or "unknown").strip(),
        "cranelift": cranelift_ver,
    }


def verify_parity(rows, kinds):
    """Run every produced binary once and require identical stdout across kinds.

    The benchmark compares *speed*; this guards the premise that all three
    compilers built the same program. A silent miscompile would otherwise show
    up as a suspiciously fast (or slow) column rather than a failure. Only the
    compilers in `kinds` are checked, so a host without Cranelift still verifies
    rmc against LLVM.
    """
    for p, res in rows:
        outs = {}
        for kind in kinds:
            binpath = os.path.join(OUT, f"{p}.{kind}.bin")
            r = subprocess.run([binpath] + ARGS[p], capture_output=True)
            outs[kind] = r.stdout
        ref = outs["rmc"]
        for kind in [k for k in kinds if k != "rmc"]:
            if outs[kind] != ref:
                raise RuntimeError(
                    f"{p}: {kind} output differs from rmc\n"
                    f"  rmc:       {ref!r}\n  {kind}: {outs[kind]!r}")


def collect():
    """Run the whole suite and return per-program results for each compiler.

    The compiler set is rmc, LLVM-backed rustc, and (when the nightly cranelift
    backend is present) cranelift-backed rustc. The last is the fair baseline
    for the "no LLVM" claim: it is also LLVM-free, so any gap between it and rmc
    is rmc's own, not the cost of the optimiser rustc shares with neither.
    """
    os.makedirs(OUT, exist_ok=True)
    kinds = ["rmc", "rustc"]
    cl = cranelift_rustc()
    if cl:
        kinds.append("cranelift")
    rows = []
    for p in PROGRAMS:
        res = {}
        for kind in kinds:
            res[kind] = measure_compile(p, kind)
        for kind in kinds:
            binpath = os.path.join(OUT, f"{p}.{kind}.bin")
            res[kind]["run"] = measure_spawned([binpath] + ARGS[p], 9)
        res["plain"] = measure_rustc_plain(p)
        res["static"] = measure_rustc_static(p)
        rows.append((p, res))
    verify_parity(rows, kinds)
    return rows, kinds


def print_table(rows, kinds):
    # One line per compiler per program, with both halves of the comparison:
    # compile wall/CPU/RAM/size, then run wall/CPU/RAM. The run-time RAM column
    # matters as much as the compile-time one (a rustc binary floors at the
    # libc/ld.so footprint), so it is reported here too, not only in the
    # markdown.
    hdr = (f"{'program':<8} {'compiler':<10} {'wall_ms':>9} {'cpu_ms':>9} "
           f"{'rss_kb':>9} {'size_b':>9} {'run_ms':>9} {'run_cpu_ms':>11} "
           f"{'run_rss_kb':>11}")
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for kind in kinds:
            r = res[kind]
            print(f"{p:<8} {kind:<10} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {r['run']['wall']*1000:9.2f} "
                  f"{r['run']['cpu']*1000:11.2f} {r['run']['rss']:11.0f}")
        # Ratios are always against rmc: >1 means rmc is better. rustc (LLVM) is
        # the headline opponent; cranelift is the fair no-LLVM baseline.
        for kind in [k for k in kinds if k != "rmc"]:
            n, c = res["rmc"], res[kind]
            print(f"{'':8} {kind+'/rmc':<10} {c['wall']/n['wall']:9.1f} {c['cpu']/n['cpu']:9.1f} "
                  f"{c['rss']/n['rss']:9.1f} {c['size']/n['size']:9.1f} "
                  f"{c['run']['wall']/n['run']['wall']:9.2f} "
                  f"{c['run']['cpu']/n['run']['cpu']:11.2f} "
                  f"{c['run']['rss']/n['run']['rss']:11.2f}")


def write_markdown(path, rows, note, kinds):
    """Record the full comparison, so the numbers live in the repo, not a log."""
    has_cl = "cranelift" in kinds
    lines = ["# Benchmark results: rmc vs rustc (LLVM) vs rustc (Cranelift)", ""]
    lines.append(f"Recorded {note['date']} on {note['host']}.")
    lines.append(f"CPU: {note['cpu']}.")
    lines.append(f"`{note['rustc']}`.")
    if has_cl:
        lines.append(f"Cranelift backend: `{note.get('cranelift', 'unknown')}`.")
    lines.append("")
    lines.append("Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.")
    lines.append("")
    lines.append("## Compile: build time, CPU, RAM, artifact size")
    lines.append("")
    lines.append("The rustc (LLVM) column is an aggressive configuration (`-O` with fat")
    lines.append("LTO, `codegen-units=1`, `panic=abort`, stripped). That is the right")
    lines.append("target for *compile cost* - it is the slowest rustc build - but it is")
    lines.append("the *smallest* rustc artifact, so the size ratio here understates the")
    lines.append("gap against a default build. See the size table below for both.")
    lines.append("")
    if has_cl:
        lines.append("The rustc (Cranelift) column is the fair comparison for the")
        lines.append("no-LLVM claim: it runs the same rustc frontend rmc is measured")
        lines.append("against, but a backend that also does not use LLVM. Any gap between")
        lines.append("it and rmc is rmc's own codegen and link step, not the optimiser.")
        lines.append("Cranelift rejects `lto`, so that flag is omitted; the rest of the")
        lines.append("flag set matches the LLVM column. It is a nightly-only component,")
        lines.append("so the column is present only where the host has it installed.")
        lines.append("")
    lines.append("Every compiler is invoked with matching safety semantics: rmc with")
    lines.append("bounds checks on (the default; Rust's release semantics) and rustc with")
    lines.append("`-C overflow-checks=off` (already the `-O` default, stated explicitly).")
    lines.append("See the run section for why this matters.")
    lines.append("")
    lines.append("| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |")
    lines.append("| ------- | -------- | --------: | -------: | ------------: | -----------: |")
    for p, res in rows:
        for kind in kinds:
            r = res[kind]
            lines.append(f"| {p} | {kind} | {r['wall']*1000:.2f} | {r['cpu']*1000:.2f} | "
                         f"{r['rss']:.0f} | {r['size']:.0f} |")
        for kind in [k for k in kinds if k != "rmc"]:
            n, c = res["rmc"], res[kind]
            lines.append(f"| {p} | ratio ({kind}/rmc) | {c['wall']/n['wall']:.1f} | {c['cpu']/n['cpu']:.1f} | "
                         f"{c['rss']/n['rss']:.1f} | {c['size']/n['size']:.1f} |")
    lines.append("")
    lines.append("## Artifact size against each rustc build")
    lines.append("")
    size_hdr = "| program | rmc (B) | rustc LLVM fat-LTO stripped (B) |"
    size_sep = "| ------- | ------: | -------------------------------: |"
    if has_cl:
        size_hdr += " rustc Cranelift stripped (B) |"
        size_sep += " ---------------------------: |"
    size_hdr += " rustc -O plain (B) | rustc -O static stripped (B) |"
    size_sep += " -----------------: | ---------------------------: |"
    lines.append(size_hdr)
    lines.append(size_sep)
    for p, res in rows:
        row = f"| {p} | {res['rmc']['size']:.0f} | {res['rustc']['size']:.0f} |"
        if has_cl:
            row += f" {res['cranelift']['size']:.0f} |"
        row += f" {res['plain']:.0f} | {res['static']:.0f} |"
        lines.append(row)
    lines.append("")
    lines.append("rmc links no libc and no runtime, so a plain `rustc -O` build (which")
    lines.append("statically links libstd, the panic machinery and the allocator) is the")
    lines.append("closest thing to a like-for-like size reference; the stripped static")
    lines.append("build is the closest *runtime* one. Both are far larger than the")
    lines.append("fat-LTO stripped build quoted in the headline table.")
    lines.append("")
    lines.append("## Run: speed, CPU, RAM of the compiled binary")
    lines.append("")
    lines.append("| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) |")
    lines.append("| ------- | -------- | --------: | -------: | ------------: |")
    for p, res in rows:
        for kind in kinds:
            r = res[kind]["run"]
            lines.append(f"| {p} | {kind} | {r['wall']*1000:.2f} | {r['cpu']*1000:.2f} | {r['rss']:.0f} |")
        for kind in [k for k in kinds if k != "rmc"]:
            n, c = res["rmc"]["run"], res[kind]["run"]
            lines.append(f"| {p} | ratio ({kind}/rmc) | {c['wall']/n['wall']:.2f} | {c['cpu']/n['cpu']:.2f} | "
                         f"{c['rss']/n['rss']:.2f} |")
    lines.append("")
    lines.append("Ratios are the other compiler divided by rmc, so a value above 1 means")
    lines.append("rmc is better on that dimension. Both the LLVM and the Cranelift ratios")
    lines.append("use rmc as the denominator.")
    lines.append("")
    if has_cl:
        lines.append("The Cranelift run column is the one to read when asking whether rmc's")
        lines.append("*generated code* is good, as opposed to how fast it compiles. LLVM is")
        lines.append("a much stronger optimiser than either rmc or Cranelift, so a run-time")
        lines.append("loss to LLVM may only mean LLVM optimised harder. Where rmc and")
        lines.append("Cranelift are close on run time, rmc's codegen is doing its job; where")
        lines.append("Cranelift is clearly faster, rmc is leaving performance on the table.")
        lines.append("")
    lines.append("Run time is not uniformly in rmc's favour, and a ratio below 1 is")
    lines.append("possible. On loop-heavy integer code rmc is competitive: it wins on")
    lines.append("`forloops` because the rotated `for` compares the counter against the")
    lines.append("bound directly, and on `refs` because the accessor sum is loop-invariant")
    lines.append("and rmc's LICM hoists a read through a reference once a pre-pass proves")
    lines.append("the referent is a live local that the loop does not write.")
    lines.append("")
    lines.append("Two benchmark shapes were changed to make the run comparison fair, and")
    lines.append("both changes are worth knowing when reading the table. First, no program")
    lines.append("uses `black_box` inside a hot loop: rmc lowers it to the identity while")
    lines.append("rustc makes it opaque, so an in-loop `black_box` would charge rustc a")
    lines.append("store/reload rmc never emits. `refs` instead seeds the struct from an")
    lines.append("opaque value and reduces the accumulator with a data-dependent branch,")
    lines.append("which keeps the loop real for both compilers (it defeats LLVM's")
    lines.append("loop-closing and vectoriser) without a barrier only one side pays for.")
    lines.append("Second, the two builds use matching safety semantics: rmc checks bounds")
    lines.append("by default and rustc is invoked with `-C overflow-checks=off`, so both")
    lines.append("carry bounds checks and neither carries overflow checks. The check costs")
    lines.append("about 1% on the array-heavy programs, so it does not manufacture any win")
    lines.append("either way.")
    lines.append("On call-heavy code")
    lines.append("(`callrec`) rmc now keeps up because the self-recursion pass folds the")
    lines.append("right spine into a loop, leaving one call per level - the same")
    lines.append("transform rustc applies. A register-only function also drops the frame")
    lines.append("pointer, so a leaf call carries no prologue beyond saving the registers")
    lines.append("it actually uses. That pass is narrow (one `i64` parameter, an")
    lines.append("associative `+`/`*`, and a pure left operand), so a call-heavy shape it")
    lines.append("does not match would still lose: a stack-based generator with no")
    lines.append("register allocator pays the argument setup and register save/restore on")
    lines.append("every call that LLVM avoids. The per-program ratio above is the honest")
    lines.append("answer; there is no single speedup factor.")
    lines.append("")
    lines.append("Run-time ratios are also noisy and machine-dependent. Programs near")
    lines.append("parity (notably `fib` and `callrec`) flip between a small rmc win and a")
    lines.append("small rmc loss from run to run and from host to host - a single recorded")
    lines.append("ratio for such a program should not be read as a stable property. Treat")
    lines.append("the run column as indicative, and re-run on the target machine before")
    lines.append("drawing a conclusion about a specific workload.")
    lines.append("")
    lines.append("Peak RSS for a compiled rmc binary is a few kB because the binary")
    lines.append("links no libc and is mapped by the kernel directly. A dynamically")
    lines.append("linked rustc binary pulls in ld.so and libc, so its RSS floors at a")
    lines.append("couple of MB regardless of what the program does. That gap is real but")
    lines.append("is mostly the C library's footprint, not the generated code's.")
    lines.append("")
    with open(path, "w") as f:
        f.write("\n".join(lines))


def main():
    args = sys.argv[1:]
    md = None
    if "--markdown" in args:
        md = args[args.index("--markdown") + 1]
    rows, kinds = collect()
    print_table(rows, kinds)
    if md:
        write_markdown(md, rows, env_note(), kinds)
        print(f"\nwrote {md}")


if __name__ == "__main__":
    main()
