#!/usr/bin/env python3
"""Four-way benchmark on one source: every pipeline compiles the *same* file.

  * ``rmc``   -- rmc's own front end and back end: ``rmc P.rs``. No rustc.
  * ``cb``    -- rustc driving rmc as a ``-Zcodegen-backend`` dylib: rustc's
                 front end and MIR optimiser, rmc's x86-64 back end.
  * ``cl``    -- rustc's Cranelift back end.
  * ``llvm``  -- rustc's LLVM back end (``-O -C lto=fat``), the reference.

All four share **one input**, written so every front end accepts it: the
runtime declarations and the panic handler sit in a ``mod`` that rmc drops and
rustc compiles, and the names are re-exported with ``use``, which rmc drops and
rustc honours (see ``bench/gen_uni.py``). A run-time difference is therefore the
compiler, not the source.

Two program sets:

  * ``core`` -- ``bench/uni/*.rs``, the twelve programs the optimisers were
                tuned against;
  * ``real`` -- ``bench/real/*.rs``, ten realistic workloads (an interpreter, a
                sort, a hash table, BFS, ...) that were *not* used for tuning.
                A result that holds on ``core`` but not on ``real`` is
                over-fitting, so report both.

A pipeline that refuses a program, or whose binary prints something different
from LLVM's, is reported as such and left out of that program's timing; the
harness does not stop. Ratios are averaged geometrically.

Usage: python3 bench/run_four.py [--set core|real|all] [--markdown PATH] [program ...]
"""
import datetime
import math
import os
import platform
import subprocess
import sys

import pipelines as rc

OUT4 = "/tmp/rmcfour"
KINDS = ("rmc", "cb", "llvm", "cl")
BE_LABEL = {"rmc": "rmc (AST)", "cb": "rustc+rmc (`cb`)",
            "llvm": "rustc LLVM", "cl": "rustc Cranelift (`cl`)"}
AXES = ("wall", "cpu", "rss", "size", "run_wall", "run_cpu", "run_rss")

SETS = {
    "core": (os.path.join(rc.ROOT, "uni"), {
        "hello": [], "fib": ["32"], "mandel": ["800"], "sieve": ["200000"],
        "mix": ["2000000"], "stats": ["2000000"], "refs": ["5000000"],
        "heap": ["200000"], "callrec": ["30"], "forloops": ["2000000"],
        "light": ["2000000"], "heavy": ["1000000000"],
    }),
    "real": (os.path.join(rc.ROOT, "real"), {
        "vm": ["300000"], "sort": ["20"], "hashmap": ["4000000"], "bfs": ["60"],
        "life": ["1500"], "nqueens": ["13"], "fannkuch": ["10"], "matmul": ["40"],
        "textparse": ["300"], "particles": ["20000"],
    }),
}


# rmc's front end with Rust's release safety semantics, matching the rustc rows
# (bounds checks on, overflow checks off), so all four build the same program.
def rmc_ast(src, out):
    return [rc.RMC, src, "-o", out]


def stages_for(src, name, runtime_obj, stub_obj):
    b = lambda k: os.path.join(OUT4, f"{name}.{k}.bin")
    o = lambda k: os.path.join(OUT4, f"{name}.{k}.o")
    link = lambda obj, out: ["ld", "-s", "-o", out, "-e", "_start", obj, runtime_obj, stub_obj]
    return {
        "rmc": [("rmc", rmc_ast(src, b("rmc")))],
        "cb": [("cb", rc.CB + [src, "-o", b("cb")])],
        "llvm": [("llvm.front", rc.RUSTC_OBJ + [src, "-o", o("llvm")]),
                 ("llvm.back", link(o("llvm"), b("llvm")))],
        "cl": [("cl.front", rc.CL + [src, "-o", o("cl")]),
               ("cl.back", link(o("cl"), b("cl")))],
    }, {k: b(k) for k in KINDS}


def first_error(cmds):
    """Run each command once; return the first failure's message, or None."""
    for _, argv in cmds:
        p = subprocess.run(argv, capture_output=True, text=True)
        if p.returncode != 0:
            lines = (p.stderr or p.stdout).strip().splitlines()
            return next((l for l in lines if "error" in l), lines[0] if lines else "failed")
    return None


def measure_program(src, name, args, runtime_obj, stub_obj, reps):
    """Measure every pipeline that builds `name` and agrees with LLVM.

    Returns (results, status) where status maps a pipeline to "ok", a refusal
    message, or "WRONG OUTPUT"."""
    stages, bins = stages_for(src, name, runtime_obj, stub_obj)
    status = {k: first_error(stages[k]) or "ok" for k in KINDS}
    if status["llvm"] != "ok":
        raise SystemExit(f"{name}: the LLVM reference does not build: {status['llvm']}")
    ref = subprocess.run([bins["llvm"]] + args, capture_output=True)
    for k in KINDS:
        if status[k] != "ok" or k == "llvm":
            continue
        got = subprocess.run([bins[k]] + args, capture_output=True)
        if (got.returncode, got.stdout) != (ref.returncode, ref.stdout):
            status[k] = "WRONG OUTPUT"
    ok = [k for k in KINDS if status[k] == "ok"]
    r = rc.measure_stages([st for k in ok for st in stages[k]], reps)
    res = {}
    for k in ok:
        names = [n for n, _ in stages[k]]
        res[k] = {"wall": sum(r[n]["wall"] for n in names),
                  "cpu": sum(r[n]["cpu"] for n in names),
                  "rss": max(r[n]["rss"] for n in names),
                  "size": os.path.getsize(bins[k])}
    runs = rc.measure_interleaved({k: bins[k] for k in ok}, args, 30)
    for k in ok:
        res[k]["run"] = runs[k]
    return res, status


def compare(rows, kind, base, axes):
    """Per-program ratios (`base / kind`, >1 = `kind` better) and geomeans."""
    table, means = [], {a: [] for a in axes}
    for p, res in rows:
        if kind not in res or base not in res:
            table.append((p, None))
            continue
        cells = []
        for a in axes:
            v = rc.speedup(res, kind, a, base)
            cells.append(v)
            if v is not None:
                means[a].append(v)
        table.append((p, cells))
    geo = {a: (math.prod(v) ** (1 / len(v)) if v else None) for a, v in means.items()}
    return table, geo


def verdict(m):
    if m is None:
        return "unmeasured"
    return "win" if m >= 1 + rc.NOISE_BAND else ("tie" if m >= 1 - rc.NOISE_BAND else "loss")


def print_pair(rows, kind, base):
    table, geo = compare(rows, kind, base, AXES)
    print(f"{kind} vs {base}  (>1 = {kind} better; +/- {rc.NOISE_BAND:.2f} is a tie)")
    print(f"{'program':<10} " + " ".join(f"{a:>9}" for a in AXES))
    for p, cells in table:
        if cells is None:
            print(f"{p:<10} (not comparable: a pipeline refused or miscompiled it)")
            continue
        print(f"{p:<10} " + " ".join("        -" if v is None else f"{v:9.2f}" for v in cells))
    print("geomean    " + " ".join("        -" if geo[a] is None else f"{geo[a]:9.3f}" for a in AXES))
    print()
    return geo


def md_pair(rows, kind, base):
    table, geo = compare(rows, kind, base, AXES)
    lines = [f"### {BE_LABEL[kind]} vs {BE_LABEL[base]}", "",
             "| program | " + " | ".join(AXES) + " |",
             "| --- | " + " | ".join("---:" for _ in AXES) + " |"]
    for p, cells in table:
        if cells is None:
            lines.append(f"| {p} | " + " | ".join("n/a" for _ in AXES) + " |")
        else:
            lines.append(f"| {p} | " + " | ".join("-" if v is None else f"{v:.2f}" for v in cells) + " |")
    lines.append("| **geomean** | " + " | ".join(
        "-" if geo[a] is None else f"**{geo[a]:.3f}** ({verdict(geo[a])})" for a in AXES) + " |")
    lines.append("")
    return lines


def cpu_model():
    try:
        with open("/proc/cpuinfo") as f:
            for line in f:
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def write_markdown(results, path):
    rustc_v = subprocess.run(rc.RUSTC + ["-V"], capture_output=True, text=True).stdout.strip()
    lines = [
        "# Four-way benchmark: rmc, rustc+rmc, Cranelift, LLVM on one source",
        "",
        f"Recorded {datetime.datetime.now(datetime.timezone.utc):%Y-%m-%d %H:%M} UTC on "
        f"{platform.platform()}; CPU: {cpu_model()}; `{rustc_v}`.",
        "",
        "Regenerate with `python3 bench/run_four.py --set all --markdown bench/RESULTS-4WAY.md`.",
        "",
        "Every pipeline compiles the same file. `core` is the twelve programs the",
        "optimisers were tuned against (`bench/uni`); `real` is ten realistic",
        "workloads that were not (`bench/real`). Ratios are `other / this` per axis,",
        f"so `>1` means the first pipeline is better; `+/- {rc.NOISE_BAND:.2f}` is a tie,",
        "and the summary row is a geometric mean. A run shorter than the",
        "process-start resolution is unmeasured (`-`). A pipeline that refused a",
        "program or printed a different result is listed below and excluded (`n/a`).",
        "",
    ]
    for set_name, rows, statuses in results:
        lines += [f"## Set `{set_name}`", ""]
        bad = [(p, k, s) for p, st in statuses for k, s in st.items() if s != "ok"]
        if bad:
            lines += ["Not measured:", ""]
            lines += [f"* `{p}` / {BE_LABEL[k]}: {s}" for p, k, s in bad]
            lines.append("")
        lines += ["| program | pipeline | compile (ms) | compile CPU (ms) | compiler RSS (kB) | "
                  "binary (B) | run (ms) | run CPU (ms) | run RSS (kB) |",
                  "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for p, res in rows:
            for k in KINDS:
                if k not in res:
                    continue
                r = res[k]
                lines.append(
                    f"| {p} | {BE_LABEL[k]} | {r['wall']*1000:.1f} | {r['cpu']*1000:.1f} | "
                    f"{r['rss']:.0f} | {r['size']:.0f} | {r['run']['wall']*1000:.2f} | "
                    f"{r['run']['cpu']*1000:.2f} | {r['run']['rss']:.0f} |")
        lines.append("")
        for kind, base in (("cb", "llvm"), ("cb", "cl"), ("rmc", "llvm"), ("rmc", "cl"), ("rmc", "cb")):
            lines += md_pair(rows, kind, base)
    with open(path, "w") as f:
        f.write("\n".join(lines))


def main():
    args = sys.argv[1:]
    md_path, set_name = None, "core"
    if "--markdown" in args:
        i = args.index("--markdown")
        md_path = args[i + 1]
        del args[i:i + 2]
    if "--set" in args:
        i = args.index("--set")
        set_name = args[i + 1]
        del args[i:i + 2]
    if not os.path.exists(rc.CBSO):
        raise SystemExit("backend not built; run backend/build.sh first")
    os.makedirs(OUT4, exist_ok=True)
    runtime_obj, stub_obj = rc.build_runtime()
    results = []
    for s in (["core", "real"] if set_name == "all" else [set_name]):
        src_dir, argv = SETS[s]
        progs = args or list(argv)
        rows, statuses = [], []
        for p in progs:
            res, status = measure_program(os.path.join(src_dir, f"{p}.rs"), p, argv[p],
                                          runtime_obj, stub_obj, reps=7)
            rows.append((p, res))
            statuses.append((p, status))
            for k, st in status.items():
                if st != "ok":
                    print(f"[{s}] {p}: {k} {st}")
        print(f"\n=== set `{s}` ===")
        for p, res in rows:
            for k in KINDS:
                if k in res:
                    r = res[k]
                    print(f"{p:<10} {k:<5} compile {r['wall']*1000:8.1f} ms  size {r['size']:8.0f}  "
                          f"run {r['run']['wall']*1000:9.2f} ms")
        print()
        for kind, base in (("cb", "llvm"), ("cb", "cl"), ("rmc", "llvm"), ("rmc", "cl"), ("rmc", "cb")):
            print_pair(rows, kind, base)
        results.append((s, rows, statuses))
    if md_path:
        write_markdown(results, md_path)
        print(f"wrote {md_path}")


if __name__ == "__main__":
    sys.exit(main())
