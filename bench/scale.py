#!/usr/bin/env python3
"""Compile-time scaling: how each pipeline's build time grows with program size.

The four-way benchmark compiles programs of a few dozen lines, where rustc's
front end dominates every rustc-based pipeline and a back end's own cost is
hidden. This harness generates programs of roughly 1k, 10k and 50k lines (many
functions with loops, branches, a `match` and a small array, in the `bench/uni`
dialect so every pipeline accepts the same file) and times each build:

  * ``rmc``          -- rmc's own front end and back end;
  * ``cb``           -- rustc + rmc as a codegen backend;
  * ``llvm-O3``      -- rustc + LLVM, ``-C opt-level=3``, default codegen units;
  * ``llvm-O0``      -- rustc + LLVM, ``-C opt-level=0`` (a debug build);
  * ``cranelift``    -- rustc + Cranelift (a debug build);
  * ``front-end``    -- ``rustc --emit=metadata``: type checking with no code
                        generation, the floor every rustc pipeline pays.

Each binary must print the same output as ``llvm-O3``. Build time is the
minimum of three builds.

Usage: python3 bench/scale.py [--quick] [--markdown PATH] [NFUNCS ...]
"""
import datetime
import os
import random
import subprocess
import sys
import time

import pipelines as rc

OUT = "/tmp/rmcscale"
R = rc.RUSTC + ["--edition", "2021", "-Awarnings", "-C", "panic=abort"]

FN = """fn f{k}(x: i64) -> i64 {{
    let mut acc: i64 = x ^ {c1};
    let mut i: i64 = 0;
    while i < 3 {{
        acc = acc.wrapping_mul({c2}).wrapping_add(i);
        if acc % {c3} == {c4} {{
            acc ^= {c5};
        }} else {{
            acc = acc.wrapping_add({c6});
        }}
        i += 1;
    }}
    let t = match acc & 3 {{
        0 => acc >> 1,
        1 => acc.wrapping_add({c7}),
        2 => acc ^ {c8},
        _ => acc.wrapping_sub({c9}),
    }};
    let mut arr: [i64; 4] = [t, x, {c10}, i];
    arr[(t & 3) as usize] += 1;
    arr[0].wrapping_add(arr[1]).wrapping_add(arr[2]).wrapping_add(arr[3])
}}
"""


def prelude():
    hello = open(os.path.join(rc.ROOT, "uni", "hello.rs")).read()
    return hello[: hello.index("#[no_mangle]")]


def generate(nfn, path):
    rnd = random.Random(nfn)
    parts = [prelude()]
    for k in range(nfn):
        parts.append(FN.format(k=k, **{f"c{j}": rnd.randint(2, 99991) for j in range(1, 11)}))
    parts.append("#[no_mangle]\npub fn main() -> i64 {\n    let x = arg(1);\n    let mut s: i64 = 0;\n")
    parts += [f"    s = s.wrapping_add(f{k}(x));\n" for k in range(nfn)]
    parts.append("    print(s);\n    0\n}\n")
    src = "".join(parts)
    with open(path, "w") as f:
        f.write(src)
    return src.count("\n")


def best_of(cmds, n=3):
    best = None
    for _ in range(n):
        t = time.perf_counter()
        for c in cmds:
            p = subprocess.run(c, capture_output=True, text=True)
            if p.returncode != 0:
                lines = (p.stderr or p.stdout).strip().splitlines()
                return None, next((l for l in lines if "error" in l), lines[0] if lines else "failed")
        dt = (time.perf_counter() - t) * 1000
        best = dt if best is None else min(best, dt)
    return best, None


def main():
    args = sys.argv[1:]
    md_path = None
    if "--markdown" in args:
        i = args.index("--markdown")
        md_path = args[i + 1]
        del args[i:i + 2]
    quick = "--quick" in args
    args = [a for a in args if a != "--quick"]
    sizes = [int(a) for a in args] or ([40, 200] if quick else [40, 400, 2000])
    if not os.path.exists(rc.CBSO):
        raise SystemExit("backend not built; run backend/build.sh first")
    os.makedirs(OUT, exist_ok=True)
    runtime_obj, _ = rc.build_runtime()
    table = []
    for nfn in sizes:
        name = f"scale{nfn}"
        src = os.path.join(OUT, f"{name}.rs")
        lines = generate(nfn, src)
        p = lambda t: os.path.join(OUT, f"{name}.{t}")
        # An object needs its own panic stubs: an unoptimised or Cranelift build
        # keeps overflow-check paths the optimised probe never sees.
        objs = {"llvm-O3": ["-C", "opt-level=3"], "llvm-O0": ["-C", "opt-level=0"],
                "cranelift": ["-Zcodegen-backend=cranelift"]}
        stubs = {}
        for k, flags in objs.items():
            subprocess.run(R + ["--emit=obj"] + flags + [src, "-o", p(k + ".o")], check=True,
                           capture_output=True)
            stubs[k] = rc.object_stubs(p(k + ".o"))
        pipes = {
            "rmc": [[rc.RMC, src, "-o", p("rmc")]],
            "cb": [rc.CB + [src, "-o", p("cb")]],
        }
        for k, flags in objs.items():
            pipes[k] = [R + ["--emit=obj"] + flags + [src, "-o", p(k + ".o")],
                        ["ld", "-s", "-o", p(k), "-e", "_start", p(k + ".o"), runtime_obj, stubs[k]]]
        pipes["front-end"] = [R + ["--emit=metadata", "--crate-type=lib", src, "-o", p("rmeta")]]
        row = {"lines": lines}
        ref = None
        print(f"== {nfn} functions, {lines} lines")
        for k in ["llvm-O3", "llvm-O0", "cranelift", "cb", "rmc", "front-end"]:
            ms, err = best_of(pipes[k])
            note = ""
            if err:
                note = f"FAILED: {err[:120]}"
            elif k != "front-end":
                out = subprocess.run([p(k), "7"], capture_output=True)
                got = (out.stdout, out.returncode)
                ref = ref or got
                note = "" if got == ref else "WRONG OUTPUT"
            row[k] = (ms, note)
            print(f"  {k:<10} " + (f"{ms:9.1f} ms" if ms is not None else "        -") + f"  {note}")
        table.append(row)
    if md_path:
        keys = ["rmc", "cb", "cranelift", "llvm-O0", "llvm-O3", "front-end"]
        out = [
            "# Compile-time scaling",
            "",
            f"Recorded {datetime.datetime.now(datetime.timezone.utc):%Y-%m-%d %H:%M} UTC; "
            f"CPU: {open('/proc/cpuinfo').read().split('model name')[1].split(':')[1].splitlines()[0].strip()}.",
            "Regenerate with `python3 bench/scale.py --markdown bench/RESULTS-SCALE.md`.",
            "",
            "Wall time of one build (best of three), in milliseconds. `front-end` is",
            "`rustc --emit=metadata`, the type-checking floor every rustc pipeline pays.",
            "",
            "| lines | " + " | ".join(keys) + " |",
            "| ---: | " + " | ".join("---:" for _ in keys) + " |",
        ]
        for row in table:
            cells = []
            for k in keys:
                ms, note = row[k]
                cells.append("-" if ms is None else f"{ms:.0f}" + (f" ({note})" if note else ""))
            out.append(f"| {row['lines']} | " + " | ".join(cells) + " |")
        with open(md_path, "w") as f:
            f.write("\n".join(out) + "\n")
        print(f"wrote {md_path}")


if __name__ == "__main__":
    sys.exit(main())
