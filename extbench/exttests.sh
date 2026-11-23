#!/bin/bash
# Compile + run the crates' OWN test suites (integration tests, --test) with
# three backends and report pass/fail. Tests use include_str! fixtures, so no
# cargo is needed; we compile the lib then each test crate against it.
set -u
EXT=$(cd "$(dirname "$0")" && pwd)
N=nightly-2026-11-17
declare -A RUSTC=( [L]="rustc +$N" [U]=${RUSTC_U:-$EXT/../cg_clif_upstream/dist/bin/rustc-clif} [M]=${RUSTC_M:-$EXT/../rakit_mod/dist/bin/rustc-rakit} )
CRATES=${CRATEWORKS:-$EXT/crateworks}/crates
OUT=$EXT/out/tests; mkdir -p "$OUT"

# pkg|dir|crate_name|tests(space sep)
SPECS=(
 "openlocationcode|openlocationcode|openlocationcode|reference readme fuzz"
 "holidays-rs|holidays-rs|holidays_rs|closest readme holidays_oracle"
 "stdnum-rs|stdnum-rs|stdnum_rs|readme"
)
for spec in "${SPECS[@]}"; do
  IFS='|' read -r pkg dir cname tests <<<"$spec"
  srcdir="$CRATES/$dir"
  for bk in L U M; do
    od="$OUT/$pkg/$bk"; rm -rf "$od"; mkdir -p "$od"
    rlib="$od/lib$cname.rlib"
    ${RUSTC[$bk]} --edition 2024 -C opt-level=3 --crate-type lib --crate-name $cname "$srcdir/src/lib.rs" -o "$rlib" 2>/dev/null
    if [ ! -f "$rlib" ]; then echo "$pkg $bk LIBFAIL"; continue; fi
    for t in $tests; do
      tb="$od/t_$t"
      ${RUSTC[$bk]} --edition 2024 -C opt-level=3 --test --extern $cname="$rlib" "$srcdir/tests/$t.rs" -o "$tb" 2>"$od/$t.err"
      if [ ! -f "$tb" ]; then echo "  $pkg/$t $bk COMPILE_FAIL: $(head -1 $od/$t.err)"; continue; fi
      line=$("$tb" 2>&1 | grep -E "^test result:" | head -1)
      echo "  $pkg/$t $bk -> ${line:-NO_RESULT}"
    done
  done
done
