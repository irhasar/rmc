#!/bin/bash
# Build and measure ripgrep with all three backends.
#
#   ./rgbench.sh [src-dir] [corpus-dir]
#
# Builds ripgrep 14.1.1 from `src-dir` (downloaded there on first run) once per
# backend, then times five search shapes over `corpus-dir`. Reports compile
# wall/cpu/peak-RSS and artifact size per backend, run wall per search, and
# requires the three backends to produce byte-identical stdout.
#
# corpus: 2000 files, ~40 MB, with the shapes below embedded:
#   find d0..d99 -name '*.txt' | xargs -I{} sh -c \
#     'printf "needle %s\nTODO fixme lorem\nname_%s@example.com\n" $(basename {}) $(basename {}) > {}'
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$BENCH")
SRC=${1:-/tmp/rg/ripgrep-14.1.1}
CORPUS=${2:-/tmp/rgcorpus}
N=nightly-2026-11-17
export RUSTUP_HOME=${RUSTUP_HOME:-/workspace/.rustup}
export CARGO_HOME=${CARGO_HOME:-/workspace/.cargo}
export PATH="$CARGO_HOME/bin:$PATH"

if [ ! -d "$SRC" ]; then
  mkdir -p "$(dirname "$SRC")"
  curl -sL https://static.crates.io/crates/ripgrep/ripgrep-14.1.1.crate |
    tar xz -C "$(dirname "$SRC")" || exit 1
fi

# `cargo` takes RUSTC as a bare executable, so the LLVM column needs the
# toolchain's own `rustc` path rather than a `rustc +toolchain` command line.
RUSTC_LLVM=$(RUSTUP_HOME="$RUSTUP_HOME" "$CARGO_HOME/bin/rustup" which --toolchain "$N" rustc)
declare -A RC=(
  [L]="$RUSTC_LLVM"
  [U]="$ROOT/cg_clif_upstream/dist/bin/rustc-clif"
  [M]="$ROOT/rakit/dist/bin/rustc-rakit"
)
MEASURE=(python3 "$BENCH/measure.py" --json --reps 1 --warmup 0 --)

printf '%-3s %-8s %-8s %-8s %-10s\n' bk Cwall Ccpu Crss size
for bk in L U M; do
  ( cd "$SRC" && rm -rf "target-$bk" )
  env CARGO_TARGET_DIR="$SRC/target-$bk" RUSTC="${RC[$bk]}" \
    "${MEASURE[@]}" cargo build --release -p ripgrep \
    --manifest-path "$SRC/Cargo.toml" 2>"$SRC/build_$bk.log" |
    python3 -c "
import sys, json
d = json.loads(sys.stdin.read().strip().splitlines()[-1])
print('%s %d %d %d %d' % ('$bk', d['wall_med'] * 1000, d['cpu_med'] * 1000,
                          d['maxrss_med_kb'], d['exit']))" ||
    { echo "$bk build FAILED"; tail -3 "$SRC/build_$bk.log"; continue; }
  printf '    size=%s\n' "$(stat -c%s "$SRC/target-$bk/release/rg")"
done

python3 - "$SRC" "$CORPUS" <<'PY'
import statistics, subprocess, sys, time

src, corpus = sys.argv[1], sys.argv[2]
searches = {
    "literal(-F needle)": ["-c", "-F", "needle"],
    "regex(captures)": ["-n", "--no-heading", "-e", r"([a-z]+)_?([0-9]+)@example\.com"],
    "regex(-i alternation)": ["-c", "-i", "-e", "todo|fixme|lorem"],
    "multi-pattern(-F x4)": ["-c", "-F", "-e", "TODO", "-e", "FIXME",
                             "-e", "fn needle", "-e", "lorem ipsum"],
    "walk(--files)": ["--files"],
}
bins = {k: f"{src}/target-{k}/release/rg" for k in "LUM"}
print(f"{'search':<24}{'L ms':>9}{'U ms':>9}{'M ms':>9}{'M/L':>7}{'M/U':>7}  out")
tot = dict.fromkeys("LUM", 0.0)
for name, args in searches.items():
    med, outs = {}, {}
    for bk in "LUM":
        out = f"/tmp/rgbench_{bk}.txt"
        ts = []
        for _ in range(6):
            with open(out, "wb") as f:
                t = time.perf_counter()
                subprocess.run([bins[bk], "--sort", "path"] + args + [corpus],
                               stdout=f, stderr=subprocess.DEVNULL)
                ts.append((time.perf_counter() - t) * 1000)
        med[bk] = statistics.median(ts[1:])
        tot[bk] += med[bk]
        outs[bk] = open(out, "rb").read()
    same = outs["L"] == outs["U"] == outs["M"]
    print(f"{name:<24}{med['L']:9.1f}{med['U']:9.1f}{med['M']:9.1f}"
          f"{med['M'] / med['L']:7.2f}{med['M'] / med['U']:7.2f}"
          f"  {'ok' if same else 'MISMATCH'}")
print(f"{'TOTAL':<24}{tot['L']:9.1f}{tot['U']:9.1f}{tot['M']:9.1f}"
      f"{tot['M'] / tot['L']:7.2f}{tot['M'] / tot['U']:7.2f}")
PY
