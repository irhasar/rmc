#!/usr/bin/env python3
"""Benchmark harness: rmc vs rustc for compile time, CPU, RAM, size, runtime.

Uses os.wait4() to collect per-child resource usage (ru_utime, ru_stime,
ru_maxrss) so CPU and peak RSS are measured for the real compiler process and
its children (as/ld for rmc, LLVM + link for rustc).
"""
import os, subprocess, sys, time, shutil, statistics

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
OUT = "/tmp/rmcbench"
RUSTFLAGS_BASE = []

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats"]
# Runtime input per program, passed as argv so the workload is opaque to the
# optimizer and cannot be constant-folded away.
ARGS = {
    "hello": [],
    "fib": ["32"],
    "mandel": ["800"],
    "sieve": ["200000"],
    "mix": ["2000000"],
    "stats": ["2000000"],
}


def run_measured(argv, cwd=None, env=None):
    """Run argv, return (wall_s, cpu_s, max_rss_kb, returncode)."""
    t0 = time.perf_counter()
    pid = os.fork()
    if pid == 0:
        try:
            if cwd:
                os.chdir(cwd)
            os.execvpe(argv[0], argv, env if env is not None else os.environ)
        finally:
            os._exit(127)
    _, status, ru = os.wait4(pid, 0)
    wall = time.perf_counter() - t0
    cpu = ru.ru_utime + ru.ru_stime
    return wall, cpu, ru.ru_maxrss, status


def median(xs):
    return statistics.median(xs)


def measure_compile(name, kind, reps=5):
    walls, cpus, rsss, sizes = [], [], [], []
    for _ in range(reps):
        if kind == "rmc":
            out = os.path.join(OUT, f"{name}.rmc.bin")
            argv = [RMC, os.path.join(ROOT, "programs", f"{name}.rms"), "-o", out]
        else:
            out = os.path.join(OUT, f"{name}.rustc.bin")
            argv = ["rustc", "-O", "-C", "opt-level=3", "-C", "lto=fat",
                    "-C", "codegen-units=1", "-C", "panic=abort",
                    "-C", "strip=symbols",
                    os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out]
        w, c, r, st = run_measured(argv)
        if st != 0:
            raise RuntimeError(f"compile failed for {name} ({kind}) status={st}")
        walls.append(w); cpus.append(c); rsss.append(r)
        sizes.append(os.path.getsize(out))
    return dict(wall=median(walls), cpu=median(cpus), rss=median(rsss), size=median(sizes))


def measure_runtime(path, args, reps=9):
    times = []
    for _ in range(reps):
        w, c, r, st = run_measured([path] + args)
        if st != 0:
            raise RuntimeError(f"run failed for {path}: status={st}")
        times.append(w)
    return median(times)


def main():
    os.makedirs(OUT, exist_ok=True)
    hdr = f"{'program':<8} {'compiler':<8} {'wall_ms':>9} {'cpu_ms':>9} {'rss_kb':>9} {'size_b':>9} {'run_ms':>9}"
    print(hdr)
    print("-" * len(hdr))
    for p in PROGRAMS:
        res = {}
        for kind in ("rmc", "rustc"):
            res[kind] = measure_compile(p, kind)
        for kind in ("rmc", "rustc"):
            r = res[kind]
            binpath = os.path.join(OUT, f"{p}.{kind}.bin")
            rt = measure_runtime(binpath, ARGS[p])
            print(f"{p:<8} {kind:<8} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {rt*1000:9.2f}")
        # ratios
        n, c = res["rmc"], res["rustc"]
        print(f"{'':8} {'ratio':<8} {c['wall']/n['wall']:9.1f} {c['cpu']/n['cpu']:9.1f} "
              f"{c['rss']/n['rss']:9.1f} {c['size']/n['size']:9.1f}")


if __name__ == "__main__":
    main()
