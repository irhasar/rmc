#!/usr/bin/env python3
"""Four-way benchmark on one source: every pipeline compiles the *same* file.

  * ``rmc``   -- rmc's own AST front end: ``rmc bench/uni/P.rs``. No rustc.
  * ``cb``    -- rustc driving rmc as a ``-Zcodegen-backend`` dylib. rustc's
                 MIR front end, rmc's x86-64 back end, one process, no LLVM.
  * ``cl``    -- rustc's Cranelift back end.
  * ``llvm``  -- rustc's LLVM back end (``-O -C lto=fat``), the reference.

The point of this harness is that the four share **one input**. The older
harnesses compare different programs (`bench/rust/*.rs` is `std`, `bench/mir/*.rs`
is freestanding), so a run-time difference there could be the source rather than
the back end. ``bench/uni/*.rs`` is written so all four accept it: the runtime
declarations and the panic handler sit in a ``mod`` that rmc drops and rustc
compiles, and the names are re-exported with ``use``, which rmc drops and rustc
honours. Both front ends then see the same calls (see ``bench/gen_uni.py``).

Every axis is per program; ``>1`` means the first of the pair is better, and
``+/- 0.02`` counts as a tie. Measurement and reporting reuse
``run_codegen.py`` so the four-way numbers are taken the same way as the
three-way ones.

Usage: python3 bench/run_four.py [program ...]
"""
import datetime
import os
import platform
import subprocess
import sys

import run_codegen as rc

UNI = os.path.join(rc.ROOT, "uni")
OUT4 = "/tmp/rmcfour"

BE_LABEL = {"rmc": "rmc (AST)", "cb": "rustc+rmc (`cb`)",
            "llvm": "rustc LLVM", "cl": "rustc Cranelift (`cl`)"}

AXES = ("wall", "cpu", "rss", "size", "run_wall", "run_cpu", "run_rss")

# `rmc`'s AST path with Rust's release safety semantics, matching the rustc rows
# (bounds checks on, overflow checks off) so all four build the same program.
def rmc_ast(src, out):
    return [rc.RMC, src, "--bounds-checks", "-o", out]


def pipeline_stages(name, runtime_obj, stub_obj):
    src = os.path.join(UNI, f"{name}.rs")
    rmc_bin = os.path.join(OUT4, f"{name}.rmc.bin")
    cb_bin = os.path.join(OUT4, f"{name}.cb.bin")
    obj = os.path.join(OUT4, f"{name}.llvm.o")
    llvm_bin = os.path.join(OUT4, f"{name}.llvm.bin")
    cl_obj = os.path.join(OUT4, f"{name}.cl.o")
    cl_bin = os.path.join(OUT4, f"{name}.cl.bin")
    stages = [
        ("rmc", rmc_ast(src, rmc_bin)),
        ("cb", rc.CB + [src, "-o", cb_bin]),
        ("llvm.front", rc.RUSTC_OBJ + [src, "-o", obj]),
        ("llvm.back", ["ld", "-s", "-o", llvm_bin, "-e", "_start", obj,
                       runtime_obj, stub_obj]),
        ("cl.front", rc.CL + [src, "-o", cl_obj]),
        ("cl.back", ["ld", "-s", "-o", cl_bin, "-e", "_start", cl_obj,
                     runtime_obj, stub_obj]),
    ]
    return stages, rmc_bin, cb_bin, llvm_bin, cl_bin


def measure_program(name, runtime_obj, stub_obj, reps=9):
    stages, rmc_bin, cb_bin, llvm_bin, cl_bin = pipeline_stages(
        name, runtime_obj, stub_obj)
    r = rc.measure_stages(stages, reps)
    return {
        "rmc": {"wall": r["rmc"]["wall"], "cpu": r["rmc"]["cpu"],
                "rss": r["rmc"]["rss"], "size": os.path.getsize(rmc_bin)},
        "cb": {"wall": r["cb"]["wall"], "cpu": r["cb"]["cpu"],
               "rss": r["cb"]["rss"], "size": os.path.getsize(cb_bin)},
        "llvm": {"wall": r["llvm.front"]["wall"] + r["llvm.back"]["wall"],
                 "cpu": r["llvm.front"]["cpu"] + r["llvm.back"]["cpu"],
                 "rss": max(r["llvm.front"]["rss"], r["llvm.back"]["rss"]),
                 "size": os.path.getsize(llvm_bin)},
        "cl": {"wall": r["cl.front"]["wall"] + r["cl.back"]["wall"],
               "cpu": r["cl.front"]["cpu"] + r["cl.back"]["cpu"],
               "rss": max(r["cl.front"]["rss"], r["cl.back"]["rss"]),
               "size": os.path.getsize(cl_bin)},
    }


def md_table(rows, base, label, axes, kind):
    """Markdown speedup table for `kind` vs `base`, mirroring `report_vs`."""
    lines = [f"### {BE_LABEL[kind]} vs {label}", "",
             "| program | " + " | ".join(axes) + " |",
             "| --- | " + " | ".join("---:" for _ in axes) + " |"]
    means = {a: [] for a in axes}
    for p, res in rows:
        cells = []
        for a in axes:
            v = rc.speedup(res, kind, a, base)
            means[a].append(v)
            cells.append("-" if v is None else f"{v:.2f}")
        lines.append(f"| {p} | " + " | ".join(cells) + " |")
    lines += ["", "| axis | mean | verdict |", "| --- | ---: | --- |"]
    for a in axes:
        vs = [v for v in means[a] if v is not None]
        if not vs:
            lines.append(f"| {a} | - | unmeasured |")
            continue
        m = sum(vs) / len(vs)
        verdict = "win" if m >= 1.0 + rc.NOISE_BAND else (
            "tie" if m >= 1.0 - rc.NOISE_BAND else "loss")
        lines.append(f"| {a} | {m:.3f} | {verdict} |")
    lines.append("")
    return lines


def md_detail(rows):
    lines = [
        "| program | backend | wall (ms) | CPU (ms) | peak RSS (kB) | "
        "artifact (B) | run (ms) | run CPU (ms) | run RSS (kB) |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for p, res in rows:
        for k in ("rmc", "cb", "llvm", "cl"):
            r = res[k]
            lines.append(
                f"| {p} | {BE_LABEL[k]} | {r['wall']*1000:.2f} | "
                f"{r['cpu']*1000:.2f} | {r['rss']:.0f} | {r['size']:.0f} | "
                f"{r['run']['wall']*1000:.2f} | {r['run']['cpu']*1000:.2f} | "
                f"{r['run']['rss']:.0f} |")
    lines.append("")
    return lines


def write_markdown(rows, verdicts, path):
    def ver(cmd):
        return subprocess.run(cmd, capture_output=True, text=True).stdout.strip()
    rustc_v = ver(["rustc", "+nightly", "-V"])
    lines = [
        "# Four-way benchmark on one source: rmc, rustc+rmc, Cranelift, LLVM",
        "",
        f"Recorded {datetime.datetime.now(datetime.timezone.utc):%Y-%m-%d %H:%M} UTC "
        f"on {platform.platform()}.",
        f"CPU: {platform.processor() or 'unknown'}.",
        f"`{rustc_v}`.",
        "",
        "Regenerate with `python3 bench/run_four.py`.",
        "",
        "All four pipelines compile the **same** file, `bench/uni/*.rs` (generated",
        "by `bench/gen_uni.py`). The runtime declarations and the panic handler sit",
        "in a `mod` that rmc drops and rustc compiles, and the names are re-exported",
        "with `use`, which rmc drops and rustc honours, so both front ends see the",
        "same calls. This is what makes the run-time columns comparable: a difference",
        "is the compiler, not the source.",
        "",
        "* `rmc (AST)` - rmc's own front end and back end, no rustc at all.",
        "* `rustc+rmc (cb)` - rustc's MIR front end, rmc's back end.",
        "* `rustc Cranelift (cl)` - rustc's Cranelift back end.",
        "* `rustc LLVM` - rustc's LLVM back end (`-O -C lto=fat`), the reference.",
        "",
        "Every axis is per program; `>1` means the first of the pair is better, and",
        f"`+/- {rc.NOISE_BAND:.2f}` counts as a tie. A run axis below the",
        "sub-millisecond process-startup resolution is left unmeasured (`-`).",
        "",
        "## Compile and run, all axes",
        "",
    ]
    lines += md_detail(rows)
    lines += md_table(rows, "cb", "rustc+rmc (`cb`) - front end only", AXES, "rmc")
    lines += md_table(rows, "llvm", "LLVM", AXES, "cb")
    lines += md_table(rows, "cl", "rustc Cranelift (`cl`)", AXES, "cb")
    lines += md_table(rows, "llvm", "LLVM", AXES, "rmc")
    lines += md_table(rows, "cl", "rustc Cranelift (`cl`)", AXES, "rmc")
    lines += ["## Verdict", ""]
    for label, ok in verdicts:
        lines.append(f"* `{label}`: {'all axes win' if ok else 'some axes lost'}")
    lines.append("")
    with open(path, "w") as f:
        f.write("\n".join(lines))


def main():
    args = sys.argv[1:]
    md_path = None
    if "--markdown" in args:
        i = args.index("--markdown")
        md_path = args[i + 1]
        del args[i:i + 2]
    progs = args or rc.PROGRAMS
    if not os.path.exists(rc.CBSO):
        raise SystemExit("backend not built; run backend/build.sh first")
    os.makedirs(OUT4, exist_ok=True)
    runtime_obj, stub_obj = rc.build_runtime()
    rows = []
    for p in progs:
        res = measure_program(p, runtime_obj, stub_obj)
        bins = {k: os.path.join(OUT4, f"{p}.{k}.bin")
                for k in ("rmc", "cb", "llvm", "cl")}
        rc.verify_outputs(bins, rc.ARGS[p])
        runs = rc.measure_interleaved(bins, rc.ARGS[p], 50)
        for k in ("rmc", "cb", "llvm", "cl"):
            res[k]["run"] = runs[k]
        rows.append((p, res))

    hdr = (f"{'program':<9} {'be':<5} {'wall_ms':>9} {'cpu_ms':>9} "
           f"{'rss_kb':>9} {'size_b':>9} {'run_ms':>9} {'run_cpu':>9} "
           f"{'run_rss':>9}")
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for k in ("rmc", "cb", "llvm", "cl"):
            r = res[k]
            print(f"{p:<9} {k:<5} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} "
                  f"{r['run']['wall']*1000:9.2f} "
                  f"{r['run']['cpu']*1000:9.2f} {r['run']['rss']:9.0f}")

    print()
    verdicts = []
    # Front end: same rmc back end, rmc's AST front end vs rustc's MIR front end.
    verdicts.append(("rmc vs cb", rc.report_vs(
        rows, "cb", "rustc+rmc (`cb`)", AXES, flag_losses=True, kind="rmc")))
    # Back end: same rustc MIR front end, rmc's back end vs each rustc back end.
    verdicts.append(("cb vs llvm", rc.report_vs(
        rows, "llvm", "LLVM", AXES, flag_losses=True, kind="cb")))
    verdicts.append(("cb vs cl", rc.report_vs(
        rows, "cl", "rustc Cranelift (`cl`)", AXES, flag_losses=True,
        kind="cb")))
    # End to end: rmc's whole pipeline vs each rustc back end.
    verdicts.append(("rmc vs llvm", rc.report_vs(
        rows, "llvm", "LLVM", AXES, kind="rmc")))
    verdicts.append(("rmc vs cl", rc.report_vs(
        rows, "cl", "rustc Cranelift (`cl`)", AXES, kind="rmc")))

    print()
    for label, ok in verdicts:
        print(f"{label}: {'all axes win' if ok else 'some axes lost'}")

    if md_path:
        write_markdown(rows, verdicts, md_path)
        print(f"\nwrote {md_path}")


if __name__ == "__main__":
    sys.exit(main())

