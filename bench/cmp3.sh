#!/bin/bash
# 3-way runtime + size + compile comparison: LLVM vs upstream cg_clif vs modified.
set -u
N=nightly-2026-11-17
L="rustc +$N"
U=/workspace/project/experiments/cg_clif_upstream/dist/bin/rustc-clif
M=/workspace/project/experiments/cg_clif_mod/dist/bin/rustc-clif
P=/workspace/project/experiments/bench/progs
D=/tmp/cmp3; mkdir -p $D
printf "%-10s %8s %8s %8s | %8s %8s %8s | %8s %8s %8s\n" prog Lrun Urun Mrun Lsz Usz Msz Lcpl Ucpl Mcpl
for src in $P/p*.rs; do
  b=$(basename $src .rs)
  for bk in L U M; do
    cmd=${!bk}
    t0=$(date +%s%N)
    $cmd -C opt-level=3 $src -o $D/${b}_$bk --edition 2021 2>/dev/null
    t1=$(date +%s%N)
    eval "cpl_$bk=$(( (t1-t0)/1000000 ))"
    eval "sz_$bk=$(stat -c%s $D/${b}_$bk 2>/dev/null || echo 0)"
    s=$(date +%s%N); $D/${b}_$bk >/dev/null 2>&1; e=$(date +%s%N)
    eval "run_$bk=$(( (e-s)/1000000 ))"
  done
  printf "%-10s %7sms %7sms %7sms | %8s %8s %8s | %7sms %7sms %7sms\n" $b $run_L $run_U $run_M $sz_L $sz_U $sz_M $cpl_L $cpl_U $cpl_M
done
