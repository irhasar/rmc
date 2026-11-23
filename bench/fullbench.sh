#!/bin/bash
# Full 3-way comparison: compile wall/cpu/RSS, binary size, run wall/cpu/RSS.
set -u
BENCH=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$BENCH")
N=nightly-2026-11-17
L="rustc +$N"; U="$ROOT/cg_clif_upstream/dist/bin/rustc-clif"; M="$ROOT/rakit/dist/bin/rustc-rakit"
P="$BENCH/progs"; D=/tmp/fb; mkdir -p $D
RC=${RC:-7}; RR=${RR:-9}
jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.1f %.1f %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb']/1024))"; }
printf "%-9s %-4s %7s %7s %7s %8s | %8s %8s %8s\n" prog bk cwall ccpu crss size rwall rcpu rrss
for src in $P/p*.rs; do
  b=$(basename $src .rs)
  for bk in L U M; do
    cmd=${!bk}; bin=$D/${b}_$bk
    cres=$(python3 $BENCH/measure.py --reps $RC --warmup 2 --json -- $cmd -C opt-level=3 $src -o $bin --edition 2021 2>/dev/null | jf)
    read cw cc cr <<<"$cres"
    sz=$(stat -c%s $bin 2>/dev/null || echo 0)
    rres=$(python3 $BENCH/measure.py --reps $RR --warmup 3 --json -- $bin 2>/dev/null | jf)
    read rw rcpu rr <<<"$rres"
    printf "%-9s %-4s %7s %7s %7s %8s | %8s %8s %8s\n" $b $bk $cw $cc $cr $sz $rw $rcpu $rr
  done
done
