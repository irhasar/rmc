#!/bin/bash
# Three-way comparison: LLVM backend vs cg_clif upstream vs modified Rakit.
#
# For each (backend, program) pair we measure:
#   * compile wall / cpu / peak RSS
#   * resulting binary size, and its .text / .data.rel.ro / .rodata / .eh_frame sizes
#   * run wall / cpu / peak RSS
#
# Two sysroot configurations are exercised so that the backend can be isolated
# from the standard library it links against:
#   - "own":  the backend's own compiled std
#   - "llvm": the stock LLVM-compiled std, via --sysroot
set -u

BENCH=$(cd "$(dirname "$0")" && pwd)
PROGS="$BENCH/progs"
OUT="$BENCH/out"
mkdir -p "$OUT"

NIGHTLY=nightly-2026-11-17
RUSTC_LLVM="rustc +$NIGHTLY"
CG_UP="$(cd "$(dirname "$0")/.." && pwd)/cg_clif_upstream/dist/bin/rustc-clif"
CG_MOD="$(cd "$(dirname "$0")/.." && pwd)/rakit_mod/dist/bin/rustc-rakit"
LLVM_SYSROOT=$(rustc +$NIGHTLY --print sysroot)

REPS_COMPILE=${REPS_COMPILE:-3}
REPS_RUN=${REPS_RUN:-5}
MEASURE=(python3 "$BENCH/measure.py" --warmup 1 --json)

json_field() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %.0f %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['maxrss_med_kb']/1024,d['exit']))"; }

section_sizes() {
  # Emit: text data.rel.ro rodata eh_frame total
  size -A "$1" 2>/dev/null | awk '
    $1==".text"{t+=$2} $1==".data.rel.ro"{d+=$2} $1==".rodata"{r+=$2}
    $1==".eh_frame"{e+=$2} $1=="Total"{tot=$2}
    END{printf "%d %d %d %d %d\n", t, d, r, e, tot}'
}

run_backend() {
  local label="$1" sysroot="$2"; shift 2
  local cmd=("$@")
  local dir="$OUT/$label"; mkdir -p "$dir"
  for src in "$PROGS"/*.rs; do
    local base; base=$(basename "$src" .rs)
    local outbin="$dir/$base"
    local cargs=()
    [ -n "$sysroot" ] && cargs+=(--sysroot "$sysroot")
    local cres; cres=$("${MEASURE[@]}" --reps "$REPS_COMPILE" -- "${cmd[@]}" "${cargs[@]}" \
        -C opt-level=3 "$src" -o "$outbin" --edition 2021 | json_field)
    if [ ! -f "$outbin" ]; then
      printf "%s %s COMPILE_FAIL %s\n" "$label" "$base" "$cres"; continue
    fi
    local cwall ccp crss cexit; read -r cwall ccp crss cexit <<<"$cres"
    local size secs; size=$(stat -c %s "$outbin"); secs=$(section_sizes "$outbin")
    local rres; rres=$("${MEASURE[@]}" --reps "$REPS_RUN" -- "$outbin" | json_field)
    local rwall rcp rrss rexit; read -r rwall rcp rrss rexit <<<"$rres"
    printf "%s %s C %s %s %s %s | S %s %s | R %s %s %s %s\n" \
      "$label" "$base" "$cwall" "$ccp" "$crss" "$cexit" \
      "$size" "$secs" "$rwall" "$rcp" "$rrss" "$rexit"
  done
}

echo "### LLVM (stock rustc $NIGHTLY)"
run_backend llvm "" $RUSTC_LLVM
echo "### cg_clif upstream (own Cranelift std)"
run_backend cgclif_up "" $CG_UP
echo "### Rakit modified (own Rakit std)"
run_backend rakit_mod "" $CG_MOD
echo "### cg_clif upstream + LLVM std (isolates backend)"
run_backend cgclif_up_llvmstd "$LLVM_SYSROOT" $CG_UP
echo "### Rakit modified + LLVM std (isolates backend)"
run_backend rakit_mod_llvmstd "$LLVM_SYSROOT" $CG_MOD
