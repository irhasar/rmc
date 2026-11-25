#!/bin/bash
# ripgrep workload: literal scan, regex with captures, case-insensitive scan,
# multi-pattern (Aho-Corasick) scan and a directory walk. `--sort path` keeps
# the output independent of walk and thread order.
# Usage: rg_work.sh <rg-binary> <corpus-dir>
set -eu
RG=$1
C=$2
"$RG" --sort path -c -F needle "$C"
"$RG" --sort path -n --no-heading -e '([a-z]+)_?([0-9]+)@example\.com' "$C"
"$RG" --sort path -c -i -e 'todo|fixme|lorem' "$C"
"$RG" --sort path -c -F -e TODO -e FIXME -e 'fn needle' -e 'lorem ipsum' "$C"
"$RG" --sort path --files "$C" | wc -l
