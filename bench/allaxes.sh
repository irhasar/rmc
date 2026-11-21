#!/bin/bash
# All-axes 3-way comparison: LLVM vs cg_clif upstream vs cg_clif modified.
#
# Axes measured:
#   compile : wall, cpu (user+sys), peak RSS, artifact size
#   run     : wall, cpu (user+sys), peak RSS
#
# Usage: allaxes.sh [prog-glob]
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$BENCH")
PROGS=${1:-"$BENCH/progs/p*.rs"}
OUT="$BENCH/out/allaxes"; mkdir -p "$OUT"
N=nightly-2026-11-17
RUSTC_LLVM="rustc +$N"
CG_UP="$ROOT/cg_clif_upstream/dist/bin/rustc-clif"
CG_MOD="$ROOT/cg_clif_mod/dist/bin/rustc-clif"
CREPS=${CREPS:-3}
RREPS=${RREPS:-5}

MEASURE=(python3 "$BENCH/measure.py" --json)
jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %d %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb'],d['exit']))"; }
mb() { python3 -c "print('%.1f'%($1/1024.0))"; }

declare -A CMD=( [L]="$RUSTC_LLVM" [U]="$CG_UP" [M]="$CG_MOD" )

hdr="prog       bk | Cwall   Ccpu    Crss   size      | Rwall   Rcpu    Rrss"
printf '%s\n' "$hdr"
printf '%s\n' "---------------------------------------------------------------------------"

for src in $PROGS; do
  b=$(basename "$src" .rs)
  for bk in L U M; do
    bin="$OUT/${b}_$bk"
    cres=$("${MEASURE[@]}" --reps "$CREPS" --warmup 1 -- ${CMD[$bk]} -C opt-level=3 "$src" -o "$bin" --edition 2021 | jf)
    read -r cw cc cr ce <<<"$cres"
    if [ "$ce" != "0" ] || [ ! -f "$bin" ]; then
      printf '%-10s %-2s | COMPILE_FAIL exit=%s\n' "$b" "$bk" "$ce"; continue
    fi
    sz=$(stat -c %s "$bin")
    rres=$("${MEASURE[@]}" --reps "$RREPS" --warmup 1 -- "$bin" | jf)
    read -r rw rc rr re <<<"$rres"
    printf '%-10s %-2s | %6sms %6sms %6sM %9s | %6sms %6sms %6sM\n' \
      "$b" "$bk" "$cw" "$cc" "$(mb $cr)" "$sz" "$rw" "$rc" "$(mb $rr)"
  done
  echo
done
