#!/bin/bash
# Deterministic text corpus for the ripgrep workload. Same bytes on every run,
# so the three backends must produce identical output.
# Usage: ./gen_corpus.sh <outdir> [files] [lines]
set -eu
out=${1:?usage: gen_corpus.sh <outdir> [files] [lines]}
files=${2:-800}
lines=${3:-300}
rm -rf "$out"
mkdir -p "$out"
for i in $(seq 1 "$files"); do
  d="$out/d$((i % 20))"
  mkdir -p "$d"
  awk -v n="$i" -v L="$lines" 'BEGIN{
    for (l = 0; l < L; l++) {
      r = (l * 7 + n * 13) % 17;
      if (r == 0)      printf("contact user%d@example.com needle here\n", (l + n) % 97);
      else if (r == 1) printf("fn needle_%d(x: u64) -> u64 { x * %d }\n", l, n);
      else if (r == 2) printf("TODO(needle): handle case %d in module %d\n", l, n);
      else if (r == 3) printf("FIXME: module %d case %d\n", n, l);
      else             printf("line %d of file %d: lorem ipsum dolor sit amet %d\n", l, n, (l * n) % 1000);
    }
  }' > "$d/f$i.txt"
done
echo "corpus: $(find "$out" -type f | wc -l) files, $(du -sh "$out" | cut -f1)"
