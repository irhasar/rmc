#!/bin/bash
# All-axes 3-way comparison: LLVM vs cg_clif upstream vs Rakit modified.
#
# Axes measured:
#   compile : wall, cpu (user+sys), peak RSS, artifact size
#   run     : wall, cpu (user+sys), peak RSS
#
# A backend is measured only after its output matches LLVM's and its run exits
# zero. Validation uses the same arguments as the timed runs.
#
# Usage: allaxes.sh [prog-glob]
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$BENCH")
PROGS=${1:-"$BENCH/progs/*.rs"}
OUT="$BENCH/out/allaxes"; mkdir -p "$OUT"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
N=nightly-2026-11-17
TARGET=$(rustc +$N -vV | sed -n 's/^host: //p')
RUSTC_LLVM="rustc +$N"
CG_UP="$ROOT/cg_clif_upstream/dist/bin/rustc-clif"
CG_MOD="env -u RAKIT_HOST_LLVM $ROOT/rakit/dist/bin/rustc-rakit"
CREPS=${CREPS:-3}
RREPS=${RREPS:-5}

MEASURE=(python3 "$BENCH/measure.py" --json)
jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %d %d %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb'],d['exit'],1 if d.get('valid',d['exit']==0) else 0))"; }
mb() { python3 -c "print('%.1f'%($1/1024.0))"; }

declare -A CMD=( [L]="$RUSTC_LLVM" [U]="$CG_UP" [M]="$CG_MOD" )

if [ "${SKIP_REGRESSIONS:-0}" != "1" ]; then
  "$BENCH/regressions.sh" || exit 1
fi

hdr="prog       bk | Cwall   Ccpu    Crss   size      | Rwall   Rcpu    Rrss | output"
printf '%s\n' "$hdr"
printf '%s\n' "--------------------------------------------------------------------------------------"

fail=0

for src in $PROGS; do
  b=$(basename "$src" .rs)

  for bk in L U M; do
    bin="$OUT/${b}_$bk"
    cres=$("${MEASURE[@]}" --reps "$CREPS" --warmup 1 -- ${CMD[$bk]} --target "$TARGET" -C opt-level=3 "$src" -o "$bin" --edition 2021 | jf)
    read -r cw cc cr ce cv <<<"$cres"
    eval "cw_$bk=$cw cc_$bk=$cc cr_$bk=$cr ce_$bk=$ce"
    if [ "$ce" != "0" ] || [ ! -f "$bin" ]; then
      eval "ok_$bk=0 state_$bk=COMPILE_FAIL"
      fail=1
    else
      eval "ok_$bk=1 state_$bk=OK sz_$bk=$(stat -c %s "$bin")"
    fi
  done

  ref="$TMP/${b}_L.out"
  valid_L=0
  if [ "${ok_L:-0}" = 1 ]; then
    if "$BENCH/validate.sh" --capture "$ref" "$OUT/${b}_L"; then
      valid_L=1
    else
      eval "state_L=RUN_FAIL"
      fail=1
    fi
  fi

  for bk in U M; do
    valid=0
    eval "ok=\${ok_$bk:-0}"
    if [ "$ok" = 1 ] && [ "$valid_L" = 1 ]; then
      "$BENCH/validate.sh" --compare "$ref" "$OUT/${b}_$bk"
      vrc=$?
      if [ "$vrc" = 0 ]; then
        valid=1
      elif [ "$vrc" = 2 ]; then
        eval "state_$bk=RUN_FAIL"
        fail=1
      else
        eval "state_$bk=OUTPUT_MISMATCH"
        fail=1
      fi
    elif [ "${ok:-0}" = 1 ]; then
      eval "state_$bk=INVALID"
      fail=1
    fi
    eval "valid_$bk=$valid"
  done
  eval "valid_L=$valid_L"

  for bk in L U M; do
    eval "ok=\${ok_$bk:-0}"
    eval "state=\${state_$bk:-INVALID}"
    if [ "$ok" != 1 ]; then
      eval "ce=\${ce_$bk:-?}"
      eval "cw=\$cw_$bk cc=\$cc_$bk cr=\$cr_$bk"
      printf '%-10s %-2s | %6sms %6sms %6sM %9s | COMPILE_FAIL exit=%s\n' \
        "$b" "$bk" "$cw" "$cc" "$(mb "$cr")" "-" "$ce"
      continue
    fi
    eval "cw=\$cw_$bk cc=\$cc_$bk cr=\$cr_$bk sz=\$sz_$bk"
    eval "valid=\${valid_$bk:-0}"
    if [ "$valid" != 1 ]; then
      printf '%-10s %-2s | %6sms %6sms %6sM %9s | %s\n' \
        "$b" "$bk" "$cw" "$cc" "$(mb "$cr")" "$sz" "$state"
      continue
    fi
    rres=$("${MEASURE[@]}" --reps "$RREPS" --warmup 1 --expect "$ref" -- "$OUT/${b}_$bk" | jf)
    read -r rw rc rr re rv <<<"$rres"
    if [ "$rv" != 1 ]; then
      fail=1
      if [ "$re" != 0 ]; then
        printf '%-10s %-2s | %6sms %6sms %6sM %9s | RUN_FAIL exit=%s\n' \
          "$b" "$bk" "$cw" "$cc" "$(mb "$cr")" "$sz" "$re"
      else
        printf '%-10s %-2s | %6sms %6sms %6sM %9s | OUTPUT_MISMATCH\n' \
          "$b" "$bk" "$cw" "$cc" "$(mb "$cr")" "$sz"
      fi
      continue
    fi
    printf '%-10s %-2s | %6sms %6sms %6sM %9s | %6sms %6sms %6sM | ok\n' \
      "$b" "$bk" "$cw" "$cc" "$(mb "$cr")" "$sz" "$rw" "$rc" "$(mb $rr)"
  done
  echo
done

exit "$fail"
