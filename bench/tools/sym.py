#!/usr/bin/env python3
"""Turn sampler output into a per-symbol histogram.

    ./libsampler.so is preloaded into the target (see sampler.c), which writes
    one instruction pointer per sample. This maps each pointer to the nearest
    preceding symbol from `nm -S --defined-only` and prints the top symbols.

    Samples that landed in a shared library are attributed to that library, so
    a profile does not silently lose the time spent in libc/libm.

    usage: sym.py <samples> <binary> [top] [--exe <captured-exe-path>]

    Each capture records the size and CRC-32 of its executable (`EXEID`), so a
    capture appended by a *different* binary is not symbolized against the
    wrong symbol table: those samples are reported as `[other-exe]` instead.

    The capture marks its executable mappings (`MAP <start> <end> exe|lib
    <path>`), so the histogram is correct even when <binary> is a copy kept
    next to an archived profile. For captures made before that marker existed,
    pass `--exe` with the executable path recorded in the capture.
"""
import os
import subprocess
import sys
import zlib
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


def file_id(path):
    """Size and CRC-32 of the executable, as recorded by the sampler.

    Content-based, so a copy kept next to an archived profile still matches.
    """
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError:
        return None
    return (len(data), zlib.crc32(data) & 0xFFFFFFFF)


def parse_args(argv):
    exe_override = None
    rest = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "--exe" and i + 1 < len(argv):
            exe_override = argv[i + 1]
            i += 2
        elif a.startswith("--exe="):
            exe_override = a.split("=", 1)[1]
            i += 1
        else:
            rest.append(a)
            i += 1
    if len(rest) < 2:
        sys.exit(__doc__.strip())
    top = int(rest[2]) if len(rest) > 2 else 20
    return rest[0], rest[1], top, exe_override


def main():
    samples, binary, top, exe_override = parse_args(sys.argv[1:])
    syms = load_symbols(binary)
    starts = [s[0] for s in syms]
    counts = Counter()
    total = 0
    unknown = 0
    base = 0
    maps = []
    cur_id = None
    binary_id = file_id(binary)
    mismatched = 0
    other_ids = set()
    # `nm` can only name the executable's own symbols, so the question "is this
    # mapping the executable" decides between symbol lookup and a library
    # bucket. A capture answers it with the `exe` tag; for an untagged capture
    # fall back to comparing paths, where `--exe` supplies the path recorded at
    # capture time (which differs when the binary was archived elsewhere).
    exe_path = os.path.realpath(exe_override) if exe_override else os.path.realpath(binary)
    for line in open(samples):
        line = line.strip()
        if not line:
            continue
        if line.startswith("BASE "):
            base = int(line.split()[1], 16)
            cur_id = None
            # Each appended capture has its own layout, so the mappings of the
            # previous one must not be used to attribute this one's addresses.
            maps.clear()
            continue
        if line.startswith("EXEID "):
            _, size, crc = line.split()
            cur_id = (int(size), int(crc, 16))
            continue
        if line.startswith("MAP "):
            start, end, tail = line[4:].split(" ", 2)
            tag = ""
            if tail.startswith("exe ") or tail.startswith("lib "):
                tag, tail = tail.split(" ", 1)
            maps.append((int(start, 16), int(end, 16), tag, tail))
            continue
        ip = int(line, 16)
        total += 1
        owner = next((m for m in maps if m[0] <= ip < m[1]), None)
        if owner is not None:
            tag, path = owner[2], owner[3]
            if tag == "exe" and cur_id is not None and binary_id is not None:
                # The capture knows which binary produced this run; if it is
                # not the one whose symbols we were given, its offsets mean
                # something else and must not be resolved against them.
                if cur_id != binary_id:
                    counts["[other-exe]"] += 1
                    mismatched += 1
                    other_ids.add(f"{cur_id[0]}/{cur_id[1]:08x}")
                    continue
            else:
                is_exe = tag == "exe" if tag else os.path.realpath(path) == exe_path
                if not is_exe:
                    counts[f"[{os.path.basename(path)}]"] += 1
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
    if mismatched:
        print(f"{mismatched} samples from a different executable ({', '.join(sorted(other_ids))})")
    for name, n in counts.most_common(top):
        print(f"{100.0 * n / max(total, 1):6.1f}%  {n:6d}  {name}")


if __name__ == "__main__":
    main()
