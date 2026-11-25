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
check t_vectorize_ops $'add 17194792273410237731 sub 10846146650447627395 rev 10846146650447627395'
check t_vectorize_f32 $'n=1000 add=7083501190089957475 mul=16326997319991784275\nn=999 add=10593356097432088209 mul=10796855508119876121\nn=5 add=14995117245246052777 mul=3891769157468395945\nn=2 add=15738008312299960571 mul=18044256215840110843\nn=0 add=1469598103934665603 mul=1469598103934665603'
check t_vectorize_i64 $'n=1000 add=17194792273410237731\nn=999 add=7573983905390497110\nn=3 add=5881159685710253342\nn=1 add=4957045732212500362\nn=0 add=1469598103934665603'
check t_vectorize_f64 $'n=1000 add=12605502316359290979 mul=15193480026440175715\nn=999 add=16431287870010424977 mul=15840584383383207569\nn=5 add=4555731617307007401 mul=573423646804646313\nn=1 add=11364037426215188377 mul=671366010930788249\nn=0 add=1469598103934665603 mul=1469598103934665603'
check t_vectorize_const $'u32mul=7406505972513382635 i64add=16879444138376719115 f32mul=10425260595837503587 f32sub=16288212200764297315 ov=4354610730778066115'
check t_vectorize_sum $'n=1000 u32=4213332 u64=6494044\nn=999 u32=4210927 u64=6481210\nn=5 u32=23274 u64=795\nn=3 u32=13985 u64=432\nn=1 u32=4660 u64=153\nn=0 u32=0 u64=0'
check t_vectorize_bitops $'n=1000 and=8679198063293307935 or=17640510044461566455\nn=999 and=425701985508822469 or=16951716591865051082\nn=5 and=3321434351676732587 or=7594011638986234852\nn=2 and=11141768042474047163 or=14494034960203992247\nn=0 and=1469598103934665603 or=1469598103934665603'
check t_vectorize_start $'s=0 mul=7406505972513382635 sum=4225677\ns=1 mul=6626696854460331983 sum=4221017\ns=3 mul=16778483497030676228 sum=4211692\ns=5 mul=12379647581177015243 sum=4202403\ns=1000 mul=10102475090843504739 sum=12345\nwrap mul=3599152629925780035 sum=12345'
check t_vectorize_scalar $'n=1000 mul=7406505972513382635 add=9424463066469146027 fmul=10425260595837503587 ov=4354610730778066115\nn=999 mul=3464759790932634684 add=8376324054428928831 fmul=12049442534097153681 ov=4713393597608891589\nn=5 mul=6468920626777132041 add=4948225117454695846 fmul=12629251342556830121 ov=18145564061338165973\nn=2 mul=2034113175188541570 add=13298927877194760002 fmul=5938173602147907835 ov=2000679104580653491\nn=0 mul=1469598103934665603 add=1469598103934665603 fmul=1469598103934665603 ov=1469598103934665603'
check t_vectorize_mul2 $'n=1000 mul2=12892184874820168067 mul2k=5843593202211838595\nn=999 mul2=8886187857571388677 mul2k=17628163510071353925\nn=5 mul2=1028372591822908160 mul2k=3165372279348321456\nn=2 mul2=13351568537748069832 mul2k=15675520182477874744\nn=0 mul2=1469598103934665603 mul2k=1469598103934665603'
check t_vectorize_u8u16 $'n=1000 17878300862133242147 15386882374342132331 5931515482962300603 104022738392422175 13886688620616272375 6413824890027366659 437350453307941203 348677590476470651 13992056364067255191 15391197080985237023\nn=999 6172660611095773782 6440929629371019306 11354769232984238750 11733095245541451461 5085264594999823562 7227458886941953405 5213201215610304887 16759044478078098675 2280043866499186332 1752871260808802334\nn=17 7285020763669773354 6304217610140279832 10601597486522502298 8110402542225378913 14576979526816206002 799705128001939673 13206878947294554777 16447621790886945449 18008307735591666209 16058502585834153985\nn=15 3032365350422506470 10467051025575009090 6640376138801412294 9043039105372344161 17386058533414242774 378773803547436381 5944699636424046455 7494878089411756299 3160188444332738440 14049413737344378914\nn=1 4953105082536992138 4953045708909068744 4953069898164889386 4953180948839338697 4953087490350940762 4953163356653287321 4953163356653287321 4953163356653287321 4953163356653287321 4953163356653287321\nn=0 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603'
check t_vectorize_minmax $'n=1000 1525913161964794528 11257568297462708664 15925876787297668091 16111070143226636155\nn=999 1963068772856787673 16884486458729255764 16967588876371895608 10840075603923670012\nn=17 10688307771081281719 14554605040048888946 14717919791827201305 7973199076920258713\nn=15 11765418009486946188 10537817726254184633 10224949873298748252 5885256550300961016\nn=1 4953197441513761862 4953257914653313467 4953163356653287321 4953163356653287321\nn=0 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603'
check t_vectorize_narrow_const $'n=1000 12597256266151776971 45912873700106395 10970574096883108195 2467173283300988691 17526415468205305563 15063556406114166635 17184738474638672179 16916651684668209067\nn=999 1567524380705550007 45678951534485875 570475198086930840 10006786153709626776 15476646075539752849 6263433654355958012 4669121117834956577 9181916829145480072\nn=17 1496932352803504692 771180002190595694 4480173573210530987 5001436096364578443 14188023185179219852 12751589190998615605 10102543714890860441 15249155791756751001\nn=15 17825320725414463927 10398562223267582035 10829702270494340816 14616066152714856640 8044266171027422913 1849089465779355276 8402261732942819825 13559566617656314104\nn=1 4953261213188198100 4953276606350993054 4953169953723056587 4953117177164902459 4953164456164915532 4957161180933462517 4953163356653287321 4989192153686505369\nn=0 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603'
check t_vectorize_shift $'n=1000 11712312636423896003 2778030800238186898 12714699042291631591 12146066278783673187 18076120378244018212\nn=999 17950719681623895601 4295731586229105149 18366224563185551094 15967729648193777297 4243624414413078198\nn=17 3133535629771302769 4250607478905603131 4627207258058668699 9393502281181800793 17523654564317111236\nn=15 15549534947945525937 12783628547635060317 5038023839363107084 14715364296271233841 1063941970688198118\nn=1 14882131693571967345 1473229973278764170 13245022570561246023 16874419546565058841 11101096919446260564\nn=0 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603'
check t_vectorize_shift_var $'n=1000 s=0 4443170450025176819 4443170450025176819 11383801699270629795 9504849664338696795\nn=999 s=3 17950719681623895601 18016764557407375040 14692724434866993588 10917167937626568097\nn=17 s=31 8133557662623086041 11946133644423489254 1529861397211175513 10913290239242810841\nn=15 s=7 9491340174319058481 17874970409326034962 15701817551614350115 14715364296271233841\nn=1 s=5 1868514751706788089 1473229973278764170 12995938077602342032 15174882978923274809\nn=0 s=2 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603\nn=11 s=32 11240454860660229021 11240454860660229021 15012539886816120206 7469783584398961825\nn=11 s=33 13630957820829976809 12994340509938189023 725659547843287726 4889044105392614561\nn=11 s=63 14304790145501818017 17709326624890133001 8455013673867603862 15589326116826180769\nn=11 s=100 14033802990936947201 15544261638437637265 9558596245463579848 12729104809402363041'
check t_vectorize_inplace $'n=1000 2067684494068040691 4955472742519183899 2257382229062978179 7219609798812422243 mix=1216567661396735\nn=999 4938945738208900473 14598870757050654369 6360620038797656489 3574130843177975441 mix=16855457192477885161\nn=17 3956057608133206998 12127250348921852964 14400859015502114329 18144213985805612505 mix=13086125181362180361\nn=15 4854181577974998993 2643014722888691377 4245625956762491497 4326354876010677041 mix=16259185720986712641\nn=1 4953162257141659110 15597959157331910276 4953163356653287321 341477803479070617 mix=4953163356653287321\nn=0 1469598103934665603 1469598103934665603 1469598103934665603 1469598103934665603 mix=1469598103934665603'
check t_vectorize_slice $'n=1000 2067684494068040691 2257382229062978179 2067684494068040691\nn=999 4938945738208900473 6360620038797656489 4938945738208900473\nn=17 3956057608133206998 14400859015502114329 3956057608133206998\nn=15 4854181577974998993 4245625956762491497 4854181577974998993\nn=1 4953162257141659110 4953163356653287321 4953162257141659110\nn=0 1469598103934665603 1469598103934665603 1469598103934665603'
check t_vectorize_guards $'n=1000 sum=499500 ctr=1000 mul2=14734897173167142259 mul8=5635034875195320451 sum2=499500 ctr2=1000\nn=4 sum=6 ctr=4 mul2=4806693891325407483 mul8=17023279625793469443 sum2=6 ctr2=4\nn=3 sum=3 ctr=3 mul2=12198867445692768159 mul8=12189314888668763241 sum2=3 ctr2=3\nn=1 sum=0 ctr=1 mul2=4953163356653287321 mul8=4953163356653287321 sum2=0 ctr2=1\nn=0 sum=0 ctr=0 mul2=1469598103934665603 mul8=1469598103934665603 sum2=0 ctr2=0'
check t_volatile $'n=1000 loop=17230722056597992339\nn=17 loop=18156365728903607819\nn=1 loop=4953257914653313467\nn=0 loop=1469598103934665603\nv256 18004264572236793056\nv12 3735928559 305419896 3405691582\nv40 16438386427504056938\nv512 74bf1cf9249bb103\nvstk 1230'

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
    od=$(objdump -d --disassemble=ops2r "$TMP/t_vectorize_ops_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$od" | grep -q psubd; then
        echo "[regression] t_vectorize_ops: no psubd in ops2r (reversed load order not vectorized)" >&2
        fail=1
    fi
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
    bd=$(objdump -d --disassemble=bandfn "$TMP/t_vectorize_bitops_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$bd" | grep -q pand; then
        echo "[regression] t_vectorize_bitops: missing pand in bandfn" >&2
        fail=1
    fi
    od=$(objdump -d --disassemble=borfn "$TMP/t_vectorize_bitops_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$od" | grep -qE '\bpor\b'; then
        echo "[regression] t_vectorize_bitops: missing por in borfn" >&2
        fail=1
    fi
    sm=$(objdump -d --disassemble=startmul "$TMP/t_vectorize_start_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$sm" | grep -q pmuludq; then
        echo "[regression] t_vectorize_start: missing pmuludq in startmul" >&2
        fail=1
    fi
    ss=$(objdump -d --disassemble=startsum "$TMP/t_vectorize_start_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$ss" | grep -q paddd; then
        echo "[regression] t_vectorize_start: missing paddd in startsum" >&2
        fail=1
    fi
    scm=$(objdump -d --disassemble=scalemul "$TMP/t_vectorize_scalar_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$scm" | grep -q pmuludq; then
        echo "[regression] t_vectorize_scalar: missing pmuludq in scalemul" >&2
        fail=1
    fi
    sca=$(objdump -d --disassemble=scaleadd "$TMP/t_vectorize_scalar_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$sca" | grep -q paddd; then
        echo "[regression] t_vectorize_scalar: missing paddd in scaleadd" >&2
        fail=1
    fi
    scf=$(objdump -d --disassemble=fscale "$TMP/t_vectorize_scalar_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$scf" | grep -q mulps; then
        echo "[regression] t_vectorize_scalar: missing mulps in fscale" >&2
        fail=1
    fi
    m2=$(objdump -d --disassemble=mul2 "$TMP/t_vectorize_mul2_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$m2" | grep -q pmuludq; then
        echo "[regression] t_vectorize_mul2: missing pmuludq in mul2" >&2
        fail=1
    fi
    m2k=$(objdump -d --disassemble=mul2k "$TMP/t_vectorize_mul2_M" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$m2k" | grep -q pmuludq; then
        echo "[regression] t_vectorize_mul2: missing pmuludq in mul2k" >&2
        fail=1
    fi
    u8m="$TMP/t_vectorize_u8u16_M"
    for spec in "u8add:paddb" "u8sub:psubb" "u8xor:pxor" "u8and:pand" "u8or:por" "u16add:paddw" "u16sub:psubw" "u16xor:pxor" "u16and:pand" "u16or:por"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$u8m" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\b$ins\b"; then
            echo "[regression] t_vectorize_u8u16: missing $ins in $fn" >&2
            fail=1
        fi
    done
    mm="$TMP/t_vectorize_minmax_M"
    for spec in "u8min:pminub" "u8max:pmaxub" "i16min:pminsw" "i16max:pmaxsw"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$mm" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\b$ins\b"; then
            echo "[regression] t_vectorize_minmax: missing $ins in $fn" >&2
            fail=1
        fi
    done
    nc="$TMP/t_vectorize_narrow_const_M"
    for spec in "u8add:paddb" "u8xor:pxor" "u8and:pand" "u8or:por" "u16add:paddw"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$nc" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\b$ins\b"; then
            echo "[regression] t_vectorize_narrow_const: missing $ins in $fn" >&2
            fail=1
        fi
    done
    sh="$TMP/t_vectorize_shift_M"
    for spec in "u32shl:pslld" "u32shr:psrld" "i32sar:psrad" "u64shl:psllq" "u64shr:psrlq"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$sh" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\b$ins\b"; then
            echo "[regression] t_vectorize_shift: missing $ins in $fn" >&2
            fail=1
        fi
    done
    sv="$TMP/t_vectorize_shift_var_M"
    for spec in "u32shlv:pslld" "u32shrv:psrld" "i32sarv:psrad" "u64shlv:psllq"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$sv" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\b$ins\b"; then
            echo "[regression] t_vectorize_shift_var: missing $ins in $fn" >&2
            fail=1
        fi
    done
    ip="$TMP/t_vectorize_inplace_M"
    for spec in "inplace_add1:paddd" "inplace_xor:pxor" "inplace_add2:paddd" "inplace_fmul:mulps"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$ip" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\b$ins\b"; then
            echo "[regression] t_vectorize_inplace: missing $ins in $fn" >&2
            fail=1
        fi
    done
    sl="$TMP/t_vectorize_slice_M"
    for fn in slice_map1 slice_map2 slice_param; do
        d=$(objdump -d --disassemble=$fn "$sl" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\bpaddd\b"; then
            echo "[regression] t_vectorize_slice: missing paddd in $fn" >&2
            fail=1
        fi
    done
fi

# Bounds failure must behave like LLVM (no disassembler needed).
pl=$("$TMP/t_vectorize_slice_L" panic 2>&1 | grep -o "index out of bounds.*")
pm=$("$TMP/t_vectorize_slice_M" panic 2>&1 | grep -o "index out of bounds.*")
if [ -z "$pm" ] || [ "$pl" != "$pm" ]; then
    echo "[regression] t_vectorize_slice: panic mismatch: L='$pl' M='$pm'" >&2
    fail=1
fi

# IR comments attach to a block's last instruction: the counted volatile loop
# must not leave an empty exit block behind (`--emit=llvm-ir` used to panic).
# The dump always exits non-zero here (it writes `<out>.rakit/`), so gate on the
# panic message and on the dumps being produced. Needs no disassembler, so this
# runs outside the `objdump` guard.
rm -rf "$TMP/vol_ir.rakit"
env -u RAKIT_HOST_LLVM "$ROOT/rakit/dist/bin/rustc-rakit" --target "$TARGET" -C opt-level=3 \
    --emit=llvm-ir -o "$TMP/vol_ir" "$BENCH/tests/t_volatile.rs" --edition 2021 \
    >"$TMP/vol_ir.log" 2>&1 || true
if grep -q panicked "$TMP/vol_ir.log"; then
    echo "[regression] t_volatile: --emit=llvm-ir panicked" >&2
    tail -5 "$TMP/vol_ir.log" >&2
    fail=1
fi

# Guard rails: power-of-two maps vectorize; volatile and header-side-effect
# loops must stay scalar. (SIMD assertions need the disassembler; the outputs
# are checked above by `check`.)
if command -v objdump >/dev/null 2>&1; then
    gd="$TMP/t_vectorize_guards_M"
    for spec in "mul2:paddd" "mul8:pslld"; do
        fn=${spec%%:*}; ins=${spec##*:}
        d=$(objdump -d --disassemble=$fn "$gd" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -qE "\\b$ins\\b"; then
            echo "[regression] t_vectorize_guards: missing $ins in $fn" >&2
            fail=1
        fi
    done
    for fn in sum_ctr sum_ctr2; do
        d=$(objdump -d --disassemble=$fn "$gd" --no-show-raw-insn 2>/dev/null)
        if printf '%s\n' "$d" | grep -qE "\b(paddb|paddd|movdqu)\b"; then
            echo "[regression] t_vectorize_guards: $fn was vectorized" >&2
            fail=1
        fi
    done
    # Volatile: never widened, every source load issued, wide values copied.
    vv="$TMP/t_volatile_M"
    d=$(objdump -d --disassemble=vol_u8_loop "$vv" --no-show-raw-insn 2>/dev/null)
    if printf '%s\n' "$d" | grep -qE "\b(paddb|paddd|movdqu)\b"; then
        echo "[regression] t_volatile: volatile byte loop was vectorized" >&2
        fail=1
    fi
    # A discarded read must issue *all* of its source loads, each sized to the
    # alignment the value guarantees (never one misaligned wide load, and never
    # a library copy that could read a read-to-clear source twice).
    check_src_loads() {
        fn=$1
        want=$2
        got=$(objdump -d --disassemble="$fn" "$vv" --no-show-raw-insn 2>/dev/null \
            | grep -cE 'mov[a-z]*[[:space:]]+[0-9a-fx]*\(%rdi[^)]*\),')
        if [ "$got" -ne "$want" ]; then
            echo "[regression] t_volatile: $fn issued $got source loads, want $want" >&2
            fail=1
        fi
        d=$(objdump -d --disassemble="$fn" "$vv" --no-show-raw-insn 2>/dev/null)
        if ! printf '%s\n' "$d" | grep -q mfence; then
            echo "[regression] t_volatile: $fn lost its fence" >&2
            fail=1
        fi
        if printf '%s\n' "$d" | grep -qE "\b(paddb|paddd|movdqu)\b"; then
            echo "[regression] t_volatile: $fn was vectorized" >&2
            fail=1
        fi
    }
    check_src_loads vol_discard_u8 1
    check_src_loads vol12_discard 3
    check_src_loads vol40_discard 5
    check_src_loads vol256_discard 4
    check_src_loads vol512_discard 1
    # The source here is a local stack slot whose stores are visible: store
    # forwarding must not substitute them for the volatile reads.
    got=$(objdump -d --disassemble=vol_stack_discard "$vv" --no-show-raw-insn 2>/dev/null \
        | grep -cE 'mov[a-z]*[[:space:]]+[0-9a-fx]*\(%r(sp|bp)[^)]*\),')
    if [ "$got" -ne 5 ]; then
        echo "[regression] t_volatile: vol_stack_discard issued $got stack loads, want 5" >&2
        fail=1
    fi
    d=$(objdump -d --disassemble=vol512_discard "$vv" --no-show-raw-insn 2>/dev/null)
    if ! printf '%s\n' "$d" | grep -q call; then
        echo "[regression] t_volatile: vol512_discard did not use a copy call" >&2
        fail=1
    fi

if [ ! -d "$TMP/vol_ir.rakit" ]; then
    echo "[regression] t_volatile: --emit=llvm-ir produced no IR dump" >&2
    fail=1
fi

fi

# The vectorized IR must pass the Cranelift verifier (shift counts are scalar),
# for constant *and* runtime shift amounts.
for f in t_vectorize_shift t_vectorize_shift_var; do
    if ! env -u RAKIT_HOST_LLVM RAKIT_VEC_VERIFY=1 "$ROOT/rakit/dist/bin/rustc-rakit" --target "$TARGET" -C opt-level=3 "$BENCH/tests/$f.rs" -o "$TMP/${f}_verify" --edition 2021 >"$TMP/${f}_verify.compile" 2>&1; then
        echo "[regression] $f: RAKIT_VEC_VERIFY=1 failed (invalid optimized IR)" >&2
        cat "$TMP/${f}_verify.compile" >&2
        fail=1
    fi
done

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
