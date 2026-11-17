#!/bin/bash
# Benchmark harness: compile-time + runtime metrics for a given rustc frontend.
# Usage: bench_all.sh <label> <rustc-command...>
set -u
LABEL="$1"; shift
RUSTC=("$@")
BENCH=$(cd "$(dirname "$0")" && pwd)
PROGS="$BENCH/progs"
OUT="$BENCH/out/$LABEL"
mkdir -p "$OUT"
REPS=${REPS:-5}
MEASURE=(python3 "$BENCH/measure.py" --reps "$REPS" --warmup 1 --json)
FMT='
import sys,json
d=json.load(sys.stdin)
print("wall=%.0fms cpu=%.0fms rss=%.0fMB exit=%d" % (
    d["wall_med"]*1000, d["cpu_med"]*1000, d["maxrss_med_kb"]/1024, d["exit"]))
'

echo "### LABEL=$LABEL  RUSTC=${RUSTC[*]}" >&2
for src in "$PROGS"/*.rs; do
  base=$(basename "$src" .rs)
  # ---- compile ----
  CMD=("${RUSTC[@]}" -C opt-level=3 "$src" -o "$OUT/$base" --edition 2021)
  CRES=$("${MEASURE[@]}" --label "compile:$base" -- "${CMD[@]}")
  echo "COMPILE $base $(echo "$CRES" | python3 -c "$FMT")"
  # ---- size ----
  if [ -f "$OUT/$base" ]; then
    SZ=$(stat -c %s "$OUT/$base")
    echo "SIZE $base $SZ"
  else
    echo "SIZE $base NA"; continue
  fi
  # ---- run ----
  RRES=$("${MEASURE[@]}" --reps 3 --label "run:$base" -- "$OUT/$base")
  echo "RUN $base $(echo "$RRES" | python3 -c "$FMT")"
done
