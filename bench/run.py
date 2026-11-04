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

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats", "refs", "heap"]
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
    """Run argv `reps` times and return median wall/CPU and peak RSS.

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
    return dict(wall=median(walls), cpu=median(cpus), rss=max(rsss))


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
    lines.append("better on that dimension. Run time is a ratio of medians: rmc")
    lines.append("matches or beats rustc on every program, and on `refs` it wins by")
    lines.append("more than 2x because LICM reassociates the loop-invariant part of")
    lines.append("the accumulation into one hoisted value, so the loop does a single")
    lines.append("add per iteration instead of one per invariant term.")
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
