#!/bin/bash
# Three-way benchmark on a real dependency-heavy crate.
#
#   L = rustc + LLVM          U = cg_clif upstream          M = Rakit modified
#
# Build cost is measured over the whole cargo process tree (treemeasure.py):
# cargo's rustc children are where the time and memory actually go, and a
# direct-child wait4 would report only cargo's own ~50 MB.
#
# Each repetition is a *clean* build. The target dir is removed between reps
# outside the timed region; measuring a warm dir reports cargo's cache hit
# (~0.1s), not the backend.
#
# LTO is forced off for every backend: rakit rejects it outright, so leaving
# it on for L would compare two different builds, not two backends.
#
# Usage: ./cratebench.sh <srcdir> [workload-file] [bin-name]
#   env: CREPS (clean builds, default 3)  RREPS (runtime reps, default 5)
set -u
EXT=$(cd "$(dirname "$0")" && pwd)
TM=$EXT/treemeasure.py
# Normalise to an absolute path: the build runs with cwd=$SRC, so a relative
# $SRC would make a relative CARGO_TARGET_DIR resolve inside $SRC twice.
SRC=$(CDPATH= cd -- "${1:?usage: cratebench.sh <srcdir> [workload] [bin]}" >/dev/null && pwd) || exit 1
# A capture directory that does not exist turns every redirect below into a
# failure that the comparison cannot see, so create it and drop stale captures.
if [ -n "${OUTDIR:-}" ]; then
  mkdir -p "$OUTDIR" || exit 1
  rm -f "$OUTDIR"/out_*.txt "$OUTDIR"/err_*.txt
fi
WORK=${2:-}
BINNAME=${3:-}
CREPS=${CREPS:-3}
RREPS=${RREPS:-5}
export CARGO_PROFILE_RELEASE_LTO=false

declare -A BIN=(
  [L]="${CARGO:-$(command -v cargo)}"
  [U]="$EXT/../cg_clif_upstream/dist/bin/cargo-clif"
  [M]="$EXT/../rakit/dist/bin/cargo-rakit"
)
declare -A TDIR=( [L]=target-bL [U]=target-bU [M]=target-bM )
# Set only when a backend built, ran and captured its output in this run, so a
# leftover capture can never stand in for a failed one.
declare -A OK=()

jf() { python3 -c "import sys,json;d=json.load(sys.stdin);print('%.0f %.0f %d %d'%(d['wall_med']*1000,d['cpu_med']*1000,d['peak_rss_med_kb'],d['exit']))"; }
med() { printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END{print (NR%2)?a[(NR+1)/2]:int((a[NR/2]+a[NR/2+1])/2)}'; }

name=$(grep -m1 '^name' "$SRC/Cargo.toml" | cut -d'"' -f2)
deps=$(cd "$SRC" && cargo tree -e normal --prefix none 2>/dev/null | sort -u | wc -l)
echo "crate: $name | deps: $deps | clean builds per backend: $CREPS"
printf '%-2s | %9s %9s %8s | %10s | %8s %8s %8s\n' bk Cwall Ccpu CrssPeak size Rwall Rcpu RrssPeak
printf '%s\n' "---------------------------------------------------------------------------"

for bk in L U M; do
  t="$SRC/${TDIR[$bk]}"
  # Only the Rakit wrapper takes the `rakit` subcommand; upstream cg_clif and
  # LLVM take plain cargo arguments.
  case $bk in L|U) sub="build" ;; M) sub="rakit build" ;; esac
  walls=(); cpus=(); rsss=()
  for _ in $(seq "$CREPS"); do
    rm -rf "$t"
    res=$(cd "$SRC" && CARGO_TARGET_DIR="$t" python3 "$TM" --reps 1 --json -- \
          "${BIN[$bk]}" $sub --release | jf)
    read -r cw cc cr ce <<<"$res"
    if [ "$ce" != 0 ]; then echo "$bk BUILD_FAIL exit=$ce"; continue 2; fi
    walls+=("$cw"); cpus+=("$cc"); rsss+=("$cr")
  done
  cw=$(med "${walls[@]}"); cc=$(med "${cpus[@]}"); cr=$(med "${rsss[@]}")

  bin="$t/release/${BINNAME:-$name}"
  if [ ! -x "$bin" ]; then echo "$bk NO_BINARY ($bin)"; continue; fi
  sz=$(stat -c %s "$bin")
  if [ -n "${RUNCMD:-}" ]; then
    # A workload that needs more than one argument (flags, a corpus dir, ...).
    # `$BIN` is replaced by the built binary; the whole tree is measured.
    rres=$(BIN="$bin" python3 "$TM" --reps "$RREPS" --warmup 1 --json -- bash -c "$RUNCMD" | jf)
    read -r rw rc rr re <<<"$rres"
    if [ -n "${OUTDIR:-}" ]; then
      if ! BIN="$bin" bash -c "$RUNCMD" >"$OUTDIR/out_$bk.txt" 2>"$OUTDIR/err_$bk.txt"; then
        echo "$bk CAPTURE_FAIL"
        continue
      fi
      out=$(md5sum <"$OUTDIR/out_$bk.txt" | cut -c1-8)
    else
      # `$?` after a pipeline is md5sum's, so the workload's own status has to be
      # kept explicitly: a failed run must not contribute a hash. The output goes
      # to a file, not a shell variable: command substitution strips trailing
      # newlines and drops NUL bytes, which would fingerprint two different
      # outputs identically.
      capf=$(mktemp) || exit 1
      if ! BIN="$bin" bash -c "$RUNCMD" >"$capf" 2>/dev/null; then
        rm -f "$capf"; echo "$bk CAPTURE_FAIL"; continue
      fi
      out=$(md5sum <"$capf" | cut -c1-8); rm -f "$capf"
    fi
  elif [ -n "$WORK" ]; then
    rres=$(python3 "$TM" --reps "$RREPS" --warmup 1 --json -- "$bin" "$WORK" | jf)
    read -r rw rc rr re <<<"$rres"
    capf=$(mktemp) || exit 1
    if ! "$bin" "$WORK" >"$capf" 2>/dev/null; then
      rm -f "$capf"; echo "$bk CAPTURE_FAIL"; continue
    fi
    out=$(md5sum <"$capf" | cut -c1-8); rm -f "$capf"
  else
    printf '%-2s | %8sms %8sms %7sM | %10s | -\n' "$bk" "$cw" "$cc" "$((cr/1024))" "$sz"
    continue
  fi
  if [ "$re" != 0 ]; then
    printf '%-2s | %8sms %8sms %7sM | %10s | RUN_FAIL exit=%s\n' \
      "$bk" "$cw" "$cc" "$((cr/1024))" "$sz" "$re"
    continue
  fi
  printf '%-2s | %8sms %8sms %7sM | %10s | %7sms %7sms %7sM  out=%s\n' \
    "$bk" "$cw" "$cc" "$((cr/1024))" "$sz" "$rw" "$rc" "$((rr/1024))" "$out"
  OK[$bk]=1
done

if [ -n "${OUTDIR:-}" ] && [ -z "$WORK" ] && [ -z "${RUNCMD:-}" ]; then
  echo "outputs: not compared (compile-only run: no workload)"
  exit 0
fi
if [ -n "${OUTDIR:-}" ]; then
  bad=0
  for bk in L U M; do
    if [ -z "${OK[$bk]:-}" ]; then
      echo "INCOMPLETE: $bk did not build, run and capture"
      bad=1
    fi
  done
  for bk in U M; do
    if [ -n "${OK[L]:-}" ] && [ -n "${OK[$bk]:-}" ] &&
       ! cmp -s "$OUTDIR/out_L.txt" "$OUTDIR/out_$bk.txt"; then
      echo "MISMATCH: out_$bk.txt differs from LLVM"; bad=1
    fi
  done
  [ "$bad" = 0 ] && echo "outputs: L == U == M (byte-identical)"
  exit "$bad"
fi
