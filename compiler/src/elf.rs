//! Emit a static ELF64 executable directly from the compiler's assembly text.
//!
//! The normal path writes the assembly to a file and shells out to GNU `as` and
//! `ld`. Those two processes are ~80% of rmc's compile time, and neither is
//! needed: the code generator emits a small, fully-determined instruction
//! subset, so this module assembles and links it in-process and hands back the
//! bytes of an executable. No `as`, no `ld`, no libc.
//!
//! Only the forms the code generator actually emits are supported. Anything
//! else is a hard error naming the line, so a new codegen form fails loudly here
//! rather than silently mis-assembling.

use std::collections::HashMap;

/// Base virtual address. The ELF is not PIE, so this is fixed.
const BASE: u64 = 0x400000;
const PAGE: u64 = 0x1000;

/// A symbol defined at a label, in `.text` or in a data section.
struct Sym {
    sec: usize,
    off: u64,
    /// True for a `.set` constant, whose `off` is its value rather than an
    /// offset into `sec`.
    abs: bool,
}

/// A reference resolved once every section has an address.
struct Fixup {
    sec: usize,
    /// Offset of the 4-byte PC-relative field to patch.
    disp_off: usize,
    /// Offset of the end of the instruction the field belongs to.
    next_off: usize,
    sym: String,
}

struct Section {
    name: &'static str,
    /// ELF `PF_` flags: 0x4 read, 0x2 write, 0x1 execute.
    flags: u64,
    align: u64,
    data: Vec<u8>,
    vaddr: u64,
    file_off: u64,
    /// A section that is entirely zero is carried as `.bss`: it takes memory
    /// but no file space, so a writable segment costs no page of padding.
    bss: bool,
}

pub struct Elf {
    sections: Vec<Section>,
    syms: HashMap<String, Sym>,
    fixups: Vec<Fixup>,
    /// 8-byte absolute symbol pointers in a data section (`.quad sym`).
    abs64: Vec<(usize, usize, String)>,
    /// 4-byte absolute symbol immediates (`mov $sym, %reg`).
    abs_imm: Vec<(usize, usize, String)>,
    text_idx: usize,
}

/// Assemble `asm` (the text `compile_to_asm` returns) into a static executable.
pub fn assemble(asm: &str) -> Result<Vec<u8>, String> {
    let mut e = Elf {
        sections: vec![Section {
            name: ".text",
            flags: 0x5,
            align: 16,
            data: Vec::new(),
            vaddr: 0,
            file_off: 0,
            bss: false,
        }],
        syms: HashMap::new(),
        fixups: Vec::new(),
        abs64: Vec::new(),
        abs_imm: Vec::new(),
        text_idx: 0,
    };
    e.parse(asm)?;
    e.layout()?;
    Ok(e.emit())
}

impl Elf {
    fn section(&mut self, name: &'static str, flags: u64, align: u64) -> usize {
        if let Some(i) = self.sections.iter().position(|s| s.name == name) {
            if self.sections[i].align < align {
                self.sections[i].align = align;
            }
            return i;
        }
        self.sections.push(Section {
            name,
            flags,
            align,
            data: Vec::new(),
            vaddr: 0,
            file_off: 0,
            bss: false,
        });
        self.sections.len() - 1
    }

    fn parse(&mut self, asm: &str) -> Result<(), String> {
        let mut cur = self.text_idx;
        let mut sets: Vec<(String, String, String)> = Vec::new();

        for (lineno, raw) in asm.lines().enumerate() {
            let mut line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            // A label may share its line with a directive (`__rmc_argc: .quad
            // 0`), so peel it off first.
            if let Some(i) = line.find(':') {
                let (name, rest) = line.split_at(i);
                let name = name.trim();
                if !name.is_empty() && !name.contains(char::is_whitespace) {
                    let off = self.sections[cur].data.len() as u64;
                    self.syms.insert(name.to_string(), Sym { sec: cur, off, abs: false });
                    line = rest[1..].trim();
                    if line.is_empty() {
                        continue;
                    }
                }
            }
            if let Some(rest) = line.strip_prefix('.') {
                self.directive(rest, &mut cur, &mut sets)
                    .map_err(|e| format!("{e} (line {})", lineno + 1))?;
                continue;
            }
            self.instruction(line, cur).map_err(|e| format!("{e} (line {})", lineno + 1))?;
        }

        for (name, a, b) in sets {
            let (va, sa) = self.sym_value(&a, cur)?;
            let (vb, _) = self.sym_value(&b, cur)?;
            self.syms.insert(name, Sym { sec: sa, off: (va - vb) as u64, abs: true });
        }
        Ok(())
    }

    fn directive(
        &mut self,
        rest: &str,
        cur: &mut usize,
        sets: &mut Vec<(String, String, String)>,
    ) -> Result<(), String> {
        let mut it = rest.splitn(2, char::is_whitespace);
        let name = it.next().unwrap_or("");
        let arg = it.next().unwrap_or("").trim();
        match name {
            "text" => *cur = self.text_idx,
            "data" => *cur = self.section(".data", 0x6, 8),
            "rodata" => *cur = self.section(".rodata", 0x4, 8),
            "section" => {
                let sec = arg.split(',').next().unwrap_or("").trim();
                match sec {
                    ".rodata" => *cur = self.section(".rodata", 0x4, 8),
                    ".data" => *cur = self.section(".data", 0x6, 8),
                    ".text" => *cur = self.text_idx,
                    // A static executable needs no PT_GNU_STACK segment to be
                    // non-executable; the note carries nothing else.
                    ".note.GNU-stack" => {}
                    other => return Err(format!("elf: unsupported section `{other}`")),
                }
            }
            // `.balign`/`.align` take a byte count; `.p2align` takes a power of
            // two, matching GNU `as`. Conflating them under-aligned every loop
            // body the generator pads with `.p2align 4`: the in-process binary
            // was 4-byte aligned where `as`+`ld` gave 16, which cost 14% on a
            // divide-bound loop (its fetch window straddled a cache line).
            "align" | "balign" => {
                let n: u64 = arg.parse().map_err(|_| format!("elf: bad .align `{arg}`"))?;
                if n > 1 {
                    self.pad_to_align(*cur, n);
                }
            }
            "p2align" => {
                let n: u32 = arg.parse().map_err(|_| format!("elf: bad .p2align `{arg}`"))?;
                if n > 0 {
                    self.pad_to_align(*cur, 1u64 << n);
                }
            }
            "globl" | "type" | "size" => {}
            "quad" => {
                if let Ok(v) = arg.parse::<i64>() {
                    self.sections[*cur].data.extend_from_slice(&(v as u64).to_le_bytes());
                } else {
                    let off = self.sections[*cur].data.len();
                    self.abs64.push((*cur, off, arg.to_string()));
                    self.sections[*cur].data.extend_from_slice(&[0; 8]);
                }
            }
            "byte" => {
                for part in arg.split(',') {
                    let v: u8 =
                        part.trim().parse().map_err(|_| format!("elf: bad .byte `{part}`"))?;
                    self.sections[*cur].data.push(v);
                }
            }
            "ascii" | "string" => {
                let s = parse_string_literal(arg)?;
                self.sections[*cur].data.extend_from_slice(s.as_bytes());
            }
            "set" => {
                let (n, expr) = arg.split_once(',').ok_or("elf: bad .set")?;
                let (a, b) = expr.split_once('-').ok_or("elf: bad .set expression")?;
                sets.push((n.trim().to_string(), a.trim().to_string(), b.trim().to_string()));
            }
            other => return Err(format!("elf: unsupported directive `.{other}`")),
        }
        Ok(())
    }

    fn pad_to_align(&mut self, sec: usize, n: u64) {
        let len = self.sections[sec].data.len() as u64;
        let pad = (n - (len % n)) % n;
        // A zero byte is `add %al,(%rax)` in an executable section, which would
        // fault if control ever reached it; pad with `nop` there. Other sections
        // (`.rodata`, `.data`) want real zeros.
        let fill = if self.sections[sec].flags & 0x1 != 0 { 0x90 } else { 0 };
        self.sections[sec].data.resize(len as usize + pad as usize, fill);
    }

    fn sym_value(&self, name: &str, cur: usize) -> Result<(i64, usize), String> {
        if name == "." {
            return Ok((self.sections[cur].data.len() as i64, cur));
        }
        let s = self.syms.get(name).ok_or_else(|| format!("elf: unknown symbol `{name}`"))?;
        Ok((s.off as i64, s.sec))
    }

    /// Assign every section a file offset and a virtual address, then resolve
    /// fixups. `vaddr = BASE + file_off`, so the kernel's `p_vaddr % p_align ==
    /// p_offset % p_align` requirement holds automatically.
    fn layout(&mut self) -> Result<(), String> {
        let nphdr = 2usize; // RX and RW
        let mut off = (64 + nphdr * 56) as u64;
        for s in self.sections.iter_mut() {
            // An all-zero *writable* section is carried as `.bss`: memory only,
            // no file bytes and no page of padding. This matters because the
            // runtime's writable globals are all zero, and page-aligning them in
            // the file would otherwise add ~4 KB to every artifact. Only a
            // writable section may be bss: a read-only `.rodata` that happens to
            // be all zeros (a lone empty-string record is `.quad 0`) is still
            // file-backed and part of the RX segment, and demoting it would
            // leave `rx_end` at its unassigned offset 0, dropping the code from
            // every segment.
            s.bss = !s.data.is_empty()
                && s.data.iter().all(|&b| b == 0)
                && s.flags & 0x2 != 0;
            if s.bss {
                continue;
            }
            // A writable section must begin on its own page: the kernel maps
            // each PT_LOAD rounded down to a page, so a RW segment sharing a
            // page with the RX one would replace the execute permission.
            let a = if s.flags & 0x2 != 0 { PAGE.max(s.align) } else { s.align };
            off = align_up(off, a.max(1));
            s.file_off = off;
            s.vaddr = BASE + off;
            off += s.data.len() as u64;
        }
        // Place each bss section on the page after the file-backed image.
        for i in 0..self.sections.len() {
            if self.sections[i].bss {
                let v = align_up(BASE + off, PAGE);
                self.sections[i].vaddr = v;
                off = (v - BASE) + self.sections[i].data.len() as u64;
            }
        }

        for f in &self.fixups {
            let target = self
                .syms
                .get(&f.sym)
                .ok_or_else(|| format!("elf: undefined symbol `{}`", f.sym))?;
            let tva = self.sections[target.sec].vaddr + target.off;
            let next = self.sections[f.sec].vaddr + f.next_off as u64;
            let disp = (tva as i64) - (next as i64);
            let b = (disp as i32).to_le_bytes();
            let d = &mut self.sections[f.sec].data;
            d[f.disp_off..f.disp_off + 4].copy_from_slice(&b);
        }

        for (sec, o, sym) in self.abs64.clone() {
            let target = self
                .syms
                .get(&sym)
                .ok_or_else(|| format!("elf: undefined symbol `{sym}`"))?;
            let va = if target.abs {
                target.off
            } else {
                self.sections[target.sec].vaddr + target.off
            };
            self.sections[sec].data[o..o + 8].copy_from_slice(&va.to_le_bytes());
        }
        for (sec, o, sym) in self.abs_imm.clone() {
            let target = self
                .syms
                .get(&sym)
                .ok_or_else(|| format!("elf: undefined symbol `{sym}`"))?;
            let v = if target.abs {
                target.off as i64
            } else {
                (self.sections[target.sec].vaddr + target.off) as i64
            };
            self.sections[sec].data[o..o + 4].copy_from_slice(&(v as i32).to_le_bytes());
        }
        Ok(())
    }

    fn emit(&self) -> Vec<u8> {
        let text = &self.sections[self.text_idx];
        let rodata = self.sections.iter().find(|s| s.name == ".rodata");
        let data = self.sections.iter().find(|s| s.name == ".data");

        // The RX segment covers the headers, .text and .rodata; the RW segment
        // is .data alone. A section that is empty is dropped.
        let rx_end = rodata
            .filter(|s| !s.data.is_empty())
            .map(|s| s.file_off + s.data.len() as u64)
            .unwrap_or(text.file_off + text.data.len() as u64);
        let rw = data.filter(|s| !s.data.is_empty());
        let nphdr = if rw.is_some() { 2u16 } else { 1 };

        let mut out = Vec::new();
        // ELF header.
        out.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        out.extend_from_slice(&0x3Eu16.to_le_bytes()); // EM_X86_64
        out.extend_from_slice(&1u32.to_le_bytes()); // version
        out.extend_from_slice(&0u64.to_le_bytes()); // e_entry, patched below
        out.extend_from_slice(&64u64.to_le_bytes()); // e_phoff
        out.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
        out.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        out.extend_from_slice(&64u16.to_le_bytes()); // e_ehsize
        out.extend_from_slice(&56u16.to_le_bytes()); // e_phentsize
        out.extend_from_slice(&nphdr.to_le_bytes()); // e_phnum
        out.extend_from_slice(&64u16.to_le_bytes()); // e_shentsize
        out.extend_from_slice(&0u16.to_le_bytes()); // e_shnum
        out.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx

        // RX program header.
        out.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        out.extend_from_slice(&0x5u32.to_le_bytes()); // R+X
        out.extend_from_slice(&0u64.to_le_bytes()); // p_offset
        out.extend_from_slice(&BASE.to_le_bytes()); // p_vaddr
        out.extend_from_slice(&BASE.to_le_bytes()); // p_paddr
        out.extend_from_slice(&rx_end.to_le_bytes()); // p_filesz
        out.extend_from_slice(&rx_end.to_le_bytes()); // p_memsz
        out.extend_from_slice(&0x1000u64.to_le_bytes()); // p_align

        if let Some(d) = rw {
            // A `.bss` segment maps no file bytes: `p_filesz` is zero and the
            // kernel supplies zeroed pages. `p_offset` must still be congruent
            // to `p_vaddr` modulo the page size, which zero and a page-aligned
            // address satisfy.
            let file_off = if d.bss { 0 } else { d.file_off };
            let filesz = if d.bss { 0 } else { d.data.len() as u64 };
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&0x6u32.to_le_bytes()); // R+W
            out.extend_from_slice(&file_off.to_le_bytes());
            out.extend_from_slice(&d.vaddr.to_le_bytes());
            out.extend_from_slice(&d.vaddr.to_le_bytes());
            out.extend_from_slice(&filesz.to_le_bytes());
            out.extend_from_slice(&(d.data.len() as u64).to_le_bytes());
            out.extend_from_slice(&0x1000u64.to_le_bytes());
        }

        let mut order: Vec<&Section> =
            self.sections.iter().filter(|s| !s.data.is_empty() && !s.bss).collect();
        order.sort_by_key(|s| s.file_off);
        for s in order {
            while (out.len() as u64) < s.file_off {
                out.push(0);
            }
            out.extend_from_slice(&s.data);
        }

        if let Some(st) = self.syms.get("_start") {
            let entry = self.sections[st.sec].vaddr + st.off;
            out[24..32].copy_from_slice(&entry.to_le_bytes());
        }
        out
    }
}

fn align_up(v: u64, a: u64) -> u64 {
    if a <= 1 {
        v
    } else {
        (v + a - 1) / a * a
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn parse_string_literal(arg: &str) -> Result<String, String> {
    let arg = arg.trim();
    let bytes = arg.as_bytes();
    if bytes.first() != Some(&b'"') || bytes.last() != Some(&b'"') || bytes.len() < 2 {
        return Err(format!("elf: bad string literal `{arg}`"));
    }
    let mut out = Vec::new();
    let mut i = 1;
    while i < bytes.len() - 1 {
        let c = bytes[i];
        if c == b'\\' && i + 1 < bytes.len() - 1 {
            i += 1;
            out.push(match bytes[i] {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                b'\\' => b'\\',
                b'"' => b'"',
                b'0' => 0,
                other => other,
            });
        } else {
            out.push(c);
        }
        i += 1;
    }
    String::from_utf8(out).map_err(|_| "elf: non-utf8 string literal".into())
}

// ---------------------------------------------------------------------------
// Instruction encoding
// ---------------------------------------------------------------------------

impl Elf {
    fn instruction(&mut self, line: &str, sec: usize) -> Result<(), String> {
        let (mnem, rest) = match line.split_once(char::is_whitespace) {
            Some((m, r)) => (m, r.trim()),
            None => (line, ""),
        };
        let ops = split_ops(rest);
        match mnem {
            "mov" | "movq" | "movb" | "movabs" | "movzbq" | "movzbl" => self.enc_mov(mnem, &ops, sec),
            "lea" => self.enc_lea(&ops, sec),
            "push" | "pop" => self.enc_push_pop(mnem, &ops, sec),
            "add" | "addq" | "sub" | "and" | "or" | "xor" | "cmp" | "cmpq" | "test" => {
                self.enc_arith(mnem, &ops, sec)
            }
            "imul" => self.enc_imul(&ops, sec),
            "inc" | "dec" | "neg" | "not" => self.enc_unary(mnem, &ops, sec),
            "div" | "idiv" => self.enc_div(mnem, &ops, sec),
            "cqto" | "cltd" => {
                self.sections[sec].data.extend_from_slice(&[0x48, 0x99]);
                Ok(())
            }
            "sal" | "shl" | "sar" | "shr" => self.enc_shift(mnem, &ops, sec),
            "jmp" => {
                self.sections[sec].data.push(0xE9);
                self.pc_rel(sec, &ops[0]);
                Ok(())
            }
            "call" => {
                self.sections[sec].data.push(0xE8);
                self.pc_rel(sec, &ops[0]);
                Ok(())
            }
            "ret" => {
                self.sections[sec].data.push(0xC3);
                Ok(())
            }
            "leave" => {
                self.sections[sec].data.push(0xC9);
                Ok(())
            }
            "nop" => {
                self.sections[sec].data.push(0x90);
                Ok(())
            }
            "syscall" => {
                self.sections[sec].data.extend_from_slice(&[0x0F, 0x05]);
                Ok(())
            }
            "sete" | "setne" | "setl" | "setle" | "setg" | "setge" | "setb" | "setbe" | "seta"
            | "setae" | "seto" | "setno" | "sets" | "setns" => self.enc_setcc(mnem, &ops, sec),
            j if j.starts_with('j') => self.enc_jcc(j, &ops, sec),
            other => Err(format!("elf: unsupported instruction `{other}`")),
        }
    }

    /// Reserve a 4-byte PC-relative field and record the fixup.
    fn pc_rel(&mut self, sec: usize, sym: &str) {
        let disp_off = self.sections[sec].data.len();
        self.fixups.push(Fixup { sec, disp_off, next_off: disp_off + 4, sym: sym.to_string() });
        self.sections[sec].data.extend_from_slice(&[0; 4]);
    }

    fn enc_mov(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let (src, dst) = (ops[0].clone(), ops[1].clone());
        match mnem {
            "movzbq" | "movzbl" => {
                let (dreg, _) = parse_reg(&dst)?;
                let w = mnem == "movzbq";
                if let Ok((sreg, s8)) = parse_reg(&src) {
                    let mut rex = 0x40;
                    if w {
                        rex |= 0x08;
                    }
                    if dreg >= 8 {
                        rex |= 0x04;
                    }
                    if sreg >= 8 {
                        rex |= 0x01;
                    }
                    // A byte source of %sil/%dil needs the prefix even with no
                    // extended register in play.
                    if rex != 0x40 || (s8 && sreg >= 4) {
                        self.sections[sec].data.push(rex);
                    }
                    self.sections[sec].data.extend_from_slice(&[0x0F, 0xB6]);
                    modrm_reg_reg(&mut self.sections[sec].data, dreg, sreg);
                    return Ok(());
                }
                let m = parse_mem(&src).ok_or_else(|| format!("elf: bad movz src `{src}`"))?;
                let mut rex = 0x40;
                if w {
                    rex |= 0x08;
                }
                if dreg >= 8 {
                    rex |= 0x04;
                }
                if m.base.map_or(false, |b| b >= 8) {
                    rex |= 0x01;
                }
                if m.index.map_or(false, |i| i >= 8) {
                    rex |= 0x02;
                }
                if rex != 0x40 {
                    self.sections[sec].data.push(rex);
                }
                self.sections[sec].data.extend_from_slice(&[0x0F, 0xB6]);
                emit_mem_modrm(&mut self.sections[sec].data, &m, dreg);
                if let Some(sym) = m.rip_label.clone() {
                    let len = self.sections[sec].data.len();
                    self.fixups.push(Fixup { sec, disp_off: len - 4, next_off: len, sym });
                }
                Ok(())
            }
            "movb" => self.enc_mov_bytes(&src, &dst, sec),
            _ => self.enc_mov_generic(mnem, &src, &dst, sec),
        }
    }

    fn enc_mov_bytes(&mut self, src: &str, dst: &str, sec: usize) -> Result<(), String> {
        if let Some(rest) = src.strip_prefix('$') {
            let v = parse_imm(rest)?;
            let m = parse_mem(dst).ok_or_else(|| format!("elf: bad movb dst `{dst}`"))?;
            self.mem_op(sec, 0xC6, &m, 0, false, false);
            self.sections[sec].data.push(v as u8);
            return Ok(());
        }
        if let Ok((sreg, s8)) = parse_reg(src) {
            if let Some(m) = parse_mem(dst) {
                // A byte register named %sil/%dil (or %spl/%bpl) needs a REX
                // prefix to mean itself; without one it is %dh/%bh/etc.
                self.mem_op(sec, 0x88, &m, sreg, false, s8 && sreg >= 4);
                return Ok(());
            }
            let (dreg, d8) = parse_reg(dst)?;
            if s8 || d8 || sreg >= 8 || dreg >= 8 {
                let mut rex = 0x40;
                if sreg >= 8 {
                    rex |= 0x04;
                }
                if dreg >= 8 {
                    rex |= 0x01;
                }
                self.sections[sec].data.push(rex);
            }
            self.sections[sec].data.push(0x88);
            modrm_reg_reg(&mut self.sections[sec].data, sreg, dreg);
            return Ok(());
        }
        if let Some(m) = parse_mem(src) {
            let (dreg, d8) = parse_reg(dst)?;
            self.mem_op(sec, 0x8A, &m, dreg, false, d8 && dreg >= 4);
            return Ok(());
        }
        Err(format!("elf: unsupported movb `{src}, {dst}`"))
    }

    /// Emit `opcode` with a memory operand, recording a fixup when the operand
    /// is RIP-relative. `emit_mem_op` writes the disp in its correct position
    /// (after the ModRM), so the fixup is recorded at the trailing 4 bytes.
    fn mem_op(&mut self, sec: usize, opcode: u8, m: &Mem, reg: u8, rex_w: bool, force_rex: bool) {
        emit_mem_op(&mut self.sections[sec].data, opcode, m, reg, rex_w, force_rex);
        if let Some(sym) = m.rip_label.clone() {
            let len = self.sections[sec].data.len();
            self.fixups.push(Fixup { sec, disp_off: len - 4, next_off: len, sym });
        }
    }

    fn enc_mov_generic(
        &mut self,
        mnem: &str,
        src: &str,
        dst: &str,
        sec: usize,
    ) -> Result<(), String> {
        if let Some(rest) = src.strip_prefix('$') {
            let v = match parse_imm(rest) {
                Ok(v) => v,
                Err(_) => {
                    // A symbol immediate, e.g. `mov $__rmc_overflow_len, %rdx`.
                    let (dreg, _) =
                        parse_reg(dst).map_err(|_| format!("elf: bad mov src `${rest}`"))?;
                    self.sections[sec].data.push(rex(true, 0, dreg));
                    self.sections[sec].data.push(0xC7);
                    modrm_reg_reg(&mut self.sections[sec].data, 0, dreg);
                    let off = self.sections[sec].data.len();
                    self.sections[sec].data.extend_from_slice(&[0; 4]);
                    self.abs_imm.push((sec, off, rest.trim().to_string()));
                    return Ok(());
                }
            };
            if let Ok((dreg, _)) = parse_reg(dst) {
                // The suffix pins the width when written; otherwise the
                // destination register's name does, so `mov $1, %eax` is 32-bit.
                // `movabs` is always the full 64-bit `B8+rd` form, even for a
                // value that would fit a shorter encoding.
                let (w, abs) = match mnem {
                    "movabs" => (true, true),
                    "movq" => (true, false),
                    "mov" => (reg_is_64(dst), false),
                    _ => (false, false),
                };
                emit_mov_imm(&mut self.sections[sec].data, dreg, v, w, abs);
                return Ok(());
            }
            let m = parse_mem(dst).ok_or_else(|| format!("elf: bad mov dst `{dst}`"))?;
            // `as` reads an unsuffixed `mov $imm, mem` as a 32-bit store (`movl`)
            // because there is no operand to infer a width from; only an explicit
            // `movq` stores 64 bits. Matching that keeps the encoder a faithful
            // stand-in and stops an unsuffixed store silently leaving the high
            // half of the destination stale. Either form sign-extends a 32-bit
            // immediate, so a wider one has no encoding at all and must be
            // materialised in a register by the caller.
            if !fits_i32(v) {
                return Err(format!("elf: `{mnem}` immediate {v} does not fit i32; use a register"));
            }
            self.mem_op(sec, 0xC7, &m, 0, mnem != "mov", false);
            self.sections[sec].data.extend_from_slice(&(v as i32).to_le_bytes());
            return Ok(());
        }
        if let Ok((sreg, s8)) = parse_reg(src) {
            if let Ok((dreg, d8)) = parse_reg(dst) {
                let byte = s8 || d8;
                // A 32-bit source selects the narrow move (`mov %eax,%ebx`);
                // only a 64-bit one sets REX.W.
                let mut rex = if byte || reg_is_32(src) { 0x40 } else { 0x48 };
                if sreg >= 8 {
                    rex |= 0x04; // source is the ModRM.reg field (REX.R)
                }
                if dreg >= 8 {
                    rex |= 0x01; // destination is ModRM.rm (REX.B)
                }
                if rex != 0x40 || (byte && (sreg >= 4 || dreg >= 4)) {
                    self.sections[sec].data.push(rex);
                }
                self.sections[sec].data.push(if byte { 0x88 } else { 0x89 });
                modrm_reg_reg(&mut self.sections[sec].data, sreg, dreg);
                return Ok(());
            }
            if let Some(m) = parse_mem(dst) {
                // `mov %dl, (%rsi)` is a one-byte store even though the
                // mnemonic carries no `b` suffix.
                self.mem_op(sec, if s8 { 0x88 } else { 0x89 }, &m, sreg, !s8, s8 && sreg >= 4);
                return Ok(());
            }
            return Err(format!("elf: bad mov dst `{dst}`"));
        }
        if let Some(m) = parse_mem(src) {
            let (dreg, d8) = parse_reg(dst)?;
            self.mem_op(sec, if d8 { 0x8A } else { 0x8B }, &m, dreg, !d8, d8 && dreg >= 4);
            return Ok(());
        }
        Err(format!("elf: unsupported mov `{src}, {dst}`"))
    }

    fn enc_lea(&mut self, ops: &[String], sec: usize) -> Result<(), String> {
        let (src, dst) = (ops[0].clone(), ops[1].clone());
        let (dreg, _) = parse_reg(&dst)?;
        let m = parse_mem(&src).ok_or_else(|| format!("elf: bad lea src `{src}`"))?;
        self.mem_op(sec, 0x8D, &m, dreg, true, false);
        Ok(())
    }

    fn enc_push_pop(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let op = &ops[0];
        if mnem == "push" {
            if let Some(rest) = op.strip_prefix('$') {
                let v = parse_imm(rest)?;
                self.sections[sec].data.push(0x68);
                self.sections[sec].data.extend_from_slice(&(v as i32).to_le_bytes());
                return Ok(());
            }
            let (reg, _) = parse_reg(op)?;
            if reg >= 8 {
                self.sections[sec].data.push(0x41);
            }
            self.sections[sec].data.push(0x50 + (reg & 7));
            return Ok(());
        }
        let (reg, _) = parse_reg(op)?;
        if reg >= 8 {
            self.sections[sec].data.push(0x41);
        }
        self.sections[sec].data.push(0x58 + (reg & 7));
        Ok(())
    }

    fn enc_arith(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let base = mnem.trim_end_matches('q');
        // (register-form opcode, /digit extension for the immediate form)
        let (rr, ext) = match base {
            "add" => (0x01u8, 0u8),
            "or" => (0x09, 1),
            "and" => (0x21, 4),
            "sub" => (0x29, 5),
            "xor" => (0x31, 6),
            "cmp" => (0x39, 7),
            "test" => (0x85, 0),
            other => return Err(format!("elf: unsupported `{other}`")),
        };
        let (src, dst) = (ops[0].clone(), ops[1].clone());
        if let Some(rest) = src.strip_prefix('$') {
            let v = parse_imm(rest)?;
            if base == "test" {
                return Err("elf: test with an immediate operand is not emitted".into());
            }
            if let Some(m) = parse_mem(&dst) {
                // `cmp $imm, mem`: 81 /7 id, or the sign-extended 83 /7 ib.
                if !fits_i32(v) {
                    return Err(format!("elf: `{mnem}` immediate {v} does not fit i32"));
                }
                if fits_i8(v) {
                    self.mem_op(sec, 0x83, &m, ext, true, false);
                    self.sections[sec].data.push(v as u8);
                } else {
                    self.mem_op(sec, 0x81, &m, ext, true, false);
                    self.sections[sec].data.extend_from_slice(&(v as i32).to_le_bytes());
                }
                return Ok(());
            }
            let (dreg, d8) = parse_reg(&dst)?;
            if d8 {
                // A byte operand uses 80 /ext ib and never REX.W; a byte
                // register %sil/%dil still needs the bare 0x40 prefix.
                let mut rex = 0x40;
                if dreg >= 8 {
                    rex |= 0x04;
                }
                if rex != 0x40 || dreg >= 4 {
                    self.sections[sec].data.push(rex);
                }
                self.sections[sec].data.push(0x80);
                modrm_reg_reg(&mut self.sections[sec].data, ext, dreg);
                self.sections[sec].data.push(v as u8);
                return Ok(());
            }
            // Width follows the destination: `add $1, %eax` must stay 32-bit
            // (`83 /0 ib`, no REX.W), like every other register form.
            if !fits_i32(v) {
                return Err(format!("elf: `{mnem}` immediate {v} does not fit i32"));
            }
            if let Some(r) = rex_if_needed(!reg_is_32(&dst), ext, dreg) {
                self.sections[sec].data.push(r);
            }
            if fits_i8(v) {
                self.sections[sec].data.push(0x83);
                modrm_reg_reg(&mut self.sections[sec].data, ext, dreg);
                self.sections[sec].data.push(v as u8);
            } else {
                self.sections[sec].data.push(0x81);
                modrm_reg_reg(&mut self.sections[sec].data, ext, dreg);
                self.sections[sec].data.extend_from_slice(&(v as i32).to_le_bytes());
            }
            return Ok(());
        }
        if let Ok((sreg, s8)) = parse_reg(&src) {
            if let Ok((dreg, d8)) = parse_reg(&dst) {
                // `op src, dst` writes dst, which is the r/m operand; src is
                // the reg field. (0x01 is `add r/m64, r64`.) REX.W follows the
                // operand width: `xor %eax,%eax` is the narrow, two-byte form
                // and must not be widened to `xor %rax,%rax`.
                let byte = s8 || d8;
                let mut rex = if byte || reg_is_32(&src) { 0x40 } else { 0x48 };
                if sreg >= 8 {
                    rex |= 0x04; // source is the ModRM.reg field (REX.R)
                }
                if dreg >= 8 {
                    rex |= 0x01; // destination is ModRM.rm (REX.B)
                }
                if rex != 0x40 || (byte && (sreg >= 4 || dreg >= 4)) {
                    self.sections[sec].data.push(rex);
                }
                self.sections[sec].data.push(rr);
                modrm_reg_reg(&mut self.sections[sec].data, sreg, dreg);
                return Ok(());
            }
            if let Some(m) = parse_mem(&dst) {
                self.mem_op(sec, rr, &m, sreg, true, false);
                return Ok(());
            }
            return Err(format!("elf: bad `{mnem} {src}, {dst}`"));
        }
        if let Some(m) = parse_mem(&src) {
            let (dreg, _) = parse_reg(&dst)?;
            self.mem_op(sec, rr | 0x2, &m, dreg, true, false);
            return Ok(());
        }
        Err(format!("elf: unsupported `{mnem} {src}, {dst}`"))
    }

    fn enc_imul(&mut self, ops: &[String], sec: usize) -> Result<(), String> {
        if ops.len() == 3 {
            let v = parse_imm(ops[0].strip_prefix('$').ok_or("elf: imul imm expected")?)?;
            let (sreg, _) = parse_reg(&ops[1])?;
            let (dreg, _) = parse_reg(&ops[2])?;
            self.sections[sec].data.push(rex(true, dreg, sreg));
            self.sections[sec].data.push(0x69);
            modrm_reg_reg(&mut self.sections[sec].data, dreg, sreg);
            self.sections[sec].data.extend_from_slice(&(v as i32).to_le_bytes());
            return Ok(());
        }
        if ops.len() == 2 && ops[0].starts_with('$') {
            // `imul $imm, %reg`: the two-operand immediate form, which is the
            // three-operand form with source and destination the same register.
            // The immediate is sign-extended from 32 bits, so a value that does
            // not fit i32 would silently multiply by a different number; refuse
            // it so the caller materialises the constant in a register instead.
            let v = parse_imm(&ops[0][1..])?;
            if !fits_i32(v) {
                return Err(format!("elf: imul immediate {v} does not fit in 32 bits"));
            }
            let (reg, _) = parse_reg(&ops[1])?;
            self.sections[sec].data.push(rex(true, reg, reg));
            self.sections[sec].data.push(0x69);
            modrm_reg_reg(&mut self.sections[sec].data, reg, reg);
            self.sections[sec].data.extend_from_slice(&(v as i32).to_le_bytes());
            return Ok(());
        }
        if ops.len() == 1 {
            // One-operand `imul %reg`: signed multiply of %rax by the operand,
            // result in %rdx:%rax. Used by the division magic multiply.
            let (reg, _) = parse_reg(&ops[0])?;
            self.sections[sec].data.push(rex(true, 5, reg));
            self.sections[sec].data.push(0xF7);
            modrm_reg_reg(&mut self.sections[sec].data, 5, reg);
            return Ok(());
        }
        if let Some(m) = parse_mem(&ops[0]) {
            // `imul mem, %reg`: two-operand form with a memory source.
            let (dreg, _) = parse_reg(&ops[1])?;
            let before = self.sections[sec].data.len();
            self.mem_op(sec, 0x0F, &m, dreg, true, false);
            // `mem_op` writes the opcode byte, but `imul r/m64` is the two-byte
            // `0F AF`; the second byte must follow the first, before the ModRM.
            // REX.W is always set here, so the layout is `[REX] 0F <ModRM...>`
            // and the splice point is right after the `0F`.
            let data = &mut self.sections[sec].data;
            data.insert(before + 2, 0xAF);
            // The splice shifted everything after the opcode, so any fixup the
            // memory operand registered (a RIP-relative disp32) moves by one.
            for fx in self.fixups.iter_mut().filter(|f| f.sec == sec && f.next_off > before) {
                fx.disp_off += 1;
                fx.next_off += 1;
            }
            return Ok(());
        }
        let (sreg, _) = parse_reg(&ops[0])?;
        let (dreg, _) = parse_reg(&ops[1])?;
        self.sections[sec].data.push(rex(true, dreg, sreg));
        self.sections[sec].data.extend_from_slice(&[0x0F, 0xAF]);
        modrm_reg_reg(&mut self.sections[sec].data, dreg, sreg);
        Ok(())
    }

    fn enc_unary(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let ext = match mnem {
            "inc" => 0u8,
            "dec" => 1,
            "not" => 2,
            "neg" => 3,
            _ => unreachable!(),
        };
        let (reg, _) = parse_reg(&ops[0])?;
        self.sections[sec].data.push(rex(true, ext, reg));
        // `inc`/`dec` are group-5 (`FF /0,/1`); `not`/`neg` are group-3
        // (`F7 /2,/3`). Using `FF` for `not`/`neg` would encode `call far`.
        self.sections[sec].data.push(if mnem == "inc" || mnem == "dec" { 0xFF } else { 0xF7 });
        modrm_reg_reg(&mut self.sections[sec].data, ext, reg);
        Ok(())
    }

    fn enc_div(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let (reg, _) = parse_reg(&ops[0])?;
        let ext = if mnem == "div" { 6u8 } else { 7u8 };
        // Operand size comes from the register the caller named: `div %ebx` is
        // a 32-bit divide and must not set REX.W. Hardcoding W turned every
        // 32-bit divide into the 64-bit form, which is ~14% slower on the
        // divide-bound `mix` loop and made the in-process output differ from
        // `as`+`ld` for the same source.
        if let Some(r) = rex_if_needed(reg_is_64(&ops[0]), ext, reg) {
            self.sections[sec].data.push(r);
        }
        self.sections[sec].data.push(0xF7);
        modrm_reg_reg(&mut self.sections[sec].data, ext, reg);
        Ok(())
    }

    fn enc_shift(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let ext = match mnem {
            "sal" | "shl" => 4u8,
            "shr" => 5,
            "sar" => 7,
            _ => unreachable!(),
        };
        let (reg, _) = parse_reg(&ops[1])?;
        if let Some(rest) = ops[0].strip_prefix('$') {
            let n = parse_imm(rest)?;
            self.sections[sec].data.push(rex(true, ext, reg));
            self.sections[sec].data.push(0xC1);
            modrm_reg_reg(&mut self.sections[sec].data, ext, reg);
            self.sections[sec].data.push((n as u8) & 0x3F);
            return Ok(());
        }
        self.sections[sec].data.push(rex(true, ext, reg));
        self.sections[sec].data.push(0xD3);
        modrm_reg_reg(&mut self.sections[sec].data, ext, reg);
        Ok(())
    }

    fn enc_setcc(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let cc = match mnem {
            "sete" => 0x94u8,
            "setne" => 0x95,
            "setl" => 0x9C,
            "setle" => 0x9E,
            "setg" => 0x9F,
            "setge" => 0x9D,
            "setb" => 0x92,
            "setbe" => 0x96,
            "seta" => 0x97,
            "setae" => 0x93,
            "seto" => 0x90,
            "setno" => 0x91,
            "sets" => 0x98,
            "setns" => 0x99,
            _ => unreachable!(),
        };
        let (reg, _) = parse_reg(&ops[0])?;
        if reg >= 8 {
            self.sections[sec].data.push(0x41);
        }
        self.sections[sec].data.extend_from_slice(&[0x0F, cc]);
        modrm_reg_reg(&mut self.sections[sec].data, 0, reg);
        Ok(())
    }

    fn enc_jcc(&mut self, mnem: &str, ops: &[String], sec: usize) -> Result<(), String> {
        let cc = match mnem {
            "je" | "jz" => 0x84u8,
            "jne" | "jnz" => 0x85,
            "jl" => 0x8C,
            "jle" => 0x8E,
            "jg" => 0x8F,
            "jge" => 0x8D,
            "jb" | "jc" => 0x82,
            "jbe" => 0x86,
            "ja" => 0x87,
            "jae" | "jnc" => 0x83,
            "js" => 0x88,
            "jns" => 0x89,
            "jo" => 0x80,
            "jno" => 0x81,
            "jp" => 0x8A,
            "jnp" => 0x8B,
            other => return Err(format!("elf: unsupported branch `{other}`")),
        };
        // Always the 6-byte rel32 form, so branch length never depends on the
        // distance to the target.
        self.sections[sec].data.extend_from_slice(&[0x0F, cc]);
        self.pc_rel(sec, &ops[0]);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Operand parsing helpers
// ---------------------------------------------------------------------------

/// Split an operand list on commas that are not inside parentheses.
fn split_ops(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn parse_imm(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x") {
        // Accept values with the top bit set (e.g. `0x8000000000000000`), which
        // do not fit a signed 64-bit parse but are valid bit patterns.
        if let Ok(v) = i64::from_str_radix(hex, 16) {
            return Ok(v);
        }
        return u64::from_str_radix(hex, 16)
            .map(|v| v as i64)
            .map_err(|_| format!("elf: bad immediate `{s}`"));
    }
    if let Some(hex) = s.strip_prefix("-0x") {
        return u64::from_str_radix(hex, 16)
            .map(|v| (v as i64).wrapping_neg())
            .map_err(|_| format!("elf: bad immediate `{s}`"));
    }
    s.parse::<i64>().map_err(|_| format!("elf: bad immediate `{s}`"))
}

fn fits_i8(v: i64) -> bool {
    (-128..=127).contains(&v)
}

fn fits_i32(v: i64) -> bool {
    (i32::MIN as i64..=i32::MAX as i64).contains(&v)
}

/// Register number, plus whether it was named with a byte-width name.
fn parse_reg(s: &str) -> Result<(u8, bool), String> {
    let s = s.trim();
    let s = s.strip_prefix('%').ok_or_else(|| format!("elf: not a register `{s}`"))?;
    let r = match s {
        "rax" => (0, false),
        "rcx" => (1, false),
        "rdx" => (2, false),
        "rbx" => (3, false),
        "rsp" => (4, false),
        "rbp" => (5, false),
        "rsi" => (6, false),
        "rdi" => (7, false),
        "r8" => (8, false),
        "r9" => (9, false),
        "r10" => (10, false),
        "r11" => (11, false),
        "r12" => (12, false),
        "r13" => (13, false),
        "r14" => (14, false),
        "r15" => (15, false),
        "eax" => (0, false),
        "ecx" => (1, false),
        "edx" => (2, false),
        "ebx" => (3, false),
        "esi" => (6, false),
        "edi" => (7, false),
        "r8d" => (8, false),
        "r9d" => (9, false),
        "r10d" => (10, false),
        "r11d" => (11, false),
        "r12d" => (12, false),
        "r13d" => (13, false),
        "r14d" => (14, false),
        "r15d" => (15, false),
        "al" => (0, true),
        "cl" => (1, true),
        "dl" => (2, true),
        "bl" => (3, true),
        "sil" => (6, true),
        "dil" => (7, true),
        "r8b" => (8, true),
        "r9b" => (9, true),
        "r10b" => (10, true),
        "r11b" => (11, true),
        "r12b" => (12, true),
        "r13b" => (13, true),
        "r14b" => (14, true),
        "r15b" => (15, true),
        other => return Err(format!("elf: unknown register `%{other}`")),
    };
    Ok(r)
}

/// A memory operand: `disp(%base)`, `disp(%base,%index,scale)` or `sym(%rip)`.
struct Mem {
    disp: i64,
    base: Option<u8>,
    index: Option<u8>,
    scale: u8,
    rip_label: Option<String>,
}

fn parse_mem(s: &str) -> Option<Mem> {
    let s = s.trim();
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    let disp_str = s[..open].trim();
    let inside = &s[open + 1..close];
    let parts: Vec<&str> = inside.split(',').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return None;
    }
    if parts[0] == "%rip" {
        return Some(Mem {
            disp: 0,
            base: None,
            index: None,
            scale: 1,
            rip_label: Some(disp_str.to_string()),
        });
    }
    let base = parse_reg(parts[0]).ok().map(|(r, _)| r);
    // `(%base,%index)` has an implied scale of 1; `(%base,%index,scale)` names
    // it. A second operand that is not a register is a scale with no index.
    let (index, scale) = if parts.len() >= 2 && parts[1].starts_with('%') {
        let idx = parse_reg(parts[1]).ok().map(|(r, _)| r);
        let sc: u8 = if parts.len() >= 3 { parts[2].parse().unwrap_or(1) } else { 1 };
        (idx, sc)
    } else {
        (None, 1)
    };
    let disp = if disp_str.is_empty() { 0 } else { parse_imm(disp_str).ok()? };
    Some(Mem { disp, base, index, scale, rip_label: None })
}

// ---------------------------------------------------------------------------
// Instruction byte emitters
// ---------------------------------------------------------------------------

/// True when a register name is a 64-bit one (`%rax`..`%r15`). The encoder
/// needs this to choose REX.W: the 32-bit aliases (`%eax`..) select the narrow
/// form, which for `div`/`idiv` is both shorter and measurably faster.
fn reg_is_64(s: &str) -> bool {
    let s = s.trim().trim_start_matches('%');
    matches!(
        s,
        "rax" | "rcx" | "rdx" | "rbx" | "rsp" | "rbp" | "rsi" | "rdi"
            | "r8" | "r9" | "r10" | "r11" | "r12" | "r13" | "r14" | "r15"
    )
}

/// True when a register name is a 32-bit one (`%eax`..`%r15d`). A 32-bit
/// operand must not set REX.W; `xor %eax,%eax` is the two-byte zeroing idiom,
/// and widening it to `xor %rax,%rax` both grows the artifact and disagrees with
/// `as`.
fn reg_is_32(s: &str) -> bool {
    matches!(
        s.trim().trim_start_matches('%'),
        "eax" | "ecx" | "edx" | "ebx" | "esp" | "ebp" | "esi" | "edi"
            | "r8d" | "r9d" | "r10d" | "r11d" | "r12d" | "r13d" | "r14d" | "r15d"
    )
}

/// The REX byte for an instruction, or `None` when the bare `0x40` prefix would
/// carry no information and `as` omits it.
fn rex_if_needed(w: bool, reg: u8, rm: u8) -> Option<u8> {
    if w || reg >= 8 || rm >= 8 {
        Some(rex(w, reg, rm))
    } else {
        None
    }
}

/// REX byte with W set, `reg` in the ModRM.reg field and `rm` in ModRM.rm.
fn rex(w: bool, reg: u8, rm: u8) -> u8 {
    let mut rex = 0x40;
    if w {
        rex |= 0x08;
    }
    if reg >= 8 {
        rex |= 0x04;
    }
    if rm >= 8 {
        rex |= 0x01;
    }
    rex
}

/// ModRM for a register/register form: mod=11, reg field, rm field.
fn modrm_reg_reg(out: &mut Vec<u8>, reg: u8, rm: u8) {
    out.push(0xC0 | ((reg & 7) << 3) | (rm & 7));
}

fn emit_mov_imm(out: &mut Vec<u8>, reg: u8, v: i64, w: bool, abs: bool) {
    // `mov $imm, %r32` is `B8+rd` with a 32-bit immediate and no REX.W. This is
    // the form `as` emits for an unsuffixed `mov $1, %eax`; widening it to the
    // 64-bit `C7 /0` form both disagrees with `as` and changes the bytes.
    if !w {
        if reg >= 8 {
            out.push(0x41); // REX.B
        }
        out.push(0xB8 + (reg & 7));
        out.extend_from_slice(&(v as i32).to_le_bytes());
        return;
    }
    // `movabs` is always the 10-byte `B8+rd` form, whatever the value. A plain
    // 64-bit `mov` sign-extends a 32-bit immediate, so a value that fits i32
    // uses `C7 /0` and anything larger needs the full immediate.
    if !abs && (i32::MIN as i64..=i32::MAX as i64).contains(&v) {
        out.push(rex(true, 0, reg));
        out.push(0xC7);
        modrm_reg_reg(out, 0, reg);
        out.extend_from_slice(&(v as i32).to_le_bytes());
        return;
    }
    out.push(rex(true, 0, reg));
    out.push(0xB8 + (reg & 7));
    out.extend_from_slice(&(v as u64).to_le_bytes());
}

/// Emit an instruction whose ModRM encodes a memory operand with `reg` in the
/// reg field. When `disp_pc` is true the caller has already reserved the
/// RIP-relative disp32, so only the opcode and ModRM are emitted here.
fn emit_mem_op(out: &mut Vec<u8>, opcode: u8, m: &Mem, reg: u8, rex_w: bool, force_rex: bool) {
    let mut rex = 0x40;
    if rex_w {
        rex |= 0x08;
    }
    if reg >= 8 {
        rex |= 0x04;
    }
    if m.rip_label.is_some() {
        if rex != 0x40 || force_rex {
            out.push(rex);
        }
        out.push(opcode);
        out.push(0x05 | ((reg & 7) << 3));
        out.extend_from_slice(&[0; 4]);
        return;
    }

    let base = m.base;
    let index = m.index;
    // With no base the SIB base field is 101, which only means "no base" when
    // mod=00 (mod=01/10 would select %rbp), so a base-less operand always uses
    // mod=00 plus a disp32. A base of %rbp/%r13 (rm 101) is the mirror image:
    // mod=00 is invalid for it, so it needs an explicit disp8 0.
    let (modbits, disp_size) = if base.is_none() {
        (0u8, 4usize)
    } else if m.disp == 0 && !base.map_or(false, |b| (b & 7) == 5) {
        (0, 0)
    } else if fits_i8(m.disp) {
        (1, 1)
    } else {
        (2, 4)
    };
    if base.map_or(false, |b| b >= 8) {
        rex |= 0x01;
    }
    if index.map_or(false, |i| i >= 8) {
        rex |= 0x02;
    }
    if rex != 0x40 || force_rex {
        out.push(rex);
    }
    out.push(opcode);

    // A SIB byte is required for an index, for no base, and for a base of
    // %rsp/%r12 (whose rm encoding is the SIB escape, 100b).
    let need_sib = index.is_some() || base.is_none() || base.map_or(false, |b| (b & 7) == 4);
    let rm_field = if need_sib { 4 } else { base.unwrap() & 7 };
    out.push((modbits << 6) | ((reg & 7) << 3) | rm_field);

    if need_sib {
        let scale_bits = match m.scale {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        let idx = index.unwrap_or(4);
        let bs = base.unwrap_or(5);
        out.push((scale_bits << 6) | ((idx & 7) << 3) | (bs & 7));
    }
    match disp_size {
        1 => out.push(m.disp as u8),
        4 => out.extend_from_slice(&(m.disp as i32).to_le_bytes()),
        _ => {}
    }
}

/// Emit only the ModRM (+SIB+disp) for a memory operand, with `reg` in the reg
/// field. Used for two-byte opcodes (`0F B6`) where the opcode bytes are
/// emitted by the caller. No REX is emitted; the caller decides.
fn emit_mem_modrm(out: &mut Vec<u8>, m: &Mem, reg: u8) {
    if m.rip_label.is_some() {
        out.push(0x05 | ((reg & 7) << 3));
        out.extend_from_slice(&[0; 4]);
        return;
    }
    let base = m.base;
    let index = m.index;
    // See `emit_mem_op`: a base-less operand is mod=00 with a disp32, while
    // %rbp/%r13 needs a disp8 because mod=00/rm=101 means "no base".
    let (modbits, disp_size) = if base.is_none() {
        (0u8, 4usize)
    } else if m.disp == 0 && !base.map_or(false, |b| (b & 7) == 5) {
        (0, 0)
    } else if fits_i8(m.disp) {
        (1, 1)
    } else {
        (2, 4)
    };
    let need_sib = index.is_some() || base.is_none() || base.map_or(false, |b| (b & 7) == 4);
    let rm_field = if need_sib { 4 } else { base.unwrap() & 7 };
    out.push((modbits << 6) | ((reg & 7) << 3) | rm_field);
    if need_sib {
        let scale_bits = match m.scale {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        let idx = index.unwrap_or(4);
        let bs = base.unwrap_or(5);
        out.push((scale_bits << 6) | ((idx & 7) << 3) | (bs & 7));
    }
    match disp_size {
        1 => out.push(m.disp as u8),
        4 => out.extend_from_slice(&(m.disp as i32).to_le_bytes()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(s: &str) -> Mem {
        parse_mem(s).expect("operand parses")
    }

    #[test]
    fn lea_with_index_and_no_base_is_mod00_disp32() {
        // `lea 0(,%rbx,4), %rdi`: the SIB base field 101 means "no base" only
        // with mod=00, so the operand must be mod=00 plus a disp32. A disp8
        // here would instead select base=%rbp and read the wrong address.
        let mut out = Vec::new();
        emit_mem_op(&mut out, 0x8D, &mem("0(,%rbx,4)"), 7, true, false);
        assert_eq!(out, [0x48, 0x8D, 0x3C, 0x9D, 0, 0, 0, 0]);
    }

    #[test]
    fn lea_with_index_and_no_base_is_mod00_disp32_in_modrm_form() {
        let mut out = Vec::new();
        emit_mem_modrm(&mut out, &mem("0(,%rbx,4)"), 0);
        assert_eq!(out, [0x04, 0x9D, 0, 0, 0, 0]);
    }

    fn test_elf(flags: u64) -> Elf {
        Elf {
            sections: vec![Section {
                name: ".text",
                flags,
                align: 16,
                data: vec![1, 2, 3],
                vaddr: 0,
                file_off: 0,
                bss: false,
            }],
            syms: HashMap::new(),
            fixups: Vec::new(),
            abs64: Vec::new(),
            abs_imm: Vec::new(),
            text_idx: 0,
        }
    }

    #[test]
    fn balign_pads_an_executable_section_with_nop() {
        // A zero byte decodes as `add %al,(%rax)`; a jump into the pad would
        // fault, so an executable section pads with the one-byte `nop`.
        let mut e = test_elf(0x5);
        e.pad_to_align(0, 8);
        assert_eq!(e.sections[0].data, [1, 2, 3, 0x90, 0x90, 0x90, 0x90, 0x90]);
    }

    #[test]
    fn balign_pads_a_data_section_with_zero() {
        let mut e = test_elf(0x6);
        e.pad_to_align(0, 8);
        assert_eq!(e.sections[0].data, [1, 2, 3, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn p2align_aligns_to_a_power_of_two() {
        // `.p2align 4` means "align to 2^4 = 16", not "align to 4". The two
        // were conflated, so every loop body the generator padded with
        // `.p2align 4` came out 4-byte aligned instead of 16, and the
        // in-process binary differed from the `as`+`ld` one for the same
        // source. Three bytes of text then pad to 16, not to 4.
        let mut e = test_elf(0x5);
        let (mut cur, mut sets) = (0usize, Vec::new());
        e.directive("p2align 4", &mut cur, &mut sets).unwrap();
        assert_eq!(e.sections[0].data.len(), 16);
    }

    #[test]
    fn balign_aligns_to_a_byte_count() {
        // `.balign n` is a byte count, unlike `.p2align n`.
        let mut e = test_elf(0x5);
        let (mut cur, mut sets) = (0usize, Vec::new());
        e.directive("balign 8", &mut cur, &mut sets).unwrap();
        assert_eq!(e.sections[0].data.len(), 8);
    }

    #[test]
    fn an_all_zero_read_only_section_is_not_demoted_to_bss() {
        // A `.rodata` holding only an empty-string record (`.quad 0`) is all
        // zeros. Demoting it to `.bss` left `rx_end` reading its unassigned
        // `file_off` of 0, so the RX segment covered 8 bytes and the program
        // segfaulted on entry; the `as`+`ld` path was unaffected. Only a
        // writable section may become `.bss`.
        let elf = assemble(
            ".section .rodata\n.Lstr0:\n.quad 0\n\
             .text\nmain:\nmov $1, %rax\nret\n\
             .globl _start\n_start:\ncall main\nret\n",
        )
        .unwrap();
        // The RX segment's file size must cover the whole image, not 8 bytes.
        let filesz = u64::from_le_bytes(elf[96..104].try_into().unwrap());
        assert!(filesz > 64, "RX segment covers the code, got {filesz}");
    }

    #[test]
    fn a_32_bit_divide_does_not_set_rex_w() {
        // `div %ebx` is the 32-bit form: `F7 /6` with no REX.W. Forcing W made
        // it a 64-bit divide, which is ~14% slower on a divide-bound loop and
        // disagreed with what `as` assembles from the same line.
        let mut e = test_elf(0x5);
        e.instruction("div %ebx", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0xF7, 0xF3]);
    }

    #[test]
    fn an_unsuffixed_immediate_store_is_32_bit_like_as() {
        // `mov $1, -8(%rbp)` assembles to `movl` (`c7 45 f8 ...`); only an
        // explicit `movq` sets REX.W.
        let mut e = test_elf(0x5);
        e.instruction("mov $1, -8(%rbp)", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0xC7, 0x45, 0xF8, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn a_q_suffixed_immediate_store_is_64_bit() {
        let mut e = test_elf(0x5);
        e.instruction("movq $1, -8(%rbp)", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x48, 0xC7, 0x45, 0xF8, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn a_32_bit_zeroing_xor_does_not_set_rex_w() {
        // `xor %eax,%eax` is the two-byte zeroing idiom; widening it to
        // `xor %rax,%rax` grows the artifact and disagrees with `as`.
        let mut e = test_elf(0x5);
        e.instruction("xor %eax, %eax", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x31, 0xC0]);
    }

    #[test]
    fn a_32_bit_immediate_add_does_not_set_rex_w() {
        let mut e = test_elf(0x5);
        e.instruction("add $1, %eax", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x83, 0xC0, 0x01]);
    }

    #[test]
    fn a_32_bit_register_move_does_not_set_rex_w() {
        let mut e = test_elf(0x5);
        e.instruction("mov %ebx, %eax", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x89, 0xD8]);
    }

    #[test]
    fn an_unsuffixed_immediate_to_a_32_bit_register_is_32_bit_like_as() {
        // `mov $1, %eax` is the `B8+rd` form with no REX.W; emitting the 64-bit
        // `C7 /0` form instead both grows the bytes and disagrees with `as`.
        let mut e = test_elf(0x5);
        e.instruction("mov $1, %eax", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0xB8, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn an_unsuffixed_immediate_to_a_64_bit_register_sign_extends() {
        let mut e = test_elf(0x5);
        e.instruction("mov $1, %rax", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn movabs_is_always_the_ten_byte_form() {
        // `movabs` names the 64-bit `B8+rd` form even for a small immediate, so
        // it must not fall back to the shorter `C7 /0` encoding.
        let mut e = test_elf(0x5);
        e.instruction("movabs $1, %rax", 0).unwrap();
        assert_eq!(
            e.sections[0].data[3..],
            [0x48, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn a_wide_immediate_to_a_64_bit_register_uses_the_full_form() {
        // `0x1_0000_0000` does not fit a sign-extended 32-bit immediate, so it
        // needs `movabs` (`B8+rd` with a 64-bit immediate).
        let mut e = test_elf(0x5);
        e.instruction("mov $4294967296, %rax", 0).unwrap();
        assert_eq!(
            e.sections[0].data[3..],
            [0x48, 0xB8, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn a_wide_immediate_to_a_32_bit_register_is_truncated_like_as() {
        // `mov $0xFFFFFFFF, %ecx` truncates the immediate to 32 bits (`as`
        // warns and shortens), so the encoder must write the low half.
        let mut e = test_elf(0x5);
        e.instruction("mov $4294967295, %ecx", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0xB9, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn an_extended_32_bit_register_still_needs_rex() {
        // `mov %r8d, %eax` needs REX.R (no REX.W); the bare 0x40 is dropped.
        let mut e = test_elf(0x5);
        e.instruction("mov %r8d, %eax", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x44, 0x89, 0xC0]);
    }

    #[test]
    fn a_64_bit_divide_sets_rex_w() {
        let mut e = test_elf(0x5);
        e.instruction("idiv %rbx", 0).unwrap();
        assert_eq!(e.sections[0].data[3..], [0x48, 0xF7, 0xFB]);
    }

    #[test]
    fn rbp_base_needs_an_explicit_disp8() {
        // mod=00/rm=101 is "no base", so %rbp as a base needs a disp8 0.
        let mut out = Vec::new();
        emit_mem_op(&mut out, 0x8B, &mem("(%rbp)"), 0, true, false);
        assert_eq!(out, [0x48, 0x8B, 0x45, 0x00]);
    }
}
