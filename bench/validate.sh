#!/bin/bash
# Shared output validation for the benchmark harnesses.
#
#   validate.sh --capture REF BIN [args...]   run BIN, require exit 0, store output
#   validate.sh --compare REF BIN [args...]   run BIN, require exit 0 and output == REF
#
# stdout and stderr are both captured, so a program that prints a diagnostic and
# exits zero still fails when its output changes. Every harness must validate a
# backend with the same arguments it later measures. During timing, measure.py
# checks output on each warmup, every timed repetition, and one post-measurement
# verification run; a zero-exit mismatch in any timed repetition is rejected.
set -u
mode=${1:?missing mode}
ref=${2:?missing reference file}
bin=${3:?missing binary}
shift 3

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

"$bin" "$@" >"$tmp" 2>&1
status=$?
if [ "$status" -ne 0 ]; then
    cat "$tmp" >&2
    echo "[validate] $bin $*: exited $status" >&2
    exit 2
fi

case "$mode" in
    --capture) cp "$tmp" "$ref" ;;
    --compare)
        if ! cmp -s "$ref" "$tmp"; then
            diff -u "$ref" "$tmp" >&2 || true
            echo "[validate] $bin $*: output differs from $ref" >&2
            exit 3
        fi
        ;;
    *)
        echo "[validate] unknown mode: $mode" >&2
        exit 64
        ;;
esac
