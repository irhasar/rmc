# Validasi backend pada crate pihak ketiga nyata

Tiga crate dari `github.com/cipansor/crateworks` (murni Rust, tanpa dependensi
eksternal, `#![forbid(unsafe_code)]`) dipakai untuk memvalidasi hasil benchmark
kernel sintetis pada kode produksi. Lihat `REPORT.md` §8.24 untuk hasilnya.

Crate yang diuji:

| crate | peran |
|-------|-------|
| `openlocationcode` | aritmetika floating-point + string (encode/decode/shorten/recover) |
| `holidays-rs` | pencarian tanggal + tabel besar (holiday/working-day queries) |
| `stdnum-rs` | checksum/parsing identifier (damm, ean, luhn, verhoeff, iban, isbn) |

## Menyiapkan sumber

```
git clone --depth 1 https://github.com/cipansor/crateworks.git
```

Harness mengasumsikan `crateworks/` ada di dalam direktori ini (yakni
`extbench/crateworks/crates/...`).

## Menjalankan

```
CREPS=3 RREPS=5 ./extbench.sh     # compile + run, tiga backend
./exttests.sh                     # test suite milik crate, tiga backend
```

Backend: `L` = `rustc +nightly-2026-11-17` (LLVM), `U` = cg_clif upstream
(`../cg_clif_upstream/dist/bin/rustc-clif`), `M` = fork modifikasi
(`../cg_clif_mod/dist/bin/rustc-clif`).

Direktori `dist/` tidak ikut masuk repo (hasil build). Bangun dulu lewat
`../build.sh` (untuk `cg_clif_mod`) dan build yang sama untuk `cg_clif_upstream`,
atau arahkan lewat variabel lingkungan:

```
RUSTC_U=/path/ke/cg_clif_upstream/dist/bin/rustc-clif \
RUSTC_M=/path/ke/cg_clif_mod/dist/bin/rustc-clif \
CRATEWORKS=/path/ke/crateworks \
BENCH_DIR=/path/ke/bench \
  ./extbench.sh
```

`exttests.sh` menerima `RUSTC_U`, `RUSTC_M`, dan `CRATEWORKS` yang sama.

Setiap crate dibangun sebagai `--crate-type lib -C opt-level=3 --edition 2024`,
lalu driver kecil di `drivers/` di-link dengan `--extern` dan dijalankan. Semua
pengukuran memakai `bench/measure.py` (`os.wait4` rusage: wall, CPU, peak RSS).
