# Usage

rustc_codegen_rakit can be used as a near-drop-in replacement for `cargo build` or `cargo run` for existing projects.

Assuming `$rakit_dir` is the directory you cloned this repo into and you followed the instructions (`y.sh prepare` and `y.sh build` or `test.sh`).

## Cargo

In the directory with your project (where you can do the usual `cargo build`), run:

```bash
$ $rakit_dir/dist/cargo-rakit build
```

This will build your project with rustc_codegen_rakit instead of the usual LLVM backend.

## Rustc

> You should prefer using the Cargo method.

```bash
$ $rakit_dir/dist/rustc-rakit my_crate.rs
```

## Jit mode

> ⚠⚠⚠ The JIT mode is highly experimental. It may be slower than AOT compilation due to lack of incremental compilation. It may also be hard to setup if you have cargo dependencies. ⚠⚠⚠

In jit mode rakit will immediately execute your code without creating an executable file.

> This requires all dependencies to be available as dynamic library.
> The jit mode will probably need cargo integration to make this possible.

```bash
$ $rakit_dir/dist/cargo-rakit jit
```

or

```bash
$ $rakit_dir/dist/rustc-rakit -Cllvm-args=jit-mode -Cprefer-dynamic my_crate.rs
```

## Shell

These are a few functions that allow you to easily run rust code from the shell using rakit as jit.

```bash
function jit_naked() {
    echo "$@" | $rakit_dir/dist/rustc-rakit - -Zunstable-options -Cllvm-args=jit-mode -Cprefer-dynamic
}

function jit() {
    jit_naked "fn main() { $@ }"
}

function jit_calc() {
    jit 'println!("0x{:x}", ' $@ ');';
}
```
