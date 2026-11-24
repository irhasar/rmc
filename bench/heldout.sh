#!/bin/bash
# Held-out benchmark harness: runtime inputs only.
#
# The programs are `bench/progs/h_*.rs`, deliberately not used for tuning.
# Every trip count and seed is read from argv, so compile-time folding cannot
# hide a codegen gap; without argv they fall back to a small default input so
# `bench/allaxes.sh` can discover and run them too. The harness reports the
# full all-axes matrix:
#   compile: wall, cpu (user+sys), peak RSS, artifact size
#   run:     wall, cpu (user+sys), peak RSS
#
# Usage: heldout.sh [program-glob]
#   INPUTS="10000:7 50000:11" heldout.sh
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$BENCH")
PROGS=${1:-"$BENCH/progs/h*.rs"}
OUT="$BENCH/out/heldout"; mkdir -p "$OUT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
N=nightly-2026-11-17
TARGET=$(rustc +$N -vV | sed -n 's/^host: //p')
RUSTC_LLVM="rustc +$N"
CG_UP="$ROOT/cg_clif_upstream/dist/bin/rustc-clif"
CG_MOD="env -u RAKIT_HOST_LLVM $ROOT/rakit/dist/bin/rustc-rakit"
CREPS=${CREPS:-3}
RREPS=${RREPS:-5}
INPUTS=${INPUTS:-"10000:7 50000:11 200000:13"}
MEASURE=(python3 "$BENCH/measure.py" --json)
jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %d %d %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb'],d['exit'],1 if d.get('valid',d['exit']==0) else 0))"; }
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
    cres=$("${MEASURE[@]}" --reps "$CREPS" --warmup 1 -- ${CMD[$bk]} --target "$TARGET" -C opt-level=3 "$src" -o "$bin" --edition 2021 | jf)
    read -r cw cc cr ce cv <<<"$cres"
    if [ "$ce" != "0" ] || [ ! -f "$bin" ]; then
      printf '%-9s %-2s %6s | %6sms %6sms %6sM %9s | COMPILE_FAIL exit=%s\n' \
        "$b" "$bk" "-" "$cw" "$cc" "$(mb "$cr")" "-" "$ce"
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
    valid_L=0
    if [ "${ok_L:-0}" = 1 ]; then
      if "$BENCH/validate.sh" --capture "$ref" "$OUT/${b}_L" "$n" "$seed"; then
        valid_L=1
      else
        printf '%-9s %-2s %6s | RUN_FAIL\n' "$b" L "$n"
        fail=1
      fi
    fi
    for bk in U M; do
      eval "valid_$bk=0"
      eval "ok=\${ok_$bk:-0}"
      if [ "$ok" != 1 ] || [ "$valid_L" != 1 ]; then continue; fi
      "$BENCH/validate.sh" --compare "$ref" "$OUT/${b}_$bk" "$n" "$seed"
      vrc=$?
      if [ "$vrc" = 2 ]; then
        printf '%-9s %-2s %6s | RUN_FAIL\n' "$b" "$bk" "$n"
        fail=1
        continue
      elif [ "$vrc" != 0 ]; then
        printf '%-9s %-2s %6s | OUTPUT_MISMATCH\n' "$b" "$bk" "$n"
        fail=1
        continue
      fi
      eval "valid_$bk=1"
    done

    for bk in L U M; do
      eval "ok=\${ok_$bk:-0}"
      eval "valid=\${valid_$bk:-0}"
      if [ "$ok" != 1 ]; then
        printf '%-9s %-2s %6s | COMPILE_FAIL\n' "$b" "$bk" "$n"
        continue
      fi
      if [ "$valid" != 1 ]; then
        printf '%-9s %-2s %6s | INVALID (no valid output)\n' "$b" "$bk" "$n"
        continue
      fi
      rres=$("${MEASURE[@]}" --reps "$RREPS" --warmup 1 --expect "$ref" -- "$OUT/${b}_$bk" "$n" "$seed" | jf)
      read -r rw rc rr re rv <<<"$rres"
      cw=$(eval echo \$cw_$bk)
      cc=$(eval echo \$cc_$bk)
      cr=$(eval echo \$cr_$bk)
      sz=$(eval echo \$sz_$bk)
      if [ "$rv" != 1 ]; then
        fail=1
        if [ "$re" != 0 ]; then
          printf '%-9s %-2s %6s | %6s %6s %6s %9s | RUN_FAIL exit=%s\n' \
            "$b" "$bk" "$n" "$cw" "$cc" "$(mb "$cr")" "$sz" "$re"
        else
          printf '%-9s %-2s %6s | %6s %6s %6s %9s | OUTPUT_MISMATCH\n' \
            "$b" "$bk" "$n" "$cw" "$cc" "$(mb "$cr")" "$sz"
        fi
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
