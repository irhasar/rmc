#!/usr/bin/env bash
# Report where Rakit's compile time goes, per first-party pass.
#
# The passes are the whole difference between Rakit's compile time and
# upstream cg_clif's, so a change that claims a compile-time win has to say
# which pass it made cheaper. `RAKIT_PASS_TIME=1` prints one line per CGU with
# the wall time of each pass; this script compiles each program, sums those
# lines across CGUs, and prints the pass cost next to the compile wall time.
#
#   ./passtime.sh              # every progs/*.rs, 3 compile reps, best of
#   ./passtime.sh p2_mem h_mem # named programs only
#   REPS=1 ./passtime.sh       # one rep (noisier, faster)
#
# The pass times come from the fastest compile: the slowest ones are dominated
# by the first-run page-cache and std-loading cost and would hide a real
# difference between two builds.
set -uo pipefail
cd "$(dirname "$0")"

RUSTC=${RUSTC:-$PWD/../rakit/dist/bin/rustc-rakit}
TARGET=${TARGET:-x86_64-unknown-linux-gnu}
REPS=${REPS:-3}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ ! -x "$RUSTC" ]; then
    echo "no rustc at $RUSTC (run rakit/y.sh build)" >&2
    exit 1
fi

progs=()
if [ $# -gt 0 ]; then
    for p in "$@"; do
        [ -f "progs/$p.rs" ] || { echo "no progs/$p.rs" >&2; exit 1; }
        progs+=("progs/$p.rs")
    done
else
    for f in progs/*.rs; do progs+=("$f"); done
fi

printf '%-12s %12s %10s %6s  %s\n' program "compile wall" "pass total" share "per pass (ms)"
for f in "${progs[@]}"; do
    name=$(basename "$f" .rs)
    best_wall=''
    failed=0
    for _ in $(seq "$REPS"); do
        start=$(date +%s%N)
        env -u RAKIT_HOST_LLVM RAKIT_PASS_TIME=1 "$RUSTC" --target "$TARGET" \
            -C opt-level=3 "$f" -o "$TMP/$name" --edition 2021 \
            > "$TMP/$name.log" 2>&1
        rc=$?
        end=$(date +%s%N)
        if [ $rc -ne 0 ]; then
            echo "$name: compile failed (rc=$rc)" >&2
            tail -3 "$TMP/$name.log" >&2
            failed=1
            break
        fi
        wall=$(( (end - start) / 1000000 ))
        if [ -z "$best_wall" ] || [ "$wall" -lt "$best_wall" ]; then
            best_wall=$wall
            cp "$TMP/$name.log" "$TMP/$name.best"
        fi
    done
    [ $failed -eq 0 ] || continue
    read -r pass_total detail < <(python3 - "$TMP/$name.best" <<'PY'
import re, sys
totals = {}
for line in open(sys.argv[1]):
    m = re.match(r"\[passtime [^\]]+\] (.*)", line)
    if not m:
        continue
    for name, value in re.findall(r"(\w+)=([0-9.]+)ms", m.group(1)):
        totals[name] = totals.get(name, 0.0) + float(value)
total = sum(totals.values())
ordered = sorted(totals.items(), key=lambda kv: -kv[1])
print(f"{total:.1f} " + " ".join(f"{k}={v:.1f}" for k, v in ordered if v >= 0.05))
PY
)
    share=$(python3 -c "print(f'{100*float('$pass_total')/max($best_wall,1):.0f}%')")
    printf '%-12s %9s ms %7s ms %6s  %s\n' "$name" "$best_wall" "$pass_total" "$share" "$detail"
done
