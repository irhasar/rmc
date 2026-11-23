#!/bin/bash
set -u
EXT=$(cd "$(dirname "$0")" && pwd); ROOT=$(cd "$EXT/.." && pwd)
BENCH=$ROOT/bench
M=$ROOT/rakit/dist/bin/rustc-rakit
CRATES=$EXT/crateworks/crates; DRV=$EXT/drivers; OUT=$EXT/out_leafab; mkdir -p "$OUT"
export PATH="$HOME/.cargo/bin:$PATH"
cm(){ python3 "$BENCH/measure.py" --reps 5 --warmup 1 --json -- "$@" | python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f'%(d['wall_med']*1000))"; }
SPECS=( "openlocationcode|openlocationcode|openlocationcode|olc_drive|1500000"
        "holidays-rs|holidays-rs|holidays_rs|hol_drive|20000"
        "stdnum-rs|stdnum-rs|stdnum_rs|sn_drive|1000000" )
for spec in "${SPECS[@]}"; do
  IFS='|' read -r pkg dir cname drv args <<<"$spec"
  srcdir="$CRATES/$dir"; od="$OUT/$pkg"; rm -rf "$od"; mkdir -p "$od"
  echo "=== $pkg ==="
  for cfg in "leaf_off|RAKIT_INLINE_LEAF_MAX=0" "leaf_on|"; do
    lab=${cfg%%|*}; envs=${cfg#*|}
    cat > "$od/b.sh" <<EOF
set -e
env $envs $M --edition 2024 -C opt-level=3 --crate-type lib --crate-name $cname -C metadata=ext -C extra-filename=-ext "$srcdir/src/lib.rs" -o "$od/lib.rlib" 2>/dev/null
env $envs $M --edition 2024 -C opt-level=3 --extern $cname="$od/lib.rlib" "$DRV/$drv.rs" -o "$od/M" 2>/dev/null
EOF
    cw=$(cm bash "$od/b.sh")
    sz=$(( $(stat -c %s "$od/lib.rlib") + $(stat -c %s "$od/M") ))
    rw=$(cm "$od/M" $args)
    printf "  %-9s Cwall=%-7s size=%-9s Rwall=%s\n" "$lab" "$cw" "$sz" "$rw"
  done
done
echo "AB_DONE"
