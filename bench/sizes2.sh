#!/bin/bash
set -u
NIGHTLY=nightly-2026-11-17
CG_UP="$(cd "$(dirname "$0")/.." && pwd)/cg_clif_upstream/dist/bin/rustc-clif"
CG_MOD="$(cd "$(dirname "$0")/.." && pwd)/rakit/dist/bin/rustc-rakit"
PROGS="$(cd "$(dirname "$0")" && pwd)/progs"
D=/tmp/sz2; mkdir -p $D
for src in $PROGS/p2_mem.rs $PROGS/p3_calls.rs; do
  b=$(basename $src .rs)
  for variant in "o3" "oz" "o3cu1"; do
    case $variant in
      o3) FL="-C opt-level=3";;
      oz) FL="-C opt-level=z";;
      o3cu1) FL="-C opt-level=3 -C codegen-units=1";;
    esac
    for bk in "llvm:rustc +$NIGHTLY" "up:$CG_UP" "mod:$CG_MOD"; do
      nm=${bk%%:*}; cmd=${bk#*:}
      out=$D/${nm}_${b}_${variant}
      $cmd $FL $src -o $out --edition 2021 2>/dev/null
      if [ -f $out ]; then
        raw=$(stat -c %s $out); cp $out ${out}_s; strip ${out}_s
        st=$(stat -c %s ${out}_s)
        printf "%-4s %-9s %-6s raw=%9d stripped=%9d\n" "$nm" "$b" "$variant" "$raw" "$st"
      fi
    done
  done
done
