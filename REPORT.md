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
  **1.3-2.5× lebih cepat dari LLVM**.
- Runtime: **1.25× (`bat`), 1.48× (`tokei`), 1.66× (`image`)** lebih cepat dari U.

**Batas yang jujur: cg_clif tetap jauh lebih lambat dari LLVM pada kode
komputasi Rust murni.** Pada `image`, U 36.9 s vs L 1.25 s (**30× lebih
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
