#!/bin/bash
# Build + run 3 real crates from github.com/cipansor/crateworks with three
# backends: L = rustc+LLVM, U = cg_clif upstream, M = Rakit modified.
#
# Each crate is built as a lib, then a small driver is linked against it and run.
# Measures compile (wall/cpu/peakRSS), artifact size, and run (wall/cpu/RSS).
set -u
EXT=$(cd "$(dirname "$0")" && pwd)
BENCH=${BENCH_DIR:-$EXT/../bench}
N=nightly-2026-11-17
RUSTC_L="rustc +$N"
RUSTC_U=${RUSTC_U:-$EXT/../cg_clif_upstream/dist/bin/rustc-clif}
RUSTC_M=${RUSTC_M:-$EXT/../rakit_mod/dist/bin/rustc-rakit}
CRATES=${CRATEWORKS:-$EXT/crateworks}/crates
DRV=$EXT/drivers
OUT=$EXT/out; mkdir -p "$OUT"
CREPS=${CREPS:-3}; RREPS=${RREPS:-5}
MEASURE=(python3 "$BENCH/measure.py" --json)
jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %d %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb'],d['exit']))"; }
mb() { python3 -c "print('%.1f'%($1/1024.0))"; }

# crate spec: name | srcdir | crate-name(underscore) | driver | args
SPECS=(
  "openlocationcode|openlocationcode|openlocationcode|olc_drive|1500000"
  "holidays-rs|holidays-rs|holidays_rs|hol_drive|20000"
  "stdnum-rs|stdnum-rs|stdnum_rs|sn_drive|1000000"
)

declare -A RUSTC=( [L]="$RUSTC_L" [U]="$RUSTC_U" [M]="$RUSTC_M" )

echo "backend: L=rustc+LLVM  U=cg_clif upstream  M=Rakit modified"
printf '%-18s %-2s | %6s %6s %7s %10s | %7s %7s %7s | %s\n' \
  crate bk Cwall Ccpu Crss "size(rlib+bin)" Rwall Rcpu Rrss out
printf '%s\n' "-----------------------------------------------------------------------------------------------------"

for spec in "${SPECS[@]}"; do
  IFS='|' read -r pkg dir cname drv args <<<"$spec"
  srcdir="$CRATES/$dir"
  outs=""
  for bk in L U M; do
    od="$OUT/$pkg/$bk"; rm -rf "$od"; mkdir -p "$od"
    rlib="$od/lib$cname.rlib"
    bin="$od/drv"
    # ---- compile (lib + driver) as one measured command ----
    cat > "$od/build.sh" <<EOF
set -e
${RUSTC[$bk]} --edition 2024 -C opt-level=3 --crate-type lib --crate-name $cname \
  -C metadata=ext -C extra-filename=-ext "$srcdir/src/lib.rs" -o "$rlib" 2>/dev/null
${RUSTC[$bk]} --edition 2024 -C opt-level=3 --extern $cname="$rlib" \
  "$DRV/$drv.rs" -o "$bin" 2>/dev/null
EOF
    chmod +x "$od/build.sh"
    cres=$("${MEASURE[@]}" --reps "$CREPS" --warmup 1 -- bash "$od/build.sh" | jf)
    read -r cw cc cr ce <<<"$cres"
    if [ "$ce" != "0" ] || [ ! -f "$bin" ]; then
      printf '%-18s %-2s | COMPILE_FAIL exit=%s\n' "$pkg" "$bk" "$ce"; continue
    fi
    sz=$(stat -c %s "$rlib"); bsz=$(stat -c %s "$bin"); tot=$((sz+bsz))
    rres=$("${MEASURE[@]}" --reps "$RREPS" --warmup 1 -- "$bin" "$args" | jf)
    read -r rw rc rr re <<<"$rres"
    o=$("$bin" "$args" 2>/dev/null)
    outs="$outs $bk=$o"
    printf '%-18s %-2s | %6sms %6sms %6sM %10s | %7sms %7sms %7sM | %s\n' \
      "$pkg" "$bk" "$cw" "$cc" "$(mb $cr)" "$tot" "$rw" "$rc" "$(mb $rr)" "$o"
  done
  echo "  outputs:$outs"
  echo
done
