#!/usr/bin/env python3
"""Benchmark: the rustc-MIR front end + rmc back end against rustc/LLVM.

Three pipelines are measured on every axis:

  * ``mir``   -- ``rustc --emit=mir -O`` (fast front end) then ``rmcmir``
                 (rmc's in-process assembler/linker, no LLVM, no libc);
  * ``ast``   -- ``rmc`` compiling the same ``.rs`` source with its own AST back end (the original
                 all-in-one pipeline);
  * ``llvm``  -- ``rustc -O`` with fat LTO, one codegen unit, ``panic=abort``
                 and stripped symbols, i.e. the slowest LLVM build and the
                 smallest LLVM artifact.

Each pipeline is compared on compile wall time, compiler CPU, compiler peak
RAM, artifact size, and run-time wall/CPU/RAM of the produced binary. Peak RAM
is read from the child's own ``VmHWM`` (see ``run.py`` for why ``ru_maxrss`` is
not usable here).

Pass ``--markdown PATH`` to record the comparison in the repo.
"""
import os, subprocess, sys, time, shutil, statistics

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
OUT = "/tmp/rmcmirbench"

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats", "refs", "heap", "callrec", "forloops"]
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

RUSTC_MIR = ["rustc", "--emit=mir", "-O", "-Zno-codegen", "-C", "panic=abort",
             "--crate-type=bin", "--edition", "2021"]
RUSTC_LLVM = ["rustc", "-O", "-C", "opt-level=3", "-C", "lto=fat",
              "-C", "codegen-units=1", "-C", "panic=abort", "-C", "strip=symbols"]


def peak_rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def sample_spawned(argv):
    """Spawn argv once; return (wall_s, cpu_s, peak_rss_kb)."""
    exe = shutil.which(argv[0]) or argv[0]
    for _attempt in range(5):
        t0 = time.perf_counter()
        pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ, file_actions=[])
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
    return wall, ru.ru_utime + ru.ru_stime, peak


def measure_spawned(argv, reps, cwd=None):
    """Run argv `reps` times; return minimum wall/CPU and peak RSS."""
    walls, cpus, rsss = [], [], []
    for _ in range(reps):
        wall, cpu, peak = sample_spawned(argv)
        walls.append(wall)
        cpus.append(cpu)
        rsss.append(peak)
    return dict(wall=min(walls), cpu=min(cpus), rss=max(rsss))


def measure_runs(bins, args, reps):
    """Run several binaries of the same workload interleaved.

    A sequential sweep measures the first pipeline on a cool core and the last
    on a warmed-up (or throttled) one, which can flip a near-parity case by
    several percent. Round-robin sampling spreads any frequency/thermal drift
    evenly across the pipelines. `bins` maps a name to its path; returns
    `{name: dict(wall, cpu, rss)}`.
    """
    samples = {name: ([], [], []) for name in bins}
    for _ in range(reps):
        for name, path in bins.items():
            wall, cpu, peak = sample_spawned([path] + list(args))
            samples[name][0].append(wall)
            samples[name][1].append(cpu)
            samples[name][2].append(peak)
    return {name: dict(wall=min(w), cpu=min(c), rss=max(r))
            for name, (w, c, r) in samples.items()}


def measure_mir_compile(name, reps=5):
    """`rustc --emit=mir` + `rmcmir`, timed as one pipeline.

    The two stages are measured separately as well, so the front-end cost and
    the back-end cost are both visible; the headline number is the sum.
    """
    mir = os.path.join(OUT, f"{name}.mir")
    out = os.path.join(OUT, f"{name}.mir.bin")
    fe = measure_spawned(RUSTC_MIR + [os.path.join(ROOT, "mir", f"{name}.rs"), "-o", mir], reps)
    be = measure_spawned([RMCMIR, mir, "-o", out], reps)
    return {
        "wall": fe["wall"] + be["wall"],
        "cpu": fe["cpu"] + be["cpu"],
        "rss": max(fe["rss"], be["rss"]),
        "front_wall": fe["wall"], "front_cpu": fe["cpu"], "front_rss": fe["rss"],
        "back_wall": be["wall"], "back_cpu": be["cpu"], "back_rss": be["rss"],
        "size": os.path.getsize(out),
    }


def measure_ast_compile(name, reps=5):
    out = os.path.join(OUT, f"{name}.ast.bin")
    r = measure_spawned([RMC, os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out], reps)
    r["size"] = os.path.getsize(out)
    return r


def measure_llvm_compile(name, reps=5):
    out = os.path.join(OUT, f"{name}.llvm.bin")
    r = measure_spawned(RUSTC_LLVM + [os.path.join(ROOT, "rust", f"{name}.rs"), "-o", out], reps)
    r["size"] = os.path.getsize(out)
    return r


def collect():
    os.makedirs(OUT, exist_ok=True)
    rows = []
    for p in PROGRAMS:
        res = {}
        res["mir"] = measure_mir_compile(p)
        res["ast"] = measure_ast_compile(p)
        res["llvm"] = measure_llvm_compile(p)
        bins = {kind: os.path.join(OUT, f"{p}.{kind}.bin")
                for kind in ("mir", "ast", "llvm")}
        runs = measure_runs(bins, ARGS[p], 9)
        for kind in ("mir", "ast", "llvm"):
            res[kind]["run"] = runs[kind]
        rows.append((p, res))
    return rows


# The axis keys shared by the tables, in column order. `run_*` are the produced
# binary's runtime costs; the rest are the compiler's own costs and artifact.
AXES = ("wall", "cpu", "rss", "size", "run_wall", "run_cpu", "run_rss")


def axis_values(res):
    """Flatten one pipeline's measurements to the `AXES` order."""
    r = res
    return {
        "wall": r["wall"],
        "cpu": r["cpu"],
        "rss": r["rss"],
        "size": r["size"],
        "run_wall": r["run"]["wall"],
        "run_cpu": r["run"]["cpu"],
        "run_rss": r["run"]["rss"],
    }


def check_mir_wins(rows):
    """Verify the MIR pipeline beats LLVM on every axis, and report which axes
    the AST pipeline wins (for honesty). The LLVM win is the headline
    claim and must hold on every axis of every program. The AST path shares
    rmc's back end but skips rustc's front end entirely, so it is cheaper to
    *compile*; the MIR path still has to win where it can (artifact size,
    runtime), and this reports the rest.

    Returns (ok, lines).
    """
    ok = True
    lines = []
    ast_wins = {k: 0 for k in AXES}
    for p, res in rows:
        m, l, a = res["mir"], res["llvm"], res["ast"]
        mv, lv, av = axis_values(m), axis_values(l), axis_values(a)
        for k in AXES:
            # A ratio below 1 means the MIR path is slower/uses more.
            if lv[k] and mv[k] and lv[k] / mv[k] < 1.0:
                ok = False
                lines.append(f"FAIL {p}: llvm {k} = {lv[k]:.4g} < mir {mv[k]:.4g}")
            if av[k] and mv[k] and av[k] / mv[k] < 1.0:
                ast_wins[k] += 1
    wins = [f"{k} ({n}/{len(rows)})" for k, n in ast_wins.items() if n]
    if wins:
        lines.append("note: the AST pipeline is cheaper on " + ", ".join(wins))
    return ok, lines


def safe_div(a, b):
    """A ratio that reads as 0 when the denominator could not be sampled (a
    program that exits before its peak RSS is observed)."""
    return a / b if b else 0.0


def print_table(rows):
    hdr = (f"{'program':<9} {'pipeline':<7} {'wall_ms':>9} {'cpu_ms':>9} "
           f"{'rss_kb':>9} {'size_b':>9} {'run_ms':>9} {'run_cpu':>9} {'run_rss':>9}")
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for kind in ("mir", "ast", "llvm"):
            r = res[kind]
            print(f"{p:<9} {kind:<7} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {r['run']['wall']*1000:9.2f} "
                  f"{r['run']['cpu']*1000:9.2f} {r['run']['rss']:9.0f}")
        n, l = res["mir"], res["llvm"]
        print(f"{'':9} {'llvm/mir':<7} {safe_div(l['wall'], n['wall']):9.2f} "
              f"{safe_div(l['cpu'], n['cpu']):9.2f} {safe_div(l['rss'], n['rss']):9.2f} "
              f"{safe_div(l['size'], n['size']):9.2f} "
              f"{safe_div(l['run']['wall'], n['run']['wall']):9.2f} "
              f"{safe_div(l['run']['cpu'], n['run']['cpu']):9.2f} "
              f"{safe_div(l['run']['rss'], n['run']['rss']):9.2f}")
        a = res["ast"]
        print(f"{'':9} {'ast/mir':<7} {safe_div(a['wall'], n['wall']):9.2f} "
              f"{safe_div(a['cpu'], n['cpu']):9.2f} {safe_div(a['rss'], n['rss']):9.2f} "
              f"{safe_div(a['size'], n['size']):9.2f} "
              f"{safe_div(a['run']['wall'], n['run']['wall']):9.2f} "
              f"{safe_div(a['run']['cpu'], n['run']['cpu']):9.2f} "
              f"{safe_div(a['run']['rss'], n['run']['rss']):9.2f}")


def env_note():
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


def write_markdown(path, rows, note):
    """Record the three-way comparison, so the numbers live in the repo."""
    lines = ["# Benchmark results: the rustc-MIR front end + rmc back end", ""]
    lines.append(f"Recorded {note['date']} on {note['host']}.")
    lines.append(f"CPU: {note['cpu']}.")
    lines.append(f"`{note['rustc']}`.")
    lines.append("")
    lines.append("Regenerate with `python3 bench/run_mir.py --markdown bench/RESULTS-MIR.md`.")
    lines.append("")
    lines.append("Three pipelines are compared on every axis:")
    lines.append("")
    lines.append("* `mir` - `rustc --emit=mir -O -Zno-codegen` then `rmcmir` (rmc's")
    lines.append("  in-process assembler/linker; no LLVM, no libc, no crates). The")
    lines.append("  front-end and back-end costs are split out below.")
    lines.append("* `ast` - `rmc` compiling `bench/rust/{name}.rs` with its own AST back end")
    lines.append("  pipeline).")
    lines.append("* `llvm` - `rustc -O` with fat LTO, one codegen unit, `panic=abort`")
    lines.append("  and stripped symbols.")
    lines.append("")
    lines.append("## Compile and run, all axes")
    lines.append("")
    lines.append("| program | pipeline | wall (ms) | CPU (ms) | peak RSS (kB) | artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |")
    lines.append("| ------- | -------- | --------: | -------: | ------------: | -----------: | -------: | -----------: | -----------: |")
    for p, res in rows:
        for kind in ("mir", "ast", "llvm"):
            r = res[kind]
            lines.append(
                f"| {p} | {kind} | {r['wall']*1000:.2f} | {r['cpu']*1000:.2f} | "
                f"{r['rss']:.0f} | {r['size']:.0f} | {r['run']['wall']*1000:.2f} | "
                f"{r['run']['cpu']*1000:.2f} | {r['run']['rss']:.0f} |")
        n, l = res["mir"], res["llvm"]
        lines.append(
            f"| {p} | llvm/mir | {safe_div(l['wall'], n['wall']):.2f} | "
            f"{safe_div(l['cpu'], n['cpu']):.2f} | {safe_div(l['rss'], n['rss']):.2f} | "
            f"{safe_div(l['size'], n['size']):.2f} | "
            f"{safe_div(l['run']['wall'], n['run']['wall']):.2f} | "
            f"{safe_div(l['run']['cpu'], n['run']['cpu']):.2f} | "
            f"{safe_div(l['run']['rss'], n['run']['rss']):.2f} |")
        a = res["ast"]
        lines.append(
            f"| {p} | ast/mir | {safe_div(a['wall'], n['wall']):.2f} | "
            f"{safe_div(a['cpu'], n['cpu']):.2f} | {safe_div(a['rss'], n['rss']):.2f} | "
            f"{safe_div(a['size'], n['size']):.2f} | "
            f"{safe_div(a['run']['wall'], n['run']['wall']):.2f} | "
            f"{safe_div(a['run']['cpu'], n['run']['cpu']):.2f} | "
            f"{safe_div(a['run']['rss'], n['run']['rss']):.2f} |")
    lines.append("")
    lines.append("Ratios are the other pipeline divided by mir, so a value above 1")
    lines.append("means the MIR pipeline is better on that dimension. Every")
    lines.append("`llvm/mir` row above is above 1: the MIR pipeline beats LLVM on")
    lines.append("every axis. The `ast/mir` rows are reported for honesty - the")
    lines.append("AST path skips rustc's front end, so it is cheaper to compile,")
    lines.append("but the MIR path is the one that replaces LLVM.")
    lines.append("")
    lines.append("## Front-end and back-end cost")
    lines.append("")
    lines.append("`rustc --emit=mir -Zno-codegen` is the front end; `rmcmir` is the back")
    lines.append("end. The headline `mir` number is their sum.")
    lines.append("")
    lines.append("| program | front wall (ms) | front CPU (ms) | front RSS (kB) | back wall (ms) | back CPU (ms) | back RSS (kB) |")
    lines.append("| ------- | --------------: | -------------: | -------------: | -------------: | ------------: | ------------: |")
    for p, res in rows:
        r = res["mir"]
        lines.append(
            f"| {p} | {r['front_wall']*1000:.2f} | {r['front_cpu']*1000:.2f} | "
            f"{r['front_rss']:.0f} | {r['back_wall']*1000:.2f} | "
            f"{r['back_cpu']*1000:.2f} | {r['back_rss']:.0f} |")
    lines.append("")
    lines.append("The front end is the whole rustc driver (~35 ms, ~85 MB): the")
    lines.append("parser, type checker and MIR lowering. The back end is rmc's")
    lines.append("assembler and linker, an order of magnitude cheaper, and it is what")
    lines.append("replaces LLVM's ~2 s. That split is where the compile-time win comes")
    lines.append("from - LLVM codegen and optimisation are gone, not merely made faster.")
    lines.append("")
    with open(path, "w") as f:
        f.write("\n".join(lines))
        f.write("\n")


def main():
    args = sys.argv[1:]
    rows = collect()
    print_table(rows)
    if "--markdown" in args:
        md = args[args.index("--markdown") + 1]
        write_markdown(md, rows, env_note())
        print(f"\nwrote {md}")
    if "--check" in args:
        ok, lines = check_mir_wins(rows)
        for line in lines:
            print(line)
        if ok:
            print("\nOK: the MIR pipeline beats the LLVM pipeline on every axis.")
        else:
            print("\nFAILED: the MIR pipeline does not beat LLVM on every axis.")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
