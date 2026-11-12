#!/bin/sh
# Build rmc as a rustc `-Zcodegen-backend` dylib.
#
# Needs the nightly toolchain with the `rustc-dev` component (the backend links
# `rustc_private` crates). At link time the toolchain's own LLVM shared object
# is needed too; it lives in the toolchain's `lib` directory, which rustc does
# not search by default, hence the explicit `-L`.
#
#   rustup toolchain install nightly --component rustc-dev
#   ./backend/build.sh
#
# Then:
#   ./backend/rmc-rustc foo.rs -o foo     # wrapper that sets the paths
set -eu
here=$(cd "$(dirname "$0")" && pwd)
sysroot=$(rustc +nightly --print sysroot)
# `-L` lets the link find `librustc_driver`; the rpath is baked in so the
# wrapper does not have to run `rustc --print sysroot` on every invocation
# (that extra process was the entire compile-time cost of the `cb` pipeline).
RUSTFLAGS="-L $sysroot/lib -C link-arg=-Wl,-rpath,$sysroot/lib" \
    cargo +nightly build --release --manifest-path "$here/Cargo.toml"
