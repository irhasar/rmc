#!/usr/bin/env bash
set -e

cd "$(dirname "$0")/../"

source ./scripts/setup_rust_fork.sh

echo "[TEST] Bootstrap of rustc"
pushd rust
rm -r compiler/rustc_codegen_rakit/{Cargo.*,src}
cp ../Cargo.* compiler/rustc_codegen_rakit/
cp -r ../src compiler/rustc_codegen_rakit/src

./x.py build --stage 1 library/std
popd
