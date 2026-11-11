#!/usr/bin/env python3
"""Fair back-end benchmark: one source, two back ends.

The `run_mir.py` harness compares rmc's MIR back end against a *different*
LLVM program: the LLVM side is built from the `bench/rust/*.rs` `std` sources
while the MIR side is built from the `bench/mir/*.rs` freestanding twins, so
the two are not the same input and the run-time numbers are not comparable.

This harness removes that variable. Both back ends compile the **same**
`bench/mir/*.rs` source and link the **same** freestanding runtime
(`rmc --emit-runtime-asm`, assembled with `as`):

  * ``mir``  -- ``rustc --emit=mir -O`` then ``rmcmir`` (rmc's in-process
                assembler/linker), the whole point of the experiment;
  * ``llvm`` -- ``rustc --emit=obj -O -C lto=fat -C codegen-units=1`` then
                ``ld`` with rmc's runtime, i.e. LLVM's best codegen for the
                identical program.

Both link against the same `print_i64`/`arg_i64`/`alloc` runtime and the same
panic stubs (both trap with exit 101), so a run-time difference is the code
generator, not the standard library.

Measured per pipeline: compile wall time, compiler CPU, compiler peak RSS,
artifact size, and the produced binary's run-time wall/CPU/RSS.
"""
import os
import subprocess
import sys
import time
import shutil

# `rmc` drives rustc with `-Zno-codegen` so the front end stops after the MIR
# optimisations instead of preparing codegen it will never use. The flag is
# nightly-only, so rmc sets `RUSTC_BOOTSTRAP=1` on the child rustc (see
# `front_end_mir` in compiler/src/main.rs); the harness mirrors that so the
# `mir` column measures the pipeline rmc actually runs.
os.environ.setdefault("RUSTC_BOOTSTRAP", "1")

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(ROOT)
RMC = os.path.join(PROJ, "compiler", "target", "release", "rmc")
RMCMIR = os.path.join(PROJ, "compiler", "target", "release", "examples", "rmcmir")
OUT = "/tmp/rmcfair"

PROGRAMS = ["hello", "fib", "mandel", "sieve", "mix", "stats", "refs",
            "heap", "callrec", "forloops"]
ARGS = {
    "hello": [],
    "fib": ["32"],
    "mandel": ["800"],
    "sieve": ["200000"],
    "mix": ["2000000"],
    "stats": ["2000000"],
    "refs": ["5000000"],
    "heap": ["200000"],
    "callrec": ["30"],
    "forloops": ["2000000"],
}

RUSTC_MIR = ["rustc", "--emit=mir", "-O", "-Zno-codegen", "-C", "panic=abort",
             "--crate-type=bin", "--edition", "2021"]
RUSTC_OBJ = ["rustc", "--emit=obj", "-O", "-C", "opt-level=3", "-C", "lto=fat",
             "-C", "codegen-units=1", "-C", "panic=abort", "--edition", "2021"]

# Panic entry points the LLVM object references (the `mir` back end lowers the
# same checks to its own trap). Both paths exit 101, so the runtime comparison
# stays apples to apples.
PANIC_SYMS = [
    "core::panicking::panic_bounds_check",
    "core::panicking::panic_const::panic_const_rem_overflow",
    "core::panicking::panic_const::panic_const_div_overflow",
    "core::panicking::panic_const::panic_const_add_overflow",
    "core::panicking::panic_const::panic_const_mul_overflow",
    "core::panicking::panic_const::panic_const_sub_overflow",
    "core::panicking::panic_const::panic_const_neg_overflow",
    "core::panicking::panic_const::panic_const_shl_overflow",
    "core::panicking::panic_const::panic_const_shr_overflow",
]


def peak_rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def measure(argv, reps, cwd=None):
    exe = shutil.which(argv[0]) or argv[0]
    walls, cpus = [], []
    # Time the child with no other work in the loop. Sampling `VmHWM` in a
    # busy-wait while the child runs adds syscall and scheduling overhead to the
    # very interval being measured and lets ambient load leak in: a run that
    # should take 3 ms could report 6. The CPU time comes from `wait4`'s rusage
    # and is exact; wall time is a clean `perf_counter` bracket. RSS is sampled
    # in its own pass below.
    for _ in range(reps):
        t0 = time.perf_counter()
        pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ,
                             file_actions=[])
        done, status, ru = os.wait4(pid, 0)
        walls.append(time.perf_counter() - t0)
        cpus.append(ru.ru_utime + ru.ru_stime)
        if status != 0:
            raise RuntimeError(f"run failed for {exe}: status={status}")
    # Peak RSS in its own pass. A short-lived process can exit before the first
    # `VmHWM` sample; retry until a nonzero peak is seen, the same way
    # `run_mir.py`'s sampler does. (When even the retries miss - a
    # sub-millisecond binary like `refs` can exit between two reads - the
    # verdict treats the 0 as unmeasured rather than as a loss; see `main`.)
    rss = 0
    for _ in range(max(3, reps)):
        pid = os.posix_spawn(exe, [exe] + list(argv[1:]), os.environ,
                             file_actions=[])
        peak = 0
        while True:
            hwm = peak_rss_kb(pid)
            if hwm > peak:
                peak = hwm
            done, status, ru = os.wait4(pid, os.WNOHANG)
            if done:
                break
            os.sched_yield()
        if status != 0:
            raise RuntimeError(f"run failed for {exe}: status={status}")
        if peak > rss:
            rss = peak
    return dict(wall=min(walls), cpu=min(cpus), rss=rss)


def build_runtime():
    asm = subprocess.run([RMC, "--emit-runtime-asm"], capture_output=True,
                         check=True).stdout
    asm_path = os.path.join(OUT, "runtime.s")
    with open(asm_path, "wb") as f:
        f.write(asm)
    obj = os.path.join(OUT, "runtime.o")
    subprocess.run(["as", "--64", "-o", obj, asm_path], check=True)

    stubs = ["\t.text", "\t.globl __rmc_trap_stub", "__rmc_trap_stub:",
             "\tmov $60, %rax", "\tmov $101, %rdi", "\tsyscall"]
    for s in PANIC_SYMS:
        stubs += [f'\t.globl "{s}"', f'"{s}":', "\tjmp __rmc_trap_stub"]
    # `lto=fat` mangles the symbols, so match the mangled name too.
    stubs += ['\t.globl _RNvNtCskADBYZwJmSU_4core9panicking18panic_bounds_check',
              '_RNvNtCskADBYZwJmSU_4core9panicking18panic_bounds_check:',
              "\tjmp __rmc_trap_stub",
              '\t.globl _RNvNtNtCskADBYZwJmSU_4core9panicking11panic_const24panic_const_rem_overflow',
              '_RNvNtNtCskADBYZwJmSU_4core9panicking11panic_const24panic_const_rem_overflow:',
              "\tjmp __rmc_trap_stub"]
    stub_s = os.path.join(OUT, "panic_stubs.s")
    with open(stub_s, "w") as f:
        f.write("\n".join(stubs) + "\n")
    stub_o = os.path.join(OUT, "panic_stubs.o")
    subprocess.run(["as", "--64", "-o", stub_o, stub_s], check=True)
    return obj, stub_o


def measure_mir(name, runtime_obj, reps=5):
    src = os.path.join(ROOT, "mir", f"{name}.rs")
    mir = os.path.join(OUT, f"{name}.mir")
    out = os.path.join(OUT, f"{name}.mir.bin")
    fe = measure(RUSTC_MIR + [src, "-o", mir], reps)
    be = measure([RMCMIR, mir, "-o", out], reps)
    return {
        "wall": fe["wall"] + be["wall"], "cpu": fe["cpu"] + be["cpu"],
        "rss": max(fe["rss"], be["rss"]),
        "front_wall": fe["wall"], "back_wall": be["wall"],
        "size": os.path.getsize(out),
    }


def measure_llvm(name, runtime_obj, stub_obj, reps=5):
    src = os.path.join(ROOT, "mir", f"{name}.rs")
    obj = os.path.join(OUT, f"{name}.llvm.o")
    out = os.path.join(OUT, f"{name}.llvm.bin")
    c = measure(RUSTC_OBJ + [src, "-o", obj], reps)
    ld = measure(["ld", "-o", out, "-e", "_start", obj, runtime_obj, stub_obj],
                 reps)
    return {
        "wall": c["wall"] + ld["wall"], "cpu": c["cpu"] + ld["cpu"],
        # The peak of the *whole* pipeline, `ld` included, so the axis matches
        # the `mir` column (which counts `rmcmir`). Counting only the compiler
        # would credit LLVM with a link step that still allocates its own RSS.
        "rss": max(c["rss"], ld["rss"]),
        "front_wall": c["wall"], "back_wall": ld["wall"],
        "size": os.path.getsize(out),
    }


def run_binary(path, args, reps=25):
    # The binary's wall/CPU is the headline runtime number, so give it more
    # samples than a compile step; `measure` returns the minimum of each, and
    # more samples means a smaller, more stable minimum.
    return measure([path] + args, reps)


def main():
    os.makedirs(OUT, exist_ok=True)
    runtime_obj, stub_obj = build_runtime()
    rows = []
    for p in PROGRAMS:
        res = {"mir": measure_mir(p, runtime_obj),
               "llvm": measure_llvm(p, runtime_obj, stub_obj)}
        for kind in ("mir", "llvm"):
            res[kind]["run"] = run_binary(os.path.join(OUT, f"{p}.{kind}.bin"),
                                          ARGS[p])
        rows.append((p, res))

    axes = ("wall", "cpu", "rss", "size", "run_wall", "run_cpu", "run_rss")
    hdr = (f"{'program':<9} {'be':<5} {'wall_ms':>9} {'cpu_ms':>9} "
           f"{'rss_kb':>9} {'size_b':>9} {'run_ms':>9} {'run_cpu':>9} "
           f"{'run_rss':>9}")
    print(hdr)
    print("-" * len(hdr))
    for p, res in rows:
        for kind in ("mir", "llvm"):
            r = res[kind]
            print(f"{p:<9} {kind:<5} {r['wall']*1000:9.2f} {r['cpu']*1000:9.2f} "
                  f"{r['rss']:9.0f} {r['size']:9.0f} {r['run']['wall']*1000:9.2f} "
                  f"{r['run']['cpu']*1000:9.2f} {r['run']['rss']:9.0f}")

    print()
    print("speedup mir vs llvm (>1 means mir is better)")
    print(f"{'program':<9} " + " ".join(f"{a:>9}" for a in axes))
    all_ok = True
    for p, res in rows:
        m, l = res["mir"], res["llvm"]
        vals = []
        ok = True
        for a in axes:
            if a == "size":
                mv, lv = m["size"], l["size"]
            elif a.startswith("run_"):
                mv, lv = m["run"][a[4:]], l["run"][a[4:]]
            else:
                mv, lv = m[a], l[a]
            # A zero means the sample never landed (see `measure`); a missing
            # number is not a loss, so leave it out of the verdict rather than
            # scoring it 0 and flagging a spurious `<-- LOSS`. `nan` also prints
            # as `n/a` and, being unequal to itself, skips the `< 1.0` test.
            ratio = lv / mv if mv else float("nan")
            vals.append(ratio)
            if ratio < 1.0:
                ok = False
        all_ok = all_ok and ok
        mark = "" if ok else "  <-- LOSS"
        cells = " ".join("      n/a" if v != v else f"{v:9.2f}" for v in vals)
        print(f"{p:<9} {cells}{mark}")
    print()
    print("ALL MIR WINS" if all_ok else "some axes lost")


if __name__ == "__main__":
    sys.exit(main())
