#!/bin/bash
# Held-out benchmark harness: runtime inputs only.
#
# These programs are deliberately not used for tuning. Every trip count and
# seed is read from argv, so compile-time folding cannot hide a codegen gap.
# The harness still reports the full all-axes matrix:
#   compile: wall, cpu (user+sys), peak RSS, artifact size
#   run:     wall, cpu (user+sys), peak RSS
#
# Usage: heldout.sh [program-glob]
#   INPUTS="10000:7 50000:11" heldout.sh
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$BENCH")
PROGS=${1:-"$BENCH/heldout/h*.rs"}
OUT="$BENCH/out/heldout"; mkdir -p "$OUT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
N=nightly-2026-11-17
RUSTC_LLVM="rustc +$N"
CG_UP="$ROOT/cg_clif_upstream/dist/bin/rustc-clif"
CG_MOD="$ROOT/rakit/dist/bin/rustc-rakit"
CREPS=${CREPS:-3}
RREPS=${RREPS:-5}
INPUTS=${INPUTS:-"10000:7 50000:11 200000:13"}
MEASURE=(python3 "$BENCH/measure.py" --json)
jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %d %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb'],d['exit']))"; }
mb() { python3 -c "print('%.1f'%($1/1024.0))"; }

declare -A CMD=( [L]="$RUSTC_LLVM" [U]="$CG_UP" [M]="$CG_MOD" )
fail=0

printf '%-9s %-2s %6s | %6s %6s %6s %9s | %6s %6s %6s | %s\n' \
  prog bk input Cwall Ccpu Crss size Rwall Rcpu Rrss output
printf '%s\n' "---------------------------------------------------------------------------------------------"

for src in $PROGS; do
  b=$(basename "$src" .rs)
  for bk in L U M; do
    bin="$OUT/${b}_$bk"
    cres=$("${MEASURE[@]}" --reps "$CREPS" --warmup 1 -- ${CMD[$bk]} -C opt-level=3 "$src" -o "$bin" --edition 2021 | jf)
    read -r cw cc cr ce <<<"$cres"
    if [ "$ce" != "0" ] || [ ! -f "$bin" ]; then
      printf '%-9s %-2s %6s | COMPILE_FAIL exit=%s\n' "$b" "$bk" "-" "$ce"
      eval "ok_$bk=0"
      fail=1
      continue
    fi
    eval "ok_$bk=1"
    eval "cw_$bk=$cw cc_$bk=$cc cr_$bk=$cr sz_$bk=$(stat -c %s "$bin")"
  done

  for pair in $INPUTS; do
    n=${pair%%:*}
    seed=${pair#*:}
    ref="$TMP/${b}_L_${n}_${seed}.out"
    if [ "${ok_L:-0}" = 1 ]; then
      if ! "$OUT/${b}_L" "$n" "$seed" >"$ref" 2>&1; then
        printf '%-9s %-2s %6s | RUN_FAIL\n' "$b" L "$n"
        fail=1
        continue
      fi
    else
      continue
    fi
    for bk in U M; do
      eval "ok=\${ok_$bk:-0}"
      if [ "$ok" != 1 ]; then continue; fi
      out="$TMP/${b}_${bk}_${n}_${seed}.out"
      if ! "$OUT/${b}_$bk" "$n" "$seed" >"$out" 2>&1; then
        printf '%-9s %-2s %6s | RUN_FAIL\n' "$b" "$bk" "$n"
        fail=1
        continue
      fi
      if ! cmp -s "$ref" "$out"; then
        printf '%-9s %-2s %6s | OUTPUT_MISMATCH\n' "$b" "$bk" "$n"
        diff -u "$ref" "$out" >&2 || true
        fail=1
      fi
    done

    for bk in L U M; do
      eval "ok=\${ok_$bk:-0}"
      if [ "$ok" != 1 ]; then
        printf '%-9s %-2s %6s | COMPILE_FAIL\n' "$b" "$bk" "$n"
        continue
      fi
      rres=$("${MEASURE[@]}" --reps "$RREPS" --warmup 1 -- "$OUT/${b}_$bk" "$n" "$seed" | jf)
      read -r rw rc rr re <<<"$rres"
      cw=$(eval echo \$cw_$bk)
      cc=$(eval echo \$cc_$bk)
      cr=$(eval echo \$cr_$bk)
      sz=$(eval echo \$sz_$bk)
      if [ "$re" != 0 ]; then
        printf '%-9s %-2s %6s | %6s %6s %6s %9s | RUN_FAIL exit=%s\n' \
          "$b" "$bk" "$n" "$cw" "$cc" "$(mb "$cr")" "$sz" "$re"
        continue
      fi
      printf '%-9s %-2s %6s | %6sms %6sms %6sM %9s | %6sms %6sms %6sM | ok\n' \
        "$b" "$bk" "$n" "$cw" "$cc" "$(mb "$cr")" "$sz" "$rw" "$rc" "$(mb $rr)"
    done
    echo
  done
done

if [ "$fail" -eq 0 ]; then
  echo "[heldout] all outputs match LLVM"
fi
exit "$fail"
