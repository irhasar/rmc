#!/bin/bash
# Compile and run the Rakit regression fixtures with both backends, then check
# their output. Invoked by allaxes.sh unless SKIP_REGRESSIONS=1.
set -u
BENCH=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$BENCH")
N=nightly-2026-11-17
TARGET=$(rustc +$N -vV | sed -n 's/^host: //p')
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fail=0

check() {
    name=$1
    expected=$2
    src="$BENCH/tests/$name.rs"
    for bk in L M; do
        if [ "$bk" = L ]; then
            cmd=(rustc +$N)
        else
            cmd=(env -u RAKIT_HOST_LLVM "$ROOT/rakit/dist/bin/rustc-rakit")
        fi
        bin="$TMP/${name}_$bk"
        if ! "${cmd[@]}" --target "$TARGET" -C opt-level=3 "$src" -o "$bin" --edition 2021 >"$TMP/${name}_$bk.compile" 2>&1; then
            echo "[regression] $name $bk: compile failed" >&2
            cat "$TMP/${name}_$bk.compile" >&2
            fail=1
            continue
        fi
        if ! "$bin" >"$TMP/${name}_$bk.out" 2>&1; then
            echo "[regression] $name $bk: run failed" >&2
            cat "$TMP/${name}_$bk.out" >&2
            fail=1
            continue
        fi
        actual=$(cat "$TMP/${name}_$bk.out")
        if [ "$actual" != "$expected" ]; then
            echo "[regression] $name $bk: unexpected output" >&2
            printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
            fail=1
        fi
    done
    if [ -f "$TMP/${name}_L" ] && [ -f "$TMP/${name}_M" ] &&
       cmp -s "$TMP/${name}_L" "$TMP/${name}_M"; then
        echo "[regression] $name: Rakit binary is identical to LLVM (backend fallback?)" >&2
        fail=1
    fi
    if [ -f "$TMP/${name}_L.out" ] && [ -f "$TMP/${name}_M.out" ] &&
       ! diff -q "$TMP/${name}_L.out" "$TMP/${name}_M.out" >/dev/null; then
        echo "[regression] $name: LLVM and Rakit differ" >&2
        diff -u "$TMP/${name}_L.out" "$TMP/${name}_M.out" >&2
        fail=1
    fi
}

check t_loop_call_args $'1\n3\n10\n32\n99\n301\n908\n2730\nacc=8197'
check t_loop_side_effect '2016 64'

if [ "$fail" -eq 0 ]; then
    echo "[regression] all fixtures passed"
fi
exit "$fail"
