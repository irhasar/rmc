#!/bin/bash
# Build the modified Rakit backend (binary + Rakit-compiled std sysroot).
#
# Prerequisites: rustup toolchain nightly-2026-11-17 with rust-src, rustc-dev
# and llvm-tools, plus the build deps listed in rakit/Readme.md.
set -eu
cd "$(dirname "$0")/rakit"
export PATH="$HOME/.cargo/bin:$PATH"
./y.sh build --sysroot rakit
echo "Done. Use dist/bin/rustc-rakit (and its own sysroot) for the modified backend."
