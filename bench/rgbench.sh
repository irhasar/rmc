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

die() { echo "rgbench: $*" >&2; exit 1; }

# A checkout is built by running its build scripts, so an existing one is only
# trusted when it is the expected crate and no other user can have modified it.
if [ -d "$SRC" ]; then
  perm=$(stat -c '%A' "$SRC")
  case "${perm:5:1}${perm:8:1}" in
    *w*) die "$SRC is writable by group/other; pass a private SRC or remove it" ;;
  esac
  [ -f "$SRC/Cargo.toml" ] || die "$SRC has no Cargo.toml; pass a clean SRC"
  grep -qx 'name = "ripgrep"' "$SRC/Cargo.toml" || die "$SRC is not ripgrep"
  grep -qx 'version = "14.1.1"' "$SRC/Cargo.toml" || die "$SRC is not ripgrep 14.1.1"
else
  mkdir -p "$(dirname "$SRC")" || exit 1
  curl -sL https://static.crates.io/crates/ripgrep/ripgrep-14.1.1.crate |
    tar xz -C "$(dirname "$SRC")" || exit 1
fi

[ -d "$CORPUS" ] || die "corpus $CORPUS is missing (see the header for how to build it)"
find "$CORPUS" -name '*.txt' -print -quit | grep -q . ||
  die "corpus $CORPUS has no *.txt files"

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

python3 - "$SRC" "$CORPUS" "$BENCH" <<'PY'
import statistics, sys, tempfile, shutil

src, corpus, bench = sys.argv[1], sys.argv[2], sys.argv[3]
sys.path.insert(0, bench)
from measure import run_once

searches = {
    "literal(-F needle)": ["-c", "-F", "needle"],
    "regex(captures)": ["-n", "--no-heading", "-e", r"([a-z]+)_?([0-9]+)@example\.com"],
    "regex(-i alternation)": ["-c", "-i", "-e", "todo|fixme|lorem"],
    "multi-pattern(-F x4)": ["-c", "-F", "-e", "TODO", "-e", "FIXME",
                             "-e", "fn needle", "-e", "lorem ipsum"],
    "walk(--files)": ["--files"],
}
bins = {k: f"{src}/target-{k}/release/rg" for k in "LUM"}
for bk, b in bins.items():
    try:
        open(b, "rb").close()
    except OSError:
        sys.exit(f"rgbench: {bk} binary missing ({b}); the build failed")

REPS = 5
tmp = tempfile.mkdtemp(prefix="rgbench-")


def ok(rc):
    """ripgrep exits 1 when nothing matched; anything above that is an error."""
    return rc in (0, 1)


print(f"{'search':<24}{'L wall':>9}{'U wall':>9}{'M wall':>9}{'M/L':>7}"
      f"{'M cpu':>9}{'M rss':>9}  out")
tot = dict.fromkeys("LUM", 0.0)
fail = 0
for name, args in searches.items():
    med, cpu, rss, outs, bad = {}, {}, {}, {}, []
    for bk in "LUM":
        out = f"{tmp}/{bk}.txt"
        samples, cpus, rsss = [], [], []
        for i in range(REPS + 1):
            r = run_once([bins[bk], "--sort", "path"] + args + [corpus], out_path=out)
            if not ok(r["exit"]):
                bad.append(f"{bk} exit {r['exit']}")
                break
            if i == 0:
                continue  # warmup: page cache, first touch
            samples.append(r["wall"] * 1000)
            cpus.append((r["user"] + r["sys"]) * 1000)
            rsss.append(r["maxrss_kb"])
        if bk in bad or not samples:
            bad.append(f"{bk} no samples")
            continue
        med[bk] = statistics.median(samples)
        cpu[bk] = statistics.median(cpus)
        rss[bk] = statistics.median(rsss)
        tot[bk] += med[bk]
        with open(out, "rb") as f:
            outs[bk] = f.read()

    if bad or len(outs) != 3:
        print(f"{name:<24}{'INVALID':>9}  {'; '.join(bad) or 'incomplete'}")
        fail = 1
        continue
    same = outs["L"] == outs["U"] == outs["M"]
    if not same:
        fail = 1
    print(f"{name:<24}{med['L']:9.1f}{med['U']:9.1f}{med['M']:9.1f}"
          f"{med['M'] / med['L']:7.2f}{cpu['M']:9.1f}{rss['M'] / 1024:8.0f}M"
          f"  {'ok' if same else 'MISMATCH'}")

if all(tot.values()):
    print(f"{'TOTAL':<24}{tot['L']:9.1f}{tot['U']:9.1f}{tot['M']:9.1f}"
          f"{tot['M'] / tot['L']:7.2f}")
else:
    print(f"{'TOTAL':<24}  (incomplete: an invalid row has no timing)")
shutil.rmtree(tmp, ignore_errors=True)
sys.exit(1 if fail else 0)
PY
