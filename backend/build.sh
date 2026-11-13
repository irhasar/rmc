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
# The toolchain is pinned by `backend/rust-toolchain.toml`; rustup selects it
# from the *working directory*, and this script may be invoked from anywhere,
# so cd here for every toolchain query. Read the channel back out rather than
# repeating it, so the pin lives in exactly one place.
cd "$here"
channel=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
sysroot=$(rustc +"$channel" --print sysroot)
# `-L` lets the link find `librustc_driver`; the rpath is baked in so the
# wrapper does not have to run `rustc --print sysroot` on every invocation
# (that extra process was the entire compile-time cost of the `cb` pipeline).
RUSTFLAGS="-L $sysroot/lib -C link-arg=-Wl,-rpath,$sysroot/lib" \
    cargo +"$channel" build --release --manifest-path "$here/Cargo.toml"
