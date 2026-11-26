#!/usr/bin/env python3
"""Self-test for sym.py.

The fixtures are real binaries. A capture that never resolves to a function
name would let a broken symbolizer pass, which is what happened to the first
version of the archive tests.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SYM = os.path.join(HERE, "sym.py")

# `#[inline(never)]` plus a call through a function pointer keeps the symbol
# in the binary; a tiny exported function is otherwise inlined away and there
# is nothing left for the symbolizer to resolve.
SRC = """
#[no_mangle]
#[inline(never)]
pub extern "C" fn bench_marker(x: u64) -> u64 {
    x.wrapping_mul(3).wrapping_add(%d)
}

fn main() {
    let f: extern "C" fn(u64) -> u64 = bench_marker;
    println!("{}", std::hint::black_box(f)(1));
}
"""

FIXTURES = {}
BASE = 0x400000
SPAN = 0x10000000


def build_fixture(name, salt):
    """Compile a real binary so samples can resolve to a function name."""
    d = FIXTURES["dir"]
    src = os.path.join(d, name + ".rs")
    binary = os.path.join(d, name)
    with open(src, "w") as f:
        f.write(SRC % salt)
    subprocess.run(
        ["rustc", "--edition", "2021", "-O", src, "-o", binary],
        check=True,
        capture_output=True,
    )
    FIXTURES[name] = binary
    return binary


def symbol_value(binary, name):
    out = subprocess.run(
        ["nm", "-S", "--defined-only", binary],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    for line in out.splitlines():
        parts = line.split()
        if parts and parts[-1] == name:
            return int(parts[0], 16)
    raise AssertionError(f"{name} not found in {binary}")


def file_id(binary):
    with open(binary, "rb") as f:
        data = f.read()
    return (len(data), zlib.crc32(data) & 0xFFFFFFFF)


def exeid(binary):
    size, crc = file_id(binary)
    return f"EXEID {size} {crc:x}"


def symbolize(lines, binary, extra=()):
    fd, path = tempfile.mkstemp(suffix=".samples")
    try:
        with os.fdopen(fd, "w") as f:
            f.write("\n".join(lines) + "\n")
        out = subprocess.run(
            [sys.executable, SYM, path, binary, "10", *extra],
            capture_output=True,
            text=True,
        )
    finally:
        os.unlink(path)
    assert out.returncode == 0, out.stderr
    return out.stdout


def capture(binary, sample_offsets, tag="exe", path=None, with_id=True):
    lines = [f"BASE {BASE:x}"]
    if with_id:
        lines.append(exeid(binary))
    lines.append(f"MAP {BASE:x} {BASE + SPAN:x} {tag} {path or binary}")
    lines += [f"{BASE + off:x}" for off in sample_offsets]
    return lines


def test_resolves_a_real_symbol():
    exe = FIXTURES["alpha"]
    off = symbol_value(exe, "bench_marker")
    out = symbolize(capture(exe, [off]), exe)
    assert "bench_marker" in out, out
    assert "1 samples, 0 outside any symbol" in out, out


def test_tagged_executable_survives_relocation():
    """An archived profile names the capture-time path; the tag still wins."""
    exe = FIXTURES["alpha"]
    off = symbol_value(exe, "bench_marker")
    copy = os.path.join(FIXTURES["dir"], "archived_alpha")
    shutil.copyfile(exe, copy)
    out = symbolize(capture(exe, [off]), copy)
    assert "bench_marker" in out, out
    assert "[" not in out, out


def test_exe_override_for_untagged_capture():
    exe = FIXTURES["alpha"]
    off = symbol_value(exe, "bench_marker")
    copy = os.path.join(FIXTURES["dir"], "archived_alpha2")
    shutil.copyfile(exe, copy)
    out = symbolize(capture(exe, [off], with_id=False), copy, extra=["--exe", exe])
    assert "bench_marker" in out, out
    assert "[" not in out, out


def test_mixed_captures_do_not_share_symbols():
    """Two runs of different binaries: only the matching one is symbolized."""
    alpha = FIXTURES["alpha"]
    beta = FIXTURES["beta"]
    off_a = symbol_value(alpha, "bench_marker")
    off_b = symbol_value(beta, "bench_marker")
    lines = capture(alpha, [off_a]) + capture(beta, [off_b])
    out = symbolize(lines, alpha)
    assert "bench_marker" in out, out
    assert "2 samples, 0 outside any symbol" in out, out
    assert "1 samples from a different executable" in out, out
    assert "[other-exe]" in out, out


def test_same_named_library_is_still_a_library():
    exe = FIXTURES["alpha"]
    lib_dir = os.path.join(FIXTURES["dir"], "lib")
    os.makedirs(lib_dir, exist_ok=True)
    lib = os.path.join(lib_dir, "alpha")
    shutil.copyfile(exe, lib)
    lines = [
        f"BASE {BASE:x}",
        exeid(exe),
        f"MAP {BASE:x} {BASE + 0x1000:x} exe {exe}",
        f"MAP {BASE + 0x2000:x} {BASE + 0x3000:x} lib {lib}",
        f"{BASE + 0x2100:x}",
    ]
    out = symbolize(lines, exe)
    assert "[alpha]" in out, out
    assert "1 samples, 0 outside any symbol" in out, out


def test_symlinked_binary_is_the_executable():
    exe = FIXTURES["alpha"]
    off = symbol_value(exe, "bench_marker")
    link = os.path.join(FIXTURES["dir"], "alpha_link")
    if os.path.lexists(link):
        os.unlink(link)
    os.symlink(exe, link)
    out = symbolize(capture(exe, [off], with_id=False), link)
    assert "bench_marker" in out, out
    assert "[" not in out, out


def test_appended_captures_use_their_own_mappings():
    exe = FIXTURES["alpha"]
    off = symbol_value(exe, "bench_marker")
    lib_dir = os.path.join(FIXTURES["dir"], "lib2")
    os.makedirs(lib_dir, exist_ok=True)
    lib = os.path.join(lib_dir, "libfirst.so")
    shutil.copyfile(exe, lib)
    lines = [
        f"BASE {BASE:x}",
        f"MAP {BASE:x} {BASE + SPAN:x} exe {exe}",
        f"{BASE + off:x}",
        f"BASE {BASE + SPAN:x}",
        f"MAP {BASE + SPAN:x} {BASE + 2 * SPAN:x} lib {lib}",
        f"{BASE + SPAN + 0x100:x}",
    ]
    out = symbolize(lines, exe)
    assert "bench_marker" in out, out
    assert "[libfirst.so]" in out, out
    assert "2 samples, 0 outside any symbol" in out, out


if __name__ == "__main__":
    tmp = tempfile.mkdtemp(prefix="sym-test-")
    FIXTURES["dir"] = tmp
    try:
        build_fixture("alpha", 1)
        build_fixture("beta", 2)
        test_resolves_a_real_symbol()
        test_tagged_executable_survives_relocation()
        test_exe_override_for_untagged_capture()
        test_mixed_captures_do_not_share_symbols()
        test_same_named_library_is_still_a_library()
        test_symlinked_binary_is_the_executable()
        test_appended_captures_use_their_own_mappings()
        print("ok")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
