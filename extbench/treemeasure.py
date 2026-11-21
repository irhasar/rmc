#!/usr/bin/env python3
"""Measure a whole process tree: wall, CPU (summed over all descendants) and
peak summed RSS. cargo spawns rustc children, so wait4's ru_maxrss (direct
child only) understates build cost badly. This samples /proc for every
descendant and reports the peak of the summed RSS.

Usage: treemeasure.py [--reps N] [--warmup N] [--json] -- cmd...
"""
import argparse
import json
import os
import statistics
import subprocess
import sys
import time


def descendants(root_pid):
    """All live descendant pids of root_pid, from /proc/<pid>/stat ppid links."""
    children = {}
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        try:
            with open(f"/proc/{name}/stat", "rb") as f:
                data = f.read()
            # comm may contain spaces/parens; ppid is field 4 after the last ')'
            rest = data[data.rfind(b")") + 2:].split()
            ppid = int(rest[1])
        except (OSError, IndexError, ValueError):
            continue
        children.setdefault(ppid, []).append(int(name))
    out, stack = [], [root_pid]
    while stack:
        for c in children.get(stack.pop(), ()):
            out.append(c)
            stack.append(c)
    return out


def tree_rss_kb(root_pid):
    """Summed RSS of root and all descendants, in KB."""
    total = 0
    pids = [root_pid] + descendants(root_pid)
    for pid in pids:
        try:
            with open(f"/proc/{pid}/statm", "rb") as f:
                total += int(f.read().split()[1]) * (os.sysconf("SC_PAGE_SIZE") // 1024)
        except (OSError, IndexError, ValueError):
            pass
    return total


def pid_cpu(pid):
    """utime+stime of one pid in seconds, or None if it is gone."""
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            data = f.read()
        rest = data[data.rfind(b")") + 2:].split()
        return (int(rest[11]) + int(rest[12])) / os.sysconf("SC_CLK_TCK")
    except (OSError, IndexError, ValueError):
        return None


def run_once(cmd, env=None, cwd=None, interval=0.01):
    t0 = time.perf_counter()
    proc = subprocess.Popen(cmd, env=env, cwd=cwd,
                            stdin=subprocess.DEVNULL,
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    peak_rss = 0
    # CPU must be accumulated as per-pid deltas: a rustc that finishes between
    # two samples disappears from /proc, and a plain max() would drop its CPU.
    seen = {}
    cpu_total = 0.0
    while proc.poll() is None:
        peak_rss = max(peak_rss, tree_rss_kb(proc.pid))
        for pid in [proc.pid] + descendants(proc.pid):
            cur = pid_cpu(pid)
            if cur is None:
                continue
            cpu_total += max(0.0, cur - seen.get(pid, 0.0))
            seen[pid] = cur
        time.sleep(interval)
    wall = time.perf_counter() - t0
    return {"wall": wall, "cpu": cpu_total, "peak_rss_kb": peak_rss,
            "exit": proc.returncode}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--reps", type=int, default=1)
    ap.add_argument("--warmup", type=int, default=0)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd
    if not cmd:
        sys.exit("no command")

    for _ in range(a.warmup):
        run_once(cmd)
    runs = [run_once(cmd) for _ in range(a.reps)]
    for r in runs:
        if r["exit"] != 0:
            print(f"FAIL exit={r['exit']}", file=sys.stderr)
    res = {
        "reps": a.reps,
        "wall_med": statistics.median(r["wall"] for r in runs),
        "cpu_med": statistics.median(r["cpu"] for r in runs),
        "peak_rss_med_kb": statistics.median(r["peak_rss_kb"] for r in runs),
        "peak_rss_max_kb": max(r["peak_rss_kb"] for r in runs),
        "exit": runs[-1]["exit"],
    }
    if a.json:
        print(json.dumps(res))
    else:
        print(f"wall={res['wall_med']*1000:.0f}ms cpu={res['cpu_med']*1000:.0f}ms "
              f"peakRSS={res['peak_rss_med_kb']/1024:.0f}MB exit={res['exit']}")


if __name__ == "__main__":
    main()
