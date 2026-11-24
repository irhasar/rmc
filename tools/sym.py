#!/usr/bin/env python3
"""Attribute sampled RIPs to symbols using nm, without perf."""
import subprocess
import sys
import bisect
from collections import Counter


def load_syms(binary):
    """Return [(start, end, name)] using symbol sizes.

    Nearest-preceding-symbol lookup misattributes padding and hot/cold split
    regions, so containment against the real extent is required.
    """
    out = subprocess.run(["nm", "-C", "-S", "--defined-only", "-n", binary],
                         capture_output=True, text=True).stdout
    syms = []
    for line in out.splitlines():
        parts = line.split(maxsplit=3)
        if len(parts) < 4:
            continue
        addr, size, kind, name = parts
        if kind.lower() not in ("t", "w"):
            continue
        try:
            start = int(addr, 16)
            length = int(size, 16)
        except ValueError:
            continue
        if length:
            syms.append((start, start + length, name))
    syms.sort()
    return syms


def main():
    binary, profile = sys.argv[1], sys.argv[2]
    syms = load_syms(binary)
    starts = [s for s, _, _ in syms]
    counts = Counter()
    total = 0
    ext = 0
    for line in open(profile):
        line = line.strip()
        if not line:
            continue
        total += 1
        if line == "ext":
            ext += 1
            continue
        rip = int(line, 16)
        i = bisect.bisect_right(starts, rip) - 1
        if i >= 0 and syms[i][0] <= rip < syms[i][1]:
            counts[syms[i][2]] += 1
        else:
            counts["<no symbol: padding or split region>"] += 1

    print(f"total samples: {total}  (in-target {total - ext}, "
          f"outside {ext} = {100.0 * ext / total:.1f}%)")
    in_target = total - ext
    for name, c in counts.most_common(18):
        print(f"{100.0 * c / in_target:6.2f}%  {c:6d}  {name[:88]}")


if __name__ == "__main__":
    main()
