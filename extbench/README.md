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
(`../rakit/dist/bin/rustc-rakit`).

Direktori `dist/` tidak ikut masuk repo (hasil build). Bangun dulu lewat
`../build.sh` (untuk `rakit`) dan build yang sama untuk `cg_clif_upstream`,
atau arahkan lewat variabel lingkungan:

```
RUSTC_U=/path/ke/cg_clif_upstream/dist/bin/rustc-clif \
RUSTC_M=/path/ke/rakit/dist/bin/rustc-rakit \
CRATEWORKS=/path/ke/crateworks \
BENCH_DIR=/path/ke/bench \
  ./extbench.sh
```

`exttests.sh` menerima `RUSTC_U`, `RUSTC_M`, dan `CRATEWORKS` yang sama.

Setiap crate dibangun sebagai `--crate-type lib -C opt-level=3 --edition 2024`,
lalu driver kecil di `drivers/` di-link dengan `--extern` dan dijalankan. Semua
pengukuran memakai `bench/measure.py` (`os.wait4` rusage: wall, CPU, peak RSS).

## Benchmark crate besar: ripgrep 14.1.1

`cratebench.sh` juga dipakai untuk proyek nyata berdependensi banyak. Target
acuan: **ripgrep 14.1.1** (41 dependensi transitif, regex + memchr +
aho-corasick + ignore).

```
git clone --depth 1 --branch 14.1.1 https://github.com/BurntSushi/ripgrep ripgrep
./gen_corpus.sh /tmp/rgcorpus 2000 400        # 2000 file / 400 baris, deterministik
OUTDIR=/tmp/rgout CREPS=2 RREPS=5 BINNAME=rg \
  RUNCMD='exec $PWD/drivers/rg_work.sh "$BIN" /tmp/rgcorpus' \
  ./cratebench.sh ripgrep "" rg
```

`RUNCMD` menjalankan beban kerja yang butuh lebih dari satu argumen; `$BIN`
diganti biner hasil build dan seluruh pohon proses tetap diukur. `OUTDIR`
menyimpan stdout tiap backend dan membandingkannya byte-per-byte di akhir
(`outputs: L == U == M`), sehingga hasil yang berbeda tidak pernah dihitung ok.

`drivers/rg_work.sh` menjalankan lima pencarian (literal, regex ber-capture,
regex case-insensitive, multi-pattern, dan walk direktori) dengan `--sort path`
supaya urutan keluaran tidak bergantung pada urutan walk atau thread.

### Hasil (CREPS=2, RREPS=5; kompilasi = wall/cpu/peak-RSS, ukuran = byte)

| backend | Cwall | Ccpu | Crss | ukuran | Rwall | Rcpu | Rrss |
|---|---|---|---|---|---|---|---|
| L (LLVM) | 27065 ms | 85910 ms | 1102 MB | 30,490,856 | 411 ms | 280 ms | 9 MB |
| U (cg_clif) | 9924 ms | 26930 ms | 904 MB | 35,356,496 | 4869 ms | 4720 ms | 16 MB |
| M (Rakit) | 11451 ms | 32220 ms | 929 MB | 27,906,952 | 3164 ms | 3040 ms | 13 MB |

M vs L: kompilasi **0.42x wall / 0.37x CPU**, RSS 0.84x, ukuran **-8.5%**,
runtime **7.7x lebih lambat**. M vs U: kompilasi 1.15x, ukuran -21%, runtime
**0.65x** (M lebih cepat). Ketiga backend menghasilkan stdout identik
(3.457.671 byte).

Akar gap runtime (bukan pass, melainkan backend): intrinsik 256-bit
(`_mm256_*`) diskalarisasi. Kernel AVX2 `memchr` berisi 0 `%ymm` dan 0 `paddb`
di M, sementara L punya 104 `%ymm` / 68 `pcmpeqb`. Terisolasi pada 2 MB x 5
lintasan: L 0.2 ms, U 21.1 ms, M 16.9 ms. Deteksi fitur CPU
(`is_x86_feature_detected!`) identik di ketiga backend, jadi bukan masalah
deteksi; `-Ctarget-feature=+avx2` (yang kini dipropagasikan, lihat AGENTS.md)
hanya memberi ~1.5% karena penskalaran terjadi di lowering intrinsik.
