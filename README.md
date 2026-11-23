# Rakit — backend codegen Rust berbasis Rakit

Repo ini berisi eksperimen terukur membandingkan tiga backend codegen Rust pada
`nightly-2026-11-17`, `x86_64-unknown-linux-gnu`, `opt-level=3`, `debuginfo=0`:

1. **LLVM** — `rustc` bawaan.
2. **cg_clif upstream** — basis `42ac63e`, tanpa perubahan.
3. **Rakit modifikasi** — fork dengan patch performa di `rakit/`.

Ketiga backend menghasilkan **output program yang identik** (diverifikasi pada
program sintetis, tiga crate nyata, dan repo 4,1 MB), jadi yang diukur adalah
kualitas codegen, bukan semantik.

> **Catatan:** branch ini menggantikan seluruh isi `main`. Isi `main`
> sebelumnya — proyek `rmc`, sebuah compiler Rust-subset ahead-of-time —
> tetap tersedia di riwayat git branch `main`.

## Isi repo

| Path | Keterangan |
|---|---|
| `rakit/` | Backend Rakit yang dimodifikasi; inti pekerjaan ada di sini. |
| `cg_clif_upstream/` | cg_clif upstream (basis `42ac63e`) sebagai baseline. |
| `bench/` | Harness tiga arah: ukuran, waktu compile, runtime. Program `progs/p1..p6*.rs`. |
| `extbench/` | Harness untuk crate nyata (`openlocationcode`, `holidays-rs`, `stdnum-rs`). |
| `REPORT.md` | Temuan lengkap: angka, akar penyebab, status tiap rekomendasi. |
| `AGENTS.md` | Catatan konteks dan riwayat sesi kerja. |
| `build.sh` | Rebuild backend Rakit + sysroot-nya. |

`rakit/` bersifat **self-contained**: pustaka codegen Rakit di-vendor di
`rakit/vendor/` (paket `rakit-codegen`, `rakit-frontend`, `rakit-object`,
dan seterusnya), sehingga tidak ada lagi dependensi ke paket eksternal bernama
lama. Sebaliknya, `cg_clif_upstream/` sengaja **tidak diubah** — tetap cg_clif
asli dengan Cranelift asli, sebagai baseline pembanding.

## Arsitektur

```
rustc MIR
  └─ rakit/src          frontend MIR → CLIF
        └─ rakit-opt    pass manager + analisis bersama + pass first-party
              rec_loop → store_forward → div_test → if_convert
              → unroll (reassoc/LCG) → sincos
        └─ rakit-codegen  CLIF → ISel (ISLE) → regalloc2 → machin
        └─ rakit-object   emisi objek
```

`rakit-opt` adalah crate terpisah (`rakit/rakit-opt/`) yang memiliki
`PassManager` dan `Analyses`: CFG, dominator tree, dan loop forest dihitung
sekali lalu dipakai bersama, dan di-invalidasi otomatis setiap kali sebuah pass
mengubah fungsi. Setiap optimisasi adalah implementasi trait `Pass`, sehingga
urutan pipeline terpusat di satu tempat (bukan rangkaian pemanggilan ad-hoc di
`base.rs`). Inliner tetap dijalankan lebih dulu karena memakai
`Context::inline`, di luar pipeline `rakit-opt`.

`rakit-ir` (`rakit/rakit-ir/`) adalah fondasi IR mid-level milik Rakit: IR SSA
mandiri (tanpa dependensi ke backend), verifier (definisi tunggal, dominasi,
tipe), optimizer pertama (constant folding, branch folding, DCE), dan
interpreter referensi untuk tes.

Jembatan CLIF↔RIR ada di `rakit-opt/src/rir.rs`. Jembatan ini menerima subset
scalar (konstanta, aritmetika int/float, `icmp`/`fcmp`, `select`, parameter
blok, `jump`/`brif`, `return`; tanpa call/memori/stack slot/global) dan menolak
sisanya, sehingga fungsi yang tidak didukung dibiarkan apa adanya. Pass
`RirPass` menjalankan round-trip ini dan nonaktif secara default; aktifkan
dengan `RAKIT_RIR=1`. `RAKIT_RIR_FORCE=1` memaksa export ulang tanpa perubahan
(untuk validasi), `RAKIT_RIR_DEBUG=1` mencetak keputusan per fungsi.

Optimizer RIR memakai identitas yang belum tentu ada di egraph Cranelift:
`x - x`, `x ^ x`, `x & x`, `x | x`, `icmp x, x`, `select(c, x, x)`, dan
`brif` ke target yang sama. Dengan `RAKIT_RIR=1` ini benar-benar mengubah kode
(mis. `x & x` → `x`) dan output tetap sama dengan build normal.

Pass RIR `div_test` (`rakit-ir/src/div_test.rs`) memindahkan fold `x % C == 0`
ke RIR untuk `I64` dan `C` ganjil > 1: `icmp eq/ne (urem x, C), 0` →
`imul x, C^-1` + `icmp ule` (dan `select` untuk `ne`). Dengan
`RAKIT_RIR=1 RAKIT_NO_DIVTEST=1`, rewrite ini dikerjakan RIR dan output tetap
sama. Pembagi genap, tipe lebih sempit, dan `brif` langsung pada remainder
masih ditangani pass CLIF.

## Build

Prasyarat: toolchain `nightly-2026-11-17` dengan komponen `rust-src`,
`rustc-dev`, dan `llvm-tools`, plus dependensi build di `rakit/Readme.md`.

```bash
./build.sh
# hasil: rakit/dist/bin/rustc-rakit beserta sysroot-nya
```

`rustc-rakit` langsung dapat dipakai untuk mengompilasi program:

```bash
rakit/dist/bin/rustc-rakit --edition=2021 -O main.rs -o main
```

## Hasil ringkas

### Waktu compile dan ukuran (repo 4,1 MB, `cipansor/Verif`)

| Backend | compile | rlib | binary |
|---|---:|---:|---:|
| LLVM | 11853 ms | 32,10 MB | 10,00 MB |
| cg_clif upstream | 3312 ms | 27,40 MB | 13,43 MB |
| **Rakit modifikasi** | **3566 ms** | 29,70 MB | **9,74 MB** |

Backend modifikasi mengompilasi **3,3× lebih cepat dari LLVM**, dengan binary
**27% lebih kecil dari cg_clif upstream** dan 2,6% lebih kecil dari LLVM.

Pada beban nyata terberat (mem-parse MIR seluruh repo, dengan `rustc` subprocess
dipatok sama untuk ketiga backend): LLVM 15858 ms, upstream 95870 ms,
**modifikasi 62089 ms — 35% lebih cepat dari upstream**, output identik.

### Runtime pada crate nyata

Wall time (ms), L/U/M:

| crate | LLVM | upstream | modifikasi |
|---|---:|---:|---:|
| openlocationcode | 3995 | 26656 | **13945** |
| holidays-rs | 8091 | 36922 | **22250** |
| stdnum-rs | 1405 | 17344 | **7710** |

Backend modifikasi **1,7–2,2× lebih cepat dari upstream** (1,91× / 1,66× /
2,25×), dan gap terhadap LLVM menyusut dari 7,3× menjadi 3,5× pada
`openlocationcode`. Pada kernel scalar murni (`p5_inline`, `p6_scalar`) backend
modifikasi justru **≈8× lebih cepat dari LLVM** (7 ms vs 58 ms).

## Akar masalah yang ditemukan

- **Ukuran.** `rakit-object` menamai semua fungsi ke satu section
  `.text.subsection` dan semua data ke `.data.rel.ro.subsection`, sehingga
  `--gc-sections` tidak bisa membuang kode mati. Patch memberi tiap simbol
  section sendiri (`.text.<sym>`, `.data.rel.ro.<sym>`), meniru LLVM.
- **Runtime.** Pustaka Rakit menyediakan infrastruktur inlining tetapi tidak
  pernah memanggilnya; docstring-nya menyebut backend ini sebagai pihak yang
  harus menyediakan heuristik. Backend tidak mengimplementasikannya, sehingga
  setiap panggilan fungsi tetap panggilan nyata.
- **Compile.** Biaya tambahan fork berasal dari pass, **bukan** dari patch
  vendor: dengan semua pass dimatikan, fork 3212 ms vs upstream 3208 ms.

Modifikasi utama ada di `rakit/src/`: `inliner.rs` (inlining bertingkat
dengan tier leaf), `if_convert.rs` (diamond → `select`/`cmove`),
`store_forward.rs` (store-to-load forwarding + dead-store elimination),
`unroll.rs` (pelebaran loop counted), dan `driver/aot.rs`. Rincian lengkap,
termasuk lever yang diuji dan terbukti netral, ada di `REPORT.md`.

## Reproduksi

```bash
# Perbandingan tiga arah: ukuran + compile + runtime
cd bench && REPS_COMPILE=3 REPS_RUN=5 ./compare3.sh

# Ukuran lintas opt-level dan stripped
cd bench && ./sizes2.sh

# Crate nyata
cd extbench && ./extbench.sh
```

## Lisensi

Kode di repo ini dilisensikan **Apache-2.0**; lihat `LICENSE`.

`rakit/` dan `cg_clif_upstream/` mempertahankan lisensi ganda aslinya
(MIT/Apache-2.0). Pustaka codegen Rakit di `rakit/vendor/` dilisensikan
**Apache-2.0 WITH LLVM-exception**, yang mengizinkan modifikasi dan
pendistribusian, termasuk untuk keperluan komersial, dengan kewajiban
mempertahankan notice lisensi dan mencantumkan perubahan. Daftar perubahan
terhadap pustaka Rakit ada di `REPORT.md` §2 dan §8; `cg_clif_upstream/` tetap
memakai Cranelift asli.
