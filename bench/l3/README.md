# L3 vectorizer experiments (NOT held-out)

Fixed-size experiments used to inspect the L3 target loop
(`c[i] = (a[i]^b[i])*K`) without the held-out argv contract. They live **outside**
`bench/progs/` on purpose: `bench/heldout.sh` sweeps `progs/h*.rs` with `n:seed`
argv and would mislabel a fixed-size program (a 4M-element workload reported as
10k/50k/200k). Run them with an explicit all-axes glob:

    bench/allaxes.sh "$PWD/bench/l3/l3_*.rs"

Files:
- `l3_iso.rs`     disjoint buffers, trip count divisible by 4 (vector fast path).
- `l3_tail.rs`    trip count 4,000,003 -> exercises the scalar remainder.
- `l3_overlap.rs` dst `v[i+2]` overlaps sources `v[i]`, `v[i+1]` -> disjointness guard must fall back to scalar.
- `l3_bw.rs`      trivial tri-stream reference (same memory traffic as L3).
- `ktime.rs`      timing probe (NOT matched by `l3_*.rs`): times init, the 20
                  kernel passes, and the hash **separately** (internal `Instant`),
                  prints `init_ns= kernel_ns= hash_ns= sum=`. Build BOTH backends
                  (the backend is selected at compile time; running the L binary
                  is not an M result):
                  ```
                  rustc -O bench/l3/ktime.rs -o ktime_L
                  HOST=$(rustc +nightly-2026-11-17 -vV | sed -n 's/^host: //p')
                  env -u RAKIT_HOST_LLVM rakit/dist/bin/rustc-rakit \
                      --target "$HOST" -C opt-level=3 bench/l3/ktime.rs \
                      -o ktime_M --edition 2021
                  ```

Each program hashes **all** output elements, so every store is checked (not only
the last).
