#!/usr/bin/env python3
"""Self-test for sym.py.

Runs the symbolizer on synthetic captures. Two behaviours are pinned, both of
which were wrong once: appended captures must not reuse the previous capture's
mappings, and a binary named through a symlink must still be recognised as the
executable rather than reported as a shared library.

    python3 bench/tools/test_sym.py
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SYM = os.path.join(HERE, "sym.py")


def symbolize(lines, binary):
    fd, path = tempfile.mkstemp(suffix=".samples")
    try:
        with os.fdopen(fd, "w") as f:
            f.write("\n".join(lines) + "\n")
        out = subprocess.run([sys.executable, SYM, path, binary, "10"],
                             capture_output=True, text=True)
    finally:
        os.unlink(path)
    assert out.returncode == 0, out.stderr
    return out.stdout


def test_appended_captures_use_their_own_mappings():
    with tempfile.TemporaryDirectory() as d:
        exe = os.path.join(d, "prog")
        open(exe, "w").close()
        out = symbolize([
            "BASE 1000",
            f"MAP 1000 2000 {exe}",
            "MAP 3000 4000 /lib/libfirst.so",
            "3100",
            "BASE 5000",
            f"MAP 5000 6000 {exe}",
            "MAP 3000 4000 /lib/libsecond.so",
            "3100",
        ], exe)
    assert "[libfirst.so]" in out, out
    assert "[libsecond.so]" in out, out
    assert out.count("100.0%") == 0, out


def test_symlinked_binary_is_the_executable():
    with tempfile.TemporaryDirectory() as d:
        real = os.path.join(d, "real.bin")
        open(real, "w").close()
        alias = os.path.join(d, "alias.bin")
        os.symlink(real, alias)
        # The mapping names the file the kernel resolved, which is `real.bin`;
        # the symbolizer is invoked with the `alias.bin` path.
        out = symbolize([
            "BASE 0",
            f"MAP 1000 2000 {real}",
            "1500",
        ], alias)
    # The bug put the executable's own samples in a library bucket, named after
    # the mapping path (`real.bin`) rather than the path we were given.
    assert "[real.bin]" not in out, out
    assert "[alias.bin]" not in out, out
    assert "1 samples, 1 outside any symbol" in out, out


def test_same_named_library_is_still_a_library():
    with tempfile.TemporaryDirectory() as d:
        exe_dir = os.path.join(d, "bin")
        lib_dir = os.path.join(d, "lib")
        os.makedirs(exe_dir)
        os.makedirs(lib_dir)
        exe = os.path.join(exe_dir, "prog")
        lib = os.path.join(lib_dir, "prog")
        open(exe, "w").close()
        open(lib, "w").close()
        out = symbolize([
            "BASE 0",
            f"MAP 1000 2000 {exe}",
            f"MAP 3000 4000 {lib}",
            "3100",
        ], exe)
    assert "[prog]" in out, out
    assert "1 samples, 0 outside any symbol" in out, out


if __name__ == "__main__":
    test_appended_captures_use_their_own_mappings()
    test_symlinked_binary_is_the_executable()
    test_same_named_library_is_still_a_library()
    print("ok")
