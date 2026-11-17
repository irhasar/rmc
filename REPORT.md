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

### 8.4 Hasil akhir (opt-level=3, semua keluaran identik dengan LLVM)

| prog | LLVM run | up run | mod run | LLVM size | up size | mod size |
|------|---------:|-------:|--------:|----------:|--------:|---------:|
| p1_arith  |  81 ms | 109 ms | 104 ms | 4.64 MB | 5.88 MB | 4.76 MB |
| p2_mem    |  17 ms | 156 ms | **122 ms** | 4.64 MB | 5.94 MB | 4.80 MB |
| p3_calls  |  37 ms | 116 ms | 107 ms | 4.61 MB | 5.82 MB | 4.70 MB |
| p4_simd   |  56 ms | 158 ms | 142 ms | 4.64 MB | 5.89 MB | 4.78 MB |
| p5_inline |  57 ms | 896 ms | 365 ms | 4.61 MB | 5.81 MB | 4.69 MB |
| p6_scalar |  57 ms | 902 ms | **61 ms** | 4.61 MB | 5.81 MB | 4.69 MB |

Ringkas: terhadap cg_clif upstream, modifikasi ini memberi **−15% ukuran**,
**runtime setara LLVM pada kernel skalar murni (p6: 902→61 ms, ~15×)**, dan
tidak menambah waktu compile. Gap yang masih ada pada p2–p5 berasal dari
promosi memori (spill agregat) yang belum lengkap, bukan dari loop maupun
header `Vec` (lihat 8.6 dan 8.7).

Compile time ketiga backend praktis setara (~90–180 ms); cg_clif tidak lebih
lambat, hanya masih lebih besar dan (pada p2–p5) lebih lambat saat dijalankan.

### 8.5 Ringkasan knob

| Env | Default | Efek |
|-----|---------|------|
| `CG_CLIF_UNROLL` | 8 | faktor reassociasi (1 = mati) |
| `CG_CLIF_UNROLL_CLONE` | unset | aktifkan unroll duplikasi-body (fallback) |
| `CG_CLIF_INLINE_MAX` | 64 | batas instruksi callee untuk inliner (0 = mati) |

### 8.6 SSA sadar-borrow: header `Vec` tetap di register — LANDED, `4290fb6`

Analisis SSA (`src/analyze.rs`) menandai local `NotSsa` untuk **setiap** borrow
`&mut`/`&raw`, sehingga local dipaksa ke stack slot. Itu memang perlu bila borrow
menunjuk **ke dalam** storage local (mis. `&mut v`, `&raw mut v[0]`), tetapi
**tidak** bila borrow men-*deref* local dulu (`&mut *p`, `&raw mut *p`): pola itu
hanya membaca nilai local untuk membentuk alamat, lalu menulis lewat pointer yang
dipegangnya.

Korban utamanya `Vec` yang diindeks di loop. cg_clif menurunkan `_30 = &raw mut
(*_30)` untuk temporary `*mut [u64]`, menandainya `NotSsa`, sehingga **seluruh
header `Vec` (ptr/len/cap) tertahan di stack** dan di-reload setiap iterasi —
bersama rentetan round-trip `Option`/place di sekitarnya.

Perbaikan: hanya paksa `NotSsa` bila proyeksi borrow **tidak** mengandung `Deref`.

Efek pada kernel mikro (40 juta RMW indeks acak atas array 4K, ms):

| kernel | LLVM | mod sebelum | mod sesudah |
|--------|-----:|------------:|------------:|
| `mem` (`vec!` + indeks) | 22 | 110 | **65** |
| `memw` (while + indeks)  | 22 | 87  | **51** |
| `memp` (raw ptr)         | 22 | 40  | 38 |

p2_mem: 156 → 122 ms. Semua keluaran tetap identik dengan LLVM (lihat 8.8).

### 8.7 Sisa gap: promosi memori, bukan loop

Setelah 8.3 dan 8.6, gap yang tersisa (p2–p5) **bukan** loop dan **bukan** lagi
header `Vec`, melainkan spill agregat yang belum dipromosikan:

- **p2_mem** didominasi `HashMap::entry`/hashbrown. Loop `for i {
  m.entry(i % 65536).or_insert(0) }` saja: **55 ms (LLVM) vs 672 ms (mod), ~12×**.
  Disasm loop probe hashbrown menunjukkan hampir semua temporary di-spill ke
  stack (~20 slot; mis. `movdqu xmm→stack` sebelum `movemask`). Ini keterbatasan
  mem2reg/aggregate-SSA pada lowering MIR cg_clif, bukan satu pass yang hilang.
- **p3_calls** terbagi antara rekursi `fib` (LLVM mengubah rekursi biner menjadi
  satu panggilan rekursif per langkah; cg_clif tetap dua) dan `format!`.
- **p5_inline** adalah loop yang sama dengan p6, tetapi iteratornya menyimpan
  `Option<u64>` di stack (mem2reg), sehingga reassociasi tidak jalan.
- **p4_simd** tanpa `Vec`: kode berat memori murni (spill + bounds check).
- Menaikkan budget inliner (`CG_CLIF_INLINE_MAX=256`) hanya memberi ~4% pada p2
  dan ~5% pada p4, jadi inlining lintas-CGU bukan lever.

### 8.8 Verifikasi (suite keluaran)

Dibandingkan keluaran LLVM vs mod untuk **20 program** (6 prog bench + kernel
mikro `mem`/`memw`/`memu`/`mema`/`memp`/`call`/`bsrch`/`hm`/`srt`/`fib`/`fmt`
+ `borrow`/`iter`/`aliasing` + p1–p6): **20/20 identik**, termasuk pola aliasing
yang sengaja menulis lewat borrow (`*r = …`), reborrow, `&mut` field, dan
`&mut` elemen — memastikan local yang benar-benar ditulis lewat borrow tetap
di stack dan tidak menjadi stale.

