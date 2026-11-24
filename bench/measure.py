#!/usr/bin/env python3
"""Measure wall time, CPU time and peak RSS of a command, N repetitions.
Uses os.wait4 for exact child rusage (ru_maxrss = peak RSS in KB on Linux)."""
import os, sys, time, json, resource, statistics, tempfile

def run_once(cmd, env=None, stdin_devnull=True, cwd=None, out_path=None):
    t0 = time.perf_counter()
    pid = os.fork()
    if pid == 0:
        try:
            if stdin_devnull:
                fd = os.open(os.devnull, os.O_RDONLY); os.dup2(fd, 0)
            if out_path is None:
                out = os.open(os.devnull, os.O_WRONLY)
            else:
                out = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            os.dup2(out, 1); os.dup2(out, 2)
            if cwd: os.chdir(cwd)
            os.execvpe(cmd[0], cmd, env if env is not None else os.environ)
        except Exception as e:
            os._exit(127)
    _, status, ru = os.wait4(pid, 0)
    wall = time.perf_counter() - t0
    if os.WIFEXITED(status):
        code = os.WEXITSTATUS(status)
    else:
        code = -os.WTERMSIG(status)
    return {"wall": wall, "user": ru.ru_utime, "sys": ru.ru_stime,
            "maxrss_kb": ru.ru_maxrss, "exit": code}

def same_output(path, expect):
    try:
        with open(path, "rb") as a, open(expect, "rb") as b:
            return a.read() == b.read()
    except OSError:
        return False

def bench(name, cmd, reps=5, warmup=1, env=None, cwd=None, expect=None):
    first_bad = None
    output_ok = True

    def checked(kind):
        nonlocal output_ok
        if expect is None:
            return run_once(cmd, env=env, cwd=cwd)
        fd, path = tempfile.mkstemp()
        os.close(fd)
        try:
            r = run_once(cmd, env=env, cwd=cwd, out_path=path)
            if not same_output(path, expect):
                print(f"OUTPUT MISMATCH {name}: {kind}", file=sys.stderr)
                output_ok = False
            return r
        finally:
            os.unlink(path)

    for _ in range(warmup):
        r = checked("warmup")
        if r["exit"] != 0:
            print(f"WARMUP FAIL {name}: exit={r['exit']}", file=sys.stderr)
            if first_bad is None:
                first_bad = r["exit"]
    runs = []
    for _ in range(reps):
        r = checked("run")
        runs.append(r)
        if r["exit"] != 0:
            print(f"RUN FAIL {name}: {r}", file=sys.stderr)
            if first_bad is None:
                first_bad = r["exit"]
    def med(k): return statistics.median(x[k] for x in runs)
    def mn(k): return min(x[k] for x in runs)
    return {"name": name, "reps": reps,
            "wall_med": med("wall"), "wall_min": mn("wall"),
            "user_med": med("user"), "sys_med": med("sys"),
            "cpu_med": med("user") + med("sys"),
            "maxrss_med_kb": med("maxrss_kb"), "maxrss_min_kb": mn("maxrss_kb"),
            "exit": first_bad if first_bad is not None else 0,
            "output_ok": output_ok,
            "valid": first_bad is None and output_ok}

if __name__ == "__main__":
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument("--reps", type=int, default=5)
    p.add_argument("--warmup", type=int, default=1)
    p.add_argument("--json", action="store_true")
    p.add_argument("--label", default="")
    p.add_argument("--expect", help="require every warmup/run to match this output file")
    p.add_argument("cmd", nargs=argparse.REMAINDER)
    a = p.parse_args()
    cmd = a.cmd
    if cmd and cmd[0] == "--": cmd = cmd[1:]
    res = bench(a.label or " ".join(cmd), cmd, reps=a.reps, warmup=a.warmup,
                expect=a.expect)
    if a.json:
        print(json.dumps(res))
    else:
        print(f"{res['name']}: wall={res['wall_med']*1000:.1f}ms (min {res['wall_min']*1000:.1f}) "
              f"cpu={res['cpu_med']*1000:.1f}ms rss={res['maxrss_med_kb']/1024:.1f}MB exit={res['exit']}")
    sys.exit(0 if res["valid"] else 1)
