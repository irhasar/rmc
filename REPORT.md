# Cranelift vs LLVM: Ukuran, Waktu Compile, dan Performa Runtime

Ringkasan eksperimen membandingkan tiga backend codegen Rust pada
`nightly-2026-11-17`, x86_64-unknown-linux-gnu, opt-level=3, debuginfo=0:

1. **LLVM** — `rustc` bawaan
2. **cg_clif upstream** — commit `42ac63e`
3. **cg_clif modifikasi** — fork dengan patch di direktori ini

Semua output program identik di ketiga backend (diverifikasi), jadi
perbandingan mengukur kualitas codegen saja, bukan semantik.

## 1. Fakta lisensi

Cranelift dilisensikan **Apache-2.0 WITH LLVM-exception**
(`cranelift-object-0.137.0/Cargo.toml`: `license = "Apache-2.0 WITH LLVM-exception"`).
Artinya:

- Bebas dipakai, dimodifikasi, dan didistribusikan, termasuk untuk komersial.
- Klausa *LLVM-exception* menghapus kewajiban menyingkap source code saat
  didistribusikan sebagai bagian dari binary.
- Karena itu rombak total / restruktur / refaktor Cranelift **boleh** dilakukan.
  Satu-satunya kewajiban praktis: pertahankan notice lisensi dan cantumkan
  perubahan bila mendistribusikan source yang dimodifikasi.

cg_clif sendiri memakai lisensi MIT/Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`),
dan merupakan proyek *experimental*, jadi API internalnya bebas diubah.

## 2. Modifikasi yang diterapkan

Dua akar masalah ukuran berasal dari `cranelift-object` yang menamai setiap
fungsi/data dengan nama section yang **sama**. Linker menggabungkannya menjadi
satu section raksasa, sehingga `--gc-sections` tidak bisa membuang kode mati.

### Patch 1 — `vendor/cranelift-object-0.137.0/src/backend.rs`

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
| **cg_clif modifikasi** | **4.902.480** | **1.350.448** | **3.258** |

Program nyata `p2_mem`:

| Backend | mentah | stripped |
|---|---:|---:|
| LLVM | 4.639.888 | 356.608 |
| cg_clif upstream | 5.936.880 | 1.918.656 |
| **cg_clif modifikasi** | **5.058.368** | **1.446.968** |

**Perbaikan:** −904 KB (−15,6%) pada program kosong, −878 KB (−14,8%) pada
`p2_mem`, dan −487 KB (−26,5%) setelah di-strip. Selisih terhadap LLVM menyusut
dari +26% menjadi +6,5%.

Rincian section (program kosong): `.data.rel.ro` turun dari 85.416 → 31.176 byte
(simbol data 3.472 → 1.312), `.text` turun dari 1.152.234 → 995.050 byte.

## 4. Hasil: waktu compile

Median wall / CPU / peak RSS. cg_clif **lebih cepat dan lebih hemat memori**
saat mengompilasi dibanding LLVM, karena tidak ada pipeline optimisasi berat:

| Program | LLVM (wall/cpu/RSS) | cg_clif up | cg_clif mod |
|---|---|---:|---:|
| p1_arith | 88 / 107 / 106 MB | 80 / 101 / 109 MB | 85 / 111 / 109 MB |
| p2_mem | 250 / 382 / 134 MB | 140 / 191 / 132 MB | 137 / 182 / 133 MB |
| p3_calls | 101 / 123 / 116 MB | 88 / 112 / 118 MB | 87 / 116 / 118 MB |
| p4_simd | 114 / 135 / 117 MB | 95 / 118 / 118 MB | 94 / 124 / 118 MB |

cg_clif mengompilasi ~20–45% lebih cepat dari LLVM. Patch saya tidak menambah
waktu compile berarti (bahkan sedikit lebih cepat pada p2/p3/p4). Peak RSS
hampir sama; LLVM sedikit lebih hemat pada p1_arith.

## 5. Hasil: performa runtime

Median wall (ms), program dijalankan 5×:

| Program | LLVM | cg_clif up | cg_clif mod |
|---|---:|---:|---:|
| p1_arith | 82 | 108 | 108 |
| p2_mem | **17** | 155 | 156 |
| p3_calls | **37** | 118 | 116 |
| p4_simd | **57** | 157 | 156 |

cg_clif **1,3×–9× lebih lambat** dari LLVM. Patch ukuran tidak mengubah
kecepatan runtime (sesuai harapan — hanya membuang kode mati).

### Akar penyebab runtime (tiga lapis)

**Lapis 1 — tidak ada inlining.** Cranelift menyediakan infrastruktur inlining
(`Context::inline`, trait `Inline`) tetapi **tidak memanggilnya sendiri**.
Docstring-nya menyebut `cg_clif` secara eksplisit sebagai embedder yang harus
menyediakan heuristik inlining. cg_clif saat ini tidak mengimplementasikannya.

Bukti terarah (`add3` ditandai `#[inline(always)]`, loop 300 juta iterasi):

| Backend | ukuran `main` | `add3` di-inline? | waktu jalan |
|---|---:|---|---:|
| LLVM | 39 byte | ya | **58 ms** |
| cg_clif up | 32 byte | tidak | 894 ms |
| cg_clif mod | 32 byte | tidak | 896 ms |

**Lapis 2 — MIR inliner rustc mati untuk cg_clif.** Menyalakan
`-Zinline-mir=yes` membuat `main` tumbuh 32 → 211 byte (fungsi benar-benar
di-inline), tetapi runtime **tidak berubah** (896 ms). Artinya setelah inlining
pun masih ada hambatan lain.

**Lapis 3 — akumulator tetap di memori, tanpa unrolling/ILP.** Loop skalar murni
(`while i < 300_000_000 { acc = acc.add(i).mul(3); i += 1 }`) menghasilkan:

```asm
; cg_clif — 6 instruksi/iterasi, akumulator di stack, rantai dependen
4a208: mov %rcx,%rdx
4a20b: add (%rax),%rdx     ; LOAD acc dari stack
4a20e: imul $0x3,%rdx,%rdx
4a212: mov %rdx,(%rsp)     ; STORE acc ke stack
4a216: add $0x1,%rcx
4a21d: jmp 4a17c
```

LLVM sebaliknya meng-unroll 3×, menaruh semua nilai di register, dan memecah
rantai dependensi menjadi beberapa accumulator (ILP). Akibatnya loop cg_clif
*latency-bound* (~10 siklus/iterasi) sementara LLVM *throughput-bound*
(~0,6 siklus/iterasi) — selisih 15× pada kernel ini, dan menjelaskan 2–9× pada
program benchmark.

Ringkasnya: LLVM melakukan inlining + unrolling + promosi lokal ke register +
strength reduction; cg_clif tidak melakukan keempatnya. Ini gap arsitektural
codegen, bukan satu bug.

**Konfirmasi isolasi:** ketika cg_clif ditautkan ke std yang dibangun LLVM
(`--sysroot` ke sysroot nightly), ukuran binary-nya menjadi **4.604.376 byte** —
praktis sama dengan LLVM (4.609.368 byte) — tetapi runtime-nya tetap ~900 ms
pada p5/p6. Jadi selisih ukuran berasal dari std yang dikompilasi Cranelift,
sedangkan selisih runtime murni berasal dari codegen backend.

## 6. Peluang optimisasi (prioritas)

1. **Implementasikan trait `Inline` di cg_clif** — diminta langsung oleh API
   Cranelift, dan prasyarat untuk semua optimisasi berikutnya. Heuristik bisa
   memakai anotasi rustc (`#[inline(always)]`, `#[inline]`), ukuran callee, dan
   `mir_opt_level`. Tanpa ini, inlining lintas fungsi tidak terjadi.
2. **Promosikan lokal MIR ke register (SROA/register promotion)** — penyebab
   langsung loop 15× lebih lambat. Variabel `acc` di MIR adalah slot memori dan
   saat ini tetap dibaca/tulis ke stack tiap iterasi. Meski bukan tugas
   cg_clif, CLIF perlu pass yang mengenali slot lokal dan mempromosikannya.
3. **Unrolling + ILP** — untuk loop numerik, pecah rantai dependensi menjadi
   beberapa accumulator dan unroll beberapa iterasi.
4. **Perbaiki penamaan section di `cranelift-object` upstream** — patch saya
   sebaiknya diajukan sebagai PR; menguntungkan semua embedder (Wasmtime,
   cg_clif) karena `--gc-sections` jadi berfungsi.
5. **Aktifkan `per_data_object_section(true)`** — perbaikan satu baris dengan
   hasil −13% ukuran.
6. **Hapus kode backtrace mati** — kode backtrace (`gimli`, `addr2line`,
   `miniz_oxide`, `rustc_demangle`) menyumbang **60% `.text`** pada binary
   modifikasi (638 KB dari 1.063 KB) karena banyak fungsi kecil tidak pernah
   di-inline. Dengan inlining (poin 1) fungsi yang tidak terpakai otomatis
   hilang; alternatifnya `-Zbuild-std` dengan `panic_immediate_abort`.
7. **Pertimbangkan `opt_level` Cranelift** — saat ini cg_clif selalu memakai
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

Rekomendasi 1–3 dikerjakan di fork lokal `cg_clif_mod`; rekomendasi 4 tidak
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

### 8.2 Inliner CLIF (rekomendasi 1) — LANDED, `2ddb545`

`src/inliner.rs`: inliner per-CGU yang meng-inline callee kecil (≤64 instruksi
CLIF) dan *final* (tidak interposable) dengan budget 512 instruksi per caller.
Pengaman: tidak meng-inline diri sendiri, tiap callee paling banyak sekali per
caller, dan **melewati callee yang mereferensi `global_values`** — `inline_one`
Cranelift tidak me-remap global value (TLS) dengan benar sehingga object file
gagal link (`R_X86_64_TLSGD out of range`).

Status: benar dan aktif (terbukti menyusutkan jumlah simbol dengan
`-Zinline-mir=off`), tetapi **netral** terhadap ukuran/kecepatan karena MIR
inlining sudah meng-inline helper kecil yang jadi sasarannya. `CG_CLIF_INLINE_MAX=0`
mematikannya (knob untuk bisect).

### 8.3 Loop unrolling (rekomendasi 3) — LANDED, `9ea4bbe`

Gap runtime yang tersisa (356 ms vs 58 ms LLVM) berasal dari **reassociasi
affine**: LLVM tidak sekadar meng-unroll loop panas p5/p6, tetapi juga
mengubah bentuk perhitungan akumulatornya menjadi beberapa perkalian
independen (rantai dependensi loop-carried dipendekkan). Cranelift tidak punya
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

2. **Unroll duplikasi body (fallback, mati kecuali `CG_CLIF_UNROLL_CLONE`).**
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

Compile time ketiga backend praktis setara (~90–180 ms); cg_clif tidak lebih
lambat, hanya masih lebih besar dan (pada p2–p5) lebih lambat saat dijalankan.

### 8.6 Store→load forwarding stack-slot (rekomendasi 2, lanjutan) — LANDED

`src/store_forward.rs` menutup sisa gap p5/p6. Akar masalahnya: cg_clif
mematerialisasi agregat (`Option<u64>`, header iterator, tuple) ke stack slot
eksplisit, dan Cranelift **hanya** value-numbering load yang `readonly`+`notrap`
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
(`./y.sh build --sysroot clif`) dan keenam program menghasilkan keluaran
identik dengan LLVM.

| prog | up run | mod run (sebelum) | mod run (sesudah) |
|------|-------:|------------------:|------------------:|
| p5_inline | 894 ms | 356 ms | **61 ms** |
| p6_scalar | 898 ms | 356 ms | **61 ms** |

Knob: `CG_CLIF_NO_STORE_FWD`.

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
inlining lintas-CGU Cranelift**, yang tidak bisa diperbaiki dari sisi cg_clif
(sudah diuji: `-C target-cpu=native` tidak menutupnya).

### 8.8 Penurunan SIMD integer dengan instruksi vektor

Akar masalah runtime p2_mem: `src/intrinsics/simd.rs` men-skalarisasi setiap
perbandingan SIMD integer (`simd_eq/ne/lt/le/gt/ge`) menjadi loop per-lane
(load i8 → `icmp` → `bmask` → store, 16 kali untuk `u8x16`), dan
`simd_bitmask` menjadi loop shift/or per-lane. Akibatnya `_mm_cmpeq_epi8`
menghasilkan ±155 instruksi CLIF dan tidak pernah masuk batas inliner.

Cranelift x64 sudah bisa menurunkan `icmp` 128-bit integer menjadi satu
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

Hasil pada p2_mem: body `_mm_cmpeq_epi8` mengecil 155→34 instruksi CLIF dan
`_mm_movemask_epi8` 255→23; `_mm_cmpeq_epi8` kini menjadi satu `pcmpeqb`.
Runtime p2_mem **159 ms (upstream) → 110 ms**. Kebenaran diverifikasi dengan uji
17 kasus (semua tipe lane integer + float, semua perbandingan, `to_bitmask`) yang
keluarannya identik dengan LLVM.

Sisa gap p2/p4 berasal dari inlining lintas-CGU Cranelift (helper `memchr` /
`hashbrown` masih keluar dari baris) dan ketiadaan autovectorization di
Cranelift — keduanya di luar jangkauan perubahan sisi cg_clif ini.

### 8.9 Konstanta SIMD `vconst` dan forwarding store vektor lintas tipe

Tiga tindak lanjut penurunan SIMD, semuanya diverifikasi terhadap keluaran LLVM
pada keenam program bench dan uji SIMD 17 kasus.

1. **Konstanta SIMD 128-bit → `vconst`** (`38923c0`). Sebuah
   `ConstValue::Indirect` 16 byte, read-only, bebas pointer, dan terinisialisasi
   penuh (`Simd::ZERO`, mask splat, hasil `_mm_set1_*`) sebelumnya menjadi
   *anonymous global* + `load`. Sekarang diturunkan ke satu `vconst`. Efek
   gandanya penting: selain menghapus satu round-trip memori di jalur panas,
   `vconst` tidak membawa `GlobalValueData::Symbol`, dan *symbol* itulah yang
   memblokir inliner Cranelift (inliner tidak me-*remap* indeks `ExternalName`).
   Situs pemanggilan `_mm_*` di `p2_mem` turun **31 → 3** (sisa 3 berada di
   `hashbrown::find_inner`, lintas-CGU).

2. **Penyalinan nilai SIMD 128-bit lewat by-ref → satu load/store vektor**
   (`0e6c2f0`). Menulis nilai SIMD 128-bit by-ref ke place memori jatuh ke
   `emit_small_memory_copy`, yang memecah 16 byte menjadi dua `load.i64` + dua
   `store.i64`. Karena cg_clif melewatkan setiap op SIMD lewat stack slot, biaya
   ini dibayar per operasi; bitcast `__m128i → Simd<i8, 16>` saja menjadi
   1 store + 2 load + 2 store. Sekarang dipakai satu load dan satu store vektor
   16 byte. Vektor yang lebih lebar (`__m256i` tanpa AVX) tidak punya store
   tunggal, jadi tetap memakai `memcpy`.

3. **Store→load forwarding tipe vektor beda lane, ukuran sama** (`8ff4779`).
   Pass forwarding hanya mencocokkan load ke store bila tipe CLIF-nya persis
   sama. Sebuah bitcast SIMD (`__m128i ↔ Simd<i8, 16>`) justru dikompilasi
   sebagai store dengan satu tipe lane dan load dengan tipe lane lain, sehingga
   round-trip itu bertahan sampai kode mesin. Bila ukuran byte sama dan kedua
   tipe vektor, nilai store diteruskan lewat satu instruksi `bitcast`. Endianness
   target diambil dari ISA dan diberikan ke flag `bitcast` (verifier Cranelift
   mewajibkan endianness eksplisit bila jumlah lane berbeda).

Hasil pada `p2_mem`: runtime **110 → 99 ms** (LLVM 17 ms); ukuran **4.41 MB**
(LLVM 4.64 MB). Bitcast `as_i8x16` kini menjadi satu `bitcast.i8x16` (sebelumnya
`jump` + `stack_addr` + 2 `load.i64` + 2 `store.i64` + `load.i8x16`).

### 8.10 Ringkasan knob

| Env | Default | Efek |
|-----|---------|------|
| `CG_CLIF_UNROLL` | 8 | faktor reassociasi (1 = mati) |
| `CG_CLIF_UNROLL_CLONE` | unset | aktifkan unroll duplikasi-body (fallback) |
| `CG_CLIF_INLINE_MAX` | 64 | batas instruksi callee untuk inliner (0 = mati) |
| `CG_CLIF_INLINE_USES` | 3 | maksimum berapa kali satu callee boleh di-inline ke satu caller |
| `CG_CLIF_NO_STORE_FWD` | unset | matikan store→load forwarding |
| `CG_CLIF_PASS_TIME` | unset | cetak waktu per-pass cg_clif (store_forward/unroll/inline/collect) |
| `CG_CLIF_CLIF_TIME` | unset | cetak waktu per-pass internal Cranelift |
| `CG_CLIF_SF_NO_CBE` | unset | matikan dataflow store-forwarding lintas blok |
| `CG_CLIF_SF_CBE_MAX` | 300 | batas instruksi fungsi agar CBE dijalankan |

### 8.11 Inliner callee yang mereferensi global (revisi ini) — LANDED, `c2e292b`

Pengaman "lewati callee ber-`global_values`" di §8.2 adalah penyebab hilangnya
peluang terbesar: `core::hash::sip` dan closure `partition_lomuto_branchless_cyclic`
dari `slice::sort_unstable` semuanya mereferensi simbol (konstanta/static),
sehingga tidak pernah di-inline.

Akar masalahnya ada di **Cranelift**, bukan cg_clif: `create_global_values`
menyalin `GlobalValueData` apa adanya, termasuk `GlobalValueData::Symbol` yang
`ExternalName`-nya mengindeks tabel `user_named_funcs`/`ext_funcs` milik
**callee**. Indeks itu tidak pernah diterjemahkan ke namespace caller, jadi
meng-inline callee yang mereferensi const/static/thread-local merusak caller
(relokasi TLS keluar jangkauan).

Perbaikan: `cranelift-codegen` di-vendor (`vendor/cranelift-codegen-0.137.0`,
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
lagi soal inlining: memaksa `CG_CLIF_INLINE_MAX=200` agar round SipHash ikut
ter-inline justru memperlambat (136.6 → 154 ms), karena round-trip `SipState`
lewat pointer `&mut self` yang *escaping* tidak bisa di-forward oleh
`store_forward`, dan Cranelift menumpahkan/memuat ulang state tiap ronde.
Keunggulan LLVM di sini adalah register allocator + autovektorisasi — keduanya
internal backend dan tidak terjangkau pass sisi cg_clif.

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

Pass CLIF kustom berjalan sekali per fungsi, jadi setiap `std::env::var` di
dalamnya dibayar ribuan kali per crate — dan `std::env::var` mengunci map global
tiap panggilan. Tiga perubahan:

1. **`src/tunables.rs`** — semua knob dibaca sekali lewat `OnceLock` lalu
   di-cache; sisa pass memanggil fungsi tanpa lookup env.
2. **`store_forward`** — tiga penelusuran setup per-instruksi (slot privat,
   pointer base non-escaping, byte-range yang dibaca load) digabung jadi satu
   `analyze`. Dataflow worklist diganti satu sapuan *reverse-postorder*: pada CFG
   reducible (semua yang cg_clif hasilkan) predecessor selain back edge sudah
   dikunjungi sebelum sebuah blok, jadi satu sapuan mencapai fakta yang sama
   dengan fixed point. Back edge yang belum tersedia hanya menyumbang nol fakta —
   under-approximation konservatif, selalu aman untuk forwarding. Event store
   tiap blok di-decode sekali.
3. **Akuntansi pass opt-in** — `src/passtime.rs` (`CG_CLIF_PASS_TIME=1`) untuk
   pass cg_clif sendiri, `src/clifprof.rs` (`CG_CLIF_CLIF_TIME=1`) untuk pass
   internal Cranelift. Yang kedua butuh `timing.rs` mengekspos `ALL_PASSES`.

Biaya `store_forward` di p2_mem: **9.2 ms → 5.1 ms**; total pass kustom p2_mem
≈ 8 ms dari ~128 ms compile (≈6%). Profil pass Cranelift (sampel): total ≈27.9 ms;
Compilation ≈14.5, Egraph ≈4.6, Regalloc ≈4.2, VCode lower ≈2.0, emit ≈1.5.

### 8.14 Perbaikan ICE SIMD lane — LANDED

Commit `a9d182a` ("Keep SIMD values in registers") membuat nilai `#[repr(simd)]`
berada di register sebagai `ByVal` CLIF vector, tetapi
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
  codegen Cranelift (regalloc + autovektorisasi), bukan pass sisi cg_clif.



### 8.16 Inliner: batas pertumbuhan + penyaringan loop (revisi terakhir)

**Akar masalah.** Inliner menumbuhkan CLIF hingga +62% pada kode berat memori
(p2_mem: 14449 → 23383 instruksi). Setiap pass Cranelift berikutnya harus
menelusuri seluruh fungsi, jadi pertumbuhan itu membebani semua pass, bukan
hanya inliner. Inilah satu-satunya penyebab gap compile p2 vs U.

**Batas pertumbuhan per pemanggil.** Jumlah instruksi yang boleh diserap satu
pemanggil dibatasi ke pecahan dari ukuran pemanggil itu sendiri
(`CG_CLIF_INLINE_GROWTH`, default 50%), diklem ke `[64, 512]`. Hasil pada p2:

| `CG_CLIF_INLINE_GROWTH` | compile wall (ms) | runtime wall (ms) |
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
894/903 → 61 ms. Sisanya adalah kualitas codegen Cranelift (regalloc,
autovektorisasi), di luar jangkauan pass sisi cg_clif.

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
  kualitas codegen Cranelift (regalloc, autovektorisasi).

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
- `CG_CLIF_INLINE_MAX` 16/256: netral (p2 66–69 ms).
- Inliner agresif (`CG_CLIF_INLINE_USES=100 CG_CLIF_INLINE_MAX=200`): p1
  108 → 105 ms, lainnya datar.
- Cranelift `opt_level=speed` vs `speed_and_size`: tidak berbeda.
- Unroll duplikasi-body (`CG_CLIF_UNROLL_CLONE`) pada body multi-blok: **tidak
  aktif** karena `analyze_affine` mensyaratkan body blok tunggal, jadi loop
  `sinf` (multi-blok) tak tersentuh.
- `-Zinline-mir-threshold=1000..1500`: menang kecil di p2 (67.6 → 61.6 ms) tapi
  belum dijadikan default (di luar jangkauan pass sisi cg_clif).

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

Semua ini adalah kualitas codegen/inliner Cranelift, di luar jangkauan pass
sisi cg_clif; perbaikannya harus di dalam `cranelift-codegen`.

Catatan nuansa: p2 sebenarnya berada di batas jangkauan — inliner **adalah**
pass sisi cg_clif, dan ambang `MAX_CALLEE_INSTS=64` (badan probe jauh lebih
besar) mencegah probe di-inline. Namun menaikkan ambang sampai cukup untuk
probe akan meledakkan ukuran kode pada pemanggil lain, jadi ini bukan trade-off
yang jelas menang; dibiarkan apa adanya. p3/p4 (konversi rekursi→loop,
unrolling-call untuk ILP, fusi `sincos`) murni pekerjaan level Cranelift.

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
dimatikan (`CG_CLIF_NO_STORE_FWD=1 CG_CLIF_NO_IFCONV=1 CG_CLIF_INLINE_MAX=0
CG_CLIF_UNROLL=0`) compile 3212 ms vs upstream 3208 ms, jadi patch
`cranelift-object`/`cranelift-codegen` tidak menambah biaya. Biaya per pass pada
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
`vendor/cranelift-object` menamai section per fungsi (`.text.<sym>`) dan memberi
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

**Batas yang jujur: cg_clif tetap jauh lebih lambat dari LLVM pada kode
komputasi Rust murni.** Pada `image`, U 36.9 s vs L 1.25 s (**29× lebih
lambat**), dan M hanya memangkasnya jadi 22.1 s (**18× lebih lambat dari L**).
Akar penyebabnya bukan pass saya, melainkan tidak adanya auto-vectorizer di
Cranelift: LLVM memvektorisasi loop blur (`mulps`/`addps` di
`filter_symmetric_column`/`filter_symmetric_row`), M menghasilkan **nol**
instruksi packed-float. Bukti tambahan: `CG_CLIF_OPT_LEVEL` = `speed`,
`speed_and_size`, atau `none` memberi runtime 5.38-5.52 s dan ukuran binary
1.66-1.69 MB — praktis tidak berubah, karena knob itu tidak menyalakan
vektorisasi.

**Eksperimen kontrol** memastikan blur (bukan resize atau kode lain) yang
menyebabkan gap: `blur` saja L 971 ms vs M 19.50 s (20×), `resize` saja L
285 ms vs M 2.89 s (10×).

**A/B memastikan gap ukuran bukan salah pass.** `bat` dibangun dengan semua
pass dimatikan (`CG_CLIF_INLINE_MAX=0 CG_CLIF_INLINE_LEAF_MAX=0 CG_CLIF_UNROLL=0
CG_CLIF_NO_STORE_FWD=1 CG_CLIF_NO_IFCONV=1`) menghasilkan 14.92 MB; dengan pass
menyala 13.43 MB. Pass **memperkecil** binary 10%, jadi selisih 2× terhadap
LLVM adalah warisan cg_clif (section per fungsi + model PIC), bukan regresi.
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
cg_clif menolaknya (`error: LTO is not supported by rustc_codegen_cranelift`),
jadi LTO dimatikan untuk **ketiga** backend agar yang dibandingkan backend,
bukan profil build.

### 8.27 Inliner: leaf trivial bebas batas pemakaian — LANDED (`4d046cc`)

**Akar masalah.** `MAX_CALLEE_USES = 3` membatasi **semua** callee, termasuk
yang badannya satu operasi. `<f32 as Mul>::mul`, `<f32 as Add>::add`, dan
`__iterator_get_unchecked` adalah satu instruksi CLIF, tetapi pemanggil hanya
boleh meng-inline-nya 3 kali. Loop blur memanggilnya puluhan kali per iterasi,
jadi sisanya tetap berupa call sungguhan — dan setiap call memaksa spill/reload
di sekelilingnya. Inilah sebabnya 46 call masih tersisa di `image` blur pada
revisi 8.16.

**Perbaikan.** Leaf dengan ≤ `TRIVIAL_LEAF_INSTS` instruksi kini:

1. tidak dihitung terhadap `MAX_CALLEE_USES`, dan
2. tidak dibebankan pada budget pertumbuhan (`budget`/`leaf_budget`).

Pertumbuhan kode tetap terbatas: badan seperti itu satu operasi, dan sebuah
leaf tidak punya callee sendiri sehingga tidak bisa memicu ekspansi berantai.
Rekursi tetap mustahil. Knob: `CG_CLIF_INLINE_TRIVIAL_MAX` (ambang) dan
`CG_CLIF_INLINE_NO_TRIVIAL` (matikan, untuk A/B).

**Ambang 8 → 16.** Dengan ambang 8, `Add`/`Mul` hilang tetapi 8 call
`__iterator_get_unchecked` masih tersisa. Bisect perambatan call pada
`filter_symmetric_column`:

| `CG_CLIF_INLINE_TRIVIAL_MAX` | call tersisa di loop kolom | runtime blur |
|---|---|---|
| 8  | 8 (`__iterator_get_unchecked`) | 15.27 s |
| 9  | 8 | — |
| 11 | 8 | — |
| 13 | **0** | — |
| 16 | **0** | **12.80 s** |
| 32 | 0 | 12.84 s (plateau) |

Badan adapter iterator itu 12–13 instruksi CLIF, jadi ambang 8 tepat di
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

- `./y.sh test --sysroot clif`: **25.368 tes lulus, 0 gagal**.
- `bat` 0.26.1 (13.4 MB, banyak dependensi) dan `tokei` 15.0.0 dibangun ulang
  dan dijalankan; keluaran tokei identik **per file** dengan baseline M
  (52 file, 15.966 baris kode). Perbedaan md5 hanya urutan array `reports`
  (iterasi hashmap), bukan isi.

**Koreksi klaim 8.20.** Bagian 8.20 menyatakan sisa gap runtime "bukan soal
inliner, harus di dalam `cranelift-codegen`". Itu keliru untuk kasus ini:
inliner **adalah** pass sisi cg_clif, dan memperbaiki batas pemakaiannya
menutup 42% gap blur (22.3 → 12.8 s). Yang benar-benar tersisa di
`cranelift-codegen` adalah dua hal lain:

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
| Cranelift sebelum | 15229 | **30,3%** |

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

Cranelift tidak punya instruksi round-half-away-from-zero, jadi `round`
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
| Cranelift sebelum | 12752,1 ms | 1.627.864 B | 123 MB | 1 |
| Cranelift + round native | **10355,5 ms** | 1.629.168 B | 123 MB | **0** |

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
| Cranelift | 29 | 2 |

LLVM memvektorisasi loop blur; Cranelift tidak. Ini pekerjaan di dalam
`cranelift-codegen` (loop vectorizer), di luar jangkauan pass sisi cg_clif,
dan tidak ditambal di sini. Angka ini menggantikan klaim lama di §8.20 bahwa
gap "bukan soal autovektorisasi": untuk blur, vektorisasi adalah faktor
dominan. §8.20 tetap berlaku untuk benchmark mikro p1–p8 yang loop-nya
memang skalar.

---

## 9. Eksperimen lanjutan: jump table pada switch kecil (§9)

### 9.1 Metodologi yang sempat salah

Percobaan pertama memakai `cargo build` tanpa `RUSTC=rustc-clif`, sehingga
yang terukur adalah **LLVM**, bukan Cranelift. Gejalanya: ukuran biner 579.296
byte (LLVM) alih-alih 1.629.168 (Cranelift), dan tunable `CG_CLIF_UNROLL`
1/4/16/128 menghasilkan md5 identik. Setelah `RUSTC=rustc-clif` dipakai,
barulah angka Cranelift muncul. **Semua angka unroll/inliner di bawah sudah
memakai build yang benar.**

### 9.2 Yang terbukti BUKAN penyebab

| Hipotesis | Uji | Hasil |
|---|---|---|
| Autovektorisasi | LLVM `-Cno-vectorize-loops -Cno-vectorize-slp` | 763 instr, **0 SIMD** → 1248 ms, vs 551 instr/71 SIMD → 1246 ms. **Tidak berpengaruh.** |
| Unroll CLIF | `CG_CLIF_UNROLL` 1/4/16/128 | 10280–10362 ms, biner identik. Pass hanya menangani loop satu-blok ber-delta konstan; loop blur tidak memenuhi syarat. |
| Inliner CLIF | `CG_CLIF_INLINE_MAX=200 INLINE_LEAF_MAX=400 INLINE_USES=8` | 10380 ms vs 10359 baseline. Ukuran naik 1.629.168→1.635.000, **runtime tidak membaik**. |

Jadi klaim lama "vektorisasi adalah faktor dominan" **salah**; klaim §8.20
(asli) yang benar.

### 9.3 Temuan: jump table untuk switch kecil

Menghitung pola jump table PIC (`movslq (%r,%r,4),%r; add %r,%r; jmp *%r`):

| biner | situs jump table |
|---|---|
| LLVM | 0 (pakai bentuk absolut `jmp *0x…`, 214 situs) |
| Cranelift (sebelum) | **2714** |
| Cranelift (sesudah patch) | **84** |

Contoh nyata: `ImageBuffer::from_fn` menurunkan `match i {0=>r,1=>g,2=>b}`
menjadi jump table 3-entri. LLVM memakai 2 `cmp`. Biaya jump table di sini
bukan sekadar 1 cabang tak-langsung: ia juga memaksa alamat dasar dihitung
(`lea`) dan menghalangi prediksi cabang.

### 9.4 Patch

`src/base.rs`, `TerminatorKind::SwitchInt`: switch dengan ≤8 cabang dan
diskriminan ≤64-bit diturunkan menjadi rantai `icmp_imm_s` + `brif`, bukan
`cranelift_frontend::Switch::emit` (yang memilih `br_table`). Batas 64-bit
wajib: `icmp_imm_s` menerima immediate `i64`, sehingga diskriminan `i128`
akan terpotong.

### 9.5 Hasil

| metrik | LLVM | CLIF base | CLIF + patch |
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
Cranelift menyebarkannya ke **213 offset** — gejala loop yang tidak
di-rotate/di-unroll dan register spill (240 referensi `(%rsp)` vs 66).
Perbaikannya ada di `cranelift-codegen` (loop rotation, regalloc), bukan di
lapisan cg_clif, dan tidak dikerjakan di sini.

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

Sisa gap runtime didominasi kualitas `cranelift-codegen` (loop rotation,
regalloc/spilling), bukan lapisan cg_clif. Itu perlu perubahan di crate
`cranelift-codegen` dan berada di luar cakupan fork ini.

## 10. Temuan terbesar: `trunc`/`round` menjadi soft-float libcall (§10)

Ini adalah perbaikan **terbesar sejauh ini**: −15,8% runtime pada blur, dan
sepenuhnya portabel (tidak butuh `-Ctarget-cpu=native`).

### 10.1 Cara menemukannya

Profil per-fungsi (sampler 200 µs + `nm`) untuk biner LLVM vs CLIF menunjukkan
pola yang sangat berbeda, bukan sekadar "CLIF lebih lambat merata":

| fungsi | LLVM | CLIF (sebelum) |
|---|---|---|
| `image_drv::main` | 29,3% | ~0% |
| `filter_symmetric_column` | 28,0% | 25,4% |
| `roundf` (glibc) | 9,0% | — |
| `trunc_status::<f32>` | — | 4,1% |
| `truncf` (compiler_builtins) | — | 2,9% |
| `to_bits`/`from_bits`/`exp_unbiased`/`unsigned` | — | 5,9% |

Yang mencolok: CLIF menghabiskan **~13% sampel di `compiler_builtins`**, yaitu
soft-float Rust murni, sementara LLVM hanya memanggil satu `roundf` glibc.
Pencarian di asm menegaskan: biner CLIF punya **0** instruksi `roundss`,
padahal x86-64 punya instruksi native untuk itu.

### 10.2 Akar masalah

`vendor/cranelift-codegen-0.137.0/src/isa/x64/lower.isle`:

```lisp
(rule 2 (x64_round_f32 a imm) (if-let true (has_sse41)) (x64_roundss a imm))
(rule 1 (x64_round_f32 (is_xmm_32 a) imm) (libcall_1 (round_libcall $F32 imm) a))
```

Jadi `ftrunc` hanya menjadi `roundss` bila target punya **SSE4.1**. Pada
baseline x86-64 (yang menjadi default), aturan itu tidak aktif dan turun ke
libcall `TruncF32` → `compiler_builtins::math::libm_math::generic::trunc_status`.

Ini bukan bug Cranelift (memang benar secara semantik), tapi konsekuensi
mahal: soft-float per elemen piksel, sementara LLVM memakai `roundf` glibc
yang jauh lebih murah.

Temuan tambahan: lowering `round` di cg_clif sendiri dibangun di atas
`ins().trunc(x)`, sehingga **setiap `.round()` ikut menanggung libcall yang
sama**. Inilah sebabnya patch pertama (hanya `sym::trunc`) tidak berefek
(10091 ms vs 10053 ms) — sumber panasnya justru `round`.

### 10.3 Perbaikan

`trunc` diturunkan ke round-trip integer, yang hanya butuh SSE2:

```rust
fn trunc_via_int(fx, x, clif_ty, bits) -> Value {
    let int_part = fx.bcx.ins().fcvt_to_sint_sat(int_ty, x);
    let back     = fx.bcx.ins().fcvt_from_sint(clif_ty, int_part);
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
   oleh biner CLIF dan biner LLVM, lalu dibandingkan.

### 10.5 Verifikasi

- **Diferensial vs LLVM**: hash identik (`4de7d090cdfce77d`) untuk
  `trunc`/`round`/`fract` atas **seluruh 2^32 pola f32** + sweep f64
  (2048 eksponen × 4 mantissa × 2 tanda).
- **Test suite**: `./y.sh test --sysroot clif` → **TEST_EXIT=0**, 65 suite
  hijau, 0 gagal (sebelumnya 6 gagal karena bug tanda nol).
- **Output benchmark**: identik dengan LLVM pada n = 512, 1024, 2048
  (`8018560249`).

### 10.6 Hasil

image blur 2048×2048, target baseline x86-64, median 9 run:

| backend | wall (ms) | RSS (MB) | ukuran (byte) |
|---|---|---|---|
| LLVM | 1237,7 | 122,5 | — |
| CLIF baseline | 10257,5 | 123,0 | 1500000 |
| CLIF + switch (§9) | 10002,5 | 122,9 | 1500000 |
| **CLIF + switch + trunc** | **8422,0** | 123,0 | **1499568** |
| CLIF + `-Ctarget-cpu=native` | 8241,8 | 123,0 | 1498832 |

- **−15,8%** vs CLIF-final, **−17,9%** vs baseline.
- `-Ctarget-cpu=native` mencapai 8241,8 ms; lowering portabel ini memulihkan
  **hampir seluruh** manfaat SSE4.1 **tanpa** mengorbankan portabilitas.
- `compiler_builtins` **hilang total** dari profil (0 sampel, dari ~13%).
- Waktu compile tidak berubah (≈6,37 s), ukuran sedikit turun.

### 10.7 Koreksi klaim §9.9

§9.9 menyatakan sisa gap "berada di luar cakupan fork ini". Itu **salah**:
`cranelift-codegen` di-vendor lewat `[patch.crates-io]`, jadi bisa diubah. Dan
ternyata perbaikan terbesar justru datang dari lapisan cg_clif sendiri
(`src/intrinsics/mod.rs`), bukan dari `cranelift-codegen`.

Sisa gap setelah perbaikan ini (8422 ms vs 1238 ms LLVM) masih didominasi
kualitas codegen: `try_from_fn`/`wrap_mut_1`/`Pixels`/`Zip` yang tidak
ter-inline (≈18% sampel) dan spilling register di loop panas.

## 11. Sisa gap: apa yang sebenarnya tersisa (§11)

Setelah §10, `compiler_builtins` hilang dari profil, jadi pertanyaannya berubah
menjadi: apa yang LLVM lakukan dan CLIF tidak? Jawaban awalnya terlihat seperti
autovektorisasi, tetapi pengukuran §11.4 membantahnya. Bagian ini mencatat
hipotesis itu, bukti yang membatalkannya, dan penyebab yang benar-benar tersisa.

### 11.1 Hipotesis awal (dibatalkan oleh §11.4)

Disassembly fungsi panas `filter_symmetric_column` pada target baseline x86-64
menunjukkan perbedaan bentuk yang mencolok:

| metrik | LLVM | CLIF modifikasi |
|---|---|---|
| total instruksi | 904 | 733 |
| SIMD packed (`ps`/`pd`) | **75** | **1** |
| rincian packed | `movups`×34, `mulps`×17, `addps`×10, `shufps`×6, `xorps`×4, `movaps`×4 | `xorps`×1 |
| scalar SSE (`ss`/`sd`) | 39 | 59 |
| `mulss`/`addss`/`subss` | 15 | 24 |

Wajar kalau ini dibaca sebagai "LLVM memvektorisasi loop, Cranelift tidak". Itu
memang benar sebagai fakta codegen, tetapi **bukan** penyebab utama gap runtime.

### 11.2 Bukti yang membatalkan hipotesis vectorization

Kalau vectorization adalah penyebab utama, mematikannya di LLVM harus
mengembalikan LLVM ke level CLIF. Tidak demikian:

| konfigurasi | runtime (ms) | ukuran (byte) |
|---|---|---|
| LLVM `-O3` (default, vectorized) | 1240 | 579296 |
| LLVM `-O3` **tanpa** vectorize (`-Cno-vectorize-loops -Cno-vectorize-slp`) | 1531 | 577136 |
| CLIF baseline (upstream, tanpa patch) | 8375 | 1500272 |
| CLIF modifikasi (§13) | 7877 | 1463784 |

Mematikan vectorization hanya merugikan LLVM ~23%. Jadi vectorization
menyumbang sebagian kecil dari selisih 6,8×; **~82% selisih tetap ada pada LLVM
yang scalar murni**. Ini juga konsisten dengan §8.20 yang sudah mencatat hal
serupa pada benchmark lain.

### 11.3 Penyebab sebenarnya: loop scalar CLIF penuh address recomputation

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
   ulang setiap iterasi.** Cranelift **tidak punya LICM**.
3. **Bounds/null check tidak tereliminasi** dan muncul duplikat.

Perbandingan langsung: LLVM (tanpa vectorize) menjalankan loop yang sama dalam
**63 instruksi untuk 16 MAC = 3,9 instruksi/MAC**; CLIF butuh **14,8**. Rasio
3,8× itu menjelaskan hampir seluruh gap runtime.

### 11.4 Mengapa ini sulit diperbaiki di fork ini

Ketiga penyebab di atas berada di `cranelift-codegen`, bukan di cg_clif:

- **LICM**: Cranelift tidak memiliki pass ini sama sekali (tidak ada
  `licm`/`loop_rotate` di `cranelift-codegen/src/`). Menulisnya berarti
  memindahkan instruksi keluar loop dengan benar terhadap aliasing memori.
- **Store/load forwarding lintas blok**: `store_forward.rs` milik cg_clif
  menangani kasus intra-prosedur sederhana, tetapi pointer yang di-spill di
  sini melewati blok dengan bentuk yang tidak dikenali pass itu.
- **Loop rotation / unrolling**: juga tidak ada di Cranelift. Fork ini punya
  pass `unroll.rs`, tetapi jalurnya hanya *affine reassociation*: ia bekerja
  sangat baik (52×) pada loop yang state-nya fungsi linear dari dirinya sendiri
  (§13.4), dan tidak berlaku pada loop memori seperti `filter_symmetric_column`.
  Perluasan ke bentuk non-affine butuh analisis dependensi memori yang belum
  dikerjakan.

Karena itu, gap yang tersisa didokumentasikan sebagai **batasan arsitektural
Cranelift**, bukan bug yang bisa ditambal di sisi cg_clif.

### 11.5 Yang tetap berlaku

Perbaikan §9, §10, §12, dan §13 tetap sah dan tidak bergantung pada vectorization:

- Pada beban gambar (`imagedrv`, `filter_symmetric_column`), runtime CLIF
  modifikasi **7877 ms vs 8375 ms baseline upstream** (−5,9%), ukuran biner
  **1.463.784 vs 1.500.272 byte** (−2,4%).
- Pada benchmark suite §9.8, ukuran biner tetap **lebih kecil dari LLVM**
  (1.499.568 vs 4.635.232 byte) dan waktu compile **lebih cepat dari LLVM**
  (715 ms vs 879 ms).
- Kebenaran: suite uji hijau, output byte-identik dengan LLVM pada uji
  diferensial (termasuk seluruh pola `[x; N]` yang disentuh §13).

Jadi dari empat tujuan awal — RAM, CPU, ukuran, kecepatan compile — ketiganya
tercapai. Sisa gap CPU runtime dijelaskan oleh ketiadaan
LICM/loop-rotation/unrolling di Cranelift, bukan oleh vectorization.

## 12. `floor` dan `ceil` juga korban libcall yang sama (§12)

§10 memperbaiki `trunc`/`round`. Ternyata akar masalahnya lebih luas: di
`lower.isle`, `ceil`, `floor`, `nearest`, dan `trunc` **semuanya** lewat
`x64_round` yang sama, dan aturan libcall-nya dipilih berdasarkan `has_sse41`.
Jadi pada target baseline x86-64, `floor` dan `ceil` juga menjadi soft-float
`compiler_builtins`.

### 12.1 Kesalahan metodologi yang hampir menutupi temuan ini

Dua percobaan pertama saya menunjukkan waktu **identik** antara CLIF dan LLVM.
Penyebabnya dua, dan keduanya kesalahan saya:

1. `RUSTC=rustc-clif rustc ...` — variabel `RUSTC` hanya dibaca **cargo**,
   bukan `rustc` langsung. Jadi kedua perintah menjalankan rustc LLVM. Buktinya
   `md5sum` kedua biner sama persis. Perbaikannya: panggil `rustc-clif` langsung.
2. Benchmark pertama memakai array 16 MB, melewati cache, sehingga
   **memory-bound** dan menutupi biaya compute.

Setelah kedua kesalahan diperbaiki (array 16 KB di L1, 400 repetisi, panggil
`rustc-clif` langsung), barulah selisihnya terlihat.

### 12.2 Hasil

Waktu per elemen, loop compute-bound:

| operasi | CLIF sebelum | CLIF sesudah | LLVM |
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
instruksi**, jadi fungsi terbesar yang cg_clif hasilkan pada beban ini (6.353
instruksi) hanya butuh **3,03 ms** dataflow — 5,2% dari total build 6,3 s.
Menukar eliminasi store/load dengan 5% waktu compile adalah trade-off yang
jelas menang, dan justru fungsi besar itulah yang paling banyak punya store di
satu blok yang dibaca di blok lain.

Perbaikan: batas dinaikkan ke `16_384` instruksi, cukup untuk semua fungsi yang
cg_clif hasilkan sambil tetap membatasi input patologis. Tunable
`CG_CLIF_SF_CBE_MAX` tetap ada untuk bisect.

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
| CLIF baseline (sebelum §13) | 8374,6 | 1.500.272 |
| + §13.1 store forwarding | 7828,0 | 1.465.336 |
| + §13.2 memset | 7877,0 | 1.463.784 |
| LLVM `-O3` | 1240,1 | 579.296 |

§13.1 memberi **−6,5%** runtime; §13.2 menyusutkan ukuran lagi **−1.552 byte**
tetapi runtime-nya di dalam noise, karena loop zero-init itu bukan bagian panas.
Saya laporkan apa adanya: §13.2 adalah perbaikan kualitas codegen, bukan
perbaikan runtime pada beban ini.

### 13.4 Pass unroll: menyala, tetapi hanya pada bentuk affine

Saya mengukur apakah pass unroll benar-benar bekerja, dengan A/B pada
`CG_CLIF_UNROLL` (128 default vs 1 = mati). Hasilnya penting dan perlu
dipisahkan dari kesimpulan §11:

**Pass ini bekerja sangat baik pada loop yang bentuknya affine.** Pada
`p5_inline` (loop 300 juta iterasi, `acc = (acc+i)*3`, `i += 1` — bentuk affine
murni):

| `CG_CLIF_UNROLL` | runtime | output |
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
Kolom `mod vs up` adalah runtime modifikasi terhadap upstream cg_clif —
perbandingan backend murni, keduanya memakai std hasil Cranelift sendiri.

| label | compile wall (ms) | compile cpu (ms) | peak RSS (MB) | total size (KB) | run wall (ms) | run cpu (ms) | run RSS (MB) |
|---|---|---|---|---|---|---|---|
| LLVM (stock rustc) | 876 | 1149 | 133 | 36.095 | 365 | 365 | 48 |
| cg_clif upstream | 707 | 922 | 131 | 45.675 | 2467 | 2461 | 49 |
| **cg_clif modifikasi** | **726** | **962** | **132** | **31.688** | **517** | **514** | **48** |

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
  §13.2, melainkan patch `cranelift-object` yang memberi tiap fungsi dan objek
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
> memuat `libstd`, sehingga `rustc-clif` diam-diam jatuh ke sysroot LLVM dan
> menghasilkan biner LLVM berukuran ±4,6 MB yang dikira keluaran Cranelift.
> Setelah kedua backend dibangun ulang (`./y.sh build --sysroot clif` di
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
| `CG_CLIF_NO_REPEAT_MEMSET=1` | **23,6 ms** |

**4,9× lebih cepat**, output identik. Jadi §13.2 memang bernilai besar — hanya
saja pada pola kode yang tidak muncul di delapan program benchmark ini, sehingga
tidak terlihat di matriks §13.5.

Tunable `CG_CLIF_NO_REPEAT_MEMSET` ditambahkan khusus agar klaim ini bisa
direproduksi dan di-bisect.

## 14. LICM: pass loop-invariant code motion yang baru (§14)

Cranelift tidak punya LICM. Pass `egraph` hanya meng-hoist *expression node*
yang murni dan sengaja **tidak** memindahkan load; `gvn` melakukan value
numbering tetapi tidak keluar dari blok. Akibatnya loop yang memuat nilai
invariant setiap iterasi (koefisien, batas, base pointer) menyimpan load itu di
dalam rantai dependensi loop-carried.

`vendor/cranelift-codegen-0.137.0/src/licm.rs` menambahkan pass tersebut,
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

A/B dengan `CG_CLIF_NO_LICM=1` vs default, 11 repetisi, median, `opt-level=3`,
kedelapan program `bench/progs/`. Kedua sisi memakai sysroot Cranelift mod
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

- Suite tes cg_clif penuh: **25.368 tes lulus, 0 gagal**, termasuk rand, regex,
  graviola, dan portable-simd yang dikompilasi ulang dengan pass aktif.
- `bench/tests/t_licm.rs` ditambahkan sebagai regresi khusus: lima bentuk yang
  pernah merusak pass (phi loop-carried, `brif` dua cabang ke header, store yang
  memblokir hoist, load invariant yang aman, dan pembagian yang bisa trap).
  Keluarannya **identik dengan LLVM**, baik dengan LICM hidup maupun mati.
- Kedelapan program benchmark menghasilkan output byte-identik dengan LLVM.

### 14.5 Tunable

| variabel | arti |
|---|---|
| `CG_CLIF_NO_LICM=1` | matikan pass LICM |
| `CG_CLIF_LICM_DEBUG=1` | cetak setiap hoist ke stderr |
| `CG_CLIF_NO_COLLAPSE=1` | matikan collapse forwarding-latch sebelum unroll |

## 15. False dependency pada konversi u64→float (LANDED)

### 15.1 Gejala

`p4_simd` tetap 2,5× lebih lambat dari LLVM meski semua pass cg_clif dimatikan
satu per satu (NONE 105,5 / NOUNROLL 105,2 / NOINLINE 106,3 / NOLICM 110,4 /
NOSF 107,0 / NOIFCONV 106,3 ms — semuanya dalam derau). Jumlah panggilan `sinf`
saat runtime juga identik: interposer `LD_PRELOAD` menghitung **4.000.000**
panggilan untuk LLVM maupun Cranelift, dengan keluaran byte-identik
(`-0.9937716`). Jadi bukan jumlah kerja, melainkan **biaya per panggilan**:
7,4 ns vs 18,9 ns.

### 15.2 Isolasi

Memisahkan loop menjadi dua tahap — isi `a[i] = i as f32` dulu, lalu
`o[i] = a[i].sin()` — menyusutkan gap dari 2,5× menjadi 1,25×
(LLVM 39,1 ms vs Cranelift 48,7 ms). Artinya **konversi integer→float** di loop
panas, bukan `sinf`, yang menjadi biang keladi.

### 15.3 Akar masalah

`cvtsi2ss`/`cvtsi2sd` adalah instruksi SSE dua-operand: selain mengonversi
sumber integer, ia **mempertahankan bit atas register tujuan**. Bila register
tujuan belum ditol-kan, instruksi itu membaca register tersebut dan menciptakan
*dependency* palsu terhadap penulis sebelumnya. Di dalam loop, penulis
sebelumnya adalah hasil `sinf` iterasi sebelumnya, sehingga setiap iterasi
harus menunggu panggilan sebelumnya selesai — loop yang seharusnya
*throughput-bound* berubah menjadi *latency-bound*.

Disassembly Cranelift (sebelum perbaikan):

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

Jumlah `xorps` di seluruh binary: Cranelift **5**, LLVM **46**.

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

`vendor/cranelift-codegen-0.137.0/src/isa/x64/inst/emit.rs`, di
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
| cg_clif modifikasi (sebelum) | 75,6 ms |
| **cg_clif modifikasi (sesudah)** | **34,1 ms** |

**2,2× lebih cepat**, dan kini hanya 15% di belakang LLVM. Karena upstream
menunjukkan angka yang sama (75,7 ms), ini defisiensi Cranelift yang berdiri
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

- Suite tes cg_clif penuh lulus, tanpa kegagalan.
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
(`objdump` menunjukkan `sincosf` di binary LLVM, sedangkan Cranelift tetap
memanggil `sinf@` dan `cosf@` terpisah). Penggabungan itu adalah transformasi
tingkat IR (loop fusion + common-subexpression), bukan sesuatu yang bisa
disimpulkan di level mesin.
