#!/usr/bin/env python3
"""Turn sampler output into a per-symbol histogram.

    ./libsampler.so is preloaded into the target (see sampler.c), which writes
    one instruction pointer per sample. This maps each pointer to the nearest
    preceding symbol from `nm -S --defined-only` and prints the top symbols.

    usage: sym.py <samples> <binary> [top]
"""
import os
import subprocess
import sys
import bisect
from collections import Counter


def load_symbols(binary):
    out = subprocess.run(["nm", "-S", "--defined-only", binary],
                         capture_output=True, text=True).stdout
    syms = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 4:
            continue
        addr, size, kind, name = parts[0], parts[1], parts[2], " ".join(parts[3:])
        if kind.lower() not in ("t", "w", "v", "d", "b", "r"):
            continue
        try:
            syms.append((int(addr, 16), int(size, 16), name))
        except ValueError:
            continue
    syms.sort()
    return syms


def main():
    samples = sys.argv[1]
    binary = sys.argv[2]
    top = int(sys.argv[3]) if len(sys.argv) > 3 else 20
    syms = load_symbols(binary)
    starts = [s[0] for s in syms]
    counts = Counter()
    total = 0
    unknown = 0
    base = 0
    maps = []
    # A capture may name the binary through a symlink, and the mapping records
    # the file it resolved to; compare whole real paths, not basenames, so a
    # library that happens to share the executable's name is still a library.
    exe_path = os.path.realpath(binary)
    for line in open(samples):
        line = line.strip()
        if not line:
            continue
        if line.startswith("BASE "):
            base = int(line.split()[1], 16)
            # Each appended capture has its own layout, so the mappings of the
            # previous one must not be used to attribute this one's addresses.
            maps.clear()
            continue
        if line.startswith("MAP "):
            _, start, end, path = line.split(" ", 3)
            maps.append((int(start, 16), int(end, 16), path))
            continue
        ip = int(line, 16)
        total += 1
        # `nm` can only name the executable's own symbols. A sample that landed
        # in a shared library (libc's memcpy, the libm calls, ...) is still real
        # work, so attribute it to that library instead of dropping it into the
        # "outside any symbol" bucket.
        owner = next((p for (lo, hi, p) in maps if lo <= ip < hi), None)
        if owner is not None and os.path.realpath(owner) != exe_path:
            counts[f"[{os.path.basename(owner)}]"] += 1
            continue
        ip -= base
        i = bisect.bisect_right(starts, ip) - 1
        if i < 0:
            unknown += 1
            continue
        addr, size, name = syms[i]
        if size and ip >= addr + size:
            unknown += 1
            continue
        counts[name] += 1
    print(f"{total} samples, {unknown} outside any symbol")
    for name, n in counts.most_common(top):
        print(f"{100.0 * n / max(total, 1):6.1f}%  {n:6d}  {name}")


if __name__ == "__main__":
    main()
