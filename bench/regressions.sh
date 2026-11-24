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
check t_pmulhu '0 0 0 0 65534 32767 0 1'
check t_rec_loop_effects '55 true true'
check t_vectorize $'unit 12776226417588255163
strided2 12092896667980768107
zerotrip 12342691297999224719 unchanged=true
small 11722458728514271619
overlap 12968996556249723189
inplace 12776226417588255163
tail 6121779849389533154
u8 2907005038580616411'
check t_vectorize_unit 'unit 12776226417588255163'
check t_vectorize_guard 'guard 17715399688492003866 tail4=true'
check t_vectorize_dyn $'dyn 12776226417588255163 n=1000
zero 12342691297999224719 unchanged=true
small 17715399688492003866 tail4=true
rem 6121779849389533154
overlap 12968996556249723189'
check t_vectorize_multi 'm1 12776226417588255163 m2 15151514424346961891'
check t_vectorize_ops 'add 17194792273410237731 sub 10846146650447627395'
check t_vectorize_f32 $'n=1000 add=7083501190089957475 mul=16326997319991784275\nn=999 add=10593356097432088209 mul=10796855508119876121\nn=5 add=14995117245246052777 mul=3891769157468395945\nn=2 add=15738008312299960571 mul=18044256215840110843\nn=0 add=1469598103934665603 mul=1469598103934665603'
check t_vectorize_i64 $'n=1000 add=17194792273410237731\nn=999 add=7573983905390497110\nn=3 add=5881159685710253342\nn=1 add=4957045732212500362\nn=0 add=1469598103934665603'
check t_vectorize_f64 $'n=1000 add=12605502316359290979 mul=15193480026440175715\nn=999 add=16431287870010424977 mul=15840584383383207569\nn=5 add=4555731617307007401 mul=573423646804646313\nn=1 add=11364037426215188377 mul=671366010930788249\nn=0 add=1469598103934665603 mul=1469598103934665603'
check t_vectorize_const 'u32mul=7406505972513382635 i64add=16879444138376719115 f32mul=10425260595837503587 f32sub=16288212200764297315'
check t_vectorize_sum $'n=1000 u32=4213332 u64=6494044\nn=999 u32=4210927 u64=6481210\nn=5 u32=23274 u64=795\nn=3 u32=13985 u64=432\nn=1 u32=4660 u64=153\nn=0 u32=0 u64=0'

# Output equality alone cannot prove the vectorizer ran. Each fixture below
# contains exactly one xor+imul loop, so a packed multiply (SSE2 lowering of
# i32x4.imul) in the binary proves *that* loop was vectorized (a tail loop
# elsewhere cannot satisfy the check). t_vectorize_guard (n=3) proves the SIMD
# guard/body is generated even though it runs zero vector iterations.
for fx in t_vectorize_unit t_vectorize_guard t_vectorize_dyn t_vectorize_multi; do
    if command -v objdump >/dev/null 2>&1; then
        if ! objdump -d "$TMP/${fx}_M" --no-show-raw-insn 2>/dev/null | grep -q pmuludq; then
            echo "[regression] $fx: no packed multiply (loop not vectorized / guard not generated)" >&2
            fail=1
        fi
    else
        echo "[regression] $fx: objdump unavailable, skipping SIMD assertion" >&2
    fi
done

# Two eligible loops in one function must BOTH vectorize (2 pmuludq each -> >=4);
# an early exit after the first match would leave the second scalar (2).
if command -v objdump >/dev/null 2>&1; then
    mc=$(objdump -d --disassemble=two_loops "$TMP/t_vectorize_multi_M" --no-show-raw-insn 2>/dev/null | grep -c pmuludq)
    if [ "$mc" -lt 4 ]; then
        echo "[regression] t_vectorize_multi: only $mc pmuludq (expected >=4; a later loop stayed scalar)" >&2
        fail=1
    fi
fi

# Add/sub combines: ops2 must contain paddd (add) and psubd (sub), in-function.
if command -v objdump >/dev/null 2>&1; then
    od=$(objdump -d --disassemble=ops2 "$TMP/t_vectorize_ops_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$od" | grep -q paddd; then
        echo "[regression] t_vectorize_ops: no paddd (add loop not vectorized)" >&2
        fail=1
    fi
    if ! printf '%s\n' "$od" | grep -q psubd; then
        echo "[regression] t_vectorize_ops: no psubd (sub loop not vectorized)" >&2
        fail=1
    fi
    f32d=$(objdump -d --disassemble=f32ops "$TMP/t_vectorize_f32_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$f32d" | grep -q addps || ! printf '%s\n' "$f32d" | grep -q mulps; then
        echo "[regression] t_vectorize_f32: missing addps/mulps in f32ops" >&2
        fail=1
    fi
    qd=$(objdump -d --disassemble=i64ops "$TMP/t_vectorize_i64_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$qd" | grep -q paddq; then
        echo "[regression] t_vectorize_i64: missing paddq in i64ops" >&2
        fail=1
    fi
    f64d=$(objdump -d --disassemble=f64ops "$TMP/t_vectorize_f64_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$f64d" | grep -q addpd || ! printf '%s\n' "$f64d" | grep -q mulpd; then
        echo "[regression] t_vectorize_f64: missing addpd/mulpd in f64ops" >&2
        fail=1
    fi
    cd_=$(objdump -d --disassemble=constops "$TMP/t_vectorize_const_M" --no-show-raw-insn 2>/dev/null)
    for ins in pmuludq paddq mulps subps; do
        if ! printf '%s\n' "$cd_" | grep -q "$ins"; then
            echo "[regression] t_vectorize_const: missing $ins in constops" >&2
            fail=1
        fi
    done
    sd=$(objdump -d --disassemble=usum "$TMP/t_vectorize_sum_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$sd" | grep -q paddd; then
        echo "[regression] t_vectorize_sum: missing paddd in usum" >&2
        fail=1
    fi
    qsd=$(objdump -d --disassemble=qsum "$TMP/t_vectorize_sum_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$qsd" | grep -q paddq; then
        echo "[regression] t_vectorize_sum: missing paddq in qsum" >&2
        fail=1
    fi
fi

# A constant trip bound must still vectorize when the egraph rematerializes the
# literal in the loop header (RAKIT_NO_LICM=1): the literal is rematerialized in
# the guard, so it needs no dominating definition. Compile/run/output always run
# (no disassembler needed); only the SIMD check is gated on objdump.
if ! env -u RAKIT_HOST_LLVM RAKIT_NO_LICM=1 "$ROOT/rakit/dist/bin/rustc-rakit" --target "$TARGET" -C opt-level=3 "$BENCH/tests/t_vectorize_unit.rs" -o "$TMP/u_nolicm" --edition 2021 >"$TMP/u_nolicm.compile" 2>&1; then
    echo "[regression] t_vectorize_unit NO_LICM=1: compile failed" >&2
    cat "$TMP/u_nolicm.compile" >&2
    fail=1
elif ! "$TMP/u_nolicm" >"$TMP/u_nolicm.out" 2>&1; then
    echo "[regression] t_vectorize_unit NO_LICM=1: run failed" >&2
    cat "$TMP/u_nolicm.out" >&2
    fail=1
elif [ "$(cat "$TMP/u_nolicm.out")" != "unit 12776226417588255163" ]; then
    echo "[regression] t_vectorize_unit NO_LICM=1: wrong output" >&2
    printf 'expected:\nunit 12776226417588255163\nactual:\n%s\n' "$(cat "$TMP/u_nolicm.out")" >&2
    fail=1
elif command -v objdump >/dev/null 2>&1; then
    if ! objdump -d "$TMP/u_nolicm" --no-show-raw-insn 2>/dev/null | grep -q pmuludq; then
        echo "[regression] t_vectorize_unit NO_LICM=1: no pmuludq (constant bound lost vectorization)" >&2
        fail=1
    fi
else
    echo "[regression] t_vectorize_unit NO_LICM=1: objdump unavailable, skipping SIMD assertion" >&2
fi

if [ "$fail" -eq 0 ]; then
    echo "[regression] all fixtures passed"
fi
exit "$fail"
