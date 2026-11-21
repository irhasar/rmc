#!/bin/bash
# Build the modified cg_clif (binary + Cranelift-compiled std sysroot).
#
# Prerequisites: rustup toolchain nightly-2026-11-17 with rust-src and
# llvm-tools, plus the build deps listed in cg_clif_mod/README.md.
set -eu
cd "$(dirname "$0")/cg_clif_mod"
export PATH="$HOME/.cargo/bin:$PATH"
./y.sh build --sysroot clif
echo "Done. Use dist/bin/rustc-clif (and its own sysroot) for the modified backend."
