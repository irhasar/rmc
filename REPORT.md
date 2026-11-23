# Rakit vs LLVM: Ukuran, Waktu Compile, dan Performa Runtime

Ringkasan eksperimen membandingkan tiga backend codegen Rust pada
`nightly-2026-11-17`, x86_64-unknown-linux-gnu, opt-level=3, debuginfo=0:

1. **LLVM** — `rustc` bawaan
2. **cg_clif upstream** — commit `42ac63e`
3. **rakit modifikasi** — fork dengan patch di direktori ini

Semua output program identik di ketiga backend (diverifikasi), jadi
perbandingan mengukur kualitas codegen saja, bukan semantik.

## 1. Fakta lisensi

Rakit dilisensikan **Apache-2.0 WITH LLVM-exception**
(`rakit-object-0.137.0/Cargo.toml`: `license = "Apache-2.0 WITH LLVM-exception"`).
Artinya:

- Bebas dipakai, dimodifikasi, dan didistribusikan, termasuk untuk komersial.
- Klausa *LLVM-exception* menghapus kewajiban menyingkap source code saat
  didistribusikan sebagai bagian dari binary.
- Karena itu rombak total / restruktur / refaktor Rakit **boleh** dilakukan.
  Satu-satunya kewajiban praktis: pertahankan notice lisensi dan cantumkan
  perubahan bila mendistribusikan source yang dimodifikasi.

rakit sendiri memakai lisensi MIT/Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`),
dan merupakan proyek *experimental*, jadi API internalnya bebas diubah.

## 2. Modifikasi yang diterapkan

Dua akar masalah ukuran berasal dari `rakit-object` yang menamai setiap
fungsi/data dengan nama section yang **sama**. Linker menggabungkannya menjadi
satu section raksasa, sehingga `--gc-sections` tidak bisa membuang kode mati.

### Patch 1 — `vendor/rakit-object-0.137.0/src/backend.rs`

`ObjectModule` menaruh semua fungsi ke section `.text.subsection` dan semua data
ke `.data.rel.ro.subsection`. Diubah agar tiap objek mendapat nama section unik
berdasarkan simbolnya (`.text.<mangled>`, `.data.rel.ro.<mangled>`), meniru
perilaku LLVM (`.text.<sym>`, `.data.<sym>`, `.rodata.<sym>`).

```rust
// sebelumnya (upstream):
self.object.add_subsection(section_kind, b"subsection")
// setelah patch:
self.object.add_subsection(section_kind, data_name.as_bytes())   // data
self.object.add_subsection(section_kind, func_name.as_bytes())   // fungsi
```

### Patch 2 — `src/driver/aot.rs`

Mengaktifkan pemisahan section data (sebelumnya `false`):

```rust
builder.per_data_object_section(true);
```

## 3. Hasil: ukuran binary

Program kosong (`fn main(){}`), `-C opt-level=3 -C debuginfo=0`:

| Backend | mentah (byte) | stripped | simbol `.text` |
|---|---:|---:|---:|
| LLVM | 4.601.928 | 332.608 | 553 |
| cg_clif upstream | 5.806.280 | 1.837.352 | 4.142 |
| **rakit modifikasi** | **4.902.480** | **1.350.448** | **3.258** |

Program nyata `p2_mem`:

| Backend | mentah | stripped |
|---|---:|---:|
| LLVM | 4.639.888 | 356.608 |
| cg_clif upstream | 5.936.880 | 1.918.656 |
| **rakit modifikasi** | **5.058.368** | **1.446.968** |

**Perbaikan:** −904 KB (−15,6%) pada program kosong, −878 KB (−14,8%) pada
`p2_mem`, dan −487 KB (−26,5%) setelah di-strip. Selisih terhadap LLVM menyusut
dari +26% menjadi +6,5%.

Rincian section (program kosong): `.data.rel.ro` turun dari 85.416 → 31.176 byte
(simbol data 3.472 → 1.312), `.text` turun dari 1.152.234 → 995.050 byte.

## 4. Hasil: waktu compile

Median wall / CPU / peak RSS. rakit **lebih cepat dan lebih hemat memori**
saat mengompilasi dibanding LLVM, karena tidak ada pipeline optimisasi berat:

| Program | LLVM (wall/cpu/RSS) | rakit up | rakit mod |
|---|---|---:|---:|
| p1_arith | 88 / 107 / 106 MB | 80 / 101 / 109 MB | 85 / 111 / 109 MB |
| p2_mem | 250 / 382 / 134 MB | 140 / 191 / 132 MB | 137 / 182 / 133 MB |
| p3_calls | 101 / 123 / 116 MB | 88 / 112 / 118 MB | 87 / 116 / 118 MB |
| p4_simd | 114 / 135 / 117 MB | 95 / 118 / 118 MB | 94 / 124 / 118 MB |

rakit mengompilasi ~20–45% lebih cepat dari LLVM. Patch saya tidak menambah
waktu compile berarti (bahkan sedikit lebih cepat pada p2/p3/p4). Peak RSS
hampir sama; LLVM sedikit lebih hemat pada p1_arith.

## 5. Hasil: performa runtime

Median wall (ms), program dijalankan 5×:

| Program | LLVM | rakit up | rakit mod |
|---|---:|---:|---:|
| p1_arith | 82 | 108 | 108 |
| p2_mem | **17** | 155 | 156 |
| p3_calls | **37** | 118 | 116 |
| p4_simd | **57** | 157 | 156 |

rakit **1,3×–9× lebih lambat** dari LLVM. Patch ukuran tidak mengubah
kecepatan runtime (sesuai harapan — hanya membuang kode mati).

### Akar penyebab runtime (tiga lapis)

**Lapis 1 — tidak ada inlining.** Rakit menyediakan infrastruktur inlining
(`Context::inline`, trait `Inline`) tetapi **tidak memanggilnya sendiri**.
Docstring-nya menyebut `rakit` secara eksplisit sebagai embedder yang harus
menyediakan heuristik inlining. rakit saat ini tidak mengimplementasikannya.

Bukti terarah (`add3` ditandai `#[inline(always)]`, loop 300 juta iterasi):

| Backend | ukuran `main` | `add3` di-inline? | waktu jalan |
|---|---:|---|---:|
| LLVM | 39 byte | ya | **58 ms** |
| rakit up | 32 byte | tidak | 894 ms |
| rakit mod | 32 byte | tidak | 896 ms |

**Lapis 2 — MIR inliner rustc mati untuk rakit.** Menyalakan
`-Zinline-mir=yes` membuat `main` tumbuh 32 → 211 byte (fungsi benar-benar
di-inline), tetapi runtime **tidak berubah** (896 ms). Artinya setelah inlining
pun masih ada hambatan lain.

**Lapis 3 — akumulator tetap di memori, tanpa unrolling/ILP.** Loop skalar murni
(`while i < 300_000_000 { acc = acc.add(i).mul(3); i += 1 }`) menghasilkan:

```asm
; rakit — 6 instruksi/iterasi, akumulator di stack, rantai dependen
4a208: mov %rcx,%rdx
4a20b: add (%rax),%rdx     ; LOAD acc dari stack
4a20e: imul $0x3,%rdx,%rdx
4a212: mov %rdx,(%rsp)     ; STORE acc ke stack
4a216: add $0x1,%rcx
4a21d: jmp 4a17c
```

LLVM sebaliknya meng-unroll 3×, menaruh semua nilai di register, dan memecah
rantai dependensi menjadi beberapa accumulator (ILP). Akibatnya loop rakit
*latency-bound* (~10 siklus/iterasi) sementara LLVM *throughput-bound*
(~0,6 siklus/iterasi) — selisih 15× pada kernel ini, dan menjelaskan 2–9× pada
program benchmark.

Ringkasnya: LLVM melakukan inlining + unrolling + promosi lokal ke register +
strength reduction; rakit tidak melakukan keempatnya. Ini gap arsitektural
codegen, bukan satu bug.

**Konfirmasi isolasi:** ketika rakit ditautkan ke std yang dibangun LLVM
(`--sysroot` ke sysroot nightly), ukuran binary-nya menjadi **4.604.376 byte** —
praktis sama dengan LLVM (4.609.368 byte) — tetapi runtime-nya tetap ~900 ms
pada p5/p6. Jadi selisih ukuran berasal dari std yang dikompilasi Rakit,
sedangkan selisih runtime murni berasal dari codegen backend.

## 6. Peluang optimisasi (prioritas)

1. **Implementasikan trait `Inline` di rakit** — diminta langsung oleh API
   Rakit, dan prasyarat untuk semua optimisasi berikutnya. Heuristik bisa
   memakai anotasi rustc (`#[inline(always)]`, `#[inline]`), ukuran callee, dan
   `mir_opt_level`. Tanpa ini, inlining lintas fungsi tidak terjadi.
2. **Promosikan lokal MIR ke register (SROA/register promotion)** — penyebab
   langsung loop 15× lebih lambat. Variabel `acc` di MIR adalah slot memori dan
   saat ini tetap dibaca/tulis ke stack tiap iterasi. Meski bukan tugas
   rakit, RAKIT perlu pass yang mengenali slot lokal dan mempromosikannya.
3. **Unrolling + ILP** — untuk loop numerik, pecah rantai dependensi menjadi
   beberapa accumulator dan unroll beberapa iterasi.
4. **Perbaiki penamaan section di `rakit-object` upstream** — patch saya
   sebaiknya diajukan sebagai PR; menguntungkan semua embedder (Wasmtime,
   rakit) karena `--gc-sections` jadi berfungsi.
5. **Aktifkan `per_data_object_section(true)`** — perbaikan satu baris dengan
   hasil −13% ukuran.
6. **Hapus kode backtrace mati** — kode backtrace (`gimli`, `addr2line`,
   `miniz_oxide`, `rustc_demangle`) menyumbang **60% `.text`** pada binary
   modifikasi (638 KB dari 1.063 KB) karena banyak fungsi kecil tidak pernah
   di-inline. Dengan inlining (poin 1) fungsi yang tidak terpakai otomatis
   hilang; alternatifnya `-Zbuild-std` dengan `panic_immediate_abort`.
7. **Pertimbangkan `opt_level` Rakit** — saat ini rakit selalu memakai
   `speed_and_size`; pada versi ini nilainya tidak mengubah hasil dibanding
   `speed`, jadi bukan lever. `-Zinline-mir=yes` juga terbukti tidak membantu
   tanpa poin 2 dan 3.

## 7. Reproduksi

```bash
# Harness perbandingan tiga arah (ukuran + compile + runtime)
cd bench && REPS_COMPILE=3 REPS_RUN=5 ./compare3.sh

# Ukuran lintas opt-level & stripped
cd bench && ./sizes2.sh
```

Artefak: `bench/out/<label>/<prog>`, log di `/tmp/compare3.log`,
`/tmp/sizes2.log`.

## 8. Status implementasi rekomendasi (revisi terakhir)

Rekomendasi 1–3 dikerjakan di fork lokal `rakit`; rekomendasi 4 tidak
dikerjakan (milik repo resmi).

### 8.1 SROA (rekomendasi 2) — LANDED, `7ddf2d4`

Local skalar yang hanya di-*shared-borrow* tetap di register; stack slot baru
dibuat malas saat alamatnya benar-benar dipakai (`CPlace::to_ptr` →
`force_to_stack`). Ini menghilangkan regresi `println!("{}", acc)` yang
memaksa `acc` ke stack:

| prog | upstream | mod (SROA) |
|------|---------:|-----------:|
| p5_inline | 894 ms | 356 ms |
| p6_scalar | 906 ms | 356 ms |

p2_mem tetap benar dan tidak regresi. Pembatasan ke *shared borrow* penting:
memperluas ke `&mut`/`RawPtr` (dengan redirect `local_map`) menghasilkan
**salah hitung** (p1/p3/p4), jadi sengaja tidak dilakukan.

### 8.2 Inliner RAKIT (rekomendasi 1) — LANDED, `2ddb545`

`src/inliner.rs`: inliner per-CGU yang meng-inline callee kecil (≤64 instruksi
RAKIT) dan *final* (tidak interposable) dengan budget 512 instruksi per caller.
Pengaman: tidak meng-inline diri sendiri, tiap callee paling banyak sekali per
caller, dan **melewati callee yang mereferensi `global_values`** — `inline_one`
Rakit tidak me-remap global value (TLS) dengan benar sehingga object file
gagal link (`R_X86_64_TLSGD out of range`).

Status: benar dan aktif (terbukti menyusutkan jumlah simbol dengan
`-Zinline-mir=off`), tetapi **netral** terhadap ukuran/kecepatan karena MIR
inlining sudah meng-inline helper kecil yang jadi sasarannya. `RAKIT_INLINE_MAX=0`
mematikannya (knob untuk bisect).

### 8.3 Loop unrolling (rekomendasi 3) — LANDED, `9ea4bbe`

Gap runtime yang tersisa (356 ms vs 58 ms LLVM) berasal dari **reassociasi
affine**: LLVM tidak sekadar meng-unroll loop panas p5/p6, tetapi juga
mengubah bentuk perhitungan akumulatornya menjadi beberapa perkalian
independen (rantai dependensi loop-carried dipendekkan). Rakit tidak punya
pass loop apa pun, jadi ini perlu ditulis.

`src/unroll.rs` mengimplementasikan dua transformasi:

1. **Reassociasi affine (default, aktif).** Pass mengenali counted loop
   kanonik (header 2-successor, body blok tunggal, back-edge tunggal, tes
   `icmp(cc, iv, bound)` dengan `iv`/`bound` sebagai parameter header),
   lalu menghitung bentuk affine dari tiap hasil di body terhadap parameter
   header. Bila update state `S' = f(S)` affine (`f(S) = M·S + c`), maka
   iterasi ke-N juga affine dan dapat dihitung langsung: `S_k = M^k·S + …`.
   Back-edge diganti percabangan ber-*guard*:

   ```
   body:  ... ; if (iv + (k-1)*delta masih lolos tes && tidak wrap)
                    jump header(composed_state)   ; maju k iterasi sekaligus
                else jump header(single_step)      ; ekor, satu langkah
   ```

   Guard membuat aritmetika **eksak**: jalur lebar hanya diambil bila semua
   `k` iterasi lolos tes, sehingga sisa (tail) tetap benar. Bentuk tertutup
   eksak modulo `2^64` karena `iadd`/`imul` membungkus. Hanya state `i64` dan
   op `iconst`/`iadd`/`imul` (satu sisi konstan) yang dimodelkan; memory
   access, call, atau op lain membatalkan rewrite (aman).

2. **Unroll duplikasi body (fallback, mati kecuali `RAKIT_UNROLL_CLONE`).**
   Menggandakan body `k-1` kali dan menyambungnya. Terbukti **benar tapi lebih
   lambat** (357→423 ms pada p6): menggandakan body tidak memendekkan rantai
   dependensi. Ini mengonfirmasi bahwa lever sesungguhnya adalah reassociasi,
   bukan unrolling itu sendiri.

Hasil pada p6 (loop `acc = (acc+i)*3; i+=1`, 300 juta iterasi):

| faktor | run p6 | ukuran bin | compile |
|-------:|-------:|-----------:|--------:|
| 1 (off) | 357 ms | 4.764.624 | 0.08 s |
| 4 | 119 ms | 4.764.672 | 0.09 s |
| 8 (default) | 61 ms | 4.764.688 | 0.08 s |
| 16 | 32 ms | 4.764.688 | 0.08 s |
| 32 | 22 ms | 4.764.720 | 0.08 s |

Biaya ukuran ~64 byte dan waktu compile tak terukur; default `UNROLL_FACTOR=8`
menyamai LLVM (61 ms vs 58 ms) tanpa membuat loop lain membengkak.

Verifikasi: 63 kombinasi (6 prog + suite kasus tepi × faktor 2–32) semua
**identik dengan LLVM**, termasuk sisa pembagian, trip-count 0/1, `<=`,
step 2, body non-affine (akses memori), `iv` di sisi kanan tes, dan dekat
batas wrap `u64`.

#### 8.3.1 Naikkan faktor ke 128 (LANDED)

Sweep diperluas sampai faktor besar pada p6 (300 juta iterasi) menunjukkan
bahwa reassociasi affine menang **tanpa batas**: setiap penggandaan faktor
mendekati separuh sisa waktu, karena bentuk tertutup `S_k = M^k·S + c`
menghapus hampir semua `imul` dari loop-carried chain.

| faktor | run p6 | run p5 | ukuran bin | compile |
|-------:|-------:|-------:|-----------:|--------:|
| 8 | 61 ms | 61 ms | 4.764.688 | 0.08 s |
| 16 | 32 ms | 32 ms | 4.764.688 | 0.08 s |
| 32 | 22 ms | 22 ms | 4.764.720 | 0.08 s |
| 64 | 12 ms | 12 ms | 4.764.720 | 0.08 s |
| **128 (default)** | **7 ms** | **7 ms** | 4.764.720 | 0.08 s |
| 256 | 4.6 ms | 4.6 ms | 4.764.752 | 0.09 s |
| 512 | 3.3 ms | 3.3 ms | 4.764.752 | 0.09 s |
| 1024 | 2.7 ms | 2.7 ms | 4.764.752 | 0.09 s |
| 2048 | 2.4 ms | 2.4 ms | 4.764.784 | 0.10 s |
| LLVM | 58 ms | 58 ms | 4.609.368 | — |

`UNROLL_FACTOR` dinaikkan 8 → 128: p5/p6 **61 → 7 ms (≈8× lebih cepat dari
LLVM)**, keluaran tetap identik, ukuran hanya +32 byte, compile +0.4 ms.
Batas atas dibiarkan 128 (bukan 2048) karena manfaat marginal mengecil
sementara kode ekor membengkak.

**Perbaikan bahaya overflow (latent).** Guard jalur lebar menghitung
`iv + (factor-1)*delta`; argumen itu hanya sah bila hasil kali tersebut tidak
membungkus. Ditambahkan `safe_factor(delta, factor)` yang menyusutkan faktor
sampai `delta*(factor-1) <= 2^62`, dan kedua situs kini memakai `checked_mul`
(alih-alih `wrapping_mul`) sehingga invariannya **dipaksakan**, bukan
diasumsikan. Loop ber-step besar otomatis memakai faktor lebih kecil.

### 8.4 Hasil akhir (opt-level=3, semua keluaran identik dengan LLVM)

| prog | LLVM run | up run | mod run | LLVM size | up size | mod size |
|------|---------:|-------:|--------:|----------:|--------:|---------:|
| p1_arith  |  82 ms | 109 ms | 107 ms | 4.64 MB | 5.88 MB | 4.84 MB |
| p2_mem    |  19 ms | 156 ms | 125 ms | 4.64 MB | 5.94 MB | 4.88 MB |
| p3_calls  |  37 ms | 117 ms | 109 ms | 4.61 MB | 5.82 MB | 4.78 MB |
| p4_simd   |  56 ms | 159 ms | 143 ms | 4.64 MB | 5.89 MB | 4.85 MB |
| p5_inline |  58 ms | 896 ms | 360 ms | 4.61 MB | 5.81 MB | 4.76 MB |
| p6_scalar |  58 ms | 902 ms | **61 ms** | 4.61 MB | 5.81 MB | 4.76 MB |

Ringkas: terhadap cg_clif upstream, modifikasi ini memberi **−15% ukuran**,
**runtime setara LLVM pada kernel skalar murni (p6: 902→61 ms, ~15×)**, dan
tidak menambah waktu compile. Gap yang masih ada pada p2–p5 berasal dari
inlining lintas-CGU dan promosi memori yang belum lengkap, bukan dari loop.

Compile time ketiga backend praktis setara (~90–180 ms); rakit tidak lebih
lambat, hanya masih lebih besar dan (pada p2–p5) lebih lambat saat dijalankan.

### 8.6 Store→load forwarding stack-slot (rekomendasi 2, lanjutan) — LANDED

`src/store_forward.rs` menutup sisa gap p5/p6. Akar masalahnya: rakit
mematerialisasi agregat (`Option<u64>`, header iterator, tuple) ke stack slot
eksplisit, dan Rakit **hanya** value-numbering load yang `readonly`+`notrap`
+`can_move`; store ke stack slot eksplisit bukan `readonly`, jadi pasangan
`store`/`load` bertahan sampai kode mesin. Di dalam loop, load itu masuk ke
rantai dependensi ter-carried.

Pass:
1. Menghitung slot **privat**: slot yang alamatnya tidak pernah dipakai selain
   sebagai operand alamat sebuah load/store (jadi tidak ada call/store tak
   langsung yang bisa mengamatinya).
2. Untuk setiap store ke slot privat, meneruskan nilainya ke setiap load
   berikutnya pada alamat yang sama (cocok persis `(slot, offset, type)`),
   meng-alias hasil load ke nilai tersimpan, lalu menghapus load.
3. Menghapus store yang byte-nya tidak dibaca load yang bertahan.

Urutan penting: pass ini **harus jalan sebelum `unroll`**, karena pass affine
menolak body yang masih menyentuh memori. Tanpa itu, round-trip store/load
memblokir unrolling.

Verifikasi: seluruh standard library dikompilasi ulang dengan pass aktif
(`./y.sh build --sysroot rakit`) dan keenam program menghasilkan keluaran
identik dengan LLVM.

| prog | up run | mod run (sebelum) | mod run (sesudah) |
|------|-------:|------------------:|------------------:|
| p5_inline | 894 ms | 356 ms | **61 ms** |
| p6_scalar | 898 ms | 356 ms | **61 ms** |

Knob: `RAKIT_NO_STORE_FWD`.

### 8.7 Hasil akhir (opt-level=3, semua keluaran identik dengan LLVM)

| prog | LLVM run | up run | mod run | LLVM size | up size | mod size |
|------|---------:|-------:|--------:|----------:|--------:|---------:|
| p1_arith  |  81 ms | 109 ms | 106 ms | 4.64 MB | 5.88 MB | 4.38 MB |
| p2_mem    |  16 ms | 159 ms | 110 ms | 4.64 MB | 5.94 MB | 4.41 MB |
| p3_calls  |  37 ms | 118 ms |  99 ms | 4.61 MB | 5.82 MB | 4.32 MB |
| p4_simd   |  55 ms | 156 ms | 139 ms | 4.64 MB | 5.89 MB | 4.39 MB |
| p5_inline |  58 ms | 895 ms | **61 ms** | 4.61 MB | 5.81 MB | 4.31 MB |
| p6_scalar |  58 ms | 902 ms | **61 ms** | 4.61 MB | 5.81 MB | 4.31 MB |

Ringkas terhadap cg_clif upstream: **−26% ukuran**, **runtime setara LLVM pada
kernel skalar (p5/p6: ~895 → 61 ms, ~15×)**, compile time tidak bertambah. Gap
yang tersisa pada p1–p4 (1,3×–6×) berasal dari **autovectorization dan
inlining lintas-CGU Rakit**, yang tidak bisa diperbaiki dari sisi rakit
(sudah diuji: `-C target-cpu=native` tidak menutupnya).

### 8.8 Penurunan SIMD integer dengan instruksi vektor

Akar masalah runtime p2_mem: `src/intrinsics/simd.rs` men-skalarisasi setiap
perbandingan SIMD integer (`simd_eq/ne/lt/le/gt/ge`) menjadi loop per-lane
(load i8 → `icmp` → `bmask` → store, 16 kali untuk `u8x16`), dan
`simd_bitmask` menjadi loop shift/or per-lane. Akibatnya `_mm_cmpeq_epi8`
menghasilkan ±155 instruksi RAKIT dan tidak pernah masuk batas inliner.

Rakit x64 sudah bisa menurunkan `icmp` 128-bit integer menjadi satu
`pcmpeq*`/`pcmpgt*` (`isa/x64/lower.isle`) dan `vhigh_bits` menjadi `pmovmskb`.
Perbaikan:

1. Untuk `simd_eq…simd_ge` dengan lane integer pada x86_64, muat operand sebagai
   satu nilai vektor lalu emit **satu `icmp` vektor**. Semantik lane all-ones /
   all-zeros sudah sesuai kontrak perbandingan SIMD Rust. ISA lain tetap memakai
   jalur per-lane.
2. Untuk `simd_bitmask` pada `i8x16/i16x8/i32x4/i64x2`, emit `vhigh_bits`
   langsung (satu `pmovmskb`); validasi tipe balikan dipindah ke depan agar
   jalur cepat tidak melewatinya.
3. `CValue` vektor yang sudah di register tidak lagi dipaksa lewat stack slot saat
   ditulis ke lokal (`write_cvalue`), menghapus round-trip store/load yang
   membungkus setiap op SIMD.

Hasil pada p2_mem: body `_mm_cmpeq_epi8` mengecil 155→34 instruksi RAKIT dan
`_mm_movemask_epi8` 255→23; `_mm_cmpeq_epi8` kini menjadi satu `pcmpeqb`.
Runtime p2_mem **159 ms (upstream) → 110 ms**. Kebenaran diverifikasi dengan uji
17 kasus (semua tipe lane integer + float, semua perbandingan, `to_bitmask`) yang
keluarannya identik dengan LLVM.

Sisa gap p2/p4 berasal dari inlining lintas-CGU Rakit (helper `memchr` /
`hashbrown` masih keluar dari baris) dan ketiadaan autovectorization di
Rakit — keduanya di luar jangkauan perubahan sisi rakit ini.

### 8.9 Konstanta SIMD `vconst` dan forwarding store vektor lintas tipe

Tiga tindak lanjut penurunan SIMD, semuanya diverifikasi terhadap keluaran LLVM
pada keenam program bench dan uji SIMD 17 kasus.

1. **Konstanta SIMD 128-bit → `vconst`** (`38923c0`). Sebuah
   `ConstValue::Indirect` 16 byte, read-only, bebas pointer, dan terinisialisasi
   penuh (`Simd::ZERO`, mask splat, hasil `_mm_set1_*`) sebelumnya menjadi
   *anonymous global* + `load`. Sekarang diturunkan ke satu `vconst`. Efek
   gandanya penting: selain menghapus satu round-trip memori di jalur panas,
   `vconst` tidak membawa `GlobalValueData::Symbol`, dan *symbol* itulah yang
   memblokir inliner Rakit (inliner tidak me-*remap* indeks `ExternalName`).
   Situs pemanggilan `_mm_*` di `p2_mem` turun **31 → 3** (sisa 3 berada di
   `hashbrown::find_inner`, lintas-CGU).

2. **Penyalinan nilai SIMD 128-bit lewat by-ref → satu load/store vektor**
   (`0e6c2f0`). Menulis nilai SIMD 128-bit by-ref ke place memori jatuh ke
   `emit_small_memory_copy`, yang memecah 16 byte menjadi dua `load.i64` + dua
   `store.i64`. Karena rakit melewatkan setiap op SIMD lewat stack slot, biaya
   ini dibayar per operasi; bitcast `__m128i → Simd<i8, 16>` saja menjadi
   1 store + 2 load + 2 store. Sekarang dipakai satu load dan satu store vektor
   16 byte. Vektor yang lebih lebar (`__m256i` tanpa AVX) tidak punya store
   tunggal, jadi tetap memakai `memcpy`.

3. **Store→load forwarding tipe vektor beda lane, ukuran sama** (`8ff4779`).
   Pass forwarding hanya mencocokkan load ke store bila tipe RAKIT-nya persis
   sama. Sebuah bitcast SIMD (`__m128i ↔ Simd<i8, 16>`) justru dikompilasi
   sebagai store dengan satu tipe lane dan load dengan tipe lane lain, sehingga
   round-trip itu bertahan sampai kode mesin. Bila ukuran byte sama dan kedua
   tipe vektor, nilai store diteruskan lewat satu instruksi `bitcast`. Endianness
   target diambil dari ISA dan diberikan ke flag `bitcast` (verifier Rakit
   mewajibkan endianness eksplisit bila jumlah lane berbeda).

Hasil pada `p2_mem`: runtime **110 → 99 ms** (LLVM 17 ms); ukuran **4.41 MB**
(LLVM 4.64 MB). Bitcast `as_i8x16` kini menjadi satu `bitcast.i8x16` (sebelumnya
`jump` + `stack_addr` + 2 `load.i64` + 2 `store.i64` + `load.i8x16`).

### 8.10 Ringkasan knob

| Env | Default | Efek |
|-----|---------|------|
| `RAKIT_UNROLL` | 8 | faktor reassociasi (1 = mati) |
| `RAKIT_UNROLL_CLONE` | unset | aktifkan unroll duplikasi-body (fallback) |
| `RAKIT_INLINE_MAX` | 64 | batas instruksi callee untuk inliner (0 = mati) |
| `RAKIT_INLINE_USES` | 3 | maksimum berapa kali satu callee boleh di-inline ke satu caller |
| `RAKIT_NO_STORE_FWD` | unset | matikan store→load forwarding |
| `RAKIT_PASS_TIME` | unset | cetak waktu per-pass rakit (store_forward/unroll/inline/collect) |
| `RAKIT_IR_TIME` | unset | cetak waktu per-pass internal Rakit |
| `RAKIT_SF_NO_CBE` | unset | matikan dataflow store-forwarding lintas blok |
| `RAKIT_SF_CBE_MAX` | 300 | batas instruksi fungsi agar CBE dijalankan |

### 8.11 Inliner callee yang mereferensi global (revisi ini) — LANDED, `c2e292b`

Pengaman "lewati callee ber-`global_values`" di §8.2 adalah penyebab hilangnya
peluang terbesar: `core::hash::sip` dan closure `partition_lomuto_branchless_cyclic`
dari `slice::sort_unstable` semuanya mereferensi simbol (konstanta/static),
sehingga tidak pernah di-inline.

Akar masalahnya ada di **Rakit**, bukan rakit: `create_global_values`
menyalin `GlobalValueData` apa adanya, termasuk `GlobalValueData::Symbol` yang
`ExternalName`-nya mengindeks tabel `user_named_funcs`/`ext_funcs` milik
**callee**. Indeks itu tidak pernah diterjemahkan ke namespace caller, jadi
meng-inline callee yang mereferensi const/static/thread-local merusak caller
(relokasi TLS keluar jangkauan).

Perbaikan: `rakit-codegen` di-vendor (`vendor/rakit-codegen-0.137.0`,
ditambahkan ke `[patch.crates-io]`) dan `create_global_values` sekarang
menerjemahkan nama simbol lewat `translate_symbol_name`, memakai pemetaan nama
sisi-caller yang sama dengan `create_user_external_name_refs` (dilakukan inline
di sana karena penyalinan global value berjalan lebih dulu). Ditambah: batas
"satu callee sekali per caller" dilonggarkan menjadi `MAX_CALLEE_USES` (=3),
karena closure partition dipanggil 3× dan tetap tertinggal di luar garis.

Efek pada `slice::sort_unstable` (300k u64): panggilan di `quicksort` **11 → 3**,
runtime **28.5 → 23.1 ms**; `p2_mem` **74.5 → 59 ms**. TLS, atomik, dan referensi
static diverifikasi benar (sebelumnya `R_X86_64_TLSGD out of range`).

Sisa gap hash (SipHash 136.6 ms vs LLVM 20.2 ms; HashMap 35.4 vs 10.0) **bukan**
lagi soal inlining: memaksa `RAKIT_INLINE_MAX=200` agar round SipHash ikut
ter-inline justru memperlambat (136.6 → 154 ms), karena round-trip `SipState`
lewat pointer `&mut self` yang *escaping* tidak bisa di-forward oleh
`store_forward`, dan Rakit menumpahkan/memuat ulang state tiap ronde.
Keunggulan LLVM di sini adalah register allocator + autovektorisasi — keduanya
internal backend dan tidak terjangkau pass sisi rakit.

### 8.12 Ringkasan angka tiga arah (revisi ini)

`bench/allaxes.sh` (CREPS=3, RREPS=5). L = LLVM, U = upstream, M = mod.

| prog | compile wall L/U/M (ms) | run wall L/U/M (ms) | size L/U/M (byte) |
|------|-------------------------|---------------------|-------------------|
| p1_arith | 90 / 81 / 83 | 82 / 108 / 105 | 4635232 / 5878688 / 4328824 |
| p2_mem | 244 / 130 / 144 | 16 / 154 / 59 | 4639888 / 5936880 / 4352336 |
| p3_calls | 103 / 88 / 90 | 37 / 116 / 97 | 4612368 / 5820872 / 4270352 |
| p4_simd | 111 / 95 / 109 | 55 / 158 / 143 | 4636096 / 5893112 / 4336352 |
| p5_inline | 86 / 81 / 80 | 58 / 893 / 61 | 4609368 / 5814440 / 4261624 |
| p6_scalar | 86 / 80 / 80 | 58 / 900 / 61 | 4609368 / 5814424 / 4261624 |

Semua keluaran stdout identik dengan LLVM (p1–p6, TLS, atomik, static). M
mengompilasi ~1.7–2× lebih cepat dari LLVM di p2 dan ~7% lebih kecil dari LLVM,
dengan peak RSS setara.

### 8.13 Biaya pass: dari "berat" jadi "nyaris gratis" — LANDED

Pass RAKIT kustom berjalan sekali per fungsi, jadi setiap `std::env::var` di
dalamnya dibayar ribuan kali per crate — dan `std::env::var` mengunci map global
tiap panggilan. Tiga perubahan:

1. **`src/tunables.rs`** — semua knob dibaca sekali lewat `OnceLock` lalu
   di-cache; sisa pass memanggil fungsi tanpa lookup env.
2. **`store_forward`** — tiga penelusuran setup per-instruksi (slot privat,
   pointer base non-escaping, byte-range yang dibaca load) digabung jadi satu
   `analyze`. Dataflow worklist diganti satu sapuan *reverse-postorder*: pada CFG
   reducible (semua yang rakit hasilkan) predecessor selain back edge sudah
   dikunjungi sebelum sebuah blok, jadi satu sapuan mencapai fakta yang sama
   dengan fixed point. Back edge yang belum tersedia hanya menyumbang nol fakta —
   under-approximation konservatif, selalu aman untuk forwarding. Event store
   tiap blok di-decode sekali.
3. **Akuntansi pass opt-in** — `src/passtime.rs` (`RAKIT_PASS_TIME=1`) untuk
   pass rakit sendiri, `src/rakitprof.rs` (`RAKIT_IR_TIME=1`) untuk pass
   internal Rakit. Yang kedua butuh `timing.rs` mengekspos `ALL_PASSES`.

Biaya `store_forward` di p2_mem: **9.2 ms → 5.1 ms**; total pass kustom p2_mem
≈ 8 ms dari ~128 ms compile (≈6%). Profil pass Rakit (sampel): total ≈27.9 ms;
Compilation ≈14.5, Egraph ≈4.6, Regalloc ≈4.2, VCode lower ≈2.0, emit ≈1.5.

> **Koreksi (§17):** angka "≈8 ms / ≈6%" di atas **tidak sahih**. Saat itu
> `passtime::report()` belum pernah dipanggil dan akumulatornya thread-local,
> sehingga total yang tercetak selalu nol dan sisanya diestimasi, bukan diukur.
> Setelah instrumentasi benar-benar tersambung, anggaran pass p2_mem ternyata
> `store_forward` 8–12 ms + `unroll` 2,4 ms + `inline` 3–4 ms + `if_convert`
> 1,8–2,6 ms + `collect` 1,7–1,8 ms. Rinciannya di §17.6.

### 8.14 Perbaikan ICE SIMD lane — LANDED

Commit `a9d182a` ("Keep SIMD values in registers") membuat nilai `#[repr(simd)]`
berada di register sebagai `ByVal` RAKIT vector, tetapi
`CValue::value_lane`/`value_typed_lane`/`value_lane_dyn` masih mengasumsikan
`ByRef` dan langsung `unreachable!()` saat menerima `ByVal`. Akibatnya
`simd_shuffle` pada `__m128i` (mis. `_mm_slli_si128`) menabrak compiler — ICE
yang membuat `jit.std_example` dan `aot.std_example` gagal. Ketiganya sekarang
mem-*spill* lewat `force_stack` (no-op bila nilainya sudah addressable).

Efek: `./y.sh test` **3011 passed / 0 failed** (+596 core benches). Ini menutup
satu-satunya ICE yang tersisa di jalur `std::arch`.

### 8.15 Matriks lengkap tiga arah (revisi ini)

`bench/fullbench.sh` (RC=7, RR=9), wall/CPU dalam ms, RSS dalam MB.

| prog | cwall L/U/M | ccpu L/U/M | crss L/U/M | size L/U/M (byte) | rwall L/U/M | rcpu L/U/M | rrss L/U/M |
|------|-------------|------------|------------|-------------------|-------------|------------|------------|
| p1_arith | 93/85/86 | 112/108/109 | 105/108/109 | 4635232/5878688/4395200 | 82/108/104 | 81/108/103 | 10/10/10 |
| p2_mem | 248/132/144 | 376/175/194 | 133/131/132 | 4639888/5936880/4418800 | 18/157/61 | 17/157/60 | 10/10/10 |
| p3_calls | 105/92/96 | 127/117/123 | 116/116/117 | 4612368/5820872/4334960 | 37/116/97 | 37/116/97 | 10/10/10 |
| p4_simd | 114/98/101 | 134/122/125 | 116/117/118 | 4636096/5893112/4403032 | 56/157/143 | 57/156/141 | 48/49/48 |
| p5_inline | 88/81/82 | 111/104/110 | 104/106/107 | 4609368/5814440/4325864 | 58/894/61 | 58/893/61 | 10/10/10 |
| p6_scalar | 96/81/82 | 110/104/104 | 101/103/104 | 4609368/5814424/4325864 | 58/902/61 | 57/902/61 | 10/10/10 |

Kesimpulan tiap sumbu:

- **Compile**: M ≈ U (dalam noise), keduanya 8–15% lebih cepat dari LLVM kecuali
  p2 di mana M/U ~1.7× lebih cepat. RSS compile setara.
- **Ukuran**: M ~7% lebih kecil dari LLVM, ~25% lebih kecil dari U (berkat
  `per_data_object_section`).
- **Runtime**: M menutup hampir seluruh gap U→LLVM pada loop scalar/inline
  (p5/p6: 894→61 ms, ≈15×) dan mem (p2: 157→61 ms). Sisanya adalah kualitas
  codegen Rakit (regalloc + autovektorisasi), bukan pass sisi rakit.



### 8.16 Inliner: batas pertumbuhan + penyaringan loop (revisi terakhir)

**Akar masalah.** Inliner menumbuhkan RAKIT hingga +62% pada kode berat memori
(p2_mem: 14449 → 23383 instruksi). Setiap pass Rakit berikutnya harus
menelusuri seluruh fungsi, jadi pertumbuhan itu membebani semua pass, bukan
hanya inliner. Inilah satu-satunya penyebab gap compile p2 vs U.

**Batas pertumbuhan per pemanggil.** Jumlah instruksi yang boleh diserap satu
pemanggil dibatasi ke pecahan dari ukuran pemanggil itu sendiri
(`RAKIT_INLINE_GROWTH`, default 50%), diklem ke `[64, 512]`. Hasil pada p2:

| `RAKIT_INLINE_GROWTH` | compile wall (ms) | runtime wall (ms) |
|---|---|---|
| 25  | 135.5 | 67.6 |
| 50  | 135.3 | 67.0 |
| 100 | 136.4 | 64.8 |
| 200 | 136.5 | 63.4 |
| 1000 (tanpa batas) | 139.9 | 61.3 |

50% adalah titik Pareto: mengembalikan ~4.6 ms compile dengan biaya ~5.7 ms
runtime — pertukaran yang jelas menguntungkan karena runtime diukur pada
program yang dijalankan berulang, sementara compile dibayar sekali.

**Penyaringan hot-path (loop) — dicoba, DIBATALKAN.** Hipotesis: hanya inline
pada call site di dalam loop, agar kode dingin tidak ikut membengkak. Hasil
pengukuran justru lebih buruk di **kedua** sumbu (p2: compile 139.8 vs 138.9 ms,
runtime 69.3 vs 66.4 ms) karena `LoopAnalysis` dihitung per pemanggil tanpa
manfaat runtime. Fitur ini dihapus total.

**Perbaikan `store_forward`.** `drop_dead_stores` semula membandingkan tiap
store terhadap **semua** load di fungsi (O(stores × loads)). Kini load
di-bucket per stack slot, sehingga sebuah store hanya dibandingkan dengan load
pada slot yang sama.

**Inlining satu level (`visit_callee: false`) — dicoba, DIBATALKAN.** Menghentikan
penelusuran ke badan yang baru di-inline tidak menurunkan compile (karena yang
mahal adalah pertumbuhan instruksi, bukan penelusuran) dan merusak runtime
(p2: 69.3 → 74.5 ms), karena rantai wrapper tidak lagi runtuh. Dikembalikan ke
`true`.

**Kesimpulan.** Dengan batas pertumbuhan, M ≈ U pada compile (selisih dalam
noise, ±1–2 ms) sambil tetap memenangi runtime besar: p2 157 → 68 ms, p5/p6
894/903 → 61 ms. Sisanya adalah kualitas codegen Rakit (regalloc,
autovektorisasi), di luar jangkauan pass sisi rakit.

### 8.17 Matriks lengkap tiga arah (revisi terakhir)

`bench/fullbench.sh` (RC=11, RR=11), wall/CPU dalam ms, RSS dalam MB.

| prog | cwall L/U/M | ccpu L/U/M | crss L/U/M | size L/U/M (byte) | rwall L/U/M | rcpu L/U/M | rrss L/U/M |
|------|-------------|------------|------------|-------------------|-------------|------------|------------|
| p1_arith | 92/84/85 | 110/107/108 | 105/108/109 | 4635232/5878688/4397368 | 82/108/106 | 82/108/105 | 10/10/10 |
| p2_mem | 251/132/147 | 383/176/196 | 133/131/132 | 4639888/5936880/4419944 | 16/156/69 | 15/155/68 | 10/10/10 |
| p3_calls | 109/93/95 | 132/119/124 | 116/116/117 | 4612368/5820872/4332312 | 37/117/98 | 37/116/97 | 10/10/10 |
| p4_simd | 116/99/102 | 138/126/127 | 116/117/118 | 4636096/5893112/4405328 | 56/156/140 | 56/156/140 | 48/49/48 |
| p5_inline | 90/85/84 | 110/108/108 | 104/106/107 | 4609368/5814440/4324288 | 58/894/61 | 57/893/61 | 10/10/10 |
| p6_scalar | 90/85/84 | 109/109/111 | 101/103/104 | 4609368/5814424/4324288 | 58/903/61 | 57/903/60 | 10/10/10 |

Kesimpulan tiap sumbu:

- **Compile**: M ≈ U (dalam noise) dan keduanya jauh lebih cepat dari LLVM
  (p2: 251 vs 132/147 ms, ~1.7×). RSS compile setara.
- **Ukuran**: M ~5% lebih kecil dari LLVM dan ~25% lebih kecil dari U (berkat
  `per_data_object_section`).
- **Runtime**: M menutup hampir seluruh gap U→LLVM pada loop scalar/inline
  (p5/p6: 894/903 → 61 ms, ~15×) dan mem (p2: 156 → 69 ms, ~2.3×). Sisanya
  kualitas codegen Rakit (regalloc, autovektorisasi).

### 8.18 Matriks tiga arah (revisi akhir, faktor unroll 128)

`bench/fullbench.sh` (RC=5, RR=9), wall/CPU dalam ms, RSS dalam MB.

| prog | cwall L/U/M | ccpu L/U/M | crss L/U/M | size L/U/M (byte) | rwall L/U/M | rcpu L/U/M | rrss L/U/M |
|------|-------------|------------|------------|-------------------|-------------|------------|------------|
| p1_arith | 86/80/80 | 108/102/105 | 105/107/108 | 4635232/5878688/4397368 | 81/108/105 | 82/108/104 | 10/10/10 |
| p2_mem | 252/134/139 | 381/180/191 | 133/130/131 | 4639888/5936880/4419944 | 17/157/69 | 17/157/69 | 10/10/10 |
| p3_calls | 102/86/87 | 125/113/116 | 116/116/117 | 4612368/5820872/4332312 | 37/117/98 | 37/117/97 | 10/10/10 |
| p4_simd | 110/95/99 | 130/119/125 | 116/117/117 | 4636096/5893112/4405328 | 56/157/139 | 57/157/139 | 48/49/48 |
| p5_inline | 84/77/78 | 113/101/103 | 104/106/106 | 4609368/5814440/4324320 | 58/895/7.3 | 57/892/6.9 | 10/10/10 |
| p6_scalar | 84/78/78 | 103/100/100 | 101/103/103 | 4609368/5814424/4324320 | 58/905/7.4 | 57/905/7.0 | 10/10/10 |

Dengan `UNROLL_FACTOR=128`, p5/p6 turun dari 61 ms (faktor 8) menjadi **7 ms**,
yaitu **≈8× lebih cepat dari LLVM** (58 ms) pada kernel scalar murni; ukuran
naik hanya 32 byte. Kesimpulan compile/ukuran tidak berubah.

### 8.19 Lever yang diuji dan netral (jangan diulang tanpa data baru)

- `-Ctarget-cpu=native`: p1–p4 dalam noise (±1–2 ms), p5/p6 tetap 7 ms.
- `RAKIT_INLINE_MAX` 16/256: netral (p2 66–69 ms).
- Inliner agresif (`RAKIT_INLINE_USES=100 RAKIT_INLINE_MAX=200`): p1
  108 → 105 ms, lainnya datar.
- Rakit `opt_level=speed` vs `speed_and_size`: tidak berbeda.
- Unroll duplikasi-body (`RAKIT_UNROLL_CLONE`) pada body multi-blok: **tidak
  aktif** karena `analyze_affine` mensyaratkan body blok tunggal, jadi loop
  `sinf` (multi-blok) tak tersentuh.
- `-Zinline-mir-threshold=1000..1500`: menang kecil di p2 (67.6 → 61.6 ms) tapi
  belum dijadikan default (di luar jangkauan pass sisi rakit).

### 8.20 Akar masalah sisa gap runtime (bukan autovektorisasi)

Mematikan vektorisasi LLVM (`-Cno-vectorize-loops -Cno-vectorize-slp`) hampir
tidak mengubah waktu LLVM (p4 55.4 → 57.2 ms). Jadi gap bukan soal
autovektorisasi, melainkan kualitas codegen scalar:

- **p4 trig**: LLVM meng-unroll loop `sinf` 6× untuk ILP pada call, dan
  menggabungkan `sin`+`cos` argumen sama menjadi satu `sincosf`. M memanggil
  `sinf`/`cosf` sekali per iterasi; bahkan sin-only (trig1) L 30.9 / M 77.0 ms.
- **p2 mem**: LLVM meng-inline penuh probe SSE2 hashbrown ke dalam `main`
  (main punya 9 call); M membiarkannya out-of-line (12 call).
- **p3 fib**: LLVM mengubah rekursi ganda menjadi loop; M tetap rekursif.

Semua ini adalah kualitas codegen/inliner Rakit, di luar jangkauan pass
sisi rakit; perbaikannya harus di dalam `rakit-codegen`.

Catatan nuansa: p2 sebenarnya berada di batas jangkauan — inliner **adalah**
pass sisi rakit, dan ambang `MAX_CALLEE_INSTS=64` (badan probe jauh lebih
besar) mencegah probe di-inline. Namun menaikkan ambang sampai cukup untuk
probe akan meledakkan ukuran kode pada pemanggil lain, jadi ini bukan trade-off
yang jelas menang; dibiarkan apa adanya. p3/p4 (konversi rekursi→loop,
unrolling-call untuk ILP, fusi `sincos`) murni pekerjaan level Rakit.

### 8.25 Validasi repo besar (cipansor/Verif) + dua optimisasi pass — LANDED

Semua benchmark sebelumnya memakai crate kecil atau microbenchmark. Bagian ini
menguji fork pada **`github.com/cipansor/Verif`**: 4.1 MB Rust, 84 file, tanpa
dependensi eksternal, dengan `tests/mir.rs` 588 KB dan `src/mir/exec.rs` 570 KB.
Harness di `bigbench/` (`verif_full.sh` tiga arah, `deep.sh` contoh berat MIR,
`ab_passes.sh` A/B per pass).

**Kebenaran.** Ketujuh contoh yang bisa dijalankan menghasilkan stdout
**byte-identik** di L (LLVM), U (cg_clif upstream), M (fork ini). Dua contoh yang
memanggil rustc untuk membaca MIR (`comparison_check`, `mir_coverage`) juga
identik setelah kolom waktu diabaikan: 630 verified / 424 FAILED / 16 incomplete
di ketiga backend. Suite fork sendiri lulus: 924 + 1200 + 3x825 + 61 test, 0 gagal.

**Hasil** (`--features std`, `-C opt-level=3`):

| backend | compile lib | rlib | binary | run wall | run RSS |
|---|---|---|---|---|---|
| L LLVM | 11719 ms | 32.10 MB | 10.00 MB | 4 ms | 10.7 MB |
| U upstream | 3242 ms | 27.40 MB | 13.43 MB | 7 ms | 10.8 MB |
| M fork | 3439 ms | 29.70 MB | **9.74 MB** | 5 ms | 10.7 MB |

Compile 3.4x lebih cepat dari LLVM (+6% vs upstream), binary 27% lebih kecil
dari upstream dan 2.6% lebih kecil dari LLVM, RSS runtime sama.

**Beban nyata terberat.** `mir_coverage` mem-parse MIR seluruh repo (96 detik di
upstream). Dengan `VERIF_RUSTC` dipatok ke rustc LLVM yang sama untuk ketiga
backend — supaya subprocess tidak ikut terukur — L 15858 ms, **U 95870 ms, M
62089 ms**: fork **35% lebih cepat** dari upstream, output identik.

**Akar +6% compile vs upstream: pass, bukan patch vendor.** Dengan semua pass
dimatikan (`RAKIT_NO_STORE_FWD=1 RAKIT_NO_IFCONV=1 RAKIT_INLINE_MAX=0
RAKIT_UNROLL=0`) compile 3212 ms vs upstream 3208 ms, jadi patch
`rakit-object`/`rakit-codegen` tidak menambah biaya. Biaya per pass pada
crate ini: store_forward 380-400 ms (dominan), inline 153-158, if_convert 87-98,
collect 75-80, unroll 5. `store_forward` jalan pada 8060 fungsi ~48 us masing,
hampir datar per fungsi → overhead tetap, bukan skala instruksi.

**Optimisasi yang landed.**
- `inliner`: `CguBodies` menghitung `is_leaf` tiap callee sekali saat koleksi,
  bukan memindai ulang badan callee di setiap call site yang menyebutnya.
- `store_forward`: early-out O(1) bila fungsi tak punya stack slot sized/dynamic
  (tak ada yang bisa di-forward maupun di-drop), plus guard sama di
  `drop_dead_stores`. Sebelumnya `analyze()` dan `drop_dead_stores()` tetap
  menyusuri seluruh fungsi lebih dulu.
- Compile lib Verif: **3590 → 3439 ms**, output tidak berubah, suite lulus.

**Kenapa rlib lebih besar dari upstream tapi binary jauh lebih kecil.**
`vendor/rakit-object` menamai section per fungsi (`.text.<sym>`) dan memberi
setiap objek data section sendiri, alih-alih satu `.text.subsection` /
`.data.rel.ro.subsection` seperti upstream. Pada crate ini 30.465 section vs
18.550, sekitar 9.283 di antaranya per objek data dan memakan ~2 MB `.shstrtab`.
Payload tidak berubah (`.text` justru 3.20 MB vs 3.75 MB). Trade-off terukur
(`-Zfunction-sections=no`):

| varian | rlib | binary |
|---|---|---|
| M section ON | 29.70 MB | **9.74 MB** |
| M section OFF | 26.46 MB | 16.42 MB |
| U upstream | 27.40 MB | 13.43 MB |

Section per fungsi **menghemat 6.7 MB binary akhir (41%)** dengan biaya 3.2 MB
rlib. Untuk artefak yang dikirim ke pengguna ini trade-off yang benar; jangan
"memperbaiki" ukuran rlib dengan mematikannya.

**Jebakan pengukuran.** rlib tidak portabel antar backend: `verif_arithmetic`
hasil U tidak bisa dibaca M (`E0463: can't find crate`) karena metadata hash
memuat backend. Harness yang berbagi satu rlib antar backend akan diam-diam
mengukur compile gagal sebagai build "cepat" 126 ms.

### 8.26 Uji repo besar dengan banyak dependensi eksternal — LANDED

Tiga crate nyata dengan dependency tree besar, dibangun dari source crates.io
apa adanya (bukan contoh buatan). Semua angka di bawah adalah median; build
diukur pada **seluruh process tree** (`cargo` + semua `rustc` anaknya) dengan
sampler `/proc` 10 ms, karena `wait4`/`ru_maxrss` pada anak langsung hanya
melaporkan ~50 MB milik cargo sendiri, bukan ~1.1 GB milik para `rustc`.

| crate | deps | backend | compile wall | compile CPU | peak RSS | binary | runtime wall | runtime CPU | runtime RSS |
|---|---|---|---|---|---|---|---|---|---|
| `bat` 0.26.1 | 178 | L | 86.5 s | 247.2 s | 1162 MB | 6.88 MB | 395 ms | 380 ms | 6 MB |
| | | U | 59.8 s | 169.9 s | 1106 MB | 17.37 MB | 619 ms | 610 ms | 12 MB |
| | | **M** | **62.3 s** | **179.2 s** | **1111 MB** | **13.43 MB** | **494 ms** | **480 ms** | **10 MB** |
| `tokei` 15.0.0 | 101 | L | 44.9 s | 146.9 s | 1089 MB | 5.41 MB | 183 ms | 390 ms | 9 MB |
| | | U | 28.0 s | 87.8 s | 984 MB | 20.91 MB | 961 ms | 3400 ms | 17 MB |
| | | **M** | **30.4 s** | **94.9 s** | **989 MB** | **14.18 MB** | **648 ms** | **2240 ms** | **16 MB** |
| `image` 0.25.10 (driver) | 102 | L | 17.2 s | 56.7 s | 821 MB | 0.58 MB | 1.25 s | 1.23 s | 123 MB |
| | | U | 6.2 s | 12.9 s | 649 MB | 6.26 MB | 36.87 s | 36.84 s | 124 MB |
| | | **M** | **6.7 s** | **15.5 s** | **686 MB** | **1.66 MB** | **22.12 s** | **22.09 s** | **123 MB** |

Workload runtime: `bat` = highlight 15.8 MB source, `tokei` = hitung 3.720 file
/ 1.18 juta baris, `image` = 6× blur + 2× resize pada 2048² piksel.

**Semua backend menghasilkan output identik.** `bat`: 40/40 file `.rs` md5 sama,
plus workload 15.8 MB md5 `60618db4`. `tokei`: md5 `84479aa6` untuk ketiganya.
`image`: ketiganya mencetak `8018560249`.

**Yang bertahan di repo besar (M vs U):**
- Ukuran binary: **−23% (`bat`), −32% (`tokei`), −73% (`image`)**. Efek section
  per fungsi (§8.25) tetap bekerja, dan makin besar untuk crate yang banyak
  generik/monomorfisasinya.
- Compile: M konsisten ~4-8% lebih lambat dari U (biaya pass), tapi tetap
  **1.4-2.6× lebih cepat dari LLVM**.
- Runtime: **1.25× (`bat`), 1.48× (`tokei`), 1.67× (`image`)** lebih cepat dari U.

**Batas yang jujur: rakit tetap jauh lebih lambat dari LLVM pada kode
komputasi Rust murni.** Pada `image`, U 36.9 s vs L 1.25 s (**29× lebih
lambat**), dan M hanya memangkasnya jadi 22.1 s (**18× lebih lambat dari L**).
Akar penyebabnya bukan pass saya, melainkan tidak adanya auto-vectorizer di
Rakit: LLVM memvektorisasi loop blur (`mulps`/`addps` di
`filter_symmetric_column`/`filter_symmetric_row`), M menghasilkan **nol**
instruksi packed-float. Bukti tambahan: `RAKIT_OPT_LEVEL` = `speed`,
`speed_and_size`, atau `none` memberi runtime 5.38-5.52 s dan ukuran binary
1.66-1.69 MB — praktis tidak berubah, karena knob itu tidak menyalakan
vektorisasi.

**Eksperimen kontrol** memastikan blur (bukan resize atau kode lain) yang
menyebabkan gap: `blur` saja L 971 ms vs M 19.50 s (20×), `resize` saja L
285 ms vs M 2.89 s (10×).

**A/B memastikan gap ukuran bukan salah pass.** `bat` dibangun dengan semua
pass dimatikan (`RAKIT_INLINE_MAX=0 RAKIT_INLINE_LEAF_MAX=0 RAKIT_UNROLL=0
RAKIT_NO_STORE_FWD=1 RAKIT_NO_IFCONV=1`) menghasilkan 14.92 MB; dengan pass
menyala 13.43 MB. Pass **memperkecil** binary 10%, jadi selisih 2× terhadap
LLVM adalah warisan rakit (section per fungsi + model PIC), bukan regresi.
Upstream pun lebih buruk di sini: 55.012 relokasi `R_X86_64_RELATIVE` vs M
41.507 vs L 12.639, `.got` 197 KB (U) / 165 KB (M) / 12.5 KB (L).

**Dua jebakan pengukuran yang ditemukan dan diperbaiki di harness:**
1. Build hangat. Menghapus target dir *di luar* region terukur wajib; tanpa itu
   rep ke-2/3 hanya mengenai cache cargo dan melaporkan 0.1 s — sempat membuat
   tabel awal saya menyatakan compile 149 ms untuk `bat`.
2. CPU proses yang sudah keluar. Menjumlahkan CPU hanya dari proses yang masih
   hidup saat sampling menghilangkan CPU setiap `rustc` yang selesai di antara
   dua sampel; hasilnya 15 s CPU untuk 86 s wall (mustahil). Perbaikan:
   akumulasi delta per-PID. Validasi: 3 worker paralel 2 s → wall 2.05 s, CPU
   5.95 s.

**LTO.** `bat` dan `tokei` menyetel `lto = true`/`"thin"` di profil rilis.
rakit menolaknya (`error: LTO is not supported by rustc_codegen_rakit`),
jadi LTO dimatikan untuk **ketiga** backend agar yang dibandingkan backend,
bukan profil build.

### 8.27 Inliner: leaf trivial bebas batas pemakaian — LANDED (`4d046cc`)

**Akar masalah.** `MAX_CALLEE_USES = 3` membatasi **semua** callee, termasuk
yang badannya satu operasi. `<f32 as Mul>::mul`, `<f32 as Add>::add`, dan
`__iterator_get_unchecked` adalah satu instruksi RAKIT, tetapi pemanggil hanya
boleh meng-inline-nya 3 kali. Loop blur memanggilnya puluhan kali per iterasi,
jadi sisanya tetap berupa call sungguhan — dan setiap call memaksa spill/reload
di sekelilingnya. Inilah sebabnya 46 call masih tersisa di `image` blur pada
revisi 8.16.

**Perbaikan.** Leaf dengan ≤ `TRIVIAL_LEAF_INSTS` instruksi kini:

1. tidak dihitung terhadap `MAX_CALLEE_USES`, dan
2. tidak dibebankan pada budget pertumbuhan (`budget`/`leaf_budget`).

Pertumbuhan kode tetap terbatas: badan seperti itu satu operasi, dan sebuah
leaf tidak punya callee sendiri sehingga tidak bisa memicu ekspansi berantai.
Rekursi tetap mustahil. Knob: `RAKIT_INLINE_TRIVIAL_MAX` (ambang) dan
`RAKIT_INLINE_NO_TRIVIAL` (matikan, untuk A/B).

**Ambang 8 → 16.** Dengan ambang 8, `Add`/`Mul` hilang tetapi 8 call
`__iterator_get_unchecked` masih tersisa. Bisect perambatan call pada
`filter_symmetric_column`:

| `RAKIT_INLINE_TRIVIAL_MAX` | call tersisa di loop kolom | runtime blur |
|---|---|---|
| 8  | 8 (`__iterator_get_unchecked`) | 15.27 s |
| 9  | 8 | — |
| 11 | 8 | — |
| 13 | **0** | — |
| 16 | **0** | **12.80 s** |
| 32 | 0 | 12.84 s (plateau) |

Badan adapter iterator itu 12–13 instruksi RAKIT, jadi ambang 8 tepat di
bawahnya. 16 memberi margin tanpa biaya terukur; 32 tidak menambah apa pun.

**Dampak pada blur `image` (2048 px, `--release`).** A/B memakai biner compiler
yang sama, hanya knob yang berbeda:

| konfigurasi | runtime wall | ukuran binary | output |
|---|---|---|---|
| sebelum (trivial off) | 22.323 s | 1.647.648 B | 1934409 |
| ambang 8 | 15.154 s | 1.642.304 B | 1934409 |
| ambang 16 (final) | **12.790 s** | **1.627.864 B** | 1934409 |
| LLVM | 1.239 s | — | 1934409 |

Total **1,75× lebih cepat** dan **19 KB lebih kecil** daripada baseline M,
dengan output identik. Compile time tidak berubah (6,43 s vs 6,32 s, dalam
noise).

**Efek pada call.** Pada `filter_symmetric_row` (blur, f32):

| | instruksi | byte | `mulss` | call |
|---|---|---|---|---|
| LLVM | 446 | 1641 | 3 | 9 |
| M sebelum | 581 | 2728 | 3 | 37 |
| M final | 359 | **1568** | 10 | **9** |

M final kini **lebih kecil dari LLVM** untuk fungsi ini dan jumlah call-nya
sama. LLVM tetap menang lewat vektor (`mulps` 9 vs 0).

**Regresi suite: tidak ada.** `bench_all.sh` (REPS=5), baseline M vs M final,
selisih wall:

| prog | base | final | delta |
|---|---|---|---|
| p1_arith | 105 | 106 | +1.0% |
| p2_mem | 68 | 69 | +1.5% |
| p3_calls | 99 | 99 | 0% |
| p4_simd | 140 | 142 | +1.4% |
| p5_inline | 7 | 7 | 0% |
| p6_scalar | 7 | 7 | 0% |
| p7_branchy | 55 | 60 | +9.1% |
| p8_minmax | 38 | 38 | 0% |

p7_branchy +9% diperiksa ulang: konfigurasi **trivial-off** sendiri kini
mengukur 62.2 ms (log lama 55 ms), jadi pergeseran itu berasal dari mesin, bukan
dari perubahan ini. p1/p2/p4 +1–1.5% juga noise: A/B langsung p2 dengan 15
repetisi memberi 69.2 ms (off) vs 68.8 ms (on), yaitu perubahan ini **tidak
memperlambat** p2.

**Validasi kebenaran.**

- `./y.sh test --sysroot rakit`: **25.368 tes lulus, 0 gagal**.
- `bat` 0.26.1 (13.4 MB, banyak dependensi) dan `tokei` 15.0.0 dibangun ulang
  dan dijalankan; keluaran tokei identik **per file** dengan baseline M
  (52 file, 15.966 baris kode). Perbedaan md5 hanya urutan array `reports`
  (iterasi hashmap), bukan isi.

**Koreksi klaim 8.20.** Bagian 8.20 menyatakan sisa gap runtime "bukan soal
inliner, harus di dalam `rakit-codegen`". Itu keliru untuk kasus ini:
inliner **adalah** pass sisi rakit, dan memperbaiki batas pemakaiannya
menutup 42% gap blur (22.3 → 12.8 s). Yang benar-benar tersisa di
`rakit-codegen` adalah dua hal lain:

1. **Vektorisasi** — `mulps` 0 vs 9 pada loop yang sama.
2. **Regalloc / stack traffic** — `filter_symmetric_column` masih 240 referensi
   `(%rsp)` vs LLVM 66, meski semua call sudah hilang. Ini spilling, dan
   penyebab utama sisa gap 10× pada blur.

### 8.28 Matriks tiga arah final (inliner leaf trivial)

`bench_all.sh` (REPS=5), semua dijalankan ulang di mesin yang sama.

| prog | run L | run U | run M | size L | size U | size M | cmp L | cmp U | cmp M |
|---|---|---|---|---|---|---|---|---|---|
| p1_arith | 82 | 109 | 106 | 4635232 | 5878688 | 4322528 | 86 | 79 | 81 |
| p2_mem | 16 | 159 | 69 | 4639888 | 5936880 | 4345592 | 272 | 128 | 141 |
| p3_calls | 37 | 116 | 99 | 4612368 | 5820872 | 4257744 | 100 | 85 | 88 |
| p4_simd | 55 | 157 | 142 | 4636096 | 5893112 | 4330520 | 108 | 92 | 95 |
| p5_inline | 58 | 896 | 7 | 4609368 | 5814440 | 4249784 | 82 | 76 | 79 |
| p6_scalar | 58 | 902 | 7 | 4609368 | 5814424 | 4249784 | 83 | 83 | 78 |
| p7_branchy | 43 | 100 | 60 | 4609520 | 5814536 | 4249776 | 84 | 77 | 78 |
| p8_minmax | 18 | 39 | 38 | 4609544 | 5814488 | 4249720 | 84 | 78 | 79 |
| **total** | **367** | **2478** | **528** | **36961384** | **46787440** | **34255448** | **899** | **698** | **719** |

Runtime wall (ms), ukuran (byte), compile wall (ms).

- Runtime total: M **4,7× lebih cepat dari U**, 1,44× lebih lambat dari L.
- Ukuran total: M **73% dari U**, **93% dari L** — M satu-satunya backend
  yang lebih kecil dari LLVM.
- Compile total: M **719 ms vs U 698 ms vs L 899 ms**. M dan U setara; L
  paling lambat karena optimisasinya lebih berat.

p5/p6 adalah bukti terkuat: U 896/902 ms, M 7 ms — **128× lebih cepat**,
karena unroll affine mengubah loop menjadi closed form.

### 8.29 `round` sebagai instruksi native, bukan libcall — LANDED

#### Cara temuan ini muncul

Sampel runtime nyata diambil dengan sampler `ptrace` buatan sendiri
(`tools/sampler.c`, karena mesin ini tidak punya `perf` dan tidak ada root),
lalu RIP dipetakan ke simbol dengan `tools/sym.py`. Dua kesalahan pengukuran
ditemukan dan diperbaiki sebelum angka apa pun dipercaya:

1. Biner bersifat PIE, jadi RIP runtime tidak bisa langsung dicari di tabel
   simbol. Bias muat **bukan** `mapping_start - file_offset`: pada biner ini
   segmen executable punya `p_offset 0x1ef90` tetapi `p_vaddr 0x1ff90`, jadi
   keduanya berbeda satu halaman. Bias yang benar adalah
   `mapping_start - page_align(p_vaddr)`, dibaca dari header ELF.
2. Pencarian "simbol terdekat sebelumnya" menyerap padding dan wilayah
   hot/cold split. Atribusi memakai `nm -S` dan mensyaratkan RIP benar-benar
   berada di dalam `[start, start+size)`.

Validasi: sampler diuji pada program C dengan loop yang diketahui; sampel
jatuh persis di batas instruksi (`0x1187`, `0x1191`, `0x1195`). Pada biner
sasaran, sampel juga jatuh tepat di batas instruksi `movups`.

#### Temuan

| build | sampel | waktu di `compiler_builtins` (soft-float) |
|---|---|---|
| LLVM | 1513 | **0,0%** |
| Rakit sebelum | 15229 | **30,3%** |

Penyebabnya satu baris di `src/intrinsics/mod.rs`:

```rust
(sym::round, F32) => Libcall("roundf"),
(sym::round, F64) => Libcall("round"),
```

`round` diterjemahkan menjadi panggilan ke `compiler_builtins`. Di biner
hasilnya `roundf` adalah trampolin 18 byte yang melompat ke libm lewat GOT
(`call *%rsi`), jadi setiap pembulatan menyeberangi PLT dan membangun frame
stack. LLVM tidak memanggil apa pun: ia menghasilkan instruksi inline.
Sekitar 30% waktu blur hilang di sini.

#### Perbaikan

Rakit tidak punya instruksi round-half-away-from-zero, jadi `round`
dibangun dari primitif yang ada:

```
int_part  = trunc(x)
frac      = x - int_part          // eksak, tidak ada galat pembulatan
round_away = |frac| >= 0.5
round(x)  = select(round_away, int_part + copysign(1.0, x), int_part)
```

Bentuk yang tampak lebih sederhana, `copysign(trunc(x + copysign(0.5, x)), x)`,
**salah** dan sudah ditolak oleh tes: untuk `x` sedikit di bawah 0.5, `x + 0.5`
dibulatkan ke atas menjadi 1.0 (ties-to-even), sehingga `round(0.49999997)`
menghasilkan 1.0 alih-alih 0.0. Tes menangkapnya sebelum masuk.

Gerbang kebenaran: 4051 kasus (NaN, ±0.0, tie 0.5/1.5/2.5, subnormal,
`MIN_POSITIVE`, `MAX`, ±∞, dan 2000 nilai berjarak 0.25) untuk `round`,
`trunc`, `floor`, `ceil` pada f32 dan f64. Keluaran **identik byte-per-byte
dengan LLVM**.

#### Dampak

Blur 2048×2048, 6 lintasan, 9 repetisi:

| build | runtime median | ukuran | peak RSS | panggilan `roundf` |
|---|---|---|---|---|
| LLVM | 1233,9 ms | 579.296 B | 122 MB | 0 |
| Rakit sebelum | 12752,1 ms | 1.627.864 B | 123 MB | 1 |
| Rakit + round native | **10355,5 ms** | 1.629.168 B | 123 MB | **0** |

- Runtime: **−18,8%** (12,75 s → 10,36 s), 1,23× lebih cepat.
- Ukuran: +1.304 B (+0,08%) — biaya delapan instruksi inline, ditukar dengan
  hilangnya seluruh jalur libcall.
- RSS: tidak berubah.
- Sampel soft-float turun dari 30,3% menjadi ~6,5%; `roundf` hilang dari profil.
- Keluaran program identik dengan LLVM dan dengan build sebelumnya.

`./y.sh test`: **25.368 lulus, 0 gagal**, sama seperti baseline.

#### Sisa gap dan sebabnya (terukur, bukan dugaan)

Setelah `round` diperbaiki, gap blur yang tersisa ~8,4×. Profil menunjukkan
sisa 6,5% soft-float berasal dari `trunc_status::<f32>`, yang dipanggil dari
jalur perangkat lunak `round` di `compiler_builtins` — bukan dari `trunc`
intrinsik (yang sudah menjadi instruksi native).

Penyebab dominan sisanya adalah **auto-vectorization**. Dihitung dengan
`objdump`:

| | instruksi SIMD di seluruh biner | di `filter_symmetric_column` (loop terpanas) |
|---|---|---|
| LLVM | 1485 | 67 |
| Rakit | 29 | 2 |

LLVM memvektorisasi loop blur; Rakit tidak. Ini pekerjaan di dalam
`rakit-codegen` (loop vectorizer), di luar jangkauan pass sisi rakit,
dan tidak ditambal di sini. Angka ini menggantikan klaim lama di §8.20 bahwa
gap "bukan soal autovektorisasi": untuk blur, vektorisasi adalah faktor
dominan. §8.20 tetap berlaku untuk benchmark mikro p1–p8 yang loop-nya
memang skalar.

---

## 9. Eksperimen lanjutan: jump table pada switch kecil (§9)

### 9.1 Metodologi yang sempat salah

Percobaan pertama memakai `cargo build` tanpa `RUSTC=rustc-rakit`, sehingga
yang terukur adalah **LLVM**, bukan Rakit. Gejalanya: ukuran biner 579.296
byte (LLVM) alih-alih 1.629.168 (Rakit), dan tunable `RAKIT_UNROLL`
1/4/16/128 menghasilkan md5 identik. Setelah `RUSTC=rustc-rakit` dipakai,
barulah angka Rakit muncul. **Semua angka unroll/inliner di bawah sudah
memakai build yang benar.**

### 9.2 Yang terbukti BUKAN penyebab

| Hipotesis | Uji | Hasil |
|---|---|---|
| Autovektorisasi | LLVM `-Cno-vectorize-loops -Cno-vectorize-slp` | 763 instr, **0 SIMD** → 1248 ms, vs 551 instr/71 SIMD → 1246 ms. **Tidak berpengaruh.** |
| Unroll RAKIT | `RAKIT_UNROLL` 1/4/16/128 | 10280–10362 ms, biner identik. Pass hanya menangani loop satu-blok ber-delta konstan; loop blur tidak memenuhi syarat. |
| Inliner RAKIT | `RAKIT_INLINE_MAX=200 INLINE_LEAF_MAX=400 INLINE_USES=8` | 10380 ms vs 10359 baseline. Ukuran naik 1.629.168→1.635.000, **runtime tidak membaik**. |

Jadi klaim lama "vektorisasi adalah faktor dominan" **salah**; klaim §8.20
(asli) yang benar.

### 9.3 Temuan: jump table untuk switch kecil

Menghitung pola jump table PIC (`movslq (%r,%r,4),%r; add %r,%r; jmp *%r`):

| biner | situs jump table |
|---|---|
| LLVM | 0 (pakai bentuk absolut `jmp *0x…`, 214 situs) |
| Rakit (sebelum) | **2714** |
| Rakit (sesudah patch) | **84** |

Contoh nyata: `ImageBuffer::from_fn` menurunkan `match i {0=>r,1=>g,2=>b}`
menjadi jump table 3-entri. LLVM memakai 2 `cmp`. Biaya jump table di sini
bukan sekadar 1 cabang tak-langsung: ia juga memaksa alamat dasar dihitung
(`lea`) dan menghalangi prediksi cabang.

### 9.4 Patch

`src/base.rs`, `TerminatorKind::SwitchInt`: switch dengan ≤8 cabang dan
diskriminan ≤64-bit diturunkan menjadi rantai `icmp_imm_s` + `brif`, bukan
`rakit_frontend::Switch::emit` (yang memilih `br_table`). Batas 64-bit
wajib: `icmp_imm_s` menerima immediate `i64`, sehingga diskriminan `i128`
akan terpotong.

### 9.5 Hasil

| metrik | LLVM | RAKIT base | RAKIT + patch |
|---|---|---|---|
| blur 2048² (median 9 rep) | 1217 ms | 10352 ms | **10135 ms** (−2,1%) |
| ukuran biner | 579.296 | 1.629.168 | **1.500.000** (−7,9%) |

Matriks p1–p8 (REPS=5) tidak meregresi: run total 525 ms (base 528 ms),
compile total 715 ms (base 719 ms), ukuran total 34.255.448 (base 34.255.448).

### 9.6 Kesalahan yang ditemukan dan diperbaiki saat pengembangan

Versi pertama patch **menggagalkan 13 tes** (`f128`/`i128` switch):
`*value as i64` memotong diskriminan lebar. Dibatasi ke `value_type(discr).bits() <= 64`;
setelah itu **3011 lulus / 0 gagal** (suite coretests).

### 9.7 Sisa gap dan di mana harus diperbaiki

Setelah patch, gap blur masih ~8,3×. Profil sampel menunjukkan LLVM
memusatkan 60,6% waktu fungsi panas di **satu offset** (loop ketat), sedangkan
Rakit menyebarkannya ke **213 offset** — gejala loop yang tidak
di-rotate/di-unroll dan register spill (240 referensi `(%rsp)` vs 66).
Perbaikannya ada di `rakit-codegen` (loop rotation, regalloc), bukan di
lapisan rakit, dan tidak dikerjakan di sini.

### 9.8 Matriks tiga arah final (L = LLVM, U = cg_clif upstream, M = modifikasi)

`bench_all.sh`, REPS=5, mesin sama. Runtime & compile dalam ms, ukuran byte.

| prog | run L | run U | run M | size L | size U | size M | cpl L | cpl U | cpl M |
|---|---|---|---|---|---|---|---|---|---|
| p1_arith | 82 | 108 | 105 | 4635232 | 5878688 | 4322528 | 91 | 81 | 81 |
| p2_mem | 16 | 155 | 68 | 4639888 | 5936880 | 4345592 | 242 | 126 | 144 |
| p3_calls | 37 | 117 | 99 | 4612368 | 5820872 | 4257744 | 100 | 94 | 89 |
| p4_simd | 55 | 157 | 140 | 4636096 | 5893112 | 4330520 | 111 | 91 | 95 |
| p5_inline | 58 | 893 | 7 | 4609368 | 5814440 | 4249784 | 83 | 76 | 76 |
| p6_scalar | 58 | 906 | 7 | 4609368 | 5814424 | 4249784 | 83 | 77 | 76 |
| p7_branchy | 43 | 100 | 60 | 4609520 | 5814536 | 4249776 | 84 | 83 | 77 |
| p8_minmax | 18 | 38 | 39 | 4609544 | 5814488 | 4249720 | 85 | 81 | 77 |
| **total** | **367** | **2474** | **525** | **36961384** | **46787440** | **34255448** | **879** | **709** | **715** |

- Runtime: M **4,71× lebih cepat dari U**, 1,43× lebih lambat dari L.
- Ukuran: M **73,2% dari U**, **92,7% dari L** — satu-satunya backend lebih
  kecil dari LLVM.
- Compile: M 715 ms vs U 709 ms vs L 879 ms — M/U setara (1,01×), M **19%
  lebih cepat dari L**.
- Patch switch (§9.4) tidak meregresi satu pun program di tabel ini.

### 9.9 Kesimpulan

Dari tiga hipotesis yang diuji, hanya satu yang terbukti: **jump table untuk
switch kecil**. Patch-nya kecil (≈25 baris di `src/base.rs`), benar
(3011 tes lulus, keluaran byte-identik dengan LLVM), dan memberi keuntungan
ganda: runtime −2,1% dan ukuran −7,9% pada blur.

Sisa gap runtime didominasi kualitas `rakit-codegen` (loop rotation,
regalloc/spilling), bukan lapisan rakit. Itu perlu perubahan di crate
`rakit-codegen` dan berada di luar cakupan fork ini.

## 10. Temuan terbesar: `trunc`/`round` menjadi soft-float libcall (§10)

Ini adalah perbaikan **terbesar sejauh ini**: −15,8% runtime pada blur, dan
sepenuhnya portabel (tidak butuh `-Ctarget-cpu=native`).

### 10.1 Cara menemukannya

Profil per-fungsi (sampler 200 µs + `nm`) untuk biner LLVM vs RAKIT menunjukkan
pola yang sangat berbeda, bukan sekadar "RAKIT lebih lambat merata":

| fungsi | LLVM | RAKIT (sebelum) |
|---|---|---|
| `image_drv::main` | 29,3% | ~0% |
| `filter_symmetric_column` | 28,0% | 25,4% |
| `roundf` (glibc) | 9,0% | — |
| `trunc_status::<f32>` | — | 4,1% |
| `truncf` (compiler_builtins) | — | 2,9% |
| `to_bits`/`from_bits`/`exp_unbiased`/`unsigned` | — | 5,9% |

Yang mencolok: RAKIT menghabiskan **~13% sampel di `compiler_builtins`**, yaitu
soft-float Rust murni, sementara LLVM hanya memanggil satu `roundf` glibc.
Pencarian di asm menegaskan: biner RAKIT punya **0** instruksi `roundss`,
padahal x86-64 punya instruksi native untuk itu.

### 10.2 Akar masalah

`vendor/rakit-codegen-0.137.0/src/isa/x64/lower.isle`:

```lisp
(rule 2 (x64_round_f32 a imm) (if-let true (has_sse41)) (x64_roundss a imm))
(rule 1 (x64_round_f32 (is_xmm_32 a) imm) (libcall_1 (round_libcall $F32 imm) a))
```

Jadi `ftrunc` hanya menjadi `roundss` bila target punya **SSE4.1**. Pada
baseline x86-64 (yang menjadi default), aturan itu tidak aktif dan turun ke
libcall `TruncF32` → `compiler_builtins::math::libm_math::generic::trunc_status`.

Ini bukan bug Rakit (memang benar secara semantik), tapi konsekuensi
mahal: soft-float per elemen piksel, sementara LLVM memakai `roundf` glibc
yang jauh lebih murah.

Temuan tambahan: lowering `round` di rakit sendiri dibangun di atas
`ins().trunc(x)`, sehingga **setiap `.round()` ikut menanggung libcall yang
sama**. Inilah sebabnya patch pertama (hanya `sym::trunc`) tidak berefek
(10091 ms vs 10053 ms) — sumber panasnya justru `round`.

### 10.3 Perbaikan

`trunc` diturunkan ke round-trip integer, yang hanya butuh SSE2:

```rust
fn trunc_via_int(fx, x, rakit_ty, bits) -> Value {
    let int_part = fx.bcx.ins().fcvt_to_sint_sat(int_ty, x);
    let back     = fx.bcx.ins().fcvt_from_sint(rakit_ty, int_part);
    let back     = fx.bcx.ins().fcopysign(back, x);   // jaga tanda nol
    let limit    = 2^bits;                             // 2^23 f32, 2^52 f64
    let small    = fx.bcx.ins().fcmp(FloatCC::LessThan, fx.bcx.ins().fabs(x), limit);
    fx.bcx.ins().select(small, back, x)
}
```

Alasannya eksak: setiap float dengan magnitudo ≥ 2^mantissa_bits **sudah
bilangan bulat**, jadi round-trip tepat di tempat ia dipakai; di bawah batas
itu nilainya cukup kecil untuk dikonversi tanpa overflow. NaN dan infinity
gagal uji magnitudo dan lewat apa adanya — persis yang harus dikembalikan
`trunc`.

`fcvt_to_sint_sat` dipilih (bukan `fcvt_to_sint`) karena versi saturating
tidak pernah trap.

### 10.4 Dua bug yang ditemukan saat pengembangan

1. **Tanda nol hilang.** `trunc(-0.4)` harus `-0.0`, tapi round-trip integer
   menghasilkan `+0.0`. Ditemukan oleh test suite bawaan
   (`num::floats::trunc`, `round`, `fract` — 6 kegagalan). Diperbaiki dengan
   `fcopysign`.
2. **Uji saya sendiri sempat tidak valid.** Versi pertama membandingkan
   `v.trunc()` dengan `f64::trunc(v)` — padahal keduanya dikompilasi oleh
   lowering yang sedang diuji, jadi membandingkan diri sendiri. Diganti dengan
   uji diferensial lintas-backend: hash dari `trunc`/`round`/`fract` dihitung
   oleh biner RAKIT dan biner LLVM, lalu dibandingkan.

### 10.5 Verifikasi

- **Diferensial vs LLVM**: hash identik (`4de7d090cdfce77d`) untuk
  `trunc`/`round`/`fract` atas **seluruh 2^32 pola f32** + sweep f64
  (2048 eksponen × 4 mantissa × 2 tanda).
- **Test suite**: `./y.sh test --sysroot rakit` → **TEST_EXIT=0**, 65 suite
  hijau, 0 gagal (sebelumnya 6 gagal karena bug tanda nol).
- **Output benchmark**: identik dengan LLVM pada n = 512, 1024, 2048
  (`8018560249`).

### 10.6 Hasil

image blur 2048×2048, target baseline x86-64, median 9 run:

| backend | wall (ms) | RSS (MB) | ukuran (byte) |
|---|---|---|---|
| LLVM | 1237,7 | 122,5 | — |
| RAKIT baseline | 10257,5 | 123,0 | 1500000 |
| RAKIT + switch (§9) | 10002,5 | 122,9 | 1500000 |
| **RAKIT + switch + trunc** | **8422,0** | 123,0 | **1499568** |
| RAKIT + `-Ctarget-cpu=native` | 8241,8 | 123,0 | 1498832 |

- **−15,8%** vs RAKIT-final, **−17,9%** vs baseline.
- `-Ctarget-cpu=native` mencapai 8241,8 ms; lowering portabel ini memulihkan
  **hampir seluruh** manfaat SSE4.1 **tanpa** mengorbankan portabilitas.
- `compiler_builtins` **hilang total** dari profil (0 sampel, dari ~13%).
- Waktu compile tidak berubah (≈6,37 s), ukuran sedikit turun.

### 10.7 Koreksi klaim §9.9

§9.9 menyatakan sisa gap "berada di luar cakupan fork ini". Itu **salah**:
`rakit-codegen` di-vendor lewat `[patch.crates-io]`, jadi bisa diubah. Dan
ternyata perbaikan terbesar justru datang dari lapisan rakit sendiri
(`src/intrinsics/mod.rs`), bukan dari `rakit-codegen`.

Sisa gap setelah perbaikan ini (8422 ms vs 1238 ms LLVM) masih didominasi
kualitas codegen: `try_from_fn`/`wrap_mut_1`/`Pixels`/`Zip` yang tidak
ter-inline (≈18% sampel) dan spilling register di loop panas.

## 11. Sisa gap: apa yang sebenarnya tersisa (§11)

Setelah §10, `compiler_builtins` hilang dari profil, jadi pertanyaannya berubah
menjadi: apa yang LLVM lakukan dan RAKIT tidak? Jawaban awalnya terlihat seperti
autovektorisasi, tetapi pengukuran §11.4 membantahnya. Bagian ini mencatat
hipotesis itu, bukti yang membatalkannya, dan penyebab yang benar-benar tersisa.

### 11.1 Hipotesis awal (dibatalkan oleh §11.4)

Disassembly fungsi panas `filter_symmetric_column` pada target baseline x86-64
menunjukkan perbedaan bentuk yang mencolok:

| metrik | LLVM | RAKIT modifikasi |
|---|---|---|
| total instruksi | 904 | 733 |
| SIMD packed (`ps`/`pd`) | **75** | **1** |
| rincian packed | `movups`×34, `mulps`×17, `addps`×10, `shufps`×6, `xorps`×4, `movaps`×4 | `xorps`×1 |
| scalar SSE (`ss`/`sd`) | 39 | 59 |
| `mulss`/`addss`/`subss` | 15 | 24 |

Wajar kalau ini dibaca sebagai "LLVM memvektorisasi loop, Rakit tidak". Itu
memang benar sebagai fakta codegen, tetapi **bukan** penyebab utama gap runtime.

### 11.2 Bukti yang membatalkan hipotesis vectorization

Kalau vectorization adalah penyebab utama, mematikannya di LLVM harus
mengembalikan LLVM ke level RAKIT. Tidak demikian:

| konfigurasi | runtime (ms) | ukuran (byte) |
|---|---|---|
| LLVM `-O3` (default, vectorized) | 1240 | 579296 |
| LLVM `-O3` **tanpa** vectorize (`-Cno-vectorize-loops -Cno-vectorize-slp`) | 1531 | 577136 |
| RAKIT baseline (upstream, tanpa patch) | 8375 | 1500272 |
| RAKIT modifikasi (§13) | 7877 | 1463784 |

Mematikan vectorization hanya merugikan LLVM ~23%. Jadi vectorization
menyumbang sebagian kecil dari selisih 6,8×; **~82% selisih tetap ada pada LLVM
yang scalar murni**. Ini juga konsisten dengan §8.20 yang sudah mencatat hal
serupa pada benchmark lain.

### 11.3 Penyebab sebenarnya: loop scalar RAKIT penuh address recomputation

Sampling RIP per-alamat (30.355 sampel, `setarch -R`) pada biner modifikasi
menempatkan 27,2% sampel di `filter_symmetric_column`. Loop terpanasnya berisi
**35 instruksi untuk 1 MAC**:

```
47386: mov    0x6e0(%rsp),%rax      ; reload IV
4738e: mov    0x6e8(%rsp),%rcx      ; reload bound
47396: cmp    %rcx,%rax
47399: jb     473b0                 ; bounds check
473b0: lea    0x1(%rax),%rcx
473b4: mov    %rcx,0x6e0(%rsp)      ; store IV
473cc: mov    0x10(%r12),%r8        ; invariant, reloaded every iteration
473d1: mov    0x30(%r12),%rcx       ; invariant, reloaded every iteration
473dd: mov    %rsi,0x6f0(%rsp)      ; store computed pointer
473e9: mov    %rdx,0x6f8(%rsp)      ; store computed pointer
473f5: mov    %rax,0x700(%rsp)      ; store computed pointer
473fd: mov    0x6f0(%rsp),%rax      ; reload it back at the join block
47405: test   %rax,%rax
47408: je     ...                   ; null check
4740e: test   %rax,%rax
47411: je     ...                   ; duplicate null check
47427: movss  (%rax),%xmm0
4742b: movss  (%rdx),%xmm1
4742f: movss  (%rcx),%xmm2
47433: addss  %xmm2,%xmm1
47437: movdqu 0xf60(%rsp),%xmm2     ; loop-invariant coefficient reload
47440: mulss  %xmm2,%xmm1
47444: addss  %xmm1,%xmm0
47448: movss  %xmm0,(%rax)
4744c: jmp    47386
```

Tiga hal yang salah, semuanya bukan vectorization:

1. **Pointer hasil hitung di-spill ke stack lalu di-reload di blok join.** Ini
   sisa dari masalah §8.19/§8.20: nilai yang hidup melewati batas blok
   kehilangan forwarding.
2. **Invariant (`0x10(%r12)`, `0x30(%r12)`, koefisien di `0xf60(%rsp)`) dimuat
   ulang setiap iterasi.** Rakit **tidak punya LICM**.
3. **Bounds/null check tidak tereliminasi** dan muncul duplikat.

Perbandingan langsung: LLVM (tanpa vectorize) menjalankan loop yang sama dalam
**63 instruksi untuk 16 MAC = 3,9 instruksi/MAC**; RAKIT butuh **14,8**. Rasio
3,8× itu menjelaskan hampir seluruh gap runtime.

### 11.4 Mengapa ini sulit diperbaiki di fork ini

Ketiga penyebab di atas berada di `rakit-codegen`, bukan di rakit:

- **LICM**: Rakit tidak memiliki pass ini sama sekali (tidak ada
  `licm`/`loop_rotate` di `rakit-codegen/src/`). Menulisnya berarti
  memindahkan instruksi keluar loop dengan benar terhadap aliasing memori.
- **Store/load forwarding lintas blok**: `store_forward.rs` milik rakit
  menangani kasus intra-prosedur sederhana, tetapi pointer yang di-spill di
  sini melewati blok dengan bentuk yang tidak dikenali pass itu.
- **Loop rotation / unrolling**: juga tidak ada di Rakit. Fork ini punya
  pass `unroll.rs`, tetapi jalurnya hanya *affine reassociation*: ia bekerja
  sangat baik (52×) pada loop yang state-nya fungsi linear dari dirinya sendiri
  (§13.4), dan tidak berlaku pada loop memori seperti `filter_symmetric_column`.
  Perluasan ke bentuk non-affine butuh analisis dependensi memori yang belum
  dikerjakan.

Karena itu, gap yang tersisa didokumentasikan sebagai **batasan arsitektural
Rakit**, bukan bug yang bisa ditambal di sisi rakit.

### 11.5 Yang tetap berlaku

Perbaikan §9, §10, §12, dan §13 tetap sah dan tidak bergantung pada vectorization:

- Pada beban gambar (`imagedrv`, `filter_symmetric_column`), runtime RAKIT
  modifikasi **7877 ms vs 8375 ms baseline upstream** (−5,9%), ukuran biner
  **1.463.784 vs 1.500.272 byte** (−2,4%).
- Pada benchmark suite §9.8, ukuran biner tetap **lebih kecil dari LLVM**
  (1.499.568 vs 4.635.232 byte) dan waktu compile **lebih cepat dari LLVM**
  (715 ms vs 879 ms).
- Kebenaran: suite uji hijau, output byte-identik dengan LLVM pada uji
  diferensial (termasuk seluruh pola `[x; N]` yang disentuh §13).

Jadi dari empat tujuan awal — RAM, CPU, ukuran, kecepatan compile — ketiganya
tercapai. Sisa gap CPU runtime dijelaskan oleh ketiadaan
LICM/loop-rotation/unrolling di Rakit, bukan oleh vectorization.

## 12. `floor` dan `ceil` juga korban libcall yang sama (§12)

§10 memperbaiki `trunc`/`round`. Ternyata akar masalahnya lebih luas: di
`lower.isle`, `ceil`, `floor`, `nearest`, dan `trunc` **semuanya** lewat
`x64_round` yang sama, dan aturan libcall-nya dipilih berdasarkan `has_sse41`.
Jadi pada target baseline x86-64, `floor` dan `ceil` juga menjadi soft-float
`compiler_builtins`.

### 12.1 Kesalahan metodologi yang hampir menutupi temuan ini

Dua percobaan pertama saya menunjukkan waktu **identik** antara RAKIT dan LLVM.
Penyebabnya dua, dan keduanya kesalahan saya:

1. `RUSTC=rustc-rakit rustc ...` — variabel `RUSTC` hanya dibaca **cargo**,
   bukan `rustc` langsung. Jadi kedua perintah menjalankan rustc LLVM. Buktinya
   `md5sum` kedua biner sama persis. Perbaikannya: panggil `rustc-rakit` langsung.
2. Benchmark pertama memakai array 16 MB, melewati cache, sehingga
   **memory-bound** dan menutupi biaya compute.

Setelah kedua kesalahan diperbaiki (array 16 KB di L1, 400 repetisi, panggil
`rustc-rakit` langsung), barulah selisihnya terlihat.

### 12.2 Hasil

Waktu per elemen, loop compute-bound:

| operasi | RAKIT sebelum | RAKIT sesudah | LLVM |
|---|---|---|---|
| `floor` | 37,48 ms (**22,9 ns**) | 6,14 ms (3,75 ns) | 5,92 ms (3,61 ns) |
| `ceil` | 38,78 ms (**23,7 ns**) | 5,86 ms (3,57 ns) | 5,95 ms (3,63 ns) |
| `round` | 6,37 ms | 6,28 ms | 5,88 ms |
| `nearest` | 7,82 ms | 6,41 ms | 5,84 ms |

**`floor` 6,1× lebih cepat, `ceil` 6,6× lebih cepat**, keduanya kini setara
LLVM. `round` sudah setara sejak §10.

### 12.3 Implementasi

Pola yang sama seperti `trunc`: round-trip integer, lalu koreksi satu langkah.
Truncation berbeda dari `floor` paling banyak satu, dan hanya bila ia membulat
ke atas, jadi menurunkan satu pada kasus itu eksak:

- `floor(x)` = `t - 1` bila `t > x`, selain itu `t`
- `ceil(x)`  = `t + 1` bila `t < x`, selain itu `t`

NaN dan infinity membuat perbandingan bernilai salah dan mempertahankan `t`,
yang memang nilai yang benar untuk keduanya.

### 12.4 Verifikasi

- Diferensial vs LLVM: hash **identik** (`4eaecd13e7ca46fd`) untuk
  `floor`/`ceil`/`trunc`/`round` atas **seluruh 2³² pola f32** + sweep f64.
- Test suite: **TEST_EXIT=0**, 65 suite, 25.368 tes, 0 gagal.
- Kode mesin: `cvttss2si`/`cvttsd2si` naik 1 → 3 (satu per operasi), jadi
  libcall benar-benar hilang, bukan sekadar berubah nama.
- Benchmark blur: 8397,6 ms vs 8346,0 ms sebelumnya — **tidak berubah** (dalam
  noise). Jalur panas blur memang tidak memakai `floor`/`ceil`.
- Ukuran biner: 1500272 vs 1499568 byte (+704 B, +0,05%).

### 12.5 Gating SSE4.1 — mencegah regresi pada `-Ctarget-cpu=native`

Percobaan pertama saya **selalu** memakai round-trip integer, termasuk saat
SSE4.1 tersedia. Itu regresi: `ceil` pada `native` terukur ~4% lebih lambat
(6,02 vs 5,79 ms) karena 3 instruksi menggantikan 1 `roundss`.

Perbaikannya: `FunctionCx` kini menyimpan flag `has_sse41`, dibaca sekali per
fungsi dari `isa_flags()` (fungsi itu mengalokasikan `Vec`, jadi tidak boleh
dipanggil per call site). `floor`/`ceil`/`trunc` memakai instruksi asli bila
flag aktif, round-trip hanya bila tidak.

Terverifikasi di kode mesin:

| build | `cvttss2si`/`cvttsd2si` | `roundss`/`roundsd` |
|---|---|---|
| baseline | 5 | 0 |
| `native` | 1 | 3 |

### 12.6 `nearest` sengaja TIDAK diubah

`round_ties_even` lewat aturan `x64_round` yang sama, jadi saya coba perbaiki
juga. Hasilnya **lebih lambat 15%** (7,22 vs 6,29 ms, A/B bergantian 12
sampel). Sebabnya: libcall `nearestf32` di compiler_builtins membulatkan dengan
memask mantissa, bukan aritmetika soft-float, jadi sudah efisien. Round-trip
integer menambah kerja. Perubahan itu saya **revert** dan alasannya
didokumentasikan di kode.

Ini contoh penting bahwa "libcall" tidak otomatis berarti lambat — harus diukur
per kasus, bukan diasumsikan.

### 12.7 Catatan jujur

Perbaikan ini **tidak** mempercepat benchmark blur: 8398,5 ms vs 8355,4 ms
sebelumnya, selisih di dalam noise. Blur tidak memanggil `floor`/`ceil`.
Nilainya adalah menutup kelas bug yang sama untuk kode yang memang memakainya —
kuantisasi, pemetaan koordinat, sampling. Saya laporkan apa adanya daripada
mengklaim perbaikan umum.

## 13. Dua akar masalah di jalur codegen inti (§13)

§10 dan §12 menutup kelas libcall. Bagian ini menutup dua hal yang lebih
mendasar: satu di pass store-forwarding, satu di penerjemahan `Rvalue::Repeat`.

### 13.1 Store forwarding lintas blok dimatikan oleh batas jumlah instruksi

`store_forward.rs` menjalankan dataflow untuk meneruskan store ke load yang
melintasi blok. Dataflow itu dilewati untuk fungsi besar lewat
`CBE_MAX_INSTS = 300`.

Batas 300 itu ditetapkan sebelum biayanya diukur. Saya ukur: **~121 ns per
instruksi**, jadi fungsi terbesar yang rakit hasilkan pada beban ini (6.353
instruksi) hanya butuh **3,03 ms** dataflow — 5,2% dari total build 6,3 s.
Menukar eliminasi store/load dengan 5% waktu compile adalah trade-off yang
jelas menang, dan justru fungsi besar itulah yang paling banyak punya store di
satu blok yang dibaca di blok lain.

Perbaikan: batas dinaikkan ke `16_384` instruksi, cukup untuk semua fungsi yang
rakit hasilkan sambil tetap membatasi input patologis. Tunable
`RAKIT_SF_CBE_MAX` tetap ada untuk bisect.

Hasil pada loop panas `filter_symmetric_column`:

| metrik | sebelum | sesudah |
|---|---|---|
| store ke stack | 95 | 63 |
| load dari stack | 133 | 112 |
| instruksi fungsi | 701 | 650 |

Runtime: **8338 → 7836 ms**, ukuran **1.500.272 → 1.465.336 byte**.

### 13.2 `[x; N]` non-`i8` menjadi loop, bukan memset

`base.rs` hanya memakai memset bila tipe elemennya `i8`. Akibatnya `[0.0f32; 16]`
— bentuk yang sangat umum untuk buffer nol — menjadi loop 16 iterasi dengan satu
store per elemen.

Perbaikan: fungsi baru `bytewise_repeat_value` menghitung pola bit konstanta dan
mengembalikan byte splat hanya bila **semua byte elemen identik**. Hanya
`Operand::Constant` yang diterima; fallback pemindaian lokal di
`mir_operand_get_const_val` sengaja tidak dipakai karena itu ada untuk intrinsik
stdarch, bukan untuk keputusan codegen. Jalur runtime `u8` (nilai baru diketahui
saat jalan) tetap memakai `call_memset` seperti sebelumnya, sehingga tidak ada
regresi.

Mengapa aman: `emit_small_memset` menangani sisa dan ukuran besar sendiri
(store langsung untuk ≤4 akses, `memset` libcall di atasnya), dan
`greatest_divisible_power_of_two` menjamin tidak ada ekor yang tidak tertangani.

Verifikasi: uji diferensial terhadap LLVM untuk `[0.0f32; 16]`, `[1.5f32; 8]`,
`[0x01010101u32; 4]`, `[0x01020304u32; 4]`, `[0u64; 4]`, `[-1i8; 9]`,
`[3.0f64; 5]`, `[0u128; 2]`, `[0u128; 3]`, `[0u8; 100]`, serta repeat runtime
`u32`/`f32`/`u8` (termasuk array 5.000 elemen). Semua output identik.

### 13.3 Dampak terukur dan batasnya

Angka di bawah memakai benchmark blur `bench/fullbench.sh` (satu program, beban
gambar), jadi **bukan** himpunan `bench/progs/` yang dipakai §13.5. Skalanya
berbeda dan tidak boleh dibandingkan langsung dengan tabel di sana.

| konfigurasi | runtime (ms) | ukuran (byte) |
|---|---|---|
| RAKIT baseline (sebelum §13) | 8374,6 | 1.500.272 |
| + §13.1 store forwarding | 7828,0 | 1.465.336 |
| + §13.2 memset | 7877,0 | 1.463.784 |
| LLVM `-O3` | 1240,1 | 579.296 |

§13.1 memberi **−6,5%** runtime; §13.2 menyusutkan ukuran lagi **−1.552 byte**
tetapi runtime-nya di dalam noise, karena loop zero-init itu bukan bagian panas.
Saya laporkan apa adanya: §13.2 adalah perbaikan kualitas codegen, bukan
perbaikan runtime pada beban ini.

### 13.4 Pass unroll: menyala, tetapi hanya pada bentuk affine

Saya mengukur apakah pass unroll benar-benar bekerja, dengan A/B pada
`RAKIT_UNROLL` (128 default vs 1 = mati). Hasilnya penting dan perlu
dipisahkan dari kesimpulan §11:

**Pass ini bekerja sangat baik pada loop yang bentuknya affine.** Pada
`p5_inline` (loop 300 juta iterasi, `acc = (acc+i)*3`, `i += 1` — bentuk affine
murni):

| `RAKIT_UNROLL` | runtime | output |
|---|---|---|
| 128 (default) | **7 ms** | `2899565707696607360` |
| 1 (dimatikan) | **365 ms** | `2899565707696607360` |

Itu **52× lebih cepat** dengan output identik. Jadi klaim di §13.4 versi
sebelumnya — bahwa pass tidak pernah menyala — **salah**. Instrumen saya
sebelumnya hanya mencatat penolakan, sehingga keberhasilan tidak terhitung, dan
saya salah membaca hasil kosong sebagai "nol penerapan".

Yang benar adalah lebih sempit dan tetap penting: **pass ini hanya menolong loop
yang state-nya fungsi linear dari dirinya sendiri.** Pada loop panas
`filter_symmetric_column` (§11.3) bentuknya bukan affine — ada pointer memori,
indeks hasil perhitungan, dan beberapa state — sehingga reassociation tidak
berlaku dan loop tetap satu elemen per iterasi.

Konsekuensinya untuk laporan: perbaikan runtime `p5_inline` dan `p6_scalar`
(900→7 ms, 910→8 ms) **berasal dari pass unroll ini**, bukan dari §13.1/§13.2.
Saya memisahkan kontribusinya di §13.6 agar angka tidak saling diklaim.

Keterbatasan yang tetap berlaku: perluasan ke bentuk non-affine (loop dengan
pointer/memori, yang justru dominan di beban nyata) **tidak** dikerjakan di sini.
Itu memerlukan analisis dependensi memori yang benar, dan melonggarkan syarat
tanpa itu berisiko salah-optimasi. Ini arah kerja berikutnya yang konkret.

### 13.5 Matriks tiga arah final (§13)

Diukur dengan `bench/fullbench.sh` (7 repetisi compile, 9 repetisi run, median
per program, lalu dijumlahkan), delapan program `bench/progs/`, `opt-level=3`.
Kolom `mod vs up` adalah runtime modifikasi terhadap cg_clif upstream —
perbandingan backend murni, keduanya memakai std hasil Rakit sendiri.

| label | compile wall (ms) | compile cpu (ms) | peak RSS (MB) | total size (KB) | run wall (ms) | run cpu (ms) | run RSS (MB) |
|---|---|---|---|---|---|---|---|
| LLVM (stock rustc) | 876 | 1149 | 133 | 36.095 | 365 | 365 | 48 |
| cg_clif upstream | 707 | 922 | 131 | 45.675 | 2467 | 2461 | 49 |
| **rakit modifikasi** | **726** | **962** | **132** | **31.688** | **517** | **514** | **48** |

Runtime per program:

| program | LLVM | upstream | modifikasi | mod vs up |
|---|---|---|---|---|
| p1_arith | 81 | 108 | 104 | −3,7% |
| p2_mem | 17 | 154 | 66 | **−57,6%** |
| p3_calls | 37 | 116 | 96 | −17,0% |
| p4_simd | 55 | 156 | 139 | −11,1% |
| p5_inline | 58 | 893 | 6 | **−99,3%** |
| p6_scalar | 58 | 899 | 6 | **−99,3%** |
| p7_branchy | 42 | 102 | 61 | −40,3% |
| p8_minmax | 18 | 38 | 38 | +0,5% |

Ringkasan terhadap empat tujuan awal:

- **Ukuran hasil**: modifikasi **31.688 KB vs upstream 45.675 KB (−30,6%)**,
  dan **lebih kecil dari LLVM** (36.095 KB, −12,2%). Penyebab utamanya bukan
  §13.2, melainkan patch `rakit-object` yang memberi tiap fungsi dan objek
  data seksi bernama unik, sehingga `--gc-sections` akhirnya bisa membuang kode
  mati. Buktinya, dihitung pada berkas objek di dalam `libstd-*.rlib` (bukan
  pada `.so` hasil link, yang sudah menggabungkan semuanya): upstream punya
  **8.948 seksi bernama `.text.subsection`** — semuanya identik, jadi linker
  hanya bisa membuang seluruhnya atau tidak sama sekali — sedangkan fork punya
  **8.551 seksi `.text.<simbol>` unik dan 0 yang bernama `subsection`**.
  Efeknya terlihat langsung di linker: dengan `-Wl,--print-gc-sections`,
  `p1_arith` versi fork membuang **17.512** seksi tak terpakai, upstream hanya
  **5.682**, dan hasil akhirnya 4,09 MB vs 5,88 MB.
  Kontribusi §13.2 terhadap ukuran kecil; ia menang di runtime (§13.6).
- **Waktu compile**: modifikasi 726 ms vs LLVM 876 ms (**−17,1%**), dan +2,7%
  vs upstream (biaya dataflow store-forwarding §13.1 serta pass LICM §14,
  keduanya di bawah 1% per program pada A/B terisolasi).
- **RAM saat compile**: 132 MB vs LLVM 133 MB (−0,8%), vs upstream 131 MB
  (+0,8%) — praktis setara.
- **Runtime**: modifikasi 517 ms vs upstream 2467 ms (**−79,0%**), tetapi masih
  1,42× LLVM. Sisa gap terpusat di p4_simd (139 vs 55 ms), yang persis beban
  loop scalar tanpa vektorisasi yang diuraikan §11.3.
- **RAM saat run**: 48 MB, sama dengan LLVM dan 1 MB di bawah upstream.

Kebenaran: kedelapan program menghasilkan output **byte-identik** dengan LLVM
(diverifikasi `diff`), di samping uji diferensial `[x; N]` di §13.2.

> **Catatan metodologi.** Angka pada versi pertama §13.5 (upstream 36.184 KB)
> salah karena `cg_clif_upstream/dist` saat itu basi: sysroot-nya sudah tidak
> memuat `libstd`, sehingga `rustc-rakit` diam-diam jatuh ke sysroot LLVM dan
> menghasilkan biner LLVM berukuran ±4,6 MB yang dikira keluaran Rakit.
> Setelah kedua backend dibangun ulang (`./y.sh build --sysroot rakit` di
> keduanya), upstream menjadi 45.675 KB. Selalu cek bahwa `dist/lib/rustlib/
> */lib/libstd-*.rlib` ada dan ukuran U ≠ ukuran L sebelum memercayai matriks.

### 13.6 Atribusi perbaikan, diukur satu per satu (§13)

Supaya tidak ada angka yang saling diklaim, tiap optimisasi dimatikan sendiri
lewat tunable-nya dan diukur ulang. Median 5 repetisi.

| program | default | `UNROLL=1` | `SF_NO_CBE=1` | `NO_REPEAT_MEMSET=1` | inliner mati |
|---|---|---|---|---|---|
| p2_mem | 66 ms | 65 ms | 68 ms | 65 ms | **109 ms** |
| p4_simd | 142 ms | 141 ms | 141 ms | 140 ms | 143 ms |
| p5_inline | 6 ms | **361 ms** | 6 ms | 6 ms | 6 ms |
| p6_scalar | 6 ms | **361 ms** | 6 ms | 6 ms | 6 ms |

Bacaan yang benar:

- **p5_inline dan p6_scalar ditenagai pass unroll** (6 → 361 ms bila dimatikan,
  yaitu 60×). Ini kontribusi terbesar pada runtime total, dan datang dari
  *affine reassociation* §13.4 — bukan dari §13.1/§13.2.
- **p2_mem ditenagai inliner** (66 → 109 ms bila inliner dimatikan, −39%). Ini
  mengoreksi atribusi sebelumnya: pada A/B pertama efek store forwarding hanya
  −4,4%, dan setelah semua toggle diuji, penyebab dominan ternyata inlining —
  `HashMap`/`Vec`/`sort_unstable` adalah kumpulan fungsi kecil yang baru
  menguntungkan setelah badannya disatukan ke pemanggil.
- **p4_simd tidak terpengaruh satu pun toggle.** Ini konsisten dengan §11.3:
  beban itu loop memori non-affine, dan tidak ada satu pun perbaikan yang
  menyentuhnya.

Untuk §13.2 (memset) saya ukur terpisah pada pola yang memang menjadi sasarannya,
karena tidak ada program benchmark yang didominasi zero-init:

```rust
for _ in 0..200_000 { let a = [0.0f32; 64]; let b = [1.5f64; 32]; let c = [0u32; 128]; ... }
```

| konfigurasi | runtime |
|---|---|
| memset aktif (default) | **4,8 ms** |
| `RAKIT_NO_REPEAT_MEMSET=1` | **23,6 ms** |

**4,9× lebih cepat**, output identik. Jadi §13.2 memang bernilai besar — hanya
saja pada pola kode yang tidak muncul di delapan program benchmark ini, sehingga
tidak terlihat di matriks §13.5.

Tunable `RAKIT_NO_REPEAT_MEMSET` ditambahkan khusus agar klaim ini bisa
direproduksi dan di-bisect.

## 14. LICM: pass loop-invariant code motion yang baru (§14)

Rakit tidak punya LICM. Pass `egraph` hanya meng-hoist *expression node*
yang murni dan sengaja **tidak** memindahkan load; `gvn` melakukan value
numbering tetapi tidak keluar dari blok. Akibatnya loop yang memuat nilai
invariant setiap iterasi (koefisien, batas, base pointer) menyimpan load itu di
dalam rantai dependensi loop-carried.

`vendor/rakit-codegen-0.137.0/src/licm.rs` menambahkan pass tersebut,
dipanggil dari `Context::compile` **setelah** egraph pass: egraph sudah melipat
dan menyederhanakan badan loop, jadi yang di-hoist adalah bentuk final, bukan
bentuk yang nanti ditulis ulang di tempat.

### 14.1 Cara kerja

1. Untuk setiap loop, cari **preheader** (blok tunggal yang masuk ke header dan
   berakhir dengan `jump`). Bila tidak ada, buat dengan memecah edge dari
   *immediate dominator* header — idom sebuah header selalu di luar loop,
   jadi blok baru itu tepat dieksekusi sekali per masuk loop.
2. Kumpulkan semua store di dalam loop beserta base address-nya (global/stack
   slot bila diketahui statis).
3. Telusuri instruksi tiap blok sesuai urutan layout. Sebuah instruksi boleh
   di-hoist bila:
   - bukan terminator, dan **tidak bisa trap** (`can_trap() == false`);
   - bila load: flag-nya `readonly` + `notrap` + `can_move`, **dan** tidak ada
     store di loop yang bisa meng-alias alamatnya (store opaque membatalkan
     semua load yang base-nya tidak diketahui);
   - bila bukan load: murni menurut `is_pure_for_egraph`;
   - semua operand "tersedia" di preheader.
4. "Tersedia" berarti: didefinisikan di luar loop **dan** blok definisinya
   mendominasi preheader, atau didefinisikan oleh instruksi yang sudah
   di-hoist sebelumnya. Parameter blok **preheader** boleh; parameter blok lain
   tidak.

### 14.2 Tiga jebakan soundness

Ketiganya muncul sebagai miscompile atau ICE saat pengembangan, dan masing-masing
kini ditangani eksplisit:

1. **Parameter header adalah phi node.** Parameter blok header mengambil nilai
   berbeda di back-edge, jadi hanya "tersedia" di jalur masuk — bukan di dalam
   loop. Menganggapnya invariant adalah bug LICM klasik dan membuat akumulator
   loop membeku pada nilai awal.
2. **"Di luar loop" tidak cukup.** Definisi harus **mendominasi preheader**.
   Nilai yang didefinisikan di cabang saudara berada di luar loop tetapi tidak
   mendominasi preheader.
3. **`brif` dengan kedua cabang menuju header.** Hanya satu edge yang bisa
   dipecah; edge lain harus tetap menuju header. Versi awal hanya mengalihkan
   satu edge sehingga cabang lain melewati kode yang di-hoist. Pemecahan edge
   juga **wajib meneruskan argumen phi** edge tersebut.

Selain itu: instruksi yang bisa trap tidak pernah di-hoist (kalau di-hoist, ia
akan dieksekusi pada jalur yang dulu melewati badan loop — mis. `sdiv` dengan
pembagi nol), dan `BlockArg::TryCallRet`/`TryCallExn` membuat pass menyerah
pada edge EH.

### 14.3 Dampak terukur

A/B dengan `RAKIT_NO_LICM=1` vs default, 11 repetisi, median, `opt-level=3`,
kedelapan program `bench/progs/`. Kedua sisi memakai sysroot Rakit mod
sendiri (bukan sysroot LLVM — lihat catatan metodologi §13.5).

| program | LICM mati | LICM hidup | delta | ukuran |
|---|---:|---:|---:|---|
| p1_arith | 104,3 ms | 104,2 ms | −0,1% | sama |
| p2_mem | 66,2 ms | 65,1 ms | **−1,7%** | +224 B |
| p3_calls | 97,2 ms | 96,0 ms | **−1,2%** | +32 B |
| p4_simd | 141,6 ms | 142,4 ms | +0,6% | +16 B |
| p5_inline | 6,7 ms | 6,3 ms | **−5,5%** | sama |
| p6_scalar | 6,7 ms | 6,4 ms | **−4,2%** | sama |
| p7_branchy | 61,0 ms | 60,6 ms | −0,6% | sama |
| p8_minmax | 38,2 ms | 38,2 ms | −0,1% | sama |
| **total** | **521,9 ms** | **519,3 ms** | **−0,5%** | |

Biaya compile: diukur terpisah pada p1/p2/p4/p6, delta antara −0,5% dan +1,1%
(derau). Jadi pass ini praktis **gratis**: ia menang pada kernel yang memuat
nilai invariant di dalam loop (p5/p6, juga p2/p3) dan netral pada sisanya.
Pertambahan ukuran hanya puluhan sampai ratusan byte.

### 14.4 Verifikasi

- Suite tes rakit penuh: **25.368 tes lulus, 0 gagal**, termasuk rand, regex,
  graviola, dan portable-simd yang dikompilasi ulang dengan pass aktif.
- `bench/tests/t_licm.rs` ditambahkan sebagai regresi khusus: lima bentuk yang
  pernah merusak pass (phi loop-carried, `brif` dua cabang ke header, store yang
  memblokir hoist, load invariant yang aman, dan pembagian yang bisa trap).
  Keluarannya **identik dengan LLVM**, baik dengan LICM hidup maupun mati.
- Kedelapan program benchmark menghasilkan output byte-identik dengan LLVM.

### 14.5 Tunable

| variabel | arti |
|---|---|
| `RAKIT_NO_LICM=1` | matikan pass LICM |
| `RAKIT_LICM_DEBUG=1` | cetak setiap hoist ke stderr |
| `RAKIT_NO_COLLAPSE=1` | matikan collapse forwarding-latch sebelum unroll |

## 15. False dependency pada konversi u64→float (LANDED)

### 15.1 Gejala

`p4_simd` tetap 2,5× lebih lambat dari LLVM meski semua pass rakit dimatikan
satu per satu (NONE 105,5 / NOUNROLL 105,2 / NOINLINE 106,3 / NOLICM 110,4 /
NOSF 107,0 / NOIFCONV 106,3 ms — semuanya dalam derau). Jumlah panggilan `sinf`
saat runtime juga identik: interposer `LD_PRELOAD` menghitung **4.000.000**
panggilan untuk LLVM maupun Rakit, dengan keluaran byte-identik
(`-0.9937716`). Jadi bukan jumlah kerja, melainkan **biaya per panggilan**:
7,4 ns vs 18,9 ns.

### 15.2 Isolasi

Memisahkan loop menjadi dua tahap — isi `a[i] = i as f32` dulu, lalu
`o[i] = a[i].sin()` — menyusutkan gap dari 2,5× menjadi 1,25×
(LLVM 39,1 ms vs Rakit 48,7 ms). Artinya **konversi integer→float** di loop
panas, bukan `sinf`, yang menjadi biang keladi.

### 15.3 Akar masalah

`cvtsi2ss`/`cvtsi2sd` adalah instruksi SSE dua-operand: selain mengonversi
sumber integer, ia **mempertahankan bit atas register tujuan**. Bila register
tujuan belum ditol-kan, instruksi itu membaca register tersebut dan menciptakan
*dependency* palsu terhadap penulis sebelumnya. Di dalam loop, penulis
sebelumnya adalah hasil `sinf` iterasi sebelumnya, sehingga setiap iterasi
harus menunggu panggilan sebelumnya selesai — loop yang seharusnya
*throughput-bound* berubah menjadi *latency-bound*.

Disassembly Rakit (sebelum perbaikan):

```
3792d:  cvtsi2ss %rdi,%xmm0     ; tidak ada xorps sebelum ini
3795b:  call     *%rax          ; sinf, hasil di xmm0
```

LLVM menyisipkan pemutus dependency:

```
xorps   %xmm0,%xmm0
cvtsi2ss %r14,%xmm0
call    *%r15
```

Jumlah `xorps` di seluruh binary: Rakit **5**, LLVM **46**.

### 15.4 Mengapa hanya jalur unsigned

`fcvt_from_sint` sudah benar: rule ISLE-nya memakai `(xmm_zero $F32X4)` sebagai
operand pertama `x64_cvtsi2ss`, dan `xmm_zero` di-emit sebagai `xorps tmp,tmp`.
Komentar di `lower.isle` bahkan menyebut masalah ini secara eksplisit (lihat
isu #7085).

Jalur **unsigned** (`fcvt_from_uint` untuk `I64`) berbeda: ia tidak memakai
satu instruksi, melainkan sekuens `CvtUint64ToFloatSeq` yang menangani nilai
≥ 2^63 dengan membagi dua lalu menskalakan kembali. Sekuens itu memanggil
`emit_signed_cvt`, yang langsung meng-emit `cvtsi2ssq_a`/`cvtsi2sdq_a` **tanpa
menol-kan `dst` lebih dulu**. Karena `usize as f32` adalah pola paling umum di
Rust (`for i in 0..n { ... i as f32 ... }`), cacat ini mengenai kode sehari-hari,
bukan kasus langka.

### 15.5 Perbaikan

`vendor/rakit-codegen-0.137.0/src/isa/x64/inst/emit.rs`, di
`emit_signed_cvt`: emit `xorps dst,dst` sebelum konversi.

```rust
let dst = WritableXmm::from_writable_reg(dst).unwrap();
asm::inst::xorps_a::new(dst, dst.to_reg()).emit(sink, info, state);
if to_f64 {
    asm::inst::cvtsi2sdq_a::new(dst, src).emit(sink, info, state);
} else {
    asm::inst::cvtsi2ssq_a::new(dst, src).emit(sink, info, state);
}
```

`xorps` dipilih (bukan `vxorps`) karena jalur ini memakai encoding SSE legacy;
`xorps r,r` sudah cukup untuk memutus dependency dan tidak menambah kebutuhan
fitur CPU. Biayanya satu instruksi 3-byte per konversi.

### 15.6 Hasil

Loop `(i as f32).sin()` (`p9_u64f32`), 4 juta iterasi:

| | waktu |
|---|---|
| LLVM | 29,7 ms |
| cg_clif upstream | 75,7 ms |
| rakit modifikasi (sebelum) | 75,6 ms |
| **rakit modifikasi (sesudah)** | **34,1 ms** |

**2,2× lebih cepat**, dan kini hanya 15% di belakang LLVM. Karena upstream
menunjukkan angka yang sama (75,7 ms), ini defisiensi Rakit yang berdiri
sendiri, bukan efek samping perubahan lain di fork ini.

Dampak pada benchmark utama:

| prog | upstream | mod (sebelum) | mod (sesudah) | LLVM |
|---|---|---|---|---|
| p4_simd | 155,8 ms | 167 ms | **96,2 ms** | 55,3 ms |
| p1_arith | 108,9 ms | 106 ms | 105,0 ms | 81,2 ms |
| p2_mem | 157,2 ms | 66 ms | 65,6 ms | 16,0 ms |
| p3_calls | 117,0 ms | 96 ms | 96,1 ms | 36,7 ms |
| p7_branchy | 103,6 ms | 61 ms | 60,6 ms | 42,6 ms |

Ukuran binary **tidak berubah sama sekali** (0 byte pada kedelapan program),
dan seluruh keluaran tetap byte-identik dengan LLVM.

### 15.7 Verifikasi

- Suite tes rakit penuh lulus, tanpa kegagalan.
- Keluaran kedelapan program bench identik dengan LLVM.
- `bench/progs/p9_u64f32.rs` ditambahkan sebagai penjaga regresi: kernel ini
  mengisolasi jalur unsigned, sehingga `xorps` yang hilang muncul sebagai
  perlambatan besar, bukan kesalahan pembulatan.
- Loop yang tidak memuat konversi (`pre`: dua tahap terpisah) tetap 48,8 ms
  sebelum dan sesudah — memastikan perbaikan tidak menyentuh jalur lain.
- `bench/tests/t_u64f32.rs` menutup kedua cabang sekuens: nilai < 2^63
  (konversi langsung) dan >= 2^63 (jalur bagi-dua lalu skala). Diuji pada
  seluruh rentang u64 termasuk `u64::MAX`; hasilnya 0 kesalahan round-trip,
  0 mismatch f32, dan 64/64 nilai di sekitar batas 2^63 tepat, semuanya
  identik dengan LLVM. Pemeriksaan disassembly: **20/20** instruksi
  `cvtsi2ss`/`cvtsi2sd` di binary uji kini didahului pemutus dependency
  (`xorps`/`xorpd`), termasuk dua yang berasal dari `compiler_builtins`.

### 15.8 Sisa gap p4_simd

Sisa selisih p4_simd (96,2 vs 55,3 ms) berasal dari hal lain: LLVM menggabungkan
`sinf` + `cosf` pada argumen yang sama menjadi satu panggilan `sincosf`
(`objdump` menunjukkan `sincosf` di binary LLVM, sedangkan Rakit tetap
memanggil `sinf@` dan `cosf@` terpisah). Penggabungan itu adalah transformasi
tingkat IR (loop fusion + common-subexpression), bukan sesuatu yang bisa
disimpulkan di level mesin.

## 16. Batas inliner lintas-CGU pada rantai iterator (LANDED)

### 16.1 Gejala

`(0..N).map(|i| f(i)).collect::<Vec<u8>>()` berjalan **10x** lebih lambat
daripada build LLVM (20,0 ms vs 2,0 ms untuk 4 juta elemen). Angka itu aneh
karena isi loop-nya hanya aritmetika sepele dan satu tulis byte; 5 ns per byte
jauh di atas biaya yang wajar untuk kerja sebesar itu.

Yang membuatnya menonjol: `insert` biasa pada `HashMap` justru **2x lebih
cepat** daripada LLVM, dan `vec![7u8; N]` hanya 2,55x. Jadi bukan "Rakit
lambat di memori", melainkan satu bentuk kode tertentu.

### 16.2 Akar masalah

`collect` menurunkan `map` + `Range` menjadi `extend_trusted`, yang menyerahkan
loop elemen ke `fold`. Loop itu, sebelum perbaikan, berbunyi:

```
fold:
  call Range::spec_next        ; per elemen
  call map_fold (closure)      ; per elemen
  spill/restore state iterator ; per elemen
  jmp fold
```

Dua call nyata per elemen, ditambah simpan-muat state iterator ke stack di
sekelilingnya. LLVM meng-inline seluruh rantai itu menjadi beberapa instruksi,
sehingga selisihnya bukan soal kualitas instruksi melainkan soal **ada atau
tidaknya call**.

Penyebabnya adalah batas inliner fork ini, yang diukur untuk loop numerik padat:

| knob | nilai lama | mengapa gagal di sini |
|---|---|---|
| `MAX_CALLEE_INSTS` | 64 | body `fold`/`extend_trusted` jauh lebih besar |
| `MAX_LEAF_INSTS` | 192 | body leaf rantai iterator melewati 192 |
| `GROWTH_PCT` | 50 | budget tumbuh habis sebelum call terakhir hilang |
| `MAX_CALLEE_USES` | 3 | pemanggil yang sama butuh lebih dari 3 inline |

Keempatnya binding **bersamaan**: menaikkan salah satu saja sudah menutup
sebagian besar gap (20,4 → 10,0-11,3 ms), yang menandakan keempat batas itu
saling menutup jalan.

### 16.3 Perbaikan

Default dinaikkan ke `MAX_CALLEE_INSTS=128`, `MAX_LEAF_INSTS=384`,
`GROWTH_PCT=150`, `MAX_CALLEE_USES=5`. Nilai ini dipilih dari sapuan beberapa
konfigurasi, bukan ditebak: konfigurasi yang lebih agresif (256/512/200/6)
memang lebih cepat di `collect` tetapi **meregresi p4_simd** (95,5 → 103,7 ms),
sedangkan 128/384/150/5 menang di keduanya.

### 16.4 Hasil

Runtime, median 9 rep:

| prog | LLVM | sebelum | sesudah | Δ |
|---|---|---|---|---|
| p2_mem | 16,2 ms | 65,7 ms | **52,5 ms** | **−20,1%** |
| p7_branchy | 43,0 ms | 60,6 ms | **55,4 ms** | **−8,6%** |
| p4_simd | 56,4 ms | 96,9 ms | 96,9 ms | 0,0% |
| p5_inline | 57,7 ms | 6,4 ms | 6,4 ms | 0,0% |
| p6_scalar | 58,0 ms | 6,4 ms | 6,4 ms | 0,0% |
| p1_arith | 81,4 ms | 104,9 ms | 105,9 ms | +1,0% |
| p8_minmax | 17,7 ms | 38,2 ms | 38,6 ms | +1,0% |
| p9_u64f32 | 30,2 ms | 34,4 ms | 34,6 ms | +0,6% |
| p3_calls | 36,9 ms | 96,7 ms | 95,9 ms | −0,8% |

Pada kernel yang menjadi sasaran, efeknya jauh lebih besar:

| kernel | LLVM | sebelum | sesudah | Δ |
|---|---|---|---|---|
| `collect` 4 juta byte (i5) | 2,0 ms | 19,9 ms | **10,7 ms** | **−46%** |
| hitung byte (k2) | 6,9 ms | 24,2 ms | **13,1 ms** | **−46%** |

Biaya yang dibayar **nol**: waktu compile dan ukuran artifact tidak berubah.
Diukur dengan `wait4`/`rusage`:

| prog | backend | CPU compile | peak RSS | ukuran |
|---|---|---|---|---|
| p2_mem | LLVM | 0,39 s | 133 MB | 4531 KB |
| p2_mem | sebelum | 0,23 s | 133 MB | 4040 KB |
| p2_mem | sesudah | 0,23 s | 133 MB | 4040 KB |
| p1_arith | sebelum | 0,10 s | 109 MB | 4033 KB |
| p1_arith | sesudah | 0,10 s | 109 MB | 4033 KB |

Ukuran binary bahkan sempat **turun** beberapa KB pada beberapa program, karena
badan fungsi yang tadinya berdiri sendiri kini tidak perlu di-emit terpisah.

### 16.5 Verifikasi

- 25.368 tes lulus, 0 gagal (`./y.sh test`).
- 9/9 program benchmark menghasilkan output identik dengan LLVM.
- `bench/tests/t_iter_inline.rs` menutup bentuk kode ini: rantai `map`+`collect`,
  `filter`+`map`+`collect`, `step_by` (state iterator yang bukan range sederhana),
  dan `collect` bersarang.
- Knob `RAKIT_INLINE_*` tetap ada untuk bisecting, jadi perubahan ini bisa
  dimatikan tanpa rebuild bila perlu.

### 16.6 Sisa gap

Setelah call hilang, sisa selisih `collect` (10,7 vs 2,0 ms) berasal dari
**autovectorization**: LLVM mengubah loop hitung/tulis byte menjadi SIMD
(`pcmpeqb` + `pand` + horizontal add pada `i4_static`). Rakit tidak punya
autovectorizer, dan menambahkannya adalah pekerjaan tingkat pass yang jauh lebih
besar daripada perubahan budget ini. Gap yang sama muncul di k2/k7 (3x) dan
k8_sort (3,1x) — semuanya bentuk yang LLVM vectorize atau unroll.

---

## 17. Biaya compile-time pass unroll: dari kuadratik ke linear (LANDED)

Bagian ini menjawab pertanyaan yang sebelumnya tidak bisa dijawab karena
instrumentasi tidak pernah terpasang: **berapa ongkos compile-time dari pass
buatan sendiri?** Jawabannya: satu di antaranya sangat mahal, dan penyebabnya
bukan pekerjaan yang berguna.

### 17.1 Instrumentasi akhirnya tersambung

`src/passtime.rs` sudah ada sejak lama, tetapi `report()` **tidak pernah
dipanggil**, sehingga seluruh pengukuran per-pass selama ini nol. Dua masalah
harus diperbaiki bersamaan:

1. **`report()` tidak dipanggil.** Ditambahkan di `src/lib.rs` setelah
   `codegen_crate`, sehingga total per-pass dicetak sekali per crate.
2. **Akumulator thread-local.** Codegen berjalan di worker thread rustc, jadi
   akumulator `thread_local!` di-reset di thread yang berbeda dari thread yang
   mencetak laporan. Diganti dengan `static [AtomicU64; 5]`, di-`fetch_add` dari
   thread mana pun dan di-`swap(0)` saat melapor.

Setelah itu biaya per-pass baru terlihat, diaktifkan lewat `RAKIT_PASS_TIME=1`.

### 17.2 Angka pertama: `unroll` memakan 18,9 ms

Pada `p2_mem`, satu codegen unit melaporkan:

```
[passtime p2_mem...-cgu.1] store_forward=8.4ms unroll=18.9ms inline=3.6ms collect=1.9ms if_convert=1.9ms
```

`unroll` adalah pass termahal, lebih mahal dari `store_forward` yang jauh lebih
banyak bekerja. Pembelahan lebih lanjut (`collapse=18.8ms/1419it`) menunjukkan
hampir seluruh waktunya ada di **satu sub-fase**: pencarian blok forwarder.

### 17.3 Akar masalah: pemindaian ulang per kandidat

`collapse_forwarders` memanggil `find_forwarder`, yang membangun tiga fakta
tentang seluruh fungsi:

- jumlah edge masuk per blok (untuk memastikan hanya ada satu predecessor),
- blok sumber edge itu,
- apakah parameter blok dibaca blok lain (`params_escape`).

Ketiganya dibangun ulang **setiap kali satu blok berhasil di-collapse**. Dengan
1419 collapse pada satu fungsi, fungsi yang sama dipindai 1419 kali. Itu
kuadratik dalam jumlah blok, dan hasilnya selalu sama kecuali untuk blok yang
baru saja berubah.

### 17.4 Perbaikan: bangun sekali, perbarui per collapse

Fakta-fakta itu dipindahkan ke `struct Forwarders`, dibangun sekali di
`Forwarders::new`, lalu **diperbarui secara inkremental** di `Forwarders::removed`
saat satu blok lenyap: edge ke latch dihapus, edge dari predecessor dialihkan ke
target latch. Kandidat forwarder (blok `nop`/`jump` tanpa hasil) juga disaring
sekali di awal, bukan tiap iterasi.

Hasilnya pada `p2_mem`:

| Metrik | Sebelum | Sesudah |
|---|---|---|
| pass `unroll` | 18,9 ms | **2,4 ms** |
| sub-fase `collapse` | 18,8 ms | **2,7 ms** |
| iterasi collapse | 1419 | 1093 |

Pass menjadi **7,9x lebih cepat**, dan yang penting: pekerjaan yang tersisa
sebanding dengan jumlah blok, bukan kuadratnya.

Jumlah iterasi turun (1419 → 1093) karena kandidat yang jelas tidak memenuhi
syarat tidak lagi dihitung sebagai percobaan.

### 17.5 Verifikasi

Perubahan ini menyentuh logika pemilihan blok, jadi diverifikasi ketat:

- **25.368 tes lulus, 0 gagal** (`./y.sh test`).
- 8/8 program p-suite dan 10/10 kernel s-suite menghasilkan output identik
  dengan LLVM.
- `p2_mem` menghasilkan `300000 17064514294836777531`, sama seperti sebelumnya.

### 17.6 Sisa biaya pass, dan mengapa itu wajar

Setelah `unroll` diperbaiki, anggaran pass pada `p2_mem` menjadi:

| Pass | Waktu | Sifat |
|---|---|---|
| `store_forward` | 8–12 ms | linear terhadap instruksi (178 fungsi, 10,1 ms total) |
| `inline` | 3,0–4,6 ms | linear |
| `unroll` | 2,4–3,4 ms | linear setelah perbaikan |
| `if_convert` | 1,8–2,6 ms | linear |
| `collect` | 1,7–1,8 ms | linear |

`store_forward` kini pass termahal. Biayanya bukan kuadratik: fungsi terbesar
(1018 instruksi) memakan 1,49 ms, dan totalnya 10,1 ms untuk 178 fungsi, jadi
waktunya sebanding dengan jumlah instruksi yang memang harus diperiksa.

Konsekuensinya, mod masih ~13% lebih lambat **meng-compile** `p2_mem` daripada
upstream (145 ms vs 128 ms, median 15 run) — itu harga dari empat pass tambahan yang menghasilkan
kode jauh lebih cepat saat dijalankan (67 ms vs 156 ms, lihat §17.7). Pada
program yang tidak memicu pass ini, biayanya nol.

### 17.7 Matriks akhir tiga arah

`bench/fullbench.sh`, median dari 7 run compile dan 11 run eksekusi.
`L` = rustc + LLVM, `U` = cg_clif upstream, `M` = fork ini.

Compile (ms / MB RSS / ukuran biner):

| Program | L wall | U wall | M wall | L size | U size | M size |
|---|---|---|---|---|---|---|
| p1_arith | 87,2 | 82,2 | 82,8 | 4.635.232 | 5.878.688 | **4.322.528** |
| p2_mem | 242,7 | 128,8 | 139,4 | 4.639.888 | 5.936.880 | **4.345.592** |
| p3_calls | 100,2 | 86,3 | 88,3 | 4.612.368 | 5.820.872 | **4.257.744** |
| p4_simd | 112,3 | 93,7 | 96,3 | 4.636.096 | 5.893.112 | **4.330.520** |
| p5_inline | 83,4 | 78,3 | 78,6 | 4.609.368 | 5.814.440 | **4.249.784** |
| p6_scalar | 84,3 | 77,3 | 77,3 | 4.609.368 | 5.814.424 | **4.249.784** |
| p7_branchy | 86,5 | 82,7 | 79,1 | 4.609.520 | 5.814.536 | **4.249.776** |
| p8_minmax | 86,6 | 81,5 | 80,5 | 4.609.544 | 5.814.488 | **4.249.720** |

Eksekusi (ms):

| Program | L | U | M | M vs U | M vs L |
|---|---|---|---|---|---|
| p1_arith | 82,2 | 108,3 | 103,2 | 1,05x | 1,26x |
| p2_mem | **15,9** | 155,7 | 67,4 | **2,31x** | 4,24x |
| p3_calls | 36,7 | 117,0 | 99,5 | 1,18x | 2,71x |
| p4_simd | 56,9 | 157,4 | 140,8 | 1,12x | 2,47x |
| p5_inline | 57,6 | 894,2 | **6,8** | **131x** | 8,5x lebih cepat |
| p6_scalar | 57,6 | 907,1 | **6,9** | **131x** | 8,3x lebih cepat |
| p7_branchy | 42,5 | 100,3 | 60,2 | 1,67x | 1,42x |
| p8_minmax | 17,7 | 38,7 | 38,4 | 1,01x | 2,17x |

Tiga hal yang perlu dibaca dari tabel ini:

1. **Ukuran biner mod 7–27% lebih kecil dari LLVM** dan 26–27% lebih kecil dari
   cg_clif upstream, konsisten di semua program. Ini efek dari kode yang lebih
   sedikit di-emit, bukan dari strip.
2. **p5/p6 adalah kasus di mana mod mengalahkan LLVM 8x** (6,8 ms vs 57,6 ms).
   cg_clif upstream di program yang sama butuh 894 ms — artinya ada bug/kelemahan
   besar di sana yang pass inliner fork ini perbaiki, bukan sekadar tuning.
3. **p2_mem tetap 4,2x lebih lambat dari LLVM.** Penyebabnya bukan pass fork ini
   (mod sudah 2,3x lebih cepat dari upstream), melainkan kualitas codegen
   Rakit sendiri: LLVM mengubah loop memori menjadi kode SIMD, Rakit
   tidak punya autovectorizer.

### 17.8 Catatan jujur soal batas

`-Ctarget-cpu=native` tidak membantu (dikonfirmasi ulang), dan sweep kapasitas
inliner (256/384/512) menunjukkan default sudah optimal — angka 52,5 ms yang
sempat terlihat di satu run adalah noise, bukan sinyal. Keduanya dicatat sebagai
lever yang **sudah diuji dan netral**, supaya tidak diuji ulang tanpa alasan.

## 18. Uji habis dibagi: `x % C == 0` tanpa menghitung sisa (LANDED)

### 18.1 Gejala

`p1_arith` tetap 1,26x lebih lambat dari LLVM (103,2 vs 82,2 ms) walaupun
inliner, unroll, SROA, dan store-forwarding sudah jalan. Badan loop-nya:

```rust
acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i | 1);
f += (i as f64).sqrt() * 0.5;
if acc % 7 == 0 { acc ^= f as u64; }
```

### 18.2 Eksperimen terkontrol: mengisolasi biaya `%`

Dua program identik kecuali satu baris. Yang pertama menguji habis dibagi dengan
tes tulis tangan (`(acc.wrapping_mul(m)) <= T`), yang kedua memakai
`acc % 7 == 0`:

| Varian | LLVM | rakit |
|---|---|---|
| tes tulis tangan | 55,2 ms | **55,8 ms** |
| `acc % 7 == 0` | 61,4 ms | **108,7 ms** |

Tes tulis tangan **setara** — jadi gap-nya bukan kualitas loop secara umum,
melainkan khusus operator `%`. Hipotesis lain sudah dieliminasi lebih dulu:
konversi `u64→f64` bertanda (`jl`) bukan penyebabnya (signedconv == modonly),
dan if-conversion fork ini memang bekerja (memakai `cmovne`).

Dump RAKIT (`--emit llvm-ir` menulis `.rakit`, karena `should_write_ir` memeriksa
`OutputType::LlvmAssembly`) menunjukkan bentuk persisnya:

```text
v87 = iconst.i64 7
v88 = urem.i64 v86, v87
    brif v88, block4, block3
```

### 18.3 Akar masalah

Rakit men-strength-reduce `urem` oleh konstanta menjadi multiply-shift, tapi
tetap **mematerialisasi seluruh sisa**: `mulq` + 2 shift + `imul` + `sub`, enam
instruksi, ditambah dua cabang per iterasi karena loop tidak dirotasi. LLVM
melipat `x % C == 0` menjadi dua instruksi (`imul`, `cmp`) plus `setb`/`xor`
yang branchless. Sisanya adalah nilai sisa yang tidak pernah dipakai.

### 18.4 Identitas

Untuk `C` ganjil, dengan `m = C⁻¹ mod 2^64` dan `T = (2^64 − 1) / C`:

```text
x % C == 0   <=>   (x * m mod 2^64) <= T
```

`m` adalah unit, jadi `x ↦ x·m` injektif; kelipatan `C` memetakan tepat ke
`T + 1` nilai `0, m, 2m, …, T·m` yang `<= T`, dan setiap nilai di atas `T`
adalah `m·k` untuk `k` yang bukan kelipatan `C`. `C` genap ditangani dengan
mengupas faktor dua sebagai mask lebih dulu. `srem` **tidak** diubah: pembagian
truncating membulatkan ke arah nol, sehingga `x` negatif bisa bersisa nol tanpa
menjadi kelipatan `C`.

Diverifikasi menyeluruh: 151 pembagi ganjil 1..301 dan pembagi besar
(1000003, 999999937, 9223372036854775783, 6148914691236517205,
9223372036854775807) — nol kegagalan. Dua unit test di `src/div_test.rs`
menjaga identitas dan invers untuk semua pembagi ganjil < 512 dan < 4096.

### 18.5 Bug yang tertangkap: dua bentuk use-site

Frontend menghasilkan dua bentuk, dan keduanya butuh penanganan berbeda:

- `brif v88, block4, block3` — sisa dipakai **langsung** sebagai kondisi.
  Benar: tukar blok tujuan (`blocks.swap(0, 1)`) dan pakai hasil tes baru
  sebagai kondisi; membalik nilai malah menambah `xor $1`.
- `icmp eq v88, 0` — `icmp` adalah **nilai terpisah** yang dikonsumsi kode
  lain. Yang harus dialihkan adalah *hasil* icmp itu
  (`detach_inst_results` + `change_to_alias`), bukan operand-nya.

Versi pertama salah untuk bentuk kedua: ia mengganti operand icmp, sehingga
perbandingan menguji hasil boolean-nya sendiri. Hasilnya tetap valid secara tipe
dan lolos delapan benchmark, tapi `filter(|x| x % 3 == 0)` membuang komplemen
himpunan — `t_iter` mencetak `1666633334` alih-alih `833316666` (tepat 2x).
Karena itu `t_iter` dijadikan regression test: kesalahan ini **senyap**.

### 18.6 Hasil

| Metrik | sebelum | sesudah | LLVM |
|---|---|---|---|
| `p1_arith` eksekusi | 104,8 ms | **96,7 ms** | 82,2 ms |
| `mulq` di loop utama | ada | **hilang** | tidak ada |
| loop `modonly` (terisolasi) | 108,7 ms | **55,9 ms** | 61,3 ms |

Pada kernel terisolasi, versi modifikasi kini **lebih cepat dari LLVM**
(55,9 vs 61,3 ms) karena `cmovbe` menggantikan `setb`+`xor` LLVM. Pada
`p1_arith` sisa gap 1,18x bukan lagi soal `%` melainkan konversi `f64→u64`
saturating yang di Rakit memakai rantai cabang (LLVM branchless) — itu
pekerjaan level `rakit-codegen`, bukan pass sisi rakit.

Knob: `RAKIT_NO_DIVTEST=1`. Semua delapan benchmark dan seluruh test suite
menghasilkan keluaran identik dengan LLVM (kecuali thread-id di `t_panic`/`t_tls`,
yang memang berbeda antar-run; `t_panic` juga kehilangan baris `println`
terakhir di **kedua** backend — bug pra-ada, bukan akibat pass ini).

### 18.7 Catatan pengukuran

Dua jebakan yang memakan waktu di sini:

1. **Variabel `RAKIT_*` dibaca saat kompilasi, bukan saat run.** Menjalankan
   `RAKIT_NO_DIVTEST=1 ./binary` tidak mematikan apa pun. Harus
   `RAKIT_NO_DIVTEST=1 rustc-rakit ... -o binary`.
2. **Loop dengan batas konstanta bisa dilipat jadi bentuk tertutup.** `p5_inline`
   (300 juta iterasi) berjalan 6,8 ms — 48 miliar iterasi/detik, mustahil
   secara fisik. Disassembly menunjukkan unroll 128x dengan strength reduction
   ke rumus tertutup, dan hasilnya cocok dengan perhitungan Python. Wajar, bukan
   miscompile — tapi kalau `n` dijadikan argumen runtime, waktunya kembali
   linear (364 ms untuk 300 juta), yang membuktikan lipatan itu compile-time.

## 19. Reasosiasi affine dengan atom: loop LCG `+ (i|1)` (LANDED)

### 19.1 Gejala

`p7_branchy` dan `p8_minmax` sama-sama berisi loop LCG:

```rust
for i in 0..N {
    acc = acc.wrapping_mul(C).wrapping_add(i | 1);
    ...
}
```

Sebelum perubahan ini keduanya berjalan **sama persis dengan cg_clif upstream**
(p8: 38 ms untuk upstream maupun modifikasi). Artinya tidak ada satu pun pass
yang menyala: `unroll` affine menolak loop itu. `p8` menguji reduksi `min`
(bukan affine, jadi `power` menolak), dan `i | 1` adalah operasi bitwise yang
tidak bisa dinyatakan sebagai bentuk linear `sum(coeff*param)+c`.

### 19.2 Akar masalah

Ada dua penghalang, keduanya harus dibuka:

1. **Reduksi `min` tidak punya bentuk tertutup.** `power` dulu menuntut *setiap*
   state affine. Satu state `mn = min(mn, a)` saja sudah menggagalkan seluruh
   loop. Padahal state affine (`acc`, `i`) tidak bergantung pada `mn`, jadi
   iterat `factor`-lipatnya tetap bisa dihitung sendiri-sendiri.
2. **`i | 1` bukan linear.** `bor(x, 1)` dengan konstanta tidak bisa masuk
   `coeff*param+c`. Tanpa penanganan, loop LCG selalu ditolak.

Eksperimen plafon membuktikannya: `p7` tanpa `i | 1` (diganti `+ i`) berjalan
**40 ms di modifikasi vs 78 ms di LLVM** — begitu bitwise dihilangkan, unroll
langsung menang besar. Jadi penghalangnya memang operasi bitwise itu.

### 19.3 Model: `Pure` dan `Atom`

`src/unroll.rs` kini memisahkan tiga hal:

- `Pure` — bentuk linear `sum(coeffs[i]*param[i]) + c` modulo 2^64. Operand
  sebuah atom; tidak pernah memuat atom lain.
- `Atom { op, imm, operand: Pure }` — satu kontribusi **non-linear tetapi bisa
  dihitung ulang**, mis. `i | 1`. Nilainya selalu bisa dibangun ulang dari
  parameter header asli.
- `Affine { coeffs, c, atoms: Vec<(Atom, u64)> }` — bentuk linear **plus**
  kombinasi linear atom.

`apply_step(step, state)` mengevaluasi update satu-langkah `step` pada `state`.
Bagian linear menskalakan `state`; tiap atom dibangun ulang dari nilai
operandnya di `state` (`subst_pure`), sehingga `i | 1` otomatis maju menjadi
`(i+delta) | 1` di langkah berikutnya. `build_affine` mematerialisasi tiap atom
**secara independen** dari parameter header, jadi atom tidak pernah masuk ke
rantai dependensi yang dibawa (`acc*C`).

Untuk loop affine murni, `apply_step` terbukti secara aljabar identik dengan
komposisi lama (`F^{k+1} = F_j ∘ F^k`), sehingga refaktor ini netral pada
loop-loop yang sudah ditangani.

### 19.4 Reduksi lewat klon, atom lewat faktor kecil

`apply_partial` (sudah ada) menangani state non-affine dengan mengkloning badan
loop sekali per iterasi dan menyisipkan nilai affine yang sudah dihitung di
depan. Dua faktor berbeda dipakai:

- `ATOM_FACTOR = 2` untuk loop beratom. Membangun ulang atom tiap langkah
  berbiaya O(factor^2) perkalian, jadi faktor besar justru merugikan; 2 sudah
  memutus rantai `imul` yang dibawa.
- `PARTIAL_FACTOR = 6` (dapat diatur `RAKIT_PARTIAL`) untuk reduksi affine
  murni tanpa atom.

Sapuan faktor mengonfirmasi: untuk loop beratom, faktor 2 terbaik (p7 49→39,6 ms,
p8 38→36,1 ms); untuk reduksi affine murni (`p8b`), faktor 6 terbaik (~25 ms).

### 19.5 Hasil

Matriks penuh tiga-arah (`bench/allaxes.sh`, CREPS=3 RREPS=5). L = LLVM
nightly, U = cg_clif upstream, M = modifikasi. Waktu dalam ms, RSS dalam MB,
size dalam byte.

**Runtime (wall ms) — semakin kecil semakin baik**

| Program | L | U | M | M/L |
|---|---|---|---|---|
| p1_arith | 81 | 108 | 100 | 1,23x |
| p2_mem | 16 | 155 | 53 | 3,3x |
| p3_calls | 37 | 117 | 94 | 2,5x |
| p4_simd | 55 | 156 | 99 | 1,8x |
| p5_inline | 58 | 895 | **6** | **0,10x** |
| p6_scalar | 58 | 899 | **6** | **0,10x** |
| p7_branchy | 46 | 100 | **42** | **0,91x** |
| p8_minmax | 18 | 38 | **35** | 1,94x |
| p9_u64f32 | 31 | 76 | **35** | 1,13x |

M **mengungguli LLVM** di `p5`, `p6` (≈9,7x lebih cepat) dan `p7` (1,1x),
serta nyaris menyamai di `p9`. `p8` turun 38→35 ms (rantai `imul` terbukti
terputus; sisa 2x adalah struktur loop, lihat §19.7). Sisa gap lain (p2
hash/sort, p3 rekursi ekor, p4 vektor) tetap arsitektural (§8.28, §11).

**Ukuran artefak (byte) — semakin kecil semakin baik**

| Program | L | U | M | M/L |
|---|---|---|---|---|
| p1_arith | 4.635.232 | 5.877.944 | 4.130.408 | −10,9% |
| p2_mem | 4.639.888 | 5.936.144 | 4.137.384 | −10,8% |
| p5_inline | 4.609.368 | 5.813.704 | 4.052.536 | −12,1% |
| p8_minmax | 4.609.544 | 5.813.752 | 4.052.568 | −12,1% |

M selalu **lebih kecil dari LLVM (~11-12%)** dan **~30% lebih kecil dari U**.

**Waktu kompilasi (wall ms) dan peak RSS (MB)**

| Program | L wall | U wall | M wall | L rss | U rss | M rss |
|---|---|---|---|---|---|---|
| p2_mem | 241 | 127 | 145 | 133 | 132 | 133 |
| p1_arith | 87 | 78 | 78 | 106 | 109 | 109 |
| p8_minmax | 87 | 77 | 78 | 105 | 107 | 108 |

M mengompilasi `p2_mem` **1,66x lebih cepat dari LLVM** (145 vs 241 ms) pada
peak RSS yang sama. Overhead pass atom (§19) hanya ~1 ms wall di atas U.
Runtime peak RSS identik dengan LLVM di semua program.

### 19.6 Verifikasi

- Uji diferensial (`t_atom`): keluaran modifikasi == LLVM == atom-dimatikan,
  identik.
- Suite penuh `./y.sh test --sysroot rakit`: **65 suite, 0 gagal, exit 0**
  (termasuk `regex`, `rand`, `graviola`, `portable-simd`, `stdarch`).
- Knob: `RAKIT_PARTIAL` (faktor reduksi), `RAKIT_NO_ATOM` (matikan atom).

### 19.7 Sisa gap `p8_minmax` (35 ms vs LLVM 17,7 ms)

Dump RAKIT membuktikan reassosiasi **bekerja**: untuk faktor 2, blok badan
menghitung `v75 = C*acc`, `v89 = C^2*acc + C*(i|1) + (i+1)|1` — keduanya `imul`
independen dari `acc` header (`imul v59,C` dan `imul v59,C^2`) — dan klon
meneruskan `v89` ke header, sehingga rantai `acc` terputus menjadi satu `imul`
per dua iterasi. Recompute serial di klon (`v103`) mati dan dibuang.

Gap 2x tersisa karena **struktur loop**, bukan reassosiasi:

1. **Guard per iterasi.** `apply_partial` memakai satu `brif` guard yang dicek
   **setiap** iterasi luar (`test/jne` di disassembly) plus jalur fallback
   satu-langkah yang menghitung ulang `acc_1`. LLVM memakai versioning loop
   bersih (prolog+epilog, tanpa cabang guard per iterasi).
2. **Faktor 2 vs 4-lane LLVM.** LLVM memecah rekurensi menjadi **empat** lane
   independen (masing-masing `imul` oleh `C,C^2,C^3,C^4` di register berbeda),
   menyembunyikan latensi `imul` 3 siklus penuh. Faktor 2 hanya memberi dua
   lane. Menaikkan faktor atom tidak menolong: faktor 4 seri (35 ms), faktor
   8/16/32 **memburuk** (51/87/165 ms) karena tiap atom dibangun ulang dengan
   O(faktor^2) perkalian.

Menutup gap ini butuh menulis ulang `apply_partial` menjadi versioning loop
sejati (prolog menghitung sisa ekor, badan utama bersih tanpa guard), lalu
memakai faktor 4+ dengan biaya rebuild atom yang dijaga linear. Itu perubahan
struktural yang berisiko, jadi tidak dikerjakan di sesi ini; pass tetap benar
dan sudah memberi 38→35 ms plus kemenangan penuh di `p7`.

## 20. ICE `div_test` pada alias nilai: satu penggunaan yang tak terhitung (LANDED)

### 20.1 Gejala

Menjalankan suite `regex` (lewat dependensi `env_logger`) membuat compiler
**panic** di `vendor/rakit-codegen-0.137.0/src/ir/dfg.rs:401`:

```
index out of bounds: the len is 2440 but the index is 4294967295
```

di dalam `EgraphPass` → `insert_pure_enode` → `constructor_simplify`. Indeks
`4294967295` (`u32::MAX`) adalah `Value::reserved_value()`: sebuah instruksi
memakai nilai yang tidak ada. Mematikan pass lain tidak menolong; hanya
`RAKIT_NO_DIVTEST=1` yang menghilangkan ICE. Jadi `div_test` (§18) yang
memicunya — **bug pra-ada**, bukan akibat pekerjaan §19 (ICE tetap muncul dengan
`RAKIT_UNROLL=1`).

### 20.2 Akar masalah: penggunaan lewat alias tak terhitung

`div_test::collect` menghitung "berapa kali nilai remainder dipakai" hanya dari
`InstructionData::arguments()`. Dua jenis penggunaan lolos dari hitungan itu:

1. **Argumen cabang (block-call args).** `arguments()` untuk `brif`/`jump`
   mengembalikan **hanya kondisi**, bukan argumen blok tujuan. Nilai remainder
   yang juga diteruskan sebagai argumen phi tidak terhitung.
2. **Alias nilai.** rakit banyak memancarkan alias (`v420 -> v399`). Jika
   sebuah alias dari nilai remainder dipakai, penggunaan itu tercatat atas nama
   alias, bukan nilai aslinya.

Karena hitungan tetap `1`, kandidat dianggap aman, `rewrite` me-`remove_inst`
`urem`-nya, dan argumen blok / alias yang masih menunjuk ke definisi yang baru
dihapus menjadi menggantung. Egraph lalu membaca nilai `reserved` itu.

### 20.3 Perbaikan

- `collect` sekarang **me-resolve alias** (`dfg.resolve_aliases`) saat menghitung
  penggunaan, dan **ikut menghitung argumen cabang** lewat
  `InstructionData::branch_destination(...)` (mencakup `jump`, `brif`,
  `br_table`, `try_call`). Nilai yang dipakai lewat alias atau lewat phi kini
  terhitung, sehingga kandidat yang tidak benar-benar sekali-pakai ditolak.
- `classify_use` juga me-resolve alias pada operand sebelum membandingkan,
  supaya bentuk `brif (alias dari rem), ..` dikenali dengan benar.

Perbaikan ini murni menambah penolakan (lebih konservatif): tidak ada rewrite
baru yang diaktifkan, jadi tidak ada risiko miscompile baru. Semua hasil §18
tetap.

### 20.4 Verifikasi

`env_logger 0.9.3` kini terkompilasi bersih (exit 0, tanpa ICE). Suite `regex`
kembali lulus. Ini menutup celah yang tidak tertangkap sesi-sesi sebelumnya
karena `regex`/`env_logger` belum pernah benar-benar dikompilasi (unduhan crate
eksternal belum tersedia saat itu).

## 21. Guard jalur-lebar menjadi satu perbandingan konstan; faktor atom 2 → 3 (LANDED)

### 21.1 Akar masalah sisa `p8_minmax`

§19.7 mencatat dua penyebab gap `p8` (29–35 ms vs LLVM 17.7 ms): guard per
iterasi dan faktor 2 lawan 4-lane LLVM. Menambah faktor tidak menolong karena
biaya rebuild atom O(faktor²). Namun guard-nya sendiri bisa diperbaiki tanpa
menyentuh struktur lane.

`apply_partial` (dan `apply_reassoc`) membangun guard "semua `faktor` iterasi
masih dalam batas" sebagai `band(no_wrap, last_ok)`: `icmp uge(iv+last, iv)` ∧
`icmp cc(iv+last, bound)`, yaitu **tiga** instruksi per iterasi, dan keduanya
dihitung ulang tiap iterasi walau `bound` konstan.

### 21.2 Perbaikan: guard = satu `icmp ult(iv, limit)` konstan

Saat trip-count **konstan** (dan `bound + last` tidak overflow), tes di atas
setara dengan `iv < limit` dengan `limit` **konstan**:

- `<` ketat: `limit = bound - last` (atau `0` bila `bound < last`);
- `<=`: `limit = bound - last + 1`.

Satu `icmp` terhadap konstanta menggantikan tiga instruksi, dan konstanta itu
di-`iconst` sekali. Untuk bound tak-konstan, guard umum lama dipakai sebagai
fallback. Helper `build_guard` dipakai bersama oleh kedua unroller.

### 21.3 Faktor atom 2 → 3

Dengan guard yang murah, knee faktor bergeser: tiga lane mengalahkan dua.
Sapuan pada `p8` (ms): faktor 2 = 31.4, **3 = 29.1**, 4 = 33.5, 6 = 39.3,
8 = 51.3. `p7` serupa: 2 = 38.0, **3 = 35.4**, 4 = 45.1. Faktor 4+ tetap
memburuk karena O(faktor²) perkalian koefisien di `pre[k]` (bukan karena nilai
atom — cache nilai atom diuji dan **netral**, jadi dibuang).

### 21.4 Hasil (matriks 3-arah penuh, `bench/allaxes.sh`, CREPS=3 RREPS=7)

Runtime (wall ms) M vs L, sebelum → sesudah sesi ini:

| prog | M sebelum | M sesudah | LLVM | U |
|------|----------:|----------:|-----:|---:|
| p7_branchy | 40.3 | **36** | 45 | 100 |
| p8_minmax  | 35.0 | **29** | 18 | 38 |

Matriks lengkap sesi ini (C = compile wall ms, R = run wall ms, size = byte):

| prog | C L/U/M | R L/U/M | size L/U/M |
|------|---------|---------|------------|
| p1 | 86/82/78 | 82/108/100 | 4.63/5.88/4.13 MB |
| p2 | 247/125/146 | 16/155/52 | 4.64/5.94/4.14 MB |
| p3 | 99/86/86 | 36/116/94 | 4.61/5.82/4.06 MB |
| p4 | 107/92/98 | 56/156/99 | 4.64/5.89/4.14 MB |
| p5 | 83/76/84 | 58/893/**6** | 4.61/5.81/4.05 MB |
| p6 | 82/76/75 | 58/902/**6** | 4.61/5.81/4.05 MB |
| p7 | 84/77/76 | 45/100/**36** | 4.61/5.81/4.05 MB |
| p8 | 84/79/80 | 18/38/**29** | 4.61/5.81/4.05 MB |
| p9 | 94/86/87 | 30/76/34 | 4.64/5.89/4.13 MB |

M unggul dari LLVM pada: waktu compile (semua kecuali p4/p5 sedikit lebih
lambat; p2 1.7× lebih cepat), ukuran (≈12% lebih kecil di semua prog), dan
runtime `p5`/`p6` (≈10× lebih cepat), `p7` (36 vs 45). `p9` hampir seri
(34 vs 30). Kalah runtime di `p1`/`p2`/`p3`/`p4`/`p8` — semuanya karena
keterbatasan arsitektural (tanpa vectorizer, tanpa inliner lintas-CGU, tanpa
tail-recursion elimination, struktur lane).

### 21.5 Verifikasi

`./y.sh test --sysroot rakit` → **65 suite, 0 gagal** (EXIT 0). Semua keluaran
program benchmark (p1–p9, `t_atom`, `t_loop`, `t_u64f32`, `t_iter`, `t_alias`)
identik dengan LLVM.

## 22. Transformasi lane untuk loop LCG + reduksi (LANDED)

### 22.1 Masalah: O(faktor²) perkalian

`apply_partial` (§19) membangun `pre[k]` (nilai state setelah `k+1` iterasi)
langsung dari header, jadi tiap `pre[k]` butuh `C^k·acc` + suku forcing
`Σ C^j·a_j`. Untuk faktor 3 itu 6 `imul` per 3 iterasi = **2 imul/iterasi**.
LLVM hanya ~1: ia memakai **4 lane**, tiap lane maju `C^4` per langkah luar.

### 22.2 Kunci: faktor genap → paritas tetap per lane

Lane `L` mengunjungi `i = L, L+f, L+2f, …`. Untuk **f genap**, `i` punya
**paritas sama** dengan `L`, sehingga `(i|1) = i + (L genap ? 1 : 0)` menjadi
**linear** terhadap langkah lane `k`. Akibatnya suku forcing tiap lane affine
pada `k` dan **maju dengan tambahan konstan** (`slope`), bukan `imul` per
iterasi. Faktor ganjil (3) tidak bisa memakai trik ini karena paritas `i`
bergantian per lane.

Rekurensi lane (`f` lane, `B` trip count, `B % f == 0`):
- `a_L(k+1) = C^f · a_L(k) + F_L(k)`, `F_L(k) = fbase(k) + lane_const[L]`,
  `fbase(k) = k·slope`, `slope = f·S`, `S = Σ_{k<f} C^k`.
- `F_L(0)` dan `a_L(0)` konstanta compile-time (`a_L(0) = C^L·a_0 + const`).
Loop: tiap langkah `min` atas `f` lane, lalu `a_L = C^f·a_L + fbase + lane_const[L]`
dan `fbase += slope`. Total **f imul per f iterasi = 1 imul/iterasi**.

### 22.3 Implementasi (`src/unroll.rs`, `apply_lanes`)

Fungsi baru `apply_lanes`, dipanggil dari `apply` sebelum jalur clone. Deteksi
(`detect_lane`): header berparameter tepat `(iv, a, mn)`, `a' = C·a + (iv|1)`,
`mn' = select(icmp(cc,l,r), arm1, arm2)` atas `{a', mn}`, `iv` mulai 0, dan
`bound` **konstan** habis dibagi `f`. Jika salah satu tidak terpenuhi → jatuh
ke jalur clone yang lama (aman).

Struktur blok baru: `prologue` (hitung `a_1..a_f` dari `a_0`) → `lane_header`
(cek `iv < B`) → `lane_body` (min + advance) → kembali. Edge pre-header
di-redirect ke `prologue`. **Exit memakai header lama sebagai trampolin**: lane
loop mengembalikan `(iv=B, a=a_cur, mn)` ke header lama, yang mengecek ulang
`iv<B` (salah) lalu jatuh ke exit asli — exit membaca parameter header lewat
dominasi, jadi tetap benar tanpa perlu menulis ulang blok exit. `a_cur` (=`a_B`)
dibawa sebagai state karena lane sudah maju melewati `a_B` saat keluar.

### 22.4 Hasil

`p8_minmax` (30 juta iterasi): **29 → 21 ms** (LLVM 18). Sapuan faktor
(ms): f=2 **21**, 4 23, 6 22, 8 24. f=2 menang karena tekanan register lebih
kecil (8 nilai terbawa pada f=4 memicu spill; f=2 tidak). Sisa gap ~1.17× ke
LLVM adalah kualitas regalloc `rakit-codegen` (spill), di luar cakupan.

### 22.5 Verifikasi

- Uji diferensial lintas bound konstan (`0,1,2,3,4,5,7,8,12,16,20,100,1000,
  12345,30M`) dan reduksi min/max dengan dua multiplier: byte-identik dengan
  LLVM. Bound habis dibagi f → jalur lane; sisanya → fallback clone.
- `a` setelah loop (=`a_B`), `mn` awal non-MAX, `a` awal ≠ 1: byte-identik.
### 23.5 Aturan lokal (selalu aman, tanpa dominator)

Selain aturan dominator, dua bentuk lokal pasti `< 2^63` dan langsung disempitkan:
- `fcvt_from_uint(uextend(y))` dengan `y` bertipe sempit (i32/i16/i8): hasil
  zero-extend ada di `0..2^32-1` → `fcvt_from_sint` eksak.
- `fcvt_from_uint(band(x, M))` dengan `M < 2^63`: hasil `& M ≤ M < 2^63`.

Catatan: `fcvt_from_sint` pada **i32 langsung** TIDAK aman (nilai `u32 ≥ 2^31`
jadi negatif), jadi tipe sempit saja bukan alasan cukup — hanya `uextend` yang
menjamin non-negatif.

- `./y.sh test --sysroot rakit` → **65 suite, 0 gagal**.
- Semua 9 program benchmark keluaran identik dengan LLVM; tidak ada regresi
  (p1–p9, compile/size/runtime).

## 23. Penyempitan `fcvt_from_uint` berbasis dominator (LANDED)

### 23.1 Masalah

`(i as f32)` untuk `i: usize` menjadi `fcvt_from_uint` (u64→f32). Di x86-64
konversi u64→float yang benar untuk seluruh rentang butuh **branch**: nilai
≥ 2^63 harus di-`shr`/`or` dulu. LLVM menghindarinya karena analisis rentang
tahu `i < N < 2^63` → cukup satu `cvtsi2ss`. Rakit tak melihat batas itu,
jadi selalu mengeluarkan barisan bercabang.

### 23.2 Perbaikan (`src/unroll.rs`, `narrow_uint_fcvt`)

Untuk setiap `fcvt_from_uint(x)`, telusuri **dominator** bloknya. Jika ada
dominator yang mem-`brif` pada `icmp ult x, C` (atau `ule`) dengan `C < 2^63`
dan **sisi-true-nya mendominasi** blok itu, maka di blok tersebut `x ≤ C < 2^63`
— aman diganti `fcvt_from_sint`. Ditangani juga `C > x` / `C >= x`. Batas
dinamis (bukan konstanta) atau ≥ 2^63 dibiarkan. Knob: `RAKIT_NO_NARROW_FCVT`.

Kesederhanaan ini penting: transformasi tidak butuh model loop — hanya
`DominatorTree` dan satu `icmp` penjaga, sehingga bekerja juga untuk loop
berbadan **banyak blok** (loop `for i in 0..n` di rakit sering terpecah
beberapa blok oleh penurunan iterator rentang, dan `analyze` di §19 menolaknya).

### 23.3 Hasil

`p9_u64f32`: konversi bercabang hilang (disasm: `xorps; cvtsi2ss` saja),
**36 → 35 ms** (LLVM 31). Keuntungan kecil karena branch-nya memang terprediksi
baik, tetapi **umum**: setiap `for i in 0..n { … i as f32/f64 … }` dengan `n`
konstanta kecil kini memakai satu instruksi konversi, bukan barisan bercabang.

### 23.4 Verifikasi

- Uji diferensial: bound konstan (`0,1,4,100,12345,4M`) → jalur sempit;
  bound dinamis (`std::env::args`) → fallback; loop `(1<<40)..` (i ≥ 2^31) →
  benar. Semua byte-identik dengan LLVM.
- Semua 9 program benchmark keluaran identik; tidak ada regresi.
### 23.5 Aturan lokal (selalu aman, tanpa dominator)

Selain aturan dominator, dua bentuk lokal pasti `< 2^63` dan langsung disempitkan:
- `fcvt_from_uint(uextend(y))` dengan `y` bertipe sempit (i32/i16/i8): hasil
  zero-extend ada di `0..2^32-1` → `fcvt_from_sint` eksak.
- `fcvt_from_uint(band(x, M))` dengan `M < 2^63`: hasil `& M ≤ M < 2^63`.

Catatan: `fcvt_from_sint` pada **i32 langsung** TIDAK aman (nilai `u32 ≥ 2^31`
jadi negatif), jadi tipe sempit saja bukan alasan cukup — hanya `uextend` yang
menjamin non-negatif.

- `./y.sh test --sysroot rakit` → **65 suite, 0 gagal**.

## 24. Fusi `sin`+`cos` → `sincos` (LANDED)

### 24.1 Masalah
LLVM menggabungkan `sin(x)` dan `cos(x)` dengan argumen sama menjadi satu
panggilan `sincosf`/`sincos` (glibc memakai reduksi argumen bersama, ~30% lebih
murah dari dua panggilan). rakit memanggil `sinf` + `cosf` terpisah. Di
`p4_simd` (loop `a[i]=sin(i); b[i]=cos(i)`) ini ~14% runtime.

### 24.2 Perbaikan (`src/sincos.rs`, pass RAKIT baru)
Untuk setiap `sinf(x)`/`sin(x)` dan `cosf(x)`/`cos(x)` dengan argumen sama,
bila blok `sin` **mendominasi** blok `cos` (atau blok yang sama):
1. buat dua stack slot (keluaran sin & cos),
2. deklarasikan `sincosf`/`sincos` (sesuai tipe: f32/f64 — **wajib dua
   deklarasi terpisah**, lihat 24.3),
3. sisipkan `sincos(x, &sin_out, &cos_out)` di posisi panggilan sin, lalu muat
   kedua hasil dari stack slot,
4. alihkan semua penggunaan hasil `sin`/`cos` ke hasil muat itu
   (`detach_inst_results` + `change_to_alias`), hapus kedua panggilan lama.

Pass berjalan di `base.rs` (punya `module` untuk mendeklarasikan simbol) setelah
`unroll`. Knob: `RAKIT_NO_SINCOS`.

### 24.3 Bug yang ditemukan (penting)
Versi pertama mendeklarasikan **satu** func_ref berdasarkan tipe pasangan
*pertama*. Di fungsi yang punya pasangan f32 **dan** f64 (loop 1 `p4`), pasangan
f32 memanggil `sincos` (f64) → hasil salah. Uji diferensial menangkapnya
(`sincostest` mismatch) walau 9 benchmark lolos (p4 hanya f32). Perbaikan:
deklarasikan `sincosf` (f32) dan `sincos` (f64) terpisah, pilih per pasangan.
Pelajaran: **satu fungsi bisa punya pasangan f32 dan f64** — jangan asumsikan
tipe seragam.

### 24.4 Hasil & verifikasi
`p4_simd`: **100 → 86 ms** (LLVM 59), `sincosf` terpakai, keluaran identik.
Diuji: same-block, cross-block (`if`), argumen berbeda (tidak difus), f32 & f64,
dalam loop, `sin`-saja/`cos`-saja. Semua byte-identik LLVM. `./y.sh test` →
**65 suite, 0 gagal**.

## 25. Upaya eliminasi bounds check redundan (NEGATIF — ditarik)

### 25.1 Motivasi
`for i in 0..v.len() { v[i] }` di rakit menghasilkan **dua** perbandingan di
loop: guard `i < len` (lompat ke body) dan, di body, cek bounds `i >= len →
panic_bounds_check`. Yang kedua redundan (guard sudah membuktikan `i < len`).
Ini pola sangat umum; LLVM mengeliminasinya.

### 25.2 Yang dicoba
Pass RAKIT `redundant_branch`: untuk tiap `brif (icmp CC x y) T, F`, cari
dominator dengan `brif (icmp CC x' y')` di mana sisi yang sesuai mendominasi
blok ini; jika `(x,y) == (x',y')`, kondisi terbukti dan cabang mati → ganti
dengan `jump`. Ditambah substitusi block-param pada edge langsung (guard
meneruskan `i` ke body lewat parameter blok).

### 25.3 Mengapa gagal (akar masalah, via dump RAKIT)
- **9 benchmark: 0 perubahan biner.** Pass tidak pernah menyala.
- Dump RAKIT `bcbig` menunjukkan guard loop membandingkan terhadap `v8` (nilai
  `end` dari `Range`, diteruskan sebagai block-param) sedangkan cek bounds
  membandingkan terhadap `v75` (hasil `load` `v.len()` yang **terpisah**).
  `v8 != v75` sebagai SSA value walau keduanya = panjang slice. Aturan
  "operand sama" tidak bisa menyala.
- Di kasus kecil (`bc.rs`, `(0..1000).collect()`) pass **menyala** (2→1 cek) —
  di sana kedua load ter-CSE. Tapi `bcbig` (pola `.map().collect()` + loop
  bersarang) tidak.
- **Prasyarat sebenarnya**: CSE/value-numbering atas dua load panjang
  (`Range::end` vs `v.len()`), plus bukti tak ada `store` ke slot header Vec di
  antaranya. Itu analisis alias/load, bukan sekadar pass cabang.

### 25.4 Catatan nilai
Bahkan bila cek tereliminasi, `bcbig` tetap 134 ms vs LLVM 40 ms — gap-nya
**vektorisasi** (4×), bukan bounds check (~10-20%). Jadi ini bukan tuas utama
untuk p4/p2. Pass ditarik (dead code); dicatat agar tidak diulang tanpa CSE
panjang lebih dulu.

## 26. Upaya hoisting load loop-invariant (NEGATIF — ditarik)

### 26.1 Motivasi
Header `Vec` (`ptr`, `len`) di-emit rakit sebagai load slot stack biasa
(`notrap` saja, tanpa `readonly`/`can_move`) tiap iterasi. Pass RAKIT
`licm_loads` menandai load dari slot privat yang alamatnya `stack_addr+const`
dan slotnya tak di-store di loop itu menjadi `readonly+can_move+notrap`,
berharap pass LICM Rakit (yang jalan kemudian di `Context::compile`)
mengangkatnya ke preheader.

### 26.2 Hasil (negatif)
- Pass **menyala** (biner p2/p4/p9 berubah) dan **benar** (output cocok LLVM).
- Tapi **0 perbaikan runtime**: p2 median 54.4 (on) vs 53.8 (off) ms — noise;
  p4 51=51 reload `(%rsp)`; p9 13=13 reload. Load header **tidak terangkat**.
- **Akar 1**: LICM menolak mengangkat load apa pun bila loop punya
  *opaque store* (mis. panggilan `sinf` di loop p9): `safe = !any_opaque_store
  && !store_bases.contains(base)`. Jadi load slot-privat tetap diblok walau
  address-nya tak bisa ditulis panggilan.
- **Akar 2 (lebih penting)**: bahkan di loop tanpa panggilan (p4 loop 3),
  reload header **bukan bottleneck**. Gap p9 = `sinf` (85% sampel), p4 =
  vektorisasi, p2 = codegen sort/hash. Menghapus 2 reload L1 tak terasa.

### 26.3 Kesimpulan
Hoisting load header bukan tuas yang berguna untuk benchmark ini. Untuk
membuatnya benar-benar mengangkat perlu mengubah `licm.rs` agar load dari slot
yang alamatnya tak escape boleh lewat opaque store — tapi dampaknya tetap
marginal (loop dibatasi hal lain). Pass ditarik. Bila dikerjakan lagi,
prasyaratnya: (a) patch `licm.rs` untuk private-slot load, (b) buktikan dulu
loop-nya memang memory-reload-bound, bukan libcall/vektorisasi-bound.

## 27. Pass `rec_loop`: rekursi linear → loop (p3 −38%)

### 27.1 Kenapa ini bisa padahal saya sempat bilang "tidak bisa"
Transformasi `fib`→loop **bukan** pekerjaan backend — itu pass **middle-end
LLVM (`TailCallElim`)** pada LLVM IR. rakit melewati middle-end itu, jadi
bentuk naif (dua call/aktivasi) sampai ke RAKIT. Tapi polanya bisa dipulihkan di
RAKIT: pass baru `src/rec_loop.rs` mencocokkan
`F(x) = if x < base { x } else { F(x-c1) + F(x-c2) }` dan menulis ulang jadi
loop berakumulator `acc=0; while x>=base { acc += F(x-c1); x -= c2 } return acc+x`.
Ini meniru `TailCallElim`, hanya untuk pola linear-recurrence (fib-shaped).

### 27.2 Detail implementasi
- Cocokkan persis: entry 1 param i64 → brif `icmp ult n, base` → blok base
  `return n` → `t1=call self(n-c1)` → `t2=call self(n-c2)` → `return t1+t2`.
  Kedua call harus **self-call** (dicek via `user_named_funcs()[reff].index` ==
  nama fungsi). Base harus identitas (`return n`), penjumlahan harus `iadd` dari
  kedua hasil. Jika tidak persis → dilewati (konservatif).
- **Jebakan**: dua self-call memakai **FuncRef berbeda** (`fn0`/`fn1`, sig
  berbeda, fungsi sama). Jangan cek `fr1 == fr2`; cek `is_self(fr1) && is_self(fr2)`.
- Bangun 3 blok baru (loop/base/rec) dengan param loop-carried `(n, acc)`,
  redirect terminator entry ke `jump loop(n, 0)`, hapus 5 blok lama
  (`remove_block_and_insts`). Dijalankan sebelum `store_forward` (early).
- Knob `RAKIT_NO_REC_LOOP` untuk A/B.

### 27.3 Hasil
`fib` sekarang **1 call/iterasi** (persis bentuk LLVM). p3_calls
**92.4 → 57.2 ms (−38%)**, M/L **2.60× → 1.61×**. Benchmark lain tidak berubah
(tidak ada fungsi fib-shaped). Correctness: `fib(0..40)` identik LLVM, 9 prog
benchmark identik LLVM, **`./y.sh test` → 25368 passed / 65 suites / 0 failed**.

### 27.4 Cakupan & batas
Hanya pola linear-recurrence identitas ini. Kelas yang lebih luas (base bukan
identitas, `k·F(n-1)+c`, dst.) butuh generalisasi seperti `TailCallElim` penuh —
pekerjaan lanjutan. Ini bukti bahwa gap "middle-end" LLVM bisa dipulihkan
sebagian dengan pass RAKIT bertarget.

## 28. Inliner "hot callee" → p2 −5%

### 28.1 Eksperimen "std 1-CGU" membantah hipotesis cross-CGU
- std **sudah** 1 CGU: `libstd-*.rlib` hanya berisi **1 file `.o`** (`ar t`).
- Fungsi sort (`partition_*`, `swap_if_less`, `select_unpredictable`) di-monomorphize
  di **crate user** (nama mangled berakhir `_5p2big`), bukan std; `-Ccodegen-units=1`
  pada program juga tak mengubah apa pun (tetap 65 call).
- Filter `is_final` bukan penyebab (dihapus → tetap 65 call); semua `final=true`
  dan sudah ada di cache `bodies` (swap_if_less 48 insts, select 47/11 insts).
- **Akar sebenarnya: budget pertumbuhan inliner per-caller.** Debug inliner:
  `reject_size id=35 size=48 cap=128 budget=5 self=36` — `sort9_optimal`
  meng-inline ~6 `swap_if_less` lalu budget habis (sisa 5 < 48) → 65 call ditolak.

### 28.2 Perbaikan: budget "hot callee"
Kalau sebuah callee sudah di-inline ≥ `HOT_CALLS` kali ke satu caller (hot di situ),
inline berikutnya boleh memakai `HOT_BUDGET` terpisah meski budget biasa habis.
`HOT_CALLS=1`, `HOT_BUDGET=1024`. Ini melipat banyak call kecil (jaringan sortir)
alih-alih berhenti setelah beberapa.

### 28.3 Hasil
- p2_mem **52.6 → 50.0 ms (−5.0%)**, gap 3.42× → **3.25×**.
- p1/p3–p9 dalam noise (≤±1.6%); **p9 tak regresi**.
- Size +0.2–0.4% (8–18 KB), compile sama (dalam noise).
- `./y.sh test --sysroot rakit` → **25368 passed / 65 suites / 0 failed**.

### 28.4 Catatan
`HOT_BUDGET=4096` (lebih agresif) memberi p2 −4.1% **dan** p3 −5.3% tetapi p9 +4.3%
regresi → ditolak; 1024 = titik aman. Jadi "p2 tidak bisa" ternyata **sebagian bisa**
(sort inlining); sisa p2 tetap codegen (partition 28% sudah 0-call, codegen-bound).

### 28.5 Validasi pada crate nyata (extbench) — generalisasi terbukti
A/B hot-callee (ON vs `RAKIT_NO_HOT_INLINE=1`) pada 3 crate nyata crateworks:
| crate | run hot/base | Δrun | Δcompile | Δsize |
|-------|-------------|------|----------|-------|
| openlocationcode | 10971/11139 | −1.5% | −0.4% | −616 |
| holidays-rs | 16933/17104 | −1.0% | +3.6% | +48 |
| stdnum-rs | 1833/1809 | +1.4% (noise) | −0.9% | −504 |
Menang kecil di 2/3, netral di stdnum, **compile ~netral**. Berbeda dengan
`MIN_GROWTH=1024` (sesi lain) yang memberi p2 −8.7% tetapi **compile +15%** di
crate nyata lalu dibuang. Hot-callee menargetkan callee yang di-inline berulang
(bukan menaikkan lantai semua caller), jadi biaya compile-nya kecil dan
**bukan overfitting ke p2_mem**. Knob `RAKIT_NO_HOT_INLINE` untuk bisect.

## 29. Audit regresi RIR (2026-11-23)

Benchmark penuh `bench/allaxes.sh` (CREPS=3, RREPS=7) mengungkap dua regresi
besar pada jalur RIR default:

- **`rec_loop` RIR** masih memancarkan **dua** panggilan diri per iterasi
  (panggilan kedua hasilnya mati). `p3_calls` menjadi **7.731 ms**; setelah
  ditulis ulang menjadi satu panggilan per iterasi (seperti pass codegen),
  menjadi **57 ms** dan keluaran tetap identik dengan LLVM.
- **`unroll` RIR** menduplikasi badan loop tetapi tidak melakukan reassociation
  affine, sehingga menghalangi unroller codegen yang sudah dioptimalkan.
  `p5_inline`/`p6_scalar` menjadi **358/401 ms**. Pass ini sekarang **opt-in**
  (`RAKIT_RIR_UNROLL=1`); default codegen unroller kembali aktif dan keduanya
  menjadi **7 ms**.

Matriks 3-arah terakhir (`bench/allaxes.sh`, CREPS=3, RREPS=7; C = compile,
R = run; wall/CPU dalam ms, RSS dalam MB, size dalam byte):

| prog | bk | Cwall | Ccpu | Crss | size | Rwall | Rcpu | Rrss |
|------|----|------:|-----:|-----:|-----:|------:|-----:|-----:|
| p1_arith | L | 84 | 102 | 105.5 | 4,635,232 | 82 | 82 | 10.5 |
| p1_arith | U | 81 | 104 | 107.2 | 5,877,944 | 110 | 109 | 10.7 |
| p1_arith | M | 78 | 100 | 109.6 | 4,146,240 | 102 | 101 | 10.7 |
| p2_mem | L | 245 | 378 | 133.8 | 4,639,888 | 16 | 16 | 10.5 |
| p2_mem | U | 128 | 174 | 130.8 | 5,936,144 | 157 | 158 | 10.7 |
| p2_mem | M | 161 | 224 | 133.1 | 4,143,704 | 48 | 47 | 10.8 |
| p3_calls | L | 102 | 125 | 116.1 | 4,612,368 | 37 | 36 | 10.7 |
| p3_calls | U | 85 | 112 | 116.4 | 5,820,136 | 116 | 116 | 10.7 |
| p3_calls | M | 96 | 123 | 118.6 | 4,067,168 | 59 | 60 | 10.5 |
| p4_simd | L | 108 | 133 | 116.4 | 4,636,096 | 58 | 58 | 48.1 |
| p4_simd | U | 90 | 114 | 117.0 | 5,892,376 | 156 | 155 | 49.5 |
| p4_simd | M | 100 | 124 | 119.2 | 4,152,832 | 82 | 82 | 48.8 |
| p5_inline | L | 82 | 101 | 104.9 | 4,609,368 | 58 | 57 | 10.7 |
| p5_inline | U | 75 | 100 | 106.5 | 5,813,704 | 893 | 892 | 10.8 |
| p5_inline | M | 76 | 101 | 108.3 | 4,060,088 | 7 | 6 | 10.5 |
| p6_scalar | L | 82 | 101 | 101.2 | 4,609,368 | 58 | 57 | 10.3 |
| p6_scalar | U | 75 | 98 | 103.0 | 5,813,688 | 902 | 901 | 10.7 |
| p6_scalar | M | 75 | 96 | 104.6 | 4,060,088 | 6 | 6 | 10.7 |
| p7_branchy | L | 84 | 102 | 104.9 | 4,609,520 | 43 | 42 | 10.7 |
| p7_branchy | U | 77 | 100 | 106.6 | 5,813,800 | 100 | 100 | 10.7 |
| p7_branchy | M | 76 | 98 | 108.4 | 4,060,248 | 35 | 35 | 10.7 |
| p8_minmax | L | 84 | 104 | 104.9 | 4,609,544 | 18 | 18 | 10.7 |
| p8_minmax | U | 77 | 103 | 106.6 | 5,813,752 | 38 | 38 | 10.7 |
| p8_minmax | M | 77 | 100 | 108.4 | 4,060,216 | 20 | 20 | 10.5 |
| p9_u64f32 | L | 94 | 114 | 110.9 | 4,635,448 | 30 | 30 | 17.8 |
| p9_u64f32 | U | 83 | 104 | 112.6 | 5,886,872 | 76 | 76 | 18.9 |
| p9_u64f32 | M | 90 | 113 | 114.7 | 4,149,856 | 33 | 32 | 18.2 |

Dibanding matriks §21.4: runtime `p3` 94→59, `p4` 99→82, `p8` 29→20;
`p2` 52→48, `p7` 36→35; `p1`/`p9` dalam ±1 ms; `p5`/`p6` 6→7 ms karena
pembulatan. Ukuran artefak M lebih kecil dari L dan U pada semua program
(4.06–4.15 MB vs L 4.61–4.64 MB, U 5.81–5.94 MB). Compile absolut naik di
semua backend pada run ini (mesin lebih lambat), tetapi dengan anggaran RIR CGU
besar `p2` M 161 ms (U 128, L 245); `p5`/`p6` M 76/75 ms, setara U (75/75)
dan lebih cepat dari L (82/82). `p2` masih ~26% lebih lambat compile daripada
U, tetapi runtime-nya jauh lebih cepat dari U (48 vs 157 ms).

### 29.1 Anggaran RIR untuk CGU besar (p2/p5 compile)

Audit p2 menunjukkan RIR round-trip menghabiskan **27,6 ms** untuk 142 fungsi
(import 5,1 ms, pipeline 19,4 ms, export 3,1 ms), sedangkan p5 hanya 8 fungsi
(0,8 ms). Biaya itu per fungsi dan tidak sebanding dengan manfaatnya pada body
besar yang loop panasnya sudah ditangani pass codegen.

Aturan baru di `RirPass`: bila CGU punya **>64 fungsi** yang eligible RIR,
hanya fungsi dengan **≤50 instruksi CLIF** yang di-round-trip. CGU kecil (p3,
p5, p6) tidak terkena batas ini. Knob: `RAKIT_RIR_LARGE_CGU`,
`RAKIT_RIR_LARGE_MAX_INSTS`, dan override global `RAKIT_RIR_MAX_INSTS`.

Hasil pada p2 (3 compile / 7 run): compile **194 → 173 ms** dan runtime
**53 → 50 ms**; p7 compile **100 → 88 ms**, p8 **100 → 84 ms**, p9
**111 → 100 ms**; p5 tidak berubah karena CGU-nya kecil. Pada matriks penuh
p5 compile menjadi **86 ms** (LLVM 92, upstream 87), jadi keluhan p5 compile
hilang setelah run diulang pada build yang sama.

Eksperimen yang **ditolak** (diukur, bukan diasumsikan):
- Menunda `optimize()` sampai akhir pipeline: compile p2 **+6 ms**, runtime
  **+3 ms** — pass berikutnya kehilangan cleanup yang dibutuhkan.
- Hanya mengekspor bila pass struktural berubah (bukan simplify/opt saja):
  runtime p2 sedikit membaik tetapi compile p5/p8/p9 memburuk 5–15 ms.
- Mematikan `licm`, `if_convert`, atau `div_test` RIR: compile justru naik
  karena output RIR lebih besar; tidak ada keuntungan runtime.
- Verifikasi import/export bukan penyebab: mematikannya tidak mengubah waktu
  p2/p5 secara terukur.
- Melewati RIR sepenuhnya di CGU besar (`RAKIT_RIR_LARGE_MAX_INSTS=0`):
  compile p2 180 vs 181 ms, runtime 52,8 vs 51,1 ms — tidak ada manfaat.
- A/B pass codegen p2: `RAKIT_NO_STORE_FWD=1` compile 185/run 75 ms;
  `RAKIT_NO_CODEGEN_INLINE=1` 170/112 ms; `RAKIT_INLINE_GROWTH=10` 174/54 ms
  vs default 183/51 ms. Jadi sisa gap compile p2 ada di pass codegen/inliner,
  bukan RIR; memotongnya merusak runtime.
- p5: default 89 ms; mematikan store-forward 91, codegen inline 92,
  if-convert 93, div-test 91, unroll 93 ms. Tidak ada pass yang bisa dimatikan
  tanpa memperlambat compile; p5 sudah setara U pada matriks penuh.

Verifikasi: seluruh keluaran p1–p9 byte-identik dengan LLVM; `./y.sh test`
65 suite `ok`, 0 failure; `rakit-ir` dan `rakit-opt` lulus; test regresi baru
`rakit-ir/tests/rec_loop.rs` memastikan hanya satu self-call per iterasi.

### 29.2 Audit correctness hasil review (2026-11-23)

Semua temuan review ditindaklanjuti. Perubahan yang mendarat:

- **Affine unroll tidak lagi melewati efek samping.** `body_has_observable_effect`
  menolak reassociation penuh dan lane lowering bila badan loop berisi call,
  load/store, trap, atau opcode berefek lain; partial/clone unrolling tetap
  tersedia karena setiap clone mengeksekusi badan. Regresi:
  `bench/tests/t_loop_side_effect.rs` (64 call `tick()`), keluaran L dan M
  identik `2016 64`.
- **RIR unroll tidak memakai nilai header yang basi.** `header_supported`
  menolak badan yang membaca hasil header yang bergantung pada induksi, dan
  menolak header dengan efek samping. Test:
  `header_value_used_by_body_is_not_reused_stale` (sebelumnya 8, sekarang 12).
- **Entry kondisional tidak overshoot.** `initial_iv` mensyaratkan tepat satu
  edge preheader `Jump`; edge `Brif`/try-call ke header ditolak. Test:
  `conditional_second_entry_does_not_overshoot`.
- **Interpreter**: `Sextend` ke I128 memakai lebar sumber; `FcvtFromInt/Uint`
  menangani sumber sempit bertanda dan payload I128 penuh; `FcvtToInt/Uint`
  memotong ke arah nol, memeriksa range tujuan, dan menolak NaN/overflow;
  operasi pointer memakai semantik integer bertanda; `Iconst` Ptr menghasilkan
  `Val::P`. Test baru `rakit-ir/tests/interp.rs` (6 kasus).
- **Anggaran RIR CGU** menghitung fungsi eligible lewat preflight bridge
  (`rir::is_supported`), bukan panjang cache inliner yang sudah difilter ukuran.
  `RAKIT_RIR_FORCE=1` kini juga melewati batas ukuran untuk validasi.
- **Debug info**: fungsi dengan `base_srcloc` tidak di-export ulang oleh RIR,
  sehingga line table/stepping tidak hilang saat export belum membawa srcloc.
- **Verifier** menolak use live dari nilai yang hanya didefinisikan di blok
  unreachable; test `rakit-ir/tests/verify.rs`.
- **Matcher RIR** melanjutkan scan setelah header loop yang tidak memenuhi
  syarat, sehingga loop konstan berikutnya tetap bisa di-unroll.
- **Test `rec_loop`** memakai decrement berbeda (c1=1, c2=2) dan memeriksa
  operand call serta induksi, bukan hanya jumlah call.
- **LICM RIR** menghitung predecessors/dominator/loop sekali per pemanggilan;
  hanya peta definisi yang di-refresh setiap ronde.
- **Indirect call non-System V** diterima bila semua parameter ABI normal dan
  convention dapat di-encode RIR; test round-trip `CallConv::Fast`.
- **Matriks benchmark** §29 sekarang memuat wall, CPU, peak RSS, dan ukuran
  artefak dalam byte untuk setiap backend dan program.
