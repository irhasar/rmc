#!/usr/bin/env python3
"""Benchmark harness: rmc vs rustc for compile time, CPU, RAM, size, runtime.

Measures both halves of the comparison, so "faster" is answered on every axis
that matters:

  * compile: build wall time, compiler CPU, compiler peak RAM, and artifact size
    (the on-disk footprint of the linked executable);
  * run:     wall time, CPU and peak RAM of the compiled program.

Wall and CPU time come from wait4(). Peak RAM comes from each process's own
`VmHWM`, not wait4()'s ru_maxrss: a spawned child shares this Python process's
address space before exec, so ru_maxrss floors at Python's RSS and hides the
real peak of a small binary. The same measurement is applied to the compilers
(as/ld for rmc, LLVM + link for rustc) and to the produced binaries.

Pass `--markdown PATH` to also record the whole comparison to a file in the
repo, so the numbers are versioned rather than lost in a terminal log.
"""
import os, subprocess, sys, time, shutil, statistics

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
OUT = "/tmp/rmcbench"
RUSTFLAGS_BASE = []

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


def measure_compile(name, kind, reps=5):
    if kind == "rmc":
        out = os.path.join(OUT, f"{name}.rmc.bin")
        argv = [RMC, os.path.join(ROOT, "programs", f"{name}.rms"), "-o", out]
    else:
        out = os.path.join(OUT, f"{name}.rustc.bin")
        argv = ["rustc", "-O", "-C", "opt-level=3", "-C", "lto=fat",
                "-C", "codegen-units=1", "-C", "panic=abort",
                "-C", "strip=symbols",
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
    return {
        "date": time.strftime("%Y-%m-%d %H:%M %Z"),
        "host": platform.platform(),
        "cpu": cpu or "unknown",
        "rustc": (rustc.stdout or "unknown").strip(),
    }


def collect():
    """Run the whole suite and return per-program results for both compilers."""
    os.makedirs(OUT, exist_ok=True)
    rows = []
    for p in PROGRAMS:
        res = {}
        for kind in ("rmc", "rustc"):
            res[kind] = measure_compile(p, kind)
        for kind in ("rmc", "rustc"):
            binpath = os.path.join(OUT, f"{p}.{kind}.bin")
            res[kind]["run"] = measure_spawned([binpath] + ARGS[p], 9)
        res["plain"] = measure_rustc_plain(p)
        res["static"] = measure_rustc_static(p)
        rows.append((p, res))
    return rows


def print_table(rows):
    hdr = f"{'program':<8} {'compiler':<8} {'wall_ms':>9} {'cpu_ms':>9} {'rss_kb':>9} {'size_b':>9} {'run_ms':>9}"
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for kind in ("rmc", "rustc"):
            r = res[kind]
            print(f"{p:<8} {kind:<8} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {r['run']['wall']*1000:9.2f}")
        n, c = res["rmc"], res["rustc"]
        print(f"{'':8} {'ratio':<8} {c['wall']/n['wall']:9.1f} {c['cpu']/n['cpu']:9.1f} "
              f"{c['rss']/n['rss']:9.1f} {c['size']/n['size']:9.1f} "
              f"{c['run']['wall']/n['run']['wall']:9.2f}")


def write_markdown(path, rows, note):
    """Record the full comparison, so the numbers live in the repo, not a log."""
    lines = ["# Benchmark results: rmc vs rustc", ""]
    lines.append(f"Recorded {note['date']} on {note['host']}.")
    lines.append(f"CPU: {note['cpu']}.")
    lines.append(f"`{note['rustc']}`.")
    lines.append("")
    lines.append("Regenerate with `python3 bench/run.py --markdown bench/RESULTS.md`.")
    lines.append("")
    lines.append("## Compile: build time, CPU, RAM, artifact size")
    lines.append("")
    lines.append("The rustc column is an aggressive configuration (`-O` with fat LTO,")
    lines.append("`codegen-units=1`, `panic=abort`, stripped). That is the right target")
    lines.append("for *compile cost* - it is the slowest rustc build - but it is the")
    lines.append("*smallest* rustc artifact, so the size ratio here understates the gap")
    lines.append("against a default build. See the size table below for both.")
    lines.append("")
    lines.append("| program | compiler | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) |")
    lines.append("| ------- | -------- | --------: | -------: | ------------: | -----------: |")
    for p, res in rows:
        for kind in ("rmc", "rustc"):
            r = res[kind]
            lines.append(f"| {p} | {kind} | {r['wall']*1000:.2f} | {r['cpu']*1000:.2f} | "
                         f"{r['rss']:.0f} | {r['size']:.0f} |")
        n, c = res["rmc"], res["rustc"]
        lines.append(f"| {p} | ratio (rustc/rmc) | {c['wall']/n['wall']:.1f} | {c['cpu']/n['cpu']:.1f} | "
                     f"{c['rss']/n['rss']:.1f} | {c['size']/n['size']:.1f} |")
    lines.append("")
    lines.append("## Artifact size against each rustc build")
    lines.append("")
    lines.append("| program | rmc (B) | rustc -O fat-LTO stripped (B) | rustc -O plain (B) | rustc -O static stripped (B) |")
    lines.append("| ------- | ------: | ----------------------------: | -----------------: | ---------------------------: |")
    for p, res in rows:
        lines.append(f"| {p} | {res['rmc']['size']:.0f} | {res['rustc']['size']:.0f} | "
                     f"{res['plain']:.0f} | {res['static']:.0f} |")
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
        for kind in ("rmc", "rustc"):
            r = res[kind]["run"]
            lines.append(f"| {p} | {kind} | {r['wall']*1000:.2f} | {r['cpu']*1000:.2f} | {r['rss']:.0f} |")
        n, c = res["rmc"]["run"], res["rustc"]["run"]
        lines.append(f"| {p} | ratio (rustc/rmc) | {c['wall']/n['wall']:.2f} | {c['cpu']/n['cpu']:.2f} | "
                     f"{c['rss']/n['rss']:.2f} |")
    lines.append("")
    lines.append("Ratios are rustc divided by rmc, so a value above 1 means rmc is")
    lines.append("better on that dimension.")
    lines.append("")
    lines.append("Run time is not uniformly in rmc's favour, and a ratio below 1 is")
    lines.append("possible. On loop-heavy integer code rmc is competitive: it wins on")
    lines.append("`forloops` because the rotated `for` compares the counter against the")
    lines.append("bound directly, and on `refs` because LICM reassociates the")
    lines.append("loop-invariant part of the accumulation into one hoisted value. On call-heavy code")
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
    rows = collect()
    print_table(rows)
    if md:
        write_markdown(md, rows, env_note())
        print(f"\nwrote {md}")


if __name__ == "__main__":
    main()
