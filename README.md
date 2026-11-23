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
asli dengan backend upstream, sebagai baseline pembanding.

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

Jembatan CLIF↔RIR ada di `rakit-opt/src/rir.rs`. Jembatan ini menerima
konstanta, aritmetika int/float (termasuk `I128`), `icmp`/`fcmp`, `select`,
parameter blok, `jump`/`brif`, `return`, stack slot tetap dengan load/store
skalar, direct call, indirect call, libcall/known-symbol, serta call dengan
nol, satu, atau banyak hasil. Target dan signature call dipertahankan saat
import/export; call convention indirect ikut disimpan. Fungsi yang memakai
fitur global value (`GlobalValueData`), dynamic stack slot/type, dan exception
edge (`try_call`, `try_call_indirect`, `ExceptionTable`) sekarang ikut
di-import/export. Fungsi yang memakai instruksi di luar subset tetap ditolak
dan dibiarkan apa adanya; tidak ada round-trip parsial.

Pass `RirPass` berjalan default untuk **semua crate**, termasuk `core`, `std`,
`alloc`, `compiler_builtins`, `unwind`, dan `rustc_*`. Rebuild std dengan
`RAKIT_RIR_DEBUG=1` menunjukkan ribuan fungsi std benar-benar melewati RIR
(bukan hanya user crate). `RAKIT_RIR=1` memaksa aktif, `RAKIT_NO_RIR=1`
memaksa nonaktif.
`RAKIT_RIR_FORCE=1` memaksa export ulang tanpa perubahan (untuk validasi),
`RAKIT_RIR_DEBUG=1` mencetak keputusan per fungsi.

Optimizer RIR memakai identitas yang belum tentu ada di egraph backend upstream:
`x - x`, `x ^ x`, `x & x`, `x | x`, `icmp x, x`, `select(c, x, x)`, dan
`brif` ke target yang sama. Dengan `RAKIT_RIR=1` ini benar-benar mengubah kode
(mis. `x & x` → `x`) dan output tetap sama dengan build normal.

Pass RIR `div_test` (`rakit-ir/src/div_test.rs`) memindahkan fold `x % C == 0`
ke RIR untuk `I8/I16/I32/I64`: `C` ganjil > 1 memakai `imul x, C^-1` +
`icmp ule`; `C = 2^s * odd` memakai mask `x & (2^s - 1)` lalu uji `odd`;
`C` pangkat dua hanya memakai mask. `eq`/`ne` dan kombinasi Bool memakai
`select`. Pass ini opt-in: `RAKIT_RIR_DIVTEST=1 RAKIT_NO_DIVTEST=1` membuat RIR
mengerjakan rewrite dan output tetap sama. `brif` langsung pada remainder masih
ditangani pass CLIF.

Semantik integer RIR (`rakit-ir/src/int_sem.rs`) memakai representasi bit
zero-extended per lebar tipe: operasi unsigned langsung benar, operasi signed
melakukan sign-extend, dan shift memakai `width-1`. Interpreter dan constant
folder memakai modul yang sama, sehingga `I8/I16/I32` konsisten.

Pass RIR `if_convert` (`rakit-ir/src/if_convert.rs`) mengubah diamond `brif` →
dua arm pendek → join/return menjadi `select` di blok head, lalu menghapus arm
dan nilai yang menjadi mati. Join/return boleh membawa banyak nilai (termasuk
nol), jadi dua `if` identik yang menghasilkan dua nilai ikut dikonversi.
Batasannya konservatif: arm tanpa parameter, satu predecessor, maksimal 8
instruksi, tanpa div/rem, dan nilai arm tidak dipakai di luar arm. Pass ini
opt-in: `RAKIT_RIR_IFCONV=1 RAKIT_NO_IFCONV=1` mengonversi `if c { x + 1 } else
{ x + 2 }` di RIR. Default-nya pass CLIF yang berjalan karena ia memiliki cost
model prediktabilitas; menjalankan versi RIR secara default sempat memperlambat
loop bercabang.

Pass `gvn` (`rakit-ir/src/gvn.rs`) melakukan value numbering global: tabel
ekspresi diwarisi sepanjang dominator tree, jadi instruksi murni duplikat
diganti `Copy` ke hasil pertama hanya jika hasil itu mendominasi penggunaan.
Key-nya adalah opcode + tipe + operand, sehingga tipe atau operand berbeda
tidak pernah digabung; dua cabang yang tidak saling mendominasi juga tidak
digabung.

Pass `simplify` (`rakit-ir/src/simplify.rs`) membuang blok yang tidak
terjangkau dari entry dan menggabungkan blok forwarder kosong
(`nop*; jump target` tanpa parameter), dengan mengarahkan ulang edge `jump`
maupun `brif`; blok dan nilai di-renumber. Forwarder yang hasil `nop`-nya
dipakai di luar blok dibiarkan, dan blok entry tidak pernah dihapus.

Selain itu, blok tanpa parameter dengan tepat satu predecessor digabung ke
predecessor tersebut bila predecessor berakhir `jump` tanpa arg; isi blok
dipindahkan dan bloknya dihapus. Penggabungan tidak dilakukan melewati edge
`brif`, blok berparameter, atau blok dengan banyak predecessor.

Pass `licm` (`rakit-ir/src/licm.rs`) memindahkan instruksi murni yang
operandnya didefinisikan di luar loop ke preheader loop, sehingga komputasi
loop-invariant hanya dihitung sekali. Loop dengan lebih dari satu predecessor
luar atau tanpa preheader dilewati; instruksi yang bergantung pada nilai loop
tidak dipindahkan. Pass berjalan sampai fixpoint agar rantai invariant ikut
terangkat.

Bridge RIR kini juga mengimpor stack slot tetap dan akses memori skalar:
`stack_addr`, `load`, dan `store` (termasuk varian tanpa offset). Alamat
direpresentasikan sebagai `I64`. Pass yang ada tetap konservatif: `gvn` dan
`licm` tidak memindahkan akses memori, `if_convert` menolak lengan yang berisi
load/store, dan DCE tidak membuang load/store. Interpreter memakai arena stack
sederhana dengan pemeriksaan batas untuk menguji round-trip memori.

Pass `mem_forward` (`rakit-ir/src/mem_forward.rs`) meneruskan `store` ke `load`
berikutnya untuk stack slot privat yang alamatnya tidak escape, baik di dalam
blok maupun lintas blok lewat must-analysis predecessor. Setelah itu pass
menghapus store mati memakai liveness byte-range lintas blok: `live-in` dan
`live-out` dihitung sampai fixpoint, lalu store yang byte-nya tidak live setelah
store tersebut dihapus, termasuk store yang tertimpa store lain di blok yang
sama atau di successor. Alamat dilacak lewat `Copy`, `Add` dengan konstanta,
`Select`, dan indeks dinamis (`StackAddr + i*size`). Store/load dinamis
diperlakukan sebagai seluruh slot, sehingga `a[i]` hanya di-forward saat
ekspresi alamatnya identik. Pass dilewati seluruhnya bila ada alamat yang
escape, store ke alamat tak dikenal, atau akses konstanta di luar batas slot.

Pass `sroa` (`rakit-ir/src/sroa.rs`) mempromosikan stack slot skalar menjadi
nilai SSA. Slot hanya dipromosikan bila alamatnya tidak escape dan semua akses
berupa load/store skalar dengan offset konstan; akses dinamis sengaja
dibiarkan ke `mem_forward` karena satu slot dapat ditulis lewat satu alamat
dinamis dan dibaca lewat alamat lain. Phi dibuat sebagai block parameter di
dominance-frontier join, lalu rename pada dominator tree mengganti load dengan
nilai SSA saat ini dan membuang store; nilai pengganti wajib mendominasi load.
Pass ini sekarang **opt-in** dengan `RAKIT_RIR_SROA=1`; default mati sampai
bukti alias untuk kode std diperkuat. Slot yang tidak lagi punya `StackAddr`
dihapus dan slot lain dinomori ulang.

RIR juga mendukung memori 16 byte lewat `Type::I128` (load/store 16 byte,
forwarding, DSE, dan SROA opt-in). Aritmetika `I128` memakai semantik wrapping 128-bit
di `int_sem::eval_bin_i128`, dan perbandingan memakai `eval_icmp_i128`; baik
interpreter maupun folder konstan memakai jalur yang sama.

RIR mendukung direct external call lewat `Inst::Call`, indirect call lewat
`Inst::CallIndirect`, dan tabel `externals` dengan target user, libcall, atau
known-symbol plus signature (termasuk multi-return). Bridge mempertahankan
target, call convention, jumlah parameter, dan jumlah hasil saat import/export.
Call adalah barrier untuk DCE; slot yang alamatnya dikirim ke call dianggap
escape sehingga tidak di-forward atau dipromosikan. Karena setiap `StackAddr`
hanya menunjuk satu slot, slot-slot RIR selalu disjoint dan tidak ada alias
antarslot. Interpreter menyediakan callback `run_with` untuk call langsung dan
`run_with_indirect` untuk call tidak langsung, sehingga semantik call bisa diuji
tanpa menjalankan kode native.

Pipeline RIR default:
`simplify → optimize → mem_forward → optimize → sroa → optimize → gvn → licm → optimize`.

Pass `rec_loop`, inliner, `div_test`, `if_convert`, dan `sincos` versi RIR
adalah duplikat dari pass CLIF dan **opt-in** (`RAKIT_RIR_DUPLICATE=1` untuk
semuanya, atau `RAKIT_RIR_{REC_LOOP,INLINE,DIVTEST,IFCONV,SINCOS}=1`). Implementasi
CLIF menjadi pemilik default karena memiliki cost model yang sudah dituning.
RIR tetap menjadi jalur produksi untuk simplify/opt/mem_forward/SROA/GVN/LICM.

Pass `sincos` (`rakit-ir/src/sincos.rs`) adalah port RIR dari fusi
`sin`/`cos` → `sincos`/`sincosf`; ia menangani pasangan call dengan argumen
identik di blok yang sama, mengganti dua hasil call dengan satu call dua hasil,
lalu menghapus nilai lama.

## Backend instruction selection dan register allocation

`rakit-ir/src/backend/` berisi jalur backend alternatif di atas RIR:

- `mach.rs`: machine IR sederhana (`MachFunction`, `MachInst`, `Operand`,
  `VReg`) dengan mov, binop, compare, setcc, select, branch, jump, dan return.
- `mod.rs`: trait `InstructionSelector` dan `RegisterAllocator`, plus
  `BackendError`. Keduanya object-safe, sehingga implementasi lain bisa
  ditukar tanpa mengubah pemanggil.
- `isel.rs`: `RirSelector` menurunkan subset integer RIR ke machine IR.
- `regalloc.rs`: `LinearScan` melakukan liveness, graph-coloring, dan spill ke
  slot; hasilnya memetakan setiap `VReg` ke register fisik atau spill slot.
- `exec.rs`: interpreter machine IR untuk membuktikan semantik hasil seleksi
  dan alokasi, termasuk jalur spill.
- `mach::to_rir`: emitter machine IR → RIR. Setelah selector dan register
  allocator berhasil, `RirPass` memakai emitter ini untuk mengganti RIR asal
  dengan RIR hasil backend sebelum di-export ke CLIF.

Aktifkan jalur ini dengan `RAKIT_CUSTOM_BACKEND=1`; `RirPass` menjalankan
selector dan register allocator untuk setiap fungsi RIR yang didukung, lalu
meng-emit RIR dari machine IR (bukan sekadar validasi). `RAKIT_RIR_DEBUG=1`
mencetak statistik vreg/register/spill. Subset emitter saat ini adalah integer
skalar; fungsi dengan memori, call, atau parameter blok non-entry tetap
memakai jalur RIR langsung. Tes ada di `rakit-ir/tests/backend.rs`
(aritmetika, branch, select, spill, penolakan memori/`I128`, object-safety
trait, dan round-trip emitter).

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
memakai backend upstream.
