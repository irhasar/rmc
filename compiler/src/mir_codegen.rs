//! Lower the MIR CFG that `mir.rs` parses to x86-64 assembly.
//!
//! This is the *backend* of the `rustc front end + rmc back end` pipeline: rustc
//! performs lexing, parsing, name resolution, type checking and MIR-level
//! optimisation, and this module emits the machine code. The generated assembly
//! is the same dialect `codegen.rs` emits, so it is assembled and linked by the
//! existing in-process `elf.rs` and linked against the existing freestanding
//! `runtime.rs` (no `as`, no `ld`, no libc, no LLVM at the back end).
//!
//! ## Model
//!
//! Every local is assigned a stack slot (`-(n+1)*8(%rbp)`), so a place is a
//! memory operand. A scalar lives in its slot; an aggregate lives in `size`
//! consecutive slots and is named by the address of its first word. A `&T` /
//! `*T` is a scalar holding an address. A projection walks the place:
//! `Field(k)` adds `k*8`, `Index(i)` adds `i*8`, `Deref` loads the pointer.
//!
//! The whole point is speed of both compile and run, so the generator keeps
//! four cheap local-promotion rules rather than a full register allocator:
//!
//! * the induction local of a counted loop lives in a callee-saved register for
//!   the whole function (it is written once per iteration);
//! * a local mentioned at least twice and never projected lives in a register;
//! * an `x = x <op> k` accumulation updates its register in place;
//! * a comparison feeds `cmp`/`jcc` directly, so `bool` temporaries never hit
//!   memory.

use crate::mir::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Callee-saved registers the generator may hand to a promoted local.
const PROMOTE_REGS: [&str; 5] = ["%rbx", "%r12", "%r13", "%r14", "%r15"];
/// Caller-saved registers used to hold a local across a region that makes no
/// call. They must be spilled around any call, so they are only handed out to
/// functions that contain no call at all.
const CALLER_REGS: [&str; 4] = ["%rsi", "%rdi", "%r8", "%r9"];

pub struct Options {
    /// Emit `jo __rmc_overflow` after `+`/`-`/`*` (rustc's overflow-checks).
    pub overflow_checks: bool,
    /// Promote hot locals into callee-saved registers (on by default).
    pub promote: bool,
    /// Align loop headers to a 16-byte boundary (on by default).
    pub align_loops: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { overflow_checks: false, promote: true, align_loops: true }
    }
}

/// A promoted local: which register holds it, and whether that register has to
/// be saved/restored across a call.
#[derive(Clone, Copy)]
struct Promo {
    reg: &'static str,
    caller_saved: bool,
}

struct FnGen<'a> {
    prog: &'a Program,
    func: &'a Function,
    opts: &'a Options,
    /// String literal content -> its `.rodata` label.
    string_ids: &'a HashMap<String, String>,
    /// `format_args!` template bytes -> its `.rodata` label.
    template_ids: &'a HashMap<Vec<u8>, String>,
    /// Static allocation id -> its data label.
    alloc_labels: &'a HashMap<u32, String>,
    /// Struct name -> field types, learned from aggregate constructions. Used to
    /// size a struct and to stride over an array of them.
    structs: &'a HashMap<String, Vec<Ty>>,
    /// Enum name -> its layout, learned from constructions and `discriminant`
    /// switches.
    enums: &'a HashMap<String, EnumLayout>,
    /// Local index -> its byte size (`8` for a scalar, `8*n` for an aggregate).
    size: Vec<i64>,
    /// Local index -> type.
    ty: Vec<Ty>,
    /// Local index -> the slot index its region starts at.
    local_base: Vec<i64>,
    /// Local index -> promoted register, when promoted.
    promo: HashMap<u32, Promo>,
    /// Local index -> shadow slot, for a promoted local whose address is taken.
    shadow: HashMap<u32, i64>,
    /// Local index -> spill slot, for a promoted *caller-saved* local (spilled
    /// around a call).
    caller_spill: HashMap<u32, i64>,
    /// Locals that are an operand of a loop's counted comparison (the counter
    /// and its bound), promoted first.
    loop_operands: Vec<u32>,
    /// Whether the function can reach a call to `alloc`. `alloc` keeps the heap
    /// pointer in a caller-saved register, so such a function cannot promote a
    /// value into one and keep it live across the call.
    allocates: bool,
    /// Callee-saved registers saved below `%rbp`; frame slots start past them.
    saved_regs: i64,
    /// Dry-pass slot index -> compacted slot index. `None` until the dry pass
    /// has revealed which slots the body actually touches.
    slot_map: Option<Vec<i64>>,
    next_slot: i64,
    out: String,
    label: u64,
    errors: Vec<String>,
}

/// Generate x86-64 assembly for `prog`, ready to concatenate with the runtime.
pub fn compile(prog: &Program, opts: &Options) -> Result<String, String> {
    let mut asm = String::new();
    asm.push_str(".text\n");

    // Emit only functions the program can reach from `main` (dead-function
    // elimination keeps the artifact small and the compile fast).
    let live = reachable(prog);
    let order: Vec<&Function> = prog.funcs.iter().filter(|f| live.contains(&f.name)).collect();

    // String literals become `.rodata` labels, emitted once.
    let (strings, string_ids) = collect_strings(prog);
    // `format_args!` templates are byte-string literals: raw bytes, no length
    // word, because the runtime walks them and reads the length from the first
    // byte of each piece.
    let (templates, template_ids) = collect_templates(prog);
    // The runtime walker renders a placeholder's value in full; it does not
    // honour a width or precision (`{:>5}`, `{:.2}`). Reject those templates
    // here so they are a diagnostic rather than silently mis-rendered.
    for t in &templates {
        if let Some(field) = template_unrendered_field(t) {
            return Err(format!(
                "mir: `format_args!` placeholder with `{field}` is not implemented on the MIR path; \
                 format the value without flags, width or precision"
            ));
        }
    }

    // Every static allocation becomes a `.rodata` blob: its bytes, plus a
    // `.quad` for each interior pointer relocation. `alloc1` (string data) and
    // `alloc2` (the promoted `static` pointer) are both reached this way.
    let alloc_labels: HashMap<u32, String> =
        prog.allocs.keys().map(|id| (*id, format!("__rmc_alloc{id}"))).collect();

    let structs = collect_structs(prog);
    let enums = collect_enums(prog);

    for f in &order {
        if is_unsupported_builtin(&f.name) {
            continue;
        }
        let mut g = FnGen::new(prog, f, opts, &string_ids, &template_ids, &alloc_labels, &structs, &enums);
        let body = g.gen_fn()?;
        asm.push_str(&body);
        if !g.errors.is_empty() {
            return Err(g.errors.join("\n"));
        }
    }

    let mut rodata = String::new();
    if !strings.is_empty() || !templates.is_empty() {
        rodata.push_str("\n.section .rodata\n");
    }
    if !strings.is_empty() {
        for s in &strings {
            let label = &string_ids[s];
            // A `&str`/`&[u8]` literal is a `{len: i64, bytes..}` record, the
            // same shape the `.rms` path uses and the one `print_str` and
            // `PtrMetadata` (`str::len`) read. The record's address is the
            // value, so word 0 must be the byte length, not the first byte.
            let _ = writeln!(rodata, ".balign 8");
            let _ = writeln!(rodata, "{label}:");
            let _ = writeln!(rodata, "  .quad {}", s.len());
            if !s.is_empty() {
                let parts: Vec<String> = s.bytes().map(|b| b.to_string()).collect();
                let _ = writeln!(rodata, "  .byte {}", parts.join(", "));
            }
        }
    }
    for t in &templates {
        let label = &template_ids[t];
        // A `format_args!` template: raw bytes with no length word. The runtime
        // walks it, reading a piece length from each piece's first byte, so the
        // first byte must be part of the encoding, not a word count.
        let _ = writeln!(rodata, ".balign 1");
        let _ = writeln!(rodata, "{label}:");
        let parts: Vec<String> = t.iter().map(|b| b.to_string()).collect();
        let _ = writeln!(rodata, "  .byte {}", parts.join(", "));
    }
    let mut alloc_ids: Vec<&u32> = prog.allocs.keys().collect();
    alloc_ids.sort();
    if !alloc_ids.is_empty() {
        if strings.is_empty() && templates.is_empty() {
            rodata.push_str("\n.section .rodata\n");
        }
        for &id in &alloc_ids {
            let data = &prog.allocs[id];
            let label = &alloc_labels[id];
            let _ = writeln!(rodata, "{label}:");
            // Emit a byte run between relocations, then the pointer word.
            let mut off = 0usize;
            let mut relocs = data.relocs.clone();
            relocs.sort_by_key(|(o, _)| *o);
            for (ro, target) in relocs {
                if ro > off {
                    let bytes: Vec<String> =
                        data.bytes[off..ro.min(data.bytes.len())].iter().map(|b| b.to_string()).collect();
                    let _ = writeln!(rodata, "  .byte {}", bytes.join(", "));
                }
                let tlabel = alloc_labels
                    .get(&target)
                    .cloned()
                    .unwrap_or_else(|| format!("__rmc_alloc{target}"));
                let _ = writeln!(rodata, "  .quad {tlabel}");
                off = ro + 8;
            }
            if off < data.bytes.len() {
                let bytes: Vec<String> = data.bytes[off..].iter().map(|b| b.to_string()).collect();
                let _ = writeln!(rodata, "  .byte {}", bytes.join(", "));
            }
        }
    }
    // Promoted constants: the scalar a `const main::promoted[N]` points at, as a
    // `.rodata` blob. A `&i32` referent is a word; a `&char` is four bytes, so
    // the blob is the referent's size, padded to a word.
    let mut promoted_names: Vec<&String> = prog.promoted.keys().collect();
    promoted_names.sort();
    if !promoted_names.is_empty() {
        if strings.is_empty() && templates.is_empty() && alloc_ids.is_empty() {
            rodata.push_str("\n.section .rodata\n");
        }
        for name in promoted_names {
            let p = &prog.promoted[name];
            let label = promoted_label(name);
            let _ = writeln!(rodata, ".balign 8");
            let _ = writeln!(rodata, "{label}:");
            let words = ((p.size + 7) / 8).max(1);
            let mut data = vec![0i64; words as usize];
            for (off, v) in &p.bytes {
                let w = (off / 8) as usize;
                if w < data.len() {
                    // A char is four bytes; the rest of its word stays zero.
                    if p.size == 4 {
                        data[w] = (*v as u32) as i64;
                    } else {
                        data[w] = *v;
                    }
                }
            }
            if let Some(s) = &p.str_ref {
                // The referent is a `&str`: the blob word is the address of the
                // string record, a relocation rather than a scalar.
                let _ = writeln!(rodata, "  .quad {}", string_ids[s]);
            } else {
                for v in &data {
                    let _ = writeln!(rodata, "  .quad {}", *v as u64);
                }
            }
        }
    }
    asm.push_str(&rodata);
    Ok(asm)
}

/// Functions reachable from `main` by direct calls.
fn reachable(prog: &Program) -> HashSet<String> {
    let mut live = HashSet::new();
    let Some(main) = prog.func("main") else { return live };
    let mut stack = vec![main.name.clone()];
    while let Some(name) = stack.pop() {
        if !live.insert(name.clone()) {
            continue;
        }
        if let Some(f) = prog.func(&name) {
            for b in &f.blocks {
                if let Terminator::Call { func: CallTarget::Direct(callee), .. } = &b.term {
                    if prog.func(callee).is_some() {
                        stack.push(callee.clone());
                    }
                }
            }
        }
    }
    live
}

/// Whether `name` can reach a call to `callee` through direct calls.
fn func_calls(prog: &Program, name: &str, callee: &str) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![name.to_string()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        if n == callee {
            return true;
        }
        if let Some(f) = prog.func(&n) {
            for b in &f.blocks {
                if let Terminator::Call { func: CallTarget::Direct(c), .. } = &b.term {
                    stack.push(c.clone());
                }
            }
        }
    }
    false
}

fn is_unsupported_builtin(name: &str) -> bool {
    name.contains("::") || name.starts_with('<') || name == "rust_begin_unwind"
}

/// Append the user-visible feature name to a back-end error, when the rejected
/// fragment matches a known Rust construct. Keeps the raw MIR in the message
/// (useful in a bug report) but tells the user what they actually wrote.
fn unsupported_note(msg: &str, fragment: &str) -> String {
    match crate::mir::classify_unsupported(fragment) {
        Some(h) => format!("{msg}\n  note: {h}"),
        None => msg.to_string(),
    }
}

/// The System V integer argument registers, in order.
const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

/// A reified formatter, reduced to the runtime helper that implements it. The
/// type text is matched loosely (a `&str` and a `&&str` share the string
/// formatter; an integer keeps its width and signedness) and the trait selects
/// the numeric base.
fn resolve_fmt(ty_name: &str, trait_name: &str) -> &'static str {
    // A reference formats as the thing it points at (`&str`, `&&str`).
    let base = ty_name.trim().trim_start_matches('&').trim_start_matches("mut ").trim();
    let debug = trait_name.contains("Debug");
    let is_str = base == "str" || base.starts_with("[u8]") || base.starts_with("[u8;");
    if is_str {
        return if debug { "__rmc_fmt_str_debug" } else { "__rmc_fmt_str" };
    }
    // Integers keep their width and signedness. `char`/`bool` have their own
    // helpers; every other scalar (a `usize` width, a pointer) is formatted
    // as a 64-bit value.
    let (signed, bits) = int_kind(base);
    if base == "bool" {
        return "__rmc_fmt_bool";
    }
    if base == "char" {
        return "__rmc_fmt_char";
    }
    let hex = trait_name.contains("Hex");
    let upper = trait_name.contains("UpperHex");
    let oct = trait_name.contains("Octal");
    let bin = trait_name.contains("Binary");
    if hex || oct || bin {
        // Hex/octal/binary print the value as an unsigned integer of its own
        // width (Rust: `{:x}` of `-2i16` is `fffe`), so the runtime has one
        // entry point per width and it loads exactly that many bytes. `usize`
        // and the non-integer scalars are 64-bit. `{:X}` shares the hex entry
        // points' shape but a distinct symbol, since the alphabet differs.
        let base = if hex {
            if upper {
                "HEX"
            } else {
                "hex"
            }
        } else if oct {
            "oct"
        } else {
            "bin"
        };
        let width = match bits {
            8 => 8,
            16 => 16,
            32 => 32,
            _ => 64,
        };
        return match (base, width) {
            ("hex", 8) => "__rmc_fmt_hex_8",
            ("hex", 16) => "__rmc_fmt_hex_16",
            ("hex", 32) => "__rmc_fmt_hex_32",
            ("hex", _) => "__rmc_fmt_hex_64",
            ("HEX", 8) => "__rmc_fmt_HEX_8",
            ("HEX", 16) => "__rmc_fmt_HEX_16",
            ("HEX", 32) => "__rmc_fmt_HEX_32",
            ("HEX", _) => "__rmc_fmt_HEX_64",
            ("oct", 8) => "__rmc_fmt_oct_8",
            ("oct", 16) => "__rmc_fmt_oct_16",
            ("oct", 32) => "__rmc_fmt_oct_32",
            ("oct", _) => "__rmc_fmt_oct_64",
            ("bin", 8) => "__rmc_fmt_bin_8",
            ("bin", 16) => "__rmc_fmt_bin_16",
            ("bin", 32) => "__rmc_fmt_bin_32",
            _ => "__rmc_fmt_bin_64",
        };
    }
    match signed {
        true if bits == 32 => "__rmc_fmt_i32",
        true => "__rmc_fmt_i64",
        false if bits == 32 => "__rmc_fmt_u32",
        false => "__rmc_fmt_u64",
    }
}

/// The signedness and bit width of an integer type name (`i32` -> signed 32,
/// `usize`/`u64` -> unsigned 64). A non-integer falls back to a signed word.
fn int_kind(name: &str) -> (bool, u32) {
    let n = name.trim();
    if let Some(rest) = n.strip_prefix('i') {
        if let Ok(bits) = rest.parse::<u32>() {
            return (true, bits);
        }
        if rest == "size" {
            return (true, 64);
        }
    }
    if let Some(rest) = n.strip_prefix('u') {
        if let Ok(bits) = rest.parse::<u32>() {
            return (false, bits);
        }
        if rest == "size" {
            return (false, 64);
        }
    }
    (true, 64)
}

/// The `.rodata` label for a promoted constant, derived from its MIR name
/// (`main::promoted[0]`) so it is stable and cannot collide with a user symbol.
fn promoted_label(name: &str) -> String {
    let mut out = String::from("__rmc_promoted_");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

/// Every distinct string literal in the program, in first-seen order, and a map
/// from its content to a `.rodata` label.
fn collect_strings(prog: &Program) -> (Vec<String>, HashMap<String, String>) {
    let mut order = Vec::new();
    let mut map = HashMap::new();
    for f in &prog.funcs {
        for b in &f.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    collect_strings_rv(rv, &mut order, &mut map);
                }
            }
        }
    }
    // A promoted `&&str` constant's literal appears only in the promoted body,
    // never in a function statement, so collect it here to give it a label.
    let mut names: Vec<&String> = prog.promoted.keys().collect();
    names.sort();
    for name in names {
        if let Some(s) = &prog.promoted[name].str_ref {
            if !map.contains_key(s) {
                map.insert(s.clone(), format!("__rmc_lit{}", map.len()));
                order.push(s.clone());
            }
        }
    }
    (order, map)
}

/// Walk a `format_args!` template and return a description of the first
/// placeholder that carries flags, a width or a precision, or `None` if every
/// placeholder is plain. The runtime walker skips those fields (it must, to
/// reach the argument index) but renders the value in full, so such a template
/// would be silently mis-rendered; the caller rejects it instead.
///
/// A placeholder byte is `0b11xxxxxx`; bits 0..3 mark a present `flags` (u32),
/// `width` (u16), `precision` (u16) and `arg_index` (u16). Only the optional
/// `arg_index` is harmless.
fn template_unrendered_field(t: &[u8]) -> Option<&'static str> {
    let mut i = 0;
    while i < t.len() {
        let b = t[i];
        if b < 0x80 {
            i += 1 + b as usize; // literal piece: length byte, then the bytes
        } else if b < 0xC0 {
            if i + 3 > t.len() {
                return None;
            }
            let len = t[i + 1] as usize | ((t[i + 2] as usize) << 8);
            i += 3 + len; // long literal: 0x80, u16 length, then the bytes
        } else {
            let bits = b & 0x0F;
            if bits & 0x01 != 0 {
                return Some("flags");
            }
            if bits & 0x02 != 0 {
                return Some("width");
            }
            if bits & 0x04 != 0 {
                return Some("precision");
            }
            i += 1;
            if bits & 0x08 != 0 {
                i += 2; // arg_index
            }
        }
    }
    None
}

/// Every distinct `format_args!` template byte string, in first-seen order, and
/// a map from its bytes to a `.rodata` label.
fn collect_templates(prog: &Program) -> (Vec<Vec<u8>>, HashMap<Vec<u8>, String>) {
    let mut order = Vec::new();
    let mut map = HashMap::new();
    for f in &prog.funcs {
        for b in &f.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    collect_templates_rv(rv, &mut order, &mut map);
                }
            }
        }
    }
    (order, map)
}

fn collect_templates_rv(rv: &Rvalue, order: &mut Vec<Vec<u8>>, map: &mut HashMap<Vec<u8>, String>) {
    let mut add = |c: &Const| {
        if let Const::Bytes(b) = c {
            if !map.contains_key(b) {
                map.insert(b.clone(), format!("__rmc_tmpl{}", map.len()));
                order.push(b.clone());
            }
        }
    };
    let mut visit = |op: &Operand| {
        if let Operand::Const(c) = op {
            add(c)
        }
    };
    match rv {
        Rvalue::Use(op) => visit(op),
        Rvalue::Binary(_, a, b) => {
            visit(a);
            visit(b);
        }
        Rvalue::Unary(_, a) => visit(a),
        Rvalue::Cast { op, .. } => visit(op),
        Rvalue::Repeat { op, .. } => visit(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            ops.iter().for_each(|op| visit(op))
        }
        _ => {}
    }
}

/// Struct name -> field types, learned from aggregate constructions.
fn collect_structs(prog: &Program) -> HashMap<String, Vec<Ty>> {
    let mut structs: HashMap<String, Vec<Ty>> = HashMap::new();
    for f in &prog.funcs {
        for l in &f.locals {
            if let Ty::Struct(name) = &*l.ty {
                structs.entry(name.clone()).or_default();
            }
        }
        for b in &f.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(dest, rv) = s {
                    if let Rvalue::Aggregate(ops) = rv {
                        if let Some(Ty::Struct(name)) =
                            f.locals.get(dest.local as usize).map(|l| (*l.ty).clone())
                        {
                            let tys = ops.iter().map(|op| rv_operand_ty(f, op)).collect();
                            structs.insert(name, tys);
                        }
                    }
                    // An aggregate *constant* (`const P { .. }`) carries its own
                    // field list, so learn the struct's shape from it too.
                    let mut aggs = Vec::new();
                    rv_const_aggs(rv, &mut aggs);
                    for (name, ops) in aggs {
                        if !name.is_empty() && !name.contains("::") {
                            let tys = ops.iter().map(|op| rv_operand_ty(f, op)).collect();
                            structs.insert(name, tys);
                        }
                    }
                }
            }
        }
    }
    structs
}

/// Every `Const::Agg` operand in `rv`, as `(name, ops)`, including nested ones.
fn rv_const_aggs(rv: &Rvalue, out: &mut Vec<(String, Vec<Operand>)>) {
    let mut visit = |op: &Operand| op_const_aggs(op, out);
    match rv {
        Rvalue::Use(op) => visit(op),
        Rvalue::Binary(_, a, b) => {
            visit(a);
            visit(b);
        }
        Rvalue::Unary(_, a) => visit(a),
        Rvalue::Cast { op, .. } => visit(op),
        Rvalue::Repeat { op, .. } => visit(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            ops.iter().for_each(|op| visit(op))
        }
        _ => {}
    }
}

fn op_const_aggs(op: &Operand, out: &mut Vec<(String, Vec<Operand>)>) {
    if let Operand::Const(Const::Agg { name, ops }) = op {
        out.push((name.clone(), ops.clone()));
        for o in ops {
            op_const_aggs(o, out);
        }
    }
}

/// How an enum is laid out: the tag value of each variant, its payload fields,
/// and where the tag and payload sit in the value.
#[derive(Debug, Clone, Default)]
struct EnumLayout {
    /// Variant name -> its tag value.
    tags: HashMap<String, u64>,
    /// Variant name -> payload field types.
    fields: HashMap<String, Vec<Ty>>,
    /// Byte offset of the tag.
    tag_off: i64,
    /// Byte offset of a payload variant's fields.
    payload_off: i64,
    /// Total byte size of the value.
    size: i64,
    /// True when every variant is a unit variant (the value is just the tag).
    fieldless: bool,
}

/// Split `Enum::Variant` or `Option::<i64>::Some` into the enum and variant
/// names, dropping any generic arguments.
fn split_ev(name: &str) -> (String, String) {
    let idx = name.rfind("::").unwrap_or(0);
    let variant = name[idx + 2..].trim().to_string();
    let head = &name[..idx];
    let head = head.split("::<").next().unwrap_or(head);
    let head = head.rsplit("::").next().unwrap_or(head);
    (head.trim().to_string(), variant)
}

/// Learn each enum's layout from how the program uses it: its constructions
/// give the payload field types, and a `discriminant` switch gives the tag
/// values. Only the shapes the backend models are described; an enum with a
/// payload field narrower than a word is left out (and rejected at codegen).
fn collect_enums(prog: &Program) -> HashMap<String, EnumLayout> {
    let mut fields: HashMap<String, HashMap<String, Vec<Ty>>> = HashMap::new();
    let mut order: HashMap<String, Vec<String>> = HashMap::new();
    for f in &prog.funcs {
        for b in &f.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    // Both an explicit construction (`E::B(..)`) and an inline
                    // constant (`const E::A`) name a variant and its payload.
                    if let Rvalue::Enum { name, ops } = rv {
                        let (e, v) = split_ev(name);
                        let tys: Vec<Ty> = ops.iter().map(|op| rv_operand_ty(f, op)).collect();
                        fields.entry(e.clone()).or_default().entry(v.clone()).or_insert(tys);
                        let ord = order.entry(e).or_default();
                        if !ord.contains(&v) {
                            ord.push(v);
                        }
                    }
                    let mut aggs = Vec::new();
                    rv_const_aggs(rv, &mut aggs);
                    for (name, ops) in aggs {
                        if !name.contains("::") {
                            continue;
                        }
                        let (e, v) = split_ev(&name);
                        let tys: Vec<Ty> = ops.iter().map(|op| rv_operand_ty(f, op)).collect();
                        fields.entry(e.clone()).or_default().entry(v.clone()).or_insert(tys);
                        let ord = order.entry(e).or_default();
                        if !ord.contains(&v) {
                            ord.push(v);
                        }
                    }
                }
            }
        }
    }
    // Tag values from `_d = discriminant(_x); switchInt(_d) -> [tag: arm, ...]`.
    let mut tags: HashMap<String, HashMap<String, u64>> = HashMap::new();
    for f in &prog.funcs {
        for b in &f.blocks {
            let Some((enum_name, disc_local)) = block_discriminant(f, b) else { continue };
            let Terminator::SwitchInt { discr, targets, .. } = &b.term else { continue };
            if operand_local(discr) != Some(disc_local) {
                continue;
            }
            // Each arm's tag is the switch value; the variant it handles is the
            // one whose payload the arm (or the blocks it falls through to)
            // reads. A fieldless variant reads none.
            let arm_blocks: Vec<u32> = targets.iter().map(|(_, t)| *t).collect();
            let mut fieldless_tags: Vec<u64> = Vec::new();
            for (tag, target) in targets {
                match arm_variant(f, *target, &arm_blocks) {
                    Some(v) => {
                        tags.entry(enum_name.clone()).or_default().insert(v, *tag as u64);
                    }
                    None => fieldless_tags.push(*tag as u64),
                }
            }
            // Fieldless variants take the remaining tag values in the order they
            // are constructed, which matches declaration order for the shapes
            // this backend accepts.
            if let Some(ord) = order.get(&enum_name) {
                let mut ft = fieldless_tags.into_iter();
                for v in ord {
                    let has_payload = fields
                        .get(&enum_name)
                        .and_then(|m| m.get(v))
                        .map(|t| !t.is_empty())
                        .unwrap_or(false);
                    if has_payload || tags.get(&enum_name).map(|m| m.contains_key(v)).unwrap_or(false) {
                        continue;
                    }
                    if let Some(t) = ft.next() {
                        tags.entry(enum_name.clone()).or_default().insert(v.clone(), t);
                    }
                }
            }
        }
    }

    let mut enums = HashMap::new();
    for (name, variants) in &fields {
        let fieldless = variants.values().all(|tys| tys.is_empty());
        let mut tag_map: HashMap<String, u64> = tags.remove(name).unwrap_or_default();
        // Assign any variant without a tag a fresh value, in construction order.
        let mut next = tag_map.values().copied().max().map(|m| m + 1).unwrap_or(0);
        for v in order.get(name).cloned().unwrap_or_default() {
            tag_map.entry(v).or_insert_with(|| {
                let t = next;
                next += 1;
                t
            });
        }
        // The backend models a payload enum only when every payload field is one
        // word wide: the tag is then one word and the payload follows it.
        let modelled = fieldless
            || variants.values().all(|tys| tys.iter().all(|t| scalar_word(t)));
        let (payload_off, size) = if fieldless {
            (0, 8)
        } else if modelled {
            (8, 16)
        } else {
            (0, 0)
        };
        enums.insert(
            name.clone(),
            EnumLayout {
                tags: tag_map,
                fields: variants.clone(),
                tag_off: 0,
                payload_off,
                size,
                fieldless,
            },
        );
    }
    // An enum seen only through `discriminant` (never constructed) still needs
    // an entry so its size is known.
    for (name, tag_map) in tags {
        enums.entry(name).or_insert_with(|| EnumLayout {
            tags: tag_map,
            fields: HashMap::new(),
            tag_off: 0,
            payload_off: 0,
            size: 8,
            fieldless: true,
        });
    }
    enums
}

/// A field type the backend can store in one word.
fn scalar_word(t: &Ty) -> bool {
    matches!(t, Ty::I64 | Ty::Bool | Ty::Int | Ty::Ptr(_) | Ty::Ref(_) | Ty::Unit | Ty::Other)
}

/// The `(_d = discriminant(_x))` in `b`, as `(enum name, _d)`.
fn block_discriminant(f: &Function, b: &BasicBlock) -> Option<(String, u32)> {
    for s in &b.stmts {
        if let Stmt::Assign(d, Rvalue::Discriminant(p)) = s {
            if let Some(Ty::Struct(en)) = f.locals.get(p.local as usize).map(|l| &*l.ty) {
                return Some((en.clone(), d.local));
            }
        }
    }
    None
}

/// The variant an arm block handles, by the payload it reads. `None` when the
/// arm reads no payload (a fieldless variant).
fn arm_variant(f: &Function, start: u32, arms: &[u32]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![start];
    while let Some(b) = stack.pop() {
        if !seen.insert(b) {
            continue;
        }
        let Some(block) = f.blocks.get(b as usize) else { continue };
        for s in &block.stmts {
            if let Stmt::Assign(_, rv) = s {
                if let Some(v) = rv_variant(rv) {
                    return Some(v);
                }
            }
        }
        match &block.term {
            Terminator::Goto(next) => {
                // Stop at a join: a block that another arm also reaches.
                if *next != start && !arms.contains(next) {
                    stack.push(*next);
                }
            }
            _ => {}
        }
    }
    None
}

/// The variant name a rvalue reads through `(x as Variant)`, if any.
fn rv_variant(rv: &Rvalue) -> Option<String> {
    let ops: Vec<&Operand> = match rv {
        Rvalue::Use(op) => vec![op],
        Rvalue::Binary(_, a, b) => vec![a, b],
        Rvalue::Unary(_, a) => vec![a],
        Rvalue::Cast { op, .. } => vec![op],
        Rvalue::Repeat { op, .. } => vec![op],
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => ops.iter().collect(),
        Rvalue::Enum { ops, .. } => ops.iter().collect(),
        _ => Vec::new(),
    };
    for op in ops {
        if let Operand::Copy(p) | Operand::Move(p) = op {
            for proj in &p.proj {
                if let Proj::Variant(v) = proj {
                    return Some(v.clone());
                }
            }
        }
    }
    None
}

fn operand_local(op: &Operand) -> Option<u32> {
    match op {
        Operand::Copy(p) | Operand::Move(p) if p.proj.is_empty() => Some(p.local),
        _ => None,
    }
}

/// The type of an operand, from its local's declared type when it names one.
fn rv_operand_ty(f: &Function, op: &Operand) -> Ty {
    match op {
        Operand::Copy(p) | Operand::Move(p) => {
            f.locals.get(p.local as usize).map(|l| (*l.ty).clone()).unwrap_or(Ty::I64)
        }
        Operand::Const(Const::Bool(_)) => Ty::Bool,
        Operand::Const(Const::Zst) => Ty::Unit,
        Operand::Const(Const::Agg { name, .. }) => {
            if name.is_empty() {
                Ty::I64
            } else {
                Ty::Struct(name.clone())
            }
        }
        _ => Ty::I64,
    }
}

fn collect_strings_rv(rv: &Rvalue, order: &mut Vec<String>, map: &mut HashMap<String, String>) {
    let mut add = |c: &Const| {
        if let Const::Str(s) = c {
            if !map.contains_key(s) {
                map.insert(s.clone(), format!("__rmc_lit{}", map.len()));
                order.push(s.clone());
            }
        }
    };
    match rv {
        Rvalue::Use(op) => {
            if let Operand::Const(c) = op {
                add(c)
            }
        }
        Rvalue::Binary(_, a, b) => {
            if let Operand::Const(c) = a {
                add(c)
            }
            if let Operand::Const(c) = b {
                add(c)
            }
        }
        Rvalue::Unary(_, a) => {
            if let Operand::Const(c) = a {
                add(c)
            }
        }
        Rvalue::Cast { op, .. } => {
            if let Operand::Const(c) = op {
                add(c)
            }
        }
        Rvalue::Repeat { op, .. } => {
            if let Operand::Const(c) = op {
                add(c)
            }
        }
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            for op in ops {
                if let Operand::Const(c) = op {
                    add(c)
                }
            }
        }
        _ => {}
    }
}

impl<'a> FnGen<'a> {
    fn new(
        prog: &'a Program,
        func: &'a Function,
        opts: &'a Options,
        string_ids: &'a HashMap<String, String>,
        template_ids: &'a HashMap<Vec<u8>, String>,
        alloc_labels: &'a HashMap<u32, String>,
        structs: &'a HashMap<String, Vec<Ty>>,
        enums: &'a HashMap<String, EnumLayout>,
    ) -> Self {
        FnGen {
            prog,
            func,
            opts,
            string_ids,
            template_ids,
            alloc_labels,
            structs,
            enums,
            size: Vec::new(),
            ty: Vec::new(),
            local_base: Vec::new(),
            promo: HashMap::new(),
            shadow: HashMap::new(),
            caller_spill: HashMap::new(),
            loop_operands: Vec::new(),
            allocates: func_calls(prog, &func.name, "alloc"),
            saved_regs: 0,
            slot_map: None,
            next_slot: 0,
            out: String::new(),
            label: 0,
            errors: Vec::new(),
        }
    }

    fn emit(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn new_label(&mut self) -> String {
        self.label += 1;
        format!(".Lrmc{}", self.label)
    }

    fn alloc_slot(&mut self) -> i64 {
        let s = self.next_slot;
        self.next_slot += 1;
        s
    }

    fn off(&self, slot: i64) -> String {
        let slot = match &self.slot_map {
            Some(m) => m.get(slot as usize).copied().unwrap_or(slot),
            None => slot,
        };
        format!("{}(%rbp)", -(slot + 1 + self.saved_regs) * 8)
    }

    /// The slot index at which `local`'s storage begins.
    fn local_base(&self, local: u32) -> i64 {
        self.local_base.get(local as usize).copied().unwrap_or(local as i64)
    }

    fn local_size(&self, local: u32) -> i64 {
        self.size.get(local as usize).copied().unwrap_or(8)
    }

    fn local_ty(&self, local: u32) -> Ty {
        self.ty.get(local as usize).cloned().unwrap_or(Ty::I64)
    }

    /// A scalar local's home: its promoted register or its frame slot.
    fn local_operand(&self, local: u32) -> String {
        match self.promo.get(&local) {
            Some(p) => p.reg.to_string(),
            None => self.off(self.local_base(local)),
        }
    }

    /// True when a scalar local is held in a register (so `p.proj` is empty and
    /// reading it needs no memory access).
    fn place_is_promoted(&self, p: &Place) -> bool {
        p.proj.is_empty() && self.promo.contains_key(&p.local)
    }

    // --- analysis ----------------------------------------------------------

    /// Compute each local's size and type, and find the operands of counted
    /// loop tests (`switchInt(Lt(counter, bound))`).
    fn analyze(&mut self) {
        // Give every local its own region sized to its type: an aggregate such
        // as `[i64; 200000]` needs 1.6 MB of frame, not one 8-byte slot. A
        // scalar takes one slot. Regions are laid out first so `local_base` can
        // be indexed directly.
        let mut slot = 0i64;
        for l in &self.func.locals {
            let size = self.size_of_ty(&l.ty);
            let slots = ((size + 7) / 8).max(1);
            self.ty.push((*l.ty).clone());
            self.size.push(size);
            // Frame slots grow downward from `%rbp`, but an aggregate is
            // addressed upward from its base, so the base is the region's
            // *lowest* slot (the highest index). `off` then names the bottom of
            // the region and a projection adds its byte offset.
            self.local_base.push(slot + slots - 1);
            slot += slots;
        }
        self.next_slot = slot;
        // For each `switchInt(cmp) -> [0: exit, otherwise: body]`, the `cmp` is
        // a comparison assignment in the same block; its operands are the loop
        // counter and bound.
        for b in &self.func.blocks {
            let Terminator::SwitchInt { discr, targets, .. } = &b.term else { continue };
            if targets.len() != 1 {
                continue;
            }
            let Some(pl) = operand_place(discr) else { continue };
            if !pl.proj.is_empty() {
                continue;
            }
            for s in &b.stmts {
                if let Stmt::Assign(p, Rvalue::Binary(op, a, c)) = s {
                    if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                        for op_ in [a, c] {
                            if let Operand::Copy(q) | Operand::Move(q) = op_ {
                                if q.proj.is_empty() {
                                    self.loop_operands.push(q.local);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Promote the operands of counted loop tests, then the hottest remaining
    /// scalars, into callee-saved registers.
    fn plan_promotion(&mut self) {
        if !self.opts.promote {
            return;
        }
        let mut used: HashSet<&'static str> = HashSet::new();
        // Loop counters and bounds first: they are touched every iteration.
        for l in self.loop_operands.clone() {
            self.promote_local(l, &mut used);
        }
        // A value computed before a loop and read inside it is loop-invariant:
        // keeping it in a register removes a load from every iteration. These
        // are promoted ahead of the general ranking, because the general count
        // weighs a value read once per iteration the same as one read once.
        let depths = self.loop_depths();
        // A local's definition depth and its deepest use. A local that is only
        // ever defined above a loop and read inside it is loop-invariant for
        // that loop: it is worth a register even when a shallower loop holds
        // the rest of the budget, because it saves a load on every iteration.
        let mut def_depth: HashMap<u32, u32> = HashMap::new();
        let mut use_depth: HashMap<u32, u32> = HashMap::new();
        let mut use_count: HashMap<u32, u32> = HashMap::new();
        for (bi, b) in self.func.blocks.iter().enumerate() {
            let d = depths.get(bi).copied().unwrap_or(0);
            for s in &b.stmts {
                if let Stmt::Assign(p, rv) = s {
                    if p.proj.is_empty() {
                        def_depth
                            .entry(p.local)
                            .and_modify(|v| *v = (*v).max(d))
                            .or_insert(d);
                    }
                    let mut lc: HashMap<u32, u32> = HashMap::new();
                    let mut pj: HashSet<u32> = HashSet::new();
                    let mut at: HashSet<u32> = HashSet::new();
                    scan_rv(rv, &mut lc, &mut pj, &mut at);
                    for (l, c) in lc {
                        use_depth
                            .entry(l)
                            .and_modify(|v| *v = (*v).max(d))
                            .or_insert(d);
                        *use_count.entry(l).or_insert(0) += c;
                    }
                }
            }
            let mut lc: HashMap<u32, u32> = HashMap::new();
            let mut pj: HashSet<u32> = HashSet::new();
            let mut at: HashSet<u32> = HashSet::new();
            scan_term(&b.term, &mut lc, &mut pj, &mut at);
            for (l, c) in lc {
                use_depth
                    .entry(l)
                    .and_modify(|v| *v = (*v).max(d))
                    .or_insert(d);
                *use_count.entry(l).or_insert(0) += c;
            }
            if let Terminator::Call { dest, .. } = &b.term {
                if dest.proj.is_empty() {
                    def_depth
                        .entry(dest.local)
                        .and_modify(|v| *v = (*v).max(d))
                        .or_insert(d);
                }
            }
        }
        let mut invariant: Vec<(u32, u32, u32)> = use_depth
            .iter()
            .filter(|(l, ud)| **ud > def_depth.get(l).copied().unwrap_or(0))
            .filter(|(l, _)| !self.promo.contains_key(l))
            .filter(|(l, _)| !self.ty.get(**l as usize).map(|t| t.is_aggregate()).unwrap_or(false))
            .map(|(l, ud)| (*l, *ud, use_count.get(l).copied().unwrap_or(1)))
            .collect();
        invariant.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        let pool = self.reg_pool();
        for (l, _, _) in invariant {
            if self.promo.len() >= pool.len() {
                break;
            }
            self.promote_local(l, &mut used);
        }

        // Then the hottest remaining scalar locals. A local with a projection
        // (a struct field, an array element) is left in memory: promoting it
        // would mean tracking its address, which the cheap rules here do not.
        //
        // A use inside a loop counts more than one outside it, weighted by the
        // loop's nesting depth: a value carried across a hot loop is worth a
        // callee-saved register even when it is named less often overall than a
        // one-shot temporary, which the frame absorbs for free.
        let mut counts: HashMap<u32, u32> = HashMap::new();
        let mut weighted: HashMap<u32, u32> = HashMap::new();
        let mut projected: HashSet<u32> = HashSet::new();
        let mut addr_taken: HashSet<u32> = HashSet::new();
        for (bi, b) in self.func.blocks.iter().enumerate() {
            let w = depths.get(bi).copied().unwrap_or(0) + 1;
            let mut local_counts: HashMap<u32, u32> = HashMap::new();
            for s in &b.stmts {
                if let Stmt::Assign(p, rv) = s {
                    if !p.proj.is_empty() {
                        projected.insert(p.local);
                    }
                    scan_rv(rv, &mut local_counts, &mut projected, &mut addr_taken);
                }
            }
            scan_term(&b.term, &mut local_counts, &mut projected, &mut addr_taken);
            for (l, c) in local_counts {
                *counts.entry(l).or_insert(0) += c;
                *weighted.entry(l).or_insert(0) += c * w;
            }
        }
        let mut ranked: Vec<(u32, u32, u32)> = counts
            .into_iter()
            .filter(|(l, c)| {
                *c >= 2 && !projected.contains(l) && !addr_taken.contains(l)
            })
            .filter(|(l, _)| !self.promo.contains_key(l))
            .filter(|(l, _)| !self.size.get(*l as usize).copied().unwrap_or(8).eq(&8) || {
                !self.ty.get(*l as usize).map(|t| t.is_aggregate()).unwrap_or(false)
            })
            .map(|(l, c)| (l, weighted.get(&l).copied().unwrap_or(c), c))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)).then(a.0.cmp(&b.0)));
        let pool = self.reg_pool();
        for (l, _, _) in ranked {
            if self.promo.len() >= pool.len() {
                break;
            }
            self.promote_local(l, &mut used);
        }
    }

    /// The nesting depth of each block: how many natural loops contain it.
    ///
    /// A back edge `latch -> header` (the header dominates the latch) defines a
    /// natural loop as the header plus every block that both the header reaches
    /// and that reaches the latch. Blocks in several loops get a higher depth, so
    /// a use in a doubly-nested inner loop outweighs one in the loop around it.
    fn loop_depths(&self) -> Vec<u32> {
        let n = self.func.blocks.len();
        let succ: Vec<Vec<u32>> = self
            .func
            .blocks
            .iter()
            .map(|b| successors(&b.term).into_iter().filter(|s| (*s as usize) < n).collect())
            .collect();
        // Transitive closure: `reach[i][j]` when `i` can reach `j`.
        let mut reach: Vec<Vec<bool>> = vec![vec![false; n]; n];
        for (i, s) in succ.iter().enumerate() {
            for &t in s {
                reach[i][t as usize] = true;
            }
        }
        for k in 0..n {
            for i in 0..n {
                if reach[i][k] {
                    for j in 0..n {
                        if reach[k][j] {
                            reach[i][j] = true;
                        }
                    }
                }
            }
        }
        let mut depths = vec![0u32; n];
        for (latch, s) in succ.iter().enumerate() {
            for &header in s {
                let h = header as usize;
                if h == latch || !reach[h][latch] {
                    continue;
                }
                // The natural loop of `h`: every block `x` with
                // `h` reaches `x` and `x` reaches `latch` (plus the header).
                for x in 0..n {
                    if x == h || (reach[h][x] && reach[x][latch]) {
                        depths[x] += 1;
                    }
                }
            }
        }
        depths
    }

    /// The registers this function may promote into. Caller-saved registers are
    /// usable too; the generator spills them around any call. A function that
    /// allocates gets only the callee-saved registers, whose values survive
    /// `alloc`: the heap pointer lives in a caller-saved register, and a
    /// promotion would keep a stale copy across the call.
    fn reg_pool(&self) -> Vec<&'static str> {
        let mut pool: Vec<&'static str> = PROMOTE_REGS.to_vec();
        if !self.allocates {
            pool.extend_from_slice(&CALLER_REGS);
        }
        pool
    }

    fn promote_local(&mut self, local: u32, used: &mut HashSet<&'static str>) {
        if self.promo.contains_key(&local) {
            return;
        }
        let ty = self.ty.get(local as usize).cloned().unwrap_or(Ty::I64);
        if ty.is_aggregate() {
            return;
        }
        // An argument is copied out of its ABI register at entry, in parallel,
        // so a caller-saved register cannot be its destination: it may still
        // hold a later argument that has not been moved yet. Restrict arguments
        // to the callee-saved pool, which never overlaps the argument registers.
        let is_arg = self.func.locals.get(local as usize).map(|l| l.is_arg).unwrap_or(false);
        let pool = if is_arg { PROMOTE_REGS.to_vec() } else { self.reg_pool() };
        let Some(reg) = pool.iter().find(|r| !used.contains(*r)).copied() else {
            return;
        };
        used.insert(reg);
        let caller_saved = CALLER_REGS.contains(&reg);
        self.promo.insert(local, Promo { reg, caller_saved });
    }

    // --- function ----------------------------------------------------------

    fn gen_fn(&mut self) -> Result<String, String> {
        self.analyze();
        self.plan_promotion();
        // A shadow slot for each promoted local, so taking its address has a
        // stable home. Allocated once, before the frame is sized.
        for l in 0..self.func.locals.len() as u32 {
            if self.promo.contains_key(&l) {
                let s = self.alloc_slot();
                self.shadow.insert(l, s);
            }
        }
        // A promoted caller-saved local needs a home across a call.
        for l in 0..self.func.locals.len() as u32 {
            if self.promo.get(&l).map(|p| p.caller_saved).unwrap_or(false) {
                let s = self.alloc_slot();
                self.caller_spill.insert(l, s);
            }
        }
        let base_slots = self.next_slot;

        // Dry pass: discover which frame slots and which callee-saved registers
        // the body actually uses, then run the real pass against a frame packed
        // with only those. A promoted local keeps its value in a register, so
        // its frame region is dead unless its address is taken; dropping the
        // dead slots and unused saved registers shrinks every frame, which is
        // what a recursive call pays for at each level.
        self.gen_body()?;
        let raw = std::mem::take(&mut self.out);
        let frame_slots = self.next_slot;
        // Slots come from the raw body: the peephole passes fold `(%rbp)`
        // operands into register-based addresses, which would hide a live slot.
        // A slot the raw body only ever stores to and never reads is dead, and
        // stays dead: the peephole passes rewrite reads, they never create one.
        // The optimised body is scanned whole -- a folded address may name a
        // slot the raw text no longer does.
        let mut used_slots = read_slots(&raw);
        // Registers come from the optimised body: coalescing can free a
        // callee-saved register the raw body named, and a `push`/`pop` for a
        // register nothing reads is pure overhead in every call.
        let opt = optimize(&raw);
        used_slots.extend(rbp_slots(&opt));
        // An aggregate's slots are reached as `base + k`, so only the base
        // appears in the text. Pin every slot of a multi-slot region: they must
        // stay contiguous, or an element access would fall outside the frame.
        // A `[i64; 200000]` local is 250k slots, so mark the runs directly
        // instead of hashing each slot -- that loop was the whole compile cost
        // of a large-array program.
        let mut used = vec![false; frame_slots as usize];
        for &s in &used_slots {
            if let Some(b) = used.get_mut(s as usize) {
                *b = true;
            }
        }
        for l in 0..self.func.locals.len() {
            let slots = (self.size[l] + 7) / 8;
            if slots > 1 {
                let base = self.local_base[l];
                let lo = (base - slots + 1).max(0) as usize;
                let hi = (base.min(frame_slots - 1)) as usize;
                if lo <= hi {
                    used[lo..=hi].fill(true);
                }
            }
        }
        let mut map = vec![-1i64; frame_slots as usize];
        let mut next = 0i64;
        for (slot, &is_used) in used.iter().enumerate() {
            if is_used {
                map[slot] = next;
                next += 1;
            }
        }
        let packed_slots = next;
        self.slot_map = Some(map);
        self.label = 0;
        self.next_slot = base_slots;

        // Save the callee-saved registers the body actually names (the ABI
        // requires it). A caller-saved register is preserved by the caller, so
        // it is spilled only around a call (see `gen_call`).
        let saved: Vec<&'static str> = PROMOTE_REGS
            .iter()
            .copied()
            .filter(|r| reg_mentions(&opt, r))
            .collect();
        // The saved registers sit below `%rbp`, so every frame slot is shifted
        // past them; `off` reads this.
        self.saved_regs = saved.len() as i64;
        let _ = writeln!(self.out, ".globl {}", self.func.name);
        let _ = writeln!(self.out, "{}:", self.func.name);
        self.out.push_str("  push %rbp\n  mov %rsp, %rbp\n");
        for r in &saved {
            let _ = writeln!(self.out, "  push {r}");
        }
        // Keep `%rsp` 16-byte aligned at every call: the saved registers plus
        // the frame must together be a multiple of 16.
        let saved_bytes = self.saved_regs * 8;
        let frame_bytes = align_up(packed_slots * 8 + saved_bytes, 16) - saved_bytes;
        if frame_bytes > 0 {
            let _ = writeln!(self.out, "  sub ${frame_bytes}, %rsp");
        }

        self.gen_body()?;

        // Epilogue: restore the saved registers and return `%rax`. The saved
        // registers live directly below `%rbp`, but `%rsp` is still at the
        // bottom of the frame, so it has to be moved back up first.
        let _ = writeln!(self.out, ".Lret_{}:", self.func.name);
        // `_start` passes `main`'s `%rax` straight to `exit(2)`, so a `main`
        // that returns `()` must define it. The ABI leaves `%rax` unspecified
        // for a unit return, so without this the process exits with whatever
        // the last statement happened to leave in the register.
        if self.func.name == "main" && self.local_ty(0) == Ty::Unit {
            self.out.push_str("  xor %eax, %eax\n");
        }
        if saved_bytes > 0 {
            let _ = writeln!(self.out, "  lea -{saved_bytes}(%rbp), %rsp");
        }
        for r in saved.iter().rev() {
            let _ = writeln!(self.out, "  pop {r}");
        }
        self.out.push_str("  leave\n  ret\n");
        Ok(optimize(&std::mem::take(&mut self.out)))
    }

    /// Argument setup plus every basic block. Run twice (dry, then real).
    fn gen_body(&mut self) -> Result<(), String> {
        // MIR numbers arguments from `_1`, so the register is chosen by the
        // argument's ordinal, not by its local index.
        let mut args: Vec<u32> = (0..self.func.locals.len() as u32)
            .filter(|i| self.func.locals[*i as usize].is_arg)
            .collect();
        args.sort_unstable();
        for (ord, li) in args.into_iter().enumerate() {
            if ord >= ARG_REGS.len() {
                return Err(format!("mir_codegen: {} takes more than 6 arguments", self.func.name));
            }
            let dst = self.local_operand(li);
            let _ = writeln!(self.out, "  mov {}, {}", ARG_REGS[ord], dst);
        }
        for i in 0..self.func.blocks.len() as u32 {
            // Align a loop header: the target of a back edge (an edge from a
            // later block). Aligning it to a cache line keeps the hot loop's
            // instructions from straddling a boundary, which matters most for a
            // tight inner loop.
            if self.opts.align_loops && self.is_loop_header(i) {
                let _ = writeln!(self.out, "  .p2align 4");
            }
            let _ = writeln!(self.out, ".Lbb{}:", self.bb_label(i));
            self.gen_block(i)?;
        }
        Ok(())
    }

    /// True when `bb` is the target of an edge from a later block — the top of a
    /// loop, given that the layout puts a loop's body after its header.
    fn is_loop_header(&self, bb: u32) -> bool {
        self.func.blocks.iter().enumerate().any(|(i, b)| {
            i as u32 > bb && successors(&b.term).contains(&bb)
        })
    }

    /// A basic-block label, qualified by the function name so that labels from
    /// different functions cannot collide (assembler-local `.L` labels share
    /// one namespace across the whole translation unit).
    fn bb_label(&self, bb: u32) -> String {
        let mut f = String::with_capacity(self.func.name.len() + 8);
        for c in self.func.name.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                f.push(c);
            } else {
                f.push('_');
            }
        }
        format!("{f}_{bb}")
    }

    fn gen_block(&mut self, bb: u32) -> Result<(), String> {
        let block = &self.func.blocks[bb as usize];
        let stmts = block.stmts.clone();
        let term = block.term.clone();
        // A `switchInt` on a comparison folds the comparison into a `jcc`; the
        // statement that computed the `bool` is then dead, so skip it.
        let skip = self.compare_branch_skip(bb, &term);
        for (i, s) in stmts.iter().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            if let Stmt::Assign(p, rv) = s {
                self.gen_assign(p, rv)?;
            }
        }
        self.gen_term(bb, &term)
    }

    /// The index of the statement a comparison terminator folds away, if any.
    /// Both a `switchInt(bool)` (an `if`/loop test) and an `assert(bool)` (a
    /// bounds check) read a `bool` local that was just computed by a comparison;
    /// the terminator emits the `cmp`+`jcc` itself, so the `bool` store is dead.
    fn compare_branch_skip(&self, bb: u32, term: &Terminator) -> Vec<usize> {
        let cond = match term {
            Terminator::SwitchInt { discr, targets, .. } => {
                if targets.len() != 1 {
                    return Vec::new();
                }
                discr
            }
            Terminator::Assert { cond, .. } => cond,
            _ => return Vec::new(),
        };
        let Some(pl) = operand_place(cond) else { return Vec::new() };
        if !pl.proj.is_empty() {
            return Vec::new();
        }
        let Some(block) = self.func.blocks.get(bb as usize) else { return Vec::new() };
        // `assert(!(c1 && c2))` (the overflow check) folds the `and` and both
        // comparisons into the terminator, so all three are dead.
        if matches!(term, Terminator::Assert { negate: true, .. }) {
            if let Some((ai, c1i, c2i)) = self.and_chain(bb, pl.local) {
                return vec![ai, c1i, c2i];
            }
        }
        let mut last = None;
        for (i, s) in block.stmts.iter().enumerate() {
            if let Stmt::Assign(p, Rvalue::Binary(op, _, _)) = s {
                if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                    last = Some(i);
                }
            }
        }
        last.into_iter().collect()
    }

    /// Recognise `_b = c1 & c2` in `bb`, where `c1`/`c2` are comparisons that
    /// each have `_b`'s block as their only definition site, and `_b` is the
    /// last statement. Returns the `and`'s statement index and the two
    /// comparisons' `(operator, lhs, rhs)`.
    #[allow(clippy::type_complexity)]
    fn and_chain(
        &self,
        bb: u32,
        b: u32,
    ) -> Option<(usize, usize, usize)> {
        let block = self.func.blocks.get(bb as usize)?;
        let ai = block.stmts.len().checked_sub(1)?;
        let Stmt::Assign(d, Rvalue::Binary(BinOp::BitAnd, x, y)) = &block.stmts[ai] else {
            return None;
        };
        if d.local != b || !d.proj.is_empty() {
            return None;
        }
        let (Operand::Copy(xp) | Operand::Move(xp)) = x else { return None };
        let (Operand::Copy(yp) | Operand::Move(yp)) = y else { return None };
        if !xp.proj.is_empty() || !yp.proj.is_empty() {
            return None;
        }
        let cmp_of = |l: u32| -> Option<(usize, BinOp, Operand, Operand)> {
            for (i, s) in block.stmts[..ai].iter().enumerate() {
                if let Stmt::Assign(p, Rvalue::Binary(op, a, c)) = s {
                    if p.local == l && p.proj.is_empty() && is_compare(*op) {
                        return Some((i, *op, a.clone(), c.clone()));
                    }
                }
            }
            None
        };
        let (c1i, _, _, _) = cmp_of(xp.local)?;
        let (c2i, _, _, _) = cmp_of(yp.local)?;
        Some((ai, c1i, c2i))
    }

    /// The two comparisons behind an `assert(!(c1 && c2))` terminator, as
    /// `(operator, lhs, rhs)` pairs.
    fn and_chain_cmps(
        &self,
        bb: u32,
        b: u32,
    ) -> Option<((BinOp, Operand, Operand), (BinOp, Operand, Operand))> {
        let block = self.func.blocks.get(bb as usize)?;
        let ai = block.stmts.len().checked_sub(1)?;
        let Stmt::Assign(d, Rvalue::Binary(BinOp::BitAnd, x, y)) = &block.stmts[ai] else {
            return None;
        };
        if d.local != b || !d.proj.is_empty() {
            return None;
        }
        let (Operand::Copy(xp) | Operand::Move(xp)) = x else { return None };
        let (Operand::Copy(yp) | Operand::Move(yp)) = y else { return None };
        let cmp_of = |l: u32| -> Option<(BinOp, Operand, Operand)> {
            for s in &block.stmts[..ai] {
                if let Stmt::Assign(p, Rvalue::Binary(op, a, c)) = s {
                    if p.local == l && p.proj.is_empty() && is_compare(*op) {
                        return Some((*op, a.clone(), c.clone()));
                    }
                }
            }
            None
        };
        Some((cmp_of(xp.local)?, cmp_of(yp.local)?))
    }

    /// Emit the `cmp` for the comparison that produced `cond` in `bb`, when
    /// there is one, and return the comparison's operator. `None` when `cond`
    /// is not a just-computed compare.
    fn emit_fused_compare(&mut self, bb: u32, cond: &Operand) -> Result<Option<BinOp>, String> {
        let Some(pl) = operand_place(cond) else { return Ok(None) };
        if !pl.proj.is_empty() {
            return Ok(None);
        }
        let Some(block) = self.func.blocks.get(bb as usize) else { return Ok(None) };
        let mut cmp = None;
        for s in &block.stmts {
            if let Stmt::Assign(p, Rvalue::Binary(op, a, b)) = s {
                if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                    cmp = Some((*op, a.clone(), b.clone()));
                }
            }
        }
        let Some((op, a, b)) = cmp else { return Ok(None) };
        self.emit_cmp(op, &a, &b)?;
        Ok(Some(op))
    }

    fn gen_term(&mut self, bb: u32, term: &Terminator) -> Result<(), String> {
        match term {
            Terminator::Goto(t) => {
                // Falling through to the next block needs no jump; the blocks
                // are emitted in index order, so `bb+1` is already the next
                // instruction. This removes a taken jump from every block that
                // would otherwise jump to its successor.
                if *t == bb + 1 && (*t as usize) < self.func.blocks.len() {
                    return Ok(());
                }
                self.emit(&format!("  jmp .Lbb{}", self.bb_label(*t)));
            }
            Terminator::Return => {
                // A non-unit return value lives in `_0`.
                if self.local_ty(0) != Ty::Unit && !self.func.locals.is_empty() {
                    let src = self.local_operand(0);
                    if src != "%rax" {
                        self.emit(&format!("  mov {src}, %rax"));
                    }
                }
                self.emit(&format!("  jmp .Lret_{}", self.func.name));
            }
            Terminator::Unreachable => {
                self.emit(&format!("  jmp .Lret_{}", self.func.name));
            }
            Terminator::Abort => {
                // A trap: exit 101, matching rustc's panic status.
                self.emit("  mov $101, %rdi");
                self.emit("  mov $60, %rax");
                self.emit("  syscall");
            }
            Terminator::Assert { cond, negate, target } => {
                // `assert(!(c1 && c2))`, the overflow check: emit the two
                // comparisons with a short-circuit jump, so neither `bool` is
                // materialised. Success needs one of them false, i.e. one
                // comparison to fail.
                if *negate {
                    if let Some(pl) = operand_place(cond) {
                        if pl.proj.is_empty() {
                            if let Some((c1, c2)) = self.and_chain_cmps(bb, pl.local) {
                                let ok = self.new_label();
                                self.emit_cmp(c1.0, &c1.1, &c1.2)?;
                                self.emit(&format!("  {} {ok}", jcc(negate_cc(c1.0))));
                                self.emit_cmp(c2.0, &c2.1, &c2.2)?;
                                self.emit(&format!("  {} {ok}", jcc(negate_cc(c2.0))));
                                self.emit("  mov $101, %rdi");
                                self.emit("  mov $60, %rax");
                                self.emit("  syscall");
                                self.emit(&format!("{ok}:"));
                                if *target != bb + 1 || *target as usize >= self.func.blocks.len() {
                                    self.emit(&format!("  jmp .Lbb{}", self.bb_label(*target)));
                                }
                                return Ok(());
                            }
                        }
                    }
                }
                // A `assert(cmp, ..)` folds the comparison into the flags, so
                // the `bool` local is never materialised.
                let jcc = match self.emit_fused_compare(bb, cond)? {
                    Some(op) => {
                        // `assert(!cmp, ..)` succeeds when the comparison is
                        // false, so jump on the complement of the comparison.
                        let op = if *negate { negate_cc(op) } else { op };
                        jcc(op)
                    }
                    None => {
                        // Otherwise evaluate the condition into `%rax` and test.
                        let (cv, _) = self.eval_operand(cond)?;
                        if cv != "%rax" {
                            self.emit(&format!("  mov {cv}, %rax"));
                        }
                        self.emit("  test %rax, %rax");
                        // `!x` succeeds when `x == 0`; a plain condition
                        // succeeds when it is non-zero.
                        if *negate { "je" } else { "jne" }
                    }
                };
                self.emit(&format!("  {jcc} .Lbb{}", self.bb_label(*target)));
                self.emit("  mov $101, %rdi");
                self.emit("  mov $60, %rax");
                self.emit("  syscall");
            }
            Terminator::SwitchInt { discr, targets, otherwise } => {
                self.gen_switch(bb, discr, targets, *otherwise)?;
            }
            Terminator::Call { dest, func, args, target } => {
                // `cold_path` is an optimisation hint with no effect here; it
                // has no body to call, so skip it and continue.
                let cold = matches!(func, CallTarget::Direct(n) if n == "cold_path" || n.ends_with("::cold_path"));
                if !cold {
                    self.gen_call(dest, func, args)?;
                }
                let t = target.unwrap_or_else(|| {
                    // A call with no return edge is `-> []` (noreturn).
                    u32::MAX
                });
                if t == u32::MAX {
                    self.emit(&format!("  jmp .Lret_{}", self.func.name));
                } else if t != bb + 1 || t as usize >= self.func.blocks.len() {
                    self.emit(&format!("  jmp .Lbb{}", self.bb_label(t)));
                }
            }
        }
        Ok(())
    }

    fn gen_switch(
        &mut self,
        bb: u32,
        discr: &Operand,
        targets: &[(i64, u32)],
        otherwise: u32,
    ) -> Result<(), String> {
        // A two-way `Lt/Le/Gt/Ge` switch is a loop/if test: emit a `cmp` plus
        // the matching `jcc` straight to the taken block.
        if targets.len() == 1 {
            let (val, t) = targets[0];
            let (taken, fallthrough) = if val == 0 { (otherwise, t) } else { (t, otherwise) };
            if self.try_branch_compare(bb, discr, taken, fallthrough)? {
                return Ok(());
            }
        }
        let (src, is_mem) = self.eval_operand(discr)?;
        if is_mem {
            self.emit(&format!("  mov {src}, %rax"));
            for (val, t) in targets {
                let v = self.wide_imm(&format!("${val}"), "%rcx");
                self.emit(&format!("  cmp {v}, %rax"));
                self.emit(&format!("  je .Lbb{}", self.bb_label(*t)));
            }
        } else {
            for (val, t) in targets {
                let v = self.wide_imm(&format!("${val}"), "%rcx");
                self.emit(&format!("  cmp {v}, {src}"));
                self.emit(&format!("  je .Lbb{}", self.bb_label(*t)));
            }
        }
        if otherwise != bb + 1 || otherwise as usize >= self.func.blocks.len() {
            self.emit(&format!("  jmp .Lbb{}", self.bb_label(otherwise)));
        }
        Ok(())
    }

    /// Try to emit a `switchInt(bool) -> [0: .., otherwise: ..]` as one
    /// `cmp`+`jcc`, folding the comparison away entirely. `taken` is the target
    /// when the comparison is true and `fallthrough` the target when it is
    /// false. Returns false when the discriminant is not a comparison.
    fn try_branch_compare(
        &mut self,
        bb: u32,
        discr: &Operand,
        taken: u32,
        fallthrough: u32,
    ) -> Result<bool, String> {
        let Some(pl) = operand_place(discr) else { return Ok(false) };
        if !pl.proj.is_empty() {
            return Ok(false);
        }
        // Find the statement that assigned this local a comparison.
        let Some(block) = self.func.blocks.get(bb as usize) else { return Ok(false) };
        let mut cmp: Option<(BinOp, Operand, Operand)> = None;
        for s in &block.stmts {
            if let Stmt::Assign(p, Rvalue::Binary(op, a, b)) = s {
                if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                    cmp = Some((*op, a.clone(), b.clone()));
                }
            }
        }
        let Some((op, a, b)) = cmp else { return Ok(false) };
        self.emit_cmp(op, &a, &b)?;
        // Jump to `taken` when the comparison holds, else to `fallthrough`.
        // Whichever arm is the next emitted block becomes the fall-through: when
        // it is `taken` the branch is emitted on the complement, so a loop body
        // that follows its header needs no jump at all.
        let next = bb + 1;
        if taken == next {
            self.emit(&format!("  {} .Lbb{}", jcc(negate_cc(op)), self.bb_label(fallthrough)));
        } else {
            self.emit(&format!("  {} .Lbb{}", jcc(op), self.bb_label(taken)));
            if fallthrough != next || fallthrough as usize >= self.func.blocks.len() {
                self.emit(&format!("  jmp .Lbb{}", self.bb_label(fallthrough)));
            }
        }
        Ok(true)
    }

    // --- operands and places ----------------------------------------------

    /// Evaluate a scalar operand to something usable as an instruction operand:
    /// a register, an immediate, or a direct memory slot. Returns the operand
    /// text and whether it is a memory operand. A complex operand is loaded
    /// into `%rax`.
    fn eval_operand(&mut self, op: &Operand) -> Result<(String, bool), String> {
        match op {
            Operand::Const(c) => Ok((self.const_operand(c)?, false)),
            Operand::Fmt { ty_name, trait_name } => {
                // A reified formatter is the address of rmc's own runtime
                // formatter for this type/trait pair.
                let label = self.fmt_label(ty_name, trait_name);
                self.emit(&format!("  lea {label}(%rip), %rax"));
                Ok(("%rax".to_string(), false))
            }
            Operand::Copy(p) | Operand::Move(p) => {
                if self.place_is_simple(p) {
                    let mem = !self.promo.contains_key(&p.local);
                    Ok((self.local_operand(p.local), mem))
                } else {
                    self.load_place_to(p, "%rax")?;
                    Ok(("%rax".to_string(), false))
                }
            }
        }
    }

    /// Evaluate a scalar operand *without* clobbering `%rax`: a simple operand
    /// is returned directly, a complex one is loaded into `%rcx`.
    fn eval_operand_keep_rax(&mut self, op: &Operand) -> Result<(String, bool), String> {
        match op {
            Operand::Fmt { ty_name, trait_name } => {
                let label = self.fmt_label(ty_name, trait_name);
                self.emit(&format!("  lea {label}(%rip), %rcx"));
                Ok(("%rcx".to_string(), false))
            }
            Operand::Const(c) => {
                if let Const::Str(s) = c {
                    let label = self.str_label(s);
                    self.emit(&format!("  lea {label}(%rip), %rcx"));
                    return Ok(("%rcx".to_string(), false));
                }
                if let Const::Bytes(b) = c {
                    let label = self.template_label(b);
                    self.emit(&format!("  lea {label}(%rip), %rcx"));
                    return Ok(("%rcx".to_string(), false));
                }
                Ok((self.const_operand(c)?, false))
            }
            Operand::Copy(p) | Operand::Move(p) => {
                if self.place_is_simple(p) {
                    let mem = !self.promo.contains_key(&p.local);
                    Ok((self.local_operand(p.local), mem))
                } else {
                    // `place_addr` computes into `%rax`, so the caller's `%rax`
                    // (which for a binary op is the left operand) is saved in a
                    // scratch slot and restored afterwards.
                    let tmp = self.alloc_slot();
                    self.emit(&format!("  mov %rax, {}", self.off(tmp)));
                    self.load_place_to(p, "%rcx")?;
                    self.emit(&format!("  mov {}, %rax", self.off(tmp)));
                    Ok(("%rcx".to_string(), false))
                }
            }
        }
    }

    /// True when an operand names a scalar local or constant, so reading it
    /// needs no code and cannot clobber a register.
    fn place_is_simple(&self, p: &Place) -> bool {
        p.proj.is_empty() && !self.local_ty(p.local).is_aggregate()
    }

    fn operand_is_simple(&self, op: &Operand) -> bool {
        match op {
            // A string literal is an address, not an immediate: reading it needs
            // a `lea`, so it is never a directly usable operand. A reified
            // formatter likewise needs a `lea`, and so does a `format_args!`
            // template byte string.
            Operand::Const(Const::Str(_))
            | Operand::Const(Const::Bytes(_))
            | Operand::Fmt { .. } => false,
            Operand::Const(_) => true,
            Operand::Copy(p) | Operand::Move(p) => self.place_is_simple(p),
        }
    }

    /// The `.rodata` label for a string literal's content.
    fn str_label(&self, s: &str) -> String {
        self.string_ids
            .get(s)
            .cloned()
            .unwrap_or_else(|| s.to_string())
    }

    /// The `.rodata` label for a promoted constant the parser folded, or `None`
    /// when the name is not a folded promoted constant.
    fn promoted_label(&self, name: &str) -> Option<String> {
        self.prog.promoted.contains_key(name).then(|| promoted_label(name))
    }

    /// The `.rodata` label for a `format_args!` template's bytes.
    fn template_label(&self, b: &[u8]) -> String {
        self.template_ids
            .get(b)
            .cloned()
            .unwrap_or_else(|| "__rmc_tmpl_missing".to_string())
    }

    /// The `.rodata` label for the runtime formatter of `ty` under `trait_name`.
    /// The runtime defines one per scalar type and numeric base; the trait name
    /// only selects the base (`Display` = decimal, `Debug` = decimal with
    /// quoting for strings, `LowerHex` = hex, ...).
    fn fmt_label(&self, ty_name: &str, trait_name: &str) -> String {
        resolve_fmt(ty_name, trait_name).to_string()
    }

    fn const_operand(&mut self, c: &Const) -> Result<String, String> {
        Ok(match c {
            Const::Int(n) => format!("${n}"),
            Const::Bool(b) => format!("${}", *b as i64),
            Const::Char(c) => format!("${c}"),
            Const::Unit => "$0".to_string(),
            Const::Str(s) => {
                // A string literal is the address of its `.rodata` record, so it
                // must be materialised with `lea`; the in-process assembler has
                // no relocation form for `mov $symbol`.
                let label = self.str_label(s);
                self.emit(&format!("  lea {label}(%rip), %rax"));
                "%rax".to_string()
            }
            Const::Named(n) => {
                // A folded promoted constant is the address of its `.rodata`
                // blob. Anything else (`{..}` aggregate the parser could not
                // model) stays a hard error rather than a wrong value.
                if let Some(label) = self.promoted_label(n) {
                    self.emit(&format!("  lea {label}(%rip), %rax"));
                    "%rax".to_string()
                } else {
                    return Err(format!("mir_codegen: unsupported constant `{n}`"));
                }
            }
            Const::Bytes(b) => {
                // A `format_args!` template: the address of its raw `.rodata`
                // bytes, materialised with `lea` like a string literal.
                let label = self.template_label(b);
                self.emit(&format!("  lea {label}(%rip), %rax"));
                "%rax".to_string()
            }
            Const::Zst => "$0".to_string(),
            Const::Agg { .. } => {
                return Err("mir_codegen: aggregate constant not yet materialised".into());
            }
            Const::Alloc { id, place } => {
                let label = self
                    .alloc_labels
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| format!("__rmc_alloc{id}"));
                // Materialise the address into `%rax`: a `.rodata` label is not
                // an immediate, and an interior pointer adds a projected offset.
                self.emit(&format!("  lea {label}(%rip), %rax"));
                if let Some(p) = place {
                    for proj in &p.proj {
                        match proj {
                            Proj::Field(k) => self.emit(&format!("  add ${}, %rax", *k as i64 * 8)),
                            Proj::IndexConst(k) => self.emit(&format!("  add ${}, %rax", *k as i64 * 8)),
                            _ => {}
                        }
                    }
                }
                "%rax".to_string()
            }
        })
    }

    /// The value of `op` as an ALU immediate, or `op` moved into a scratch
    /// register when it does not fit the instruction's sign-extended i32 field.
    /// An ALU immediate is only i32 wide, so a 64-bit constant must be
    /// materialised first or it would be silently truncated.
    fn wide_imm(&mut self, op: &str, scratch: &str) -> String {
        if let Some(v) = imm_value(op) {
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                self.emit(&format!("  mov ${v}, {scratch}"));
                return scratch.to_string();
            }
        }
        op.to_string()
    }

    /// Leave the address of `place` in `%rax`.
    fn place_addr(&mut self, p: &Place) -> Result<(), String> {
        let starts_deref = matches!(p.proj.first(), Some(Proj::Deref));
        if starts_deref {
            let src = self.local_operand(p.local);
            self.emit(&format!("  mov {src}, %rax"));
        } else if p.proj.is_empty() {
            // A promoted scalar has no address in memory; materialise it into
            // its shadow slot so the address is stable.
            if self.promo.contains_key(&p.local) {
                let reg = self.promo[&p.local].reg;
                let slot = self.shadow[&p.local];
                self.emit(&format!("  mov {reg}, {}", self.off(slot)));
                self.emit(&format!("  lea {}, %rax", self.off(slot)));
            } else {
                self.emit(&format!("  lea {}, %rax", self.off(self.local_base(p.local))));
            }
        } else {
            self.emit(&format!("  lea {}, %rax", self.off(self.local_base(p.local))));
        }
        let rest = if starts_deref { &p.proj[1..] } else { &p.proj[..] };
        // Track the type at each step so an array index scales by the element
        // size, not a fixed 8: `[[i64; 3]; 2]` strides 24 bytes, and a struct
        // field's offset is the sum of the sizes before it.
        let mut ty = self.local_ty(p.local);
        if starts_deref {
            ty = match &ty {
                Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                _ => Ty::I64,
            };
        }
        let mut variant_ctx: Option<(String, String)> = None;
        for proj in rest {
            match proj {
                Proj::Field(k) => {
                    // After `(x as Variant)`, the payload offset is already
                    // applied, so the field index is a word offset within it.
                    if let Some((en, v)) = &variant_ctx {
                        self.emit(&format!("  add ${}, %rax", *k as i64 * 8));
                        ty = self.enum_field_ty(en, v, *k as u64);
                        variant_ctx = None;
                        continue;
                    }
                    let off = self.field_offset(&ty, *k as u64);
                    self.emit(&format!("  add ${off}, %rax"));
                    ty = self.field_ty(&ty, *k as u64);
                }
                Proj::Variant(v) => {
                    // `(x as Variant)` selects the variant's payload: move to the
                    // payload offset (0 for a fieldless enum) and remember the
                    // variant for the field that follows.
                    if let Ty::Struct(en) = &ty {
                        let off = self.enum_field_offset(en, v, 0);
                        self.emit(&format!("  add ${off}, %rax"));
                        variant_ctx = Some((en.clone(), v.clone()));
                    }
                }
                Proj::IndexConst(i) => {
                    let stride = self.size_of_ty(&elem_ty(&ty)).max(8);
                    self.emit(&format!("  add ${}, %rax", *i as i64 * stride));
                    ty = elem_ty(&ty);
                }
                Proj::IndexLocal(l) => {
                    let stride = self.size_of_ty(&elem_ty(&ty)).max(8);
                    let src = self.local_operand(*l);
                    self.emit(&format!("  mov {src}, %rcx"));
                    match stride {
                        1 => self.emit("  add %rcx, %rax"),
                        2 => self.emit("  lea (%rax,%rcx,2), %rax"),
                        4 => self.emit("  lea (%rax,%rcx,4), %rax"),
                        8 => self.emit("  lea (%rax,%rcx,8), %rax"),
                        s => {
                            self.emit(&format!("  imul ${s}, %rcx, %rcx"));
                            self.emit("  add %rcx, %rax");
                        }
                    }
                    ty = elem_ty(&ty);
                }
                Proj::Deref => {
                    self.emit("  mov (%rax), %rax");
                    ty = match &ty {
                        Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                        _ => Ty::I64,
                    };
                }
                Proj::RawRef => {}
            }
        }
        Ok(())
    }

    /// The byte offset of field `k` within `ty`, for the layouts the backend
    /// models: a tuple is packed in order, and a struct of equally sized fields
    /// is in declaration order (rustc only reorders to improve alignment, which
    /// cannot change the offsets of equal-sized fields).
    fn field_offset(&self, ty: &Ty, k: u64) -> i64 {
        match ty {
            Ty::Tuple(tys) => tys
                .iter()
                .take(k as usize)
                .map(|t| self.size_of_ty(t))
                .sum::<i64>()
                .max(8 * k as i64),
            Ty::Struct(name) => self.struct_layout(name).map(|(offs, _)| offs.get(k as usize).copied().unwrap_or(k as i64 * 8)).unwrap_or(k as i64 * 8),
            _ => k as i64 * 8,
        }
    }

    fn field_ty(&self, ty: &Ty, k: u64) -> Ty {
        match ty {
            Ty::Tuple(tys) => tys.get(k as usize).map(|t| (**t).clone()).unwrap_or(Ty::I64),
            Ty::Struct(name) => self
                .struct_layout(name)
                .and_then(|(_, f)| f.get(k as usize).cloned())
                .unwrap_or(Ty::I64),
            _ => Ty::I64,
        }
    }

    /// Byte size of a type as the backend lays it out. A struct is the sum of
    /// its fields, which `structs` supplies.
    fn size_of_ty(&self, ty: &Ty) -> i64 {
        match ty {
            // `()` occupies no storage, so a zero-sized field (`PhantomData`,
            // which the MIR printer spells as a unit const) must not reserve a
            // word: `core::fmt::rt::ArgumentType` is 16 bytes, not 24. A
            // *whole* local of type `()` still gets a slot from `analyze`'s
            // `.max(1)`.
            Ty::Unit => 0,
            Ty::Struct(name) => {
                if let Some(l) = self.enums.get(name) {
                    return l.size.max(8);
                }
                self.structs
                    .get(name)
                    .map(|fs| fs.iter().map(|t| self.size_of_ty(t)).sum::<i64>())
                    .unwrap_or(8)
                    .max(8)
            }
            Ty::Array(elem, n) => self.size_of_ty(elem) * (*n as i64).max(1),
            Ty::Tuple(tys) => tys.iter().map(|t| self.size_of_ty(t)).sum::<i64>().max(8),
            _ => 8,
        }
    }

    /// The layout of an enum, if the backend models it.
    fn enum_layout(&self, name: &str) -> Option<&EnumLayout> {
        self.enums.get(name)
    }

    /// The byte offset of variant `v`'s field `k` within an enum, or the offset
    /// of the tag when `k` is `None`.
    fn enum_field_offset(&self, name: &str, _v: &str, k: u64) -> i64 {
        let Some(l) = self.enums.get(name) else { return k as i64 * 8 };
        let base = if l.fieldless { l.tag_off } else { l.payload_off };
        base + k as i64 * 8
    }

    /// The type of variant `v`'s field `k` within an enum.
    fn enum_field_ty(&self, name: &str, v: &str, k: u64) -> Ty {
        self.enums
            .get(name)
            .and_then(|l| l.fields.get(v))
            .and_then(|f| f.get(k as usize).cloned())
            .unwrap_or(Ty::I64)
    }

    /// The tag value of variant `v` of an enum.
    fn variant_tag(&self, name: &str, v: &str) -> u64 {
        self.enums
            .get(name)
            .and_then(|l| l.tags.get(v))
            .copied()
            .unwrap_or(0)
    }

    /// The field offsets and types of a struct, learned from its aggregate
    /// constructions: fields are laid out in declaration order, which matches
    /// rustc's `repr(Rust)` layout for the programs the backend compiles.
    fn struct_layout(&self, name: &str) -> Option<(Vec<i64>, Vec<Ty>)> {
        let tys = self.structs.get(name)?;
        let mut offs = Vec::with_capacity(tys.len());
        let mut acc = 0i64;
        for t in tys {
            offs.push(acc);
            acc += self.size_of_ty(t);
        }
        Some((offs, tys.clone()))
    }

    /// Leave the scalar value at `place` in `%rax`.
    fn load_place_value(&mut self, p: &Place) -> Result<(), String> {
        self.load_place_to(p, "%rax")
    }

    /// Load the scalar value at `place` into `reg` (which may be `%rax`).
    fn load_place_to(&mut self, p: &Place, reg: &str) -> Result<(), String> {
        self.place_addr(p)?;
        if reg != "%rax" {
            self.emit(&format!("  mov (%rax), {reg}"));
        } else {
            self.emit("  mov (%rax), %rax");
        }
        Ok(())
    }

    /// Evaluate an operand that names an aggregate, leaving its address in
    /// `%rax`. An aggregate *constant* is materialised into a fresh frame region
    /// first, so the caller can copy from a stable address.
    fn eval_aggregate_addr(&mut self, op: &Operand) -> Result<(), String> {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.place_addr(p),
            Operand::Const(c) => self.materialize_const_agg(c),
            Operand::Fmt { .. } => {
                Err("mir_codegen: a reified formatter is not an aggregate".into())
            }
        }
    }

    /// Write an aggregate constant into a fresh frame region and leave its
    /// address in `%rax`.
    fn materialize_const_agg(&mut self, c: &Const) -> Result<(), String> {
        let Const::Agg { name, ops } = c else {
            return Err("mir_codegen: aggregate constant unsupported".into());
        };
        // The aggregate's kind and the byte offset of each field.
        let (offs, size): (Vec<i64>, i64) = if name.contains("::") {
            let (en, v) = split_ev(name);
            let Some(l) = self.enums.get(&en) else {
                return Err(format!("mir_codegen: unknown enum `{en}`"));
            };
            // Word 0 is the tag; a payload variant's fields follow it.
            let mut offs = vec![0i64];
            if !l.fieldless {
                for k in 0..ops.len() {
                    offs.push(self.enum_field_offset(&en, &v, k as u64));
                }
            }
            (offs, l.size.max(8))
        } else if name.is_empty() {
            ((0..ops.len()).map(|k| k as i64 * 8).collect(), (ops.len() as i64 * 8).max(8))
        } else {
            let tys = self.structs.get(name).cloned().unwrap_or_default();
            let mut offs = Vec::new();
            let mut acc = 0i64;
            for t in &tys {
                offs.push(acc);
                acc += self.size_of_ty(t);
            }
            (offs, acc.max(8))
        };

        let slots = ((size + 7) / 8).max(1);
        let first = self.next_slot;
        for _ in 0..slots {
            self.alloc_slot();
        }
        let base = first + slots - 1;

        // Evaluate each field value into its own scratch word first, so a later
        // `lea` for the region base cannot clobber an earlier value. An enum's
        // first word is its tag, an immediate.
        let mut vals: Vec<(i64, Option<i64>, Option<i64>)> = Vec::new();
        let tag_first = name.contains("::");
        for (k, op) in ops.iter().enumerate() {
            // A zero-sized field stores nothing and takes no word, so it must
            // not consume a slot (nor shift the fields after it).
            if matches!(rv_operand_ty(&self.func, op), Ty::Unit) {
                continue;
            }
            let (src, _) = self.eval_operand(op)?;
            let tmp = self.alloc_slot();
            self.emit(&format!("  mov {src}, {}", self.off(tmp)));
            let off = offs.get(k + tag_first as usize).copied().unwrap_or(k as i64 * 8);
            vals.push((off, None, Some(tmp)));
        }
        if tag_first {
            let (en, v) = split_ev(name);
            let tag = self.variant_tag(&en, &v) as i64;
            vals.insert(0, (0, Some(tag), None));
        }
        for (off, imm, tmp) in vals {
            self.emit(&format!("  lea {}, %rax", self.off(base)));
            if off != 0 {
                self.emit(&format!("  add ${off}, %rax"));
            }
            if let Some(v) = imm {
                self.emit(&format!("  mov ${v}, %rcx"));
            } else if let Some(t) = tmp {
                self.emit(&format!("  mov {}, %rcx", self.off(t)));
            }
            self.emit("  mov %rcx, (%rax)");
        }
        self.emit(&format!("  lea {}, %rax", self.off(base)));
        Ok(())
    }

    /// Copy `size` bytes from the address in `%rsi` to the address in `%rdi`.
    fn copy_words(&mut self, size: i64) {
        if size <= 0 {
            return;
        }
        if size <= 64 {
            let n = size / 8;
            for i in 0..n {
                let d = i * 8;
                self.emit(&format!("  mov {d}(%rsi), %rcx"));
                self.emit(&format!("  mov %rcx, {d}(%rdi)"));
            }
        } else {
            self.emit("  mov %rdi, %rdx");
            self.emit("  mov %rsi, %r8");
            self.emit("  mov %rdx, %rdi");
            self.emit("  mov %r8, %rsi");
            self.emit(&format!("  mov ${}, %rcx", size / 8));
            let loop_l = self.new_label();
            self.emit(&format!("{loop_l}:"));
            self.emit("  mov (%rsi), %rax");
            self.emit("  mov %rax, (%rdi)");
            self.emit("  add $8, %rsi");
            self.emit("  add $8, %rdi");
            self.emit("  dec %rcx");
            self.emit(&format!("  jnz {loop_l}"));
        }
    }

    // --- assignment --------------------------------------------------------

    fn gen_assign(&mut self, dest: &Place, rv: &Rvalue) -> Result<(), String> {
        match rv {
            Rvalue::Use(op) => {
                let ty = self.place_ty(dest);
                if ty.is_aggregate() {
                    // Copy the aggregate words into the destination.
                    let size = self.place_size(dest);
                    self.eval_aggregate_addr(op)?;
                    self.emit("  mov %rax, %rsi");
                    self.place_addr(dest)?;
                    self.emit("  mov %rax, %rdi");
                    self.copy_words(size);
                } else {
                    let (src, _) = self.eval_operand(op)?;
                    self.store_scalar(dest, &src)?;
                }
            }
            Rvalue::Binary(op, a, b) => self.gen_binary(dest, *op, a, b)?,
            Rvalue::Unary(op, a) => {
                let (src, _) = self.eval_operand(a)?;
                if src != "%rax" {
                    self.emit(&format!("  mov {src}, %rax"));
                }
                match op {
                    UnOp::Not => self.emit("  xor $1, %rax"),
                    UnOp::Neg => self.emit("  neg %rax"),
                    UnOp::PtrMetadata => {
                        // `str::len` / slice length. rmc represents a `&str`
                        // (or `&[T]`) literal as the address of a
                        // `{len: i64, bytes...}` record, so the length is the
                        // record's first word.
                        self.emit("  mov (%rax), %rax");
                    }
                }
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Cast { op, .. } => {
                let (src, _) = self.eval_operand(op)?;
                self.store_scalar(dest, &src)?;
            }
            Rvalue::Ref { place, .. } | Rvalue::RawRef { place, .. } => {
                self.place_addr(place)?;
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Repeat { op, count } => {
                // Materialise the value once, then store it `count` times with a
                // small runtime loop. The value must survive `place_addr` (which
                // uses `%rax`), so it lives in a scratch slot; the destination
                // base address is kept in `%rcx` and the count in `%rdx`.
                let (src, _) = self.eval_operand(op)?;
                let tmp = self.alloc_slot();
                self.emit(&format!("  mov {src}, {}", self.off(tmp)));
                self.place_addr(dest)?;
                self.emit("  mov %rax, %rcx");
                let val = self.off(tmp);
                if *count == 0 {
                    // Nothing to store.
                } else {
                    let loop_lbl = self.new_label();
                    self.emit(&format!("  mov ${count}, %rdx"));
                    self.emit(&format!("  mov {val}, %rax"));
                    self.emit(&format!("{loop_lbl}:"));
                    self.emit("  mov %rax, (%rcx)");
                    self.emit("  add $8, %rcx");
                    self.emit("  dec %rdx");
                    self.emit(&format!("  jne {loop_lbl}"));
                }
            }
            Rvalue::Array(ops) | Rvalue::Aggregate(ops) => {
                let mut field = 0u64;
                for op in ops.iter() {
                    let target = self.subplace(dest, field);
                    let ty = self.place_ty(&target);
                    // A zero-sized field (`PhantomData`) takes no storage and
                    // must not advance the field index, or the next field would
                    // land one word too far.
                    if matches!(ty, Ty::Unit) {
                        continue;
                    }
                    field += 1;
                    if ty.is_aggregate() {
                        let size = self.place_size(&target);
                        self.eval_aggregate_addr(op)?;
                        self.emit("  mov %rax, %rsi");
                        self.place_addr(&target)?;
                        self.emit("  mov %rax, %rdi");
                        self.copy_words(size);
                    } else {
                        let (src, _) = self.eval_operand(op)?;
                        self.store_scalar(&target, &src)?;
                    }
                }
            }
            Rvalue::Enum { name, ops } => {
                let (en, v) = split_ev(name);
                let (tag, fieldless) = match self.enums.get(&en) {
                    Some(l) => (l.tags.get(&v).copied().unwrap_or(0), l.fieldless),
                    None => (0, false),
                };
                // The tag is word 0; a payload variant's fields follow it.
                let tag_place = self.subplace(dest, 0);
                self.store_scalar(&tag_place, &format!("${tag}"))?;
                if !fieldless {
                    for (i, op) in ops.iter().enumerate() {
                        let target = self.subplace(dest, i as u64 + 1);
                        let (src, _) = self.eval_operand(op)?;
                        self.store_scalar(&target, &src)?;
                    }
                }
            }
            Rvalue::Len(place) => {
                let len = self.array_len(place);
                self.emit(&format!("  mov ${len}, %rax"));
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Discriminant(place) => {
                // The tag of an enum place lives at its first word.
                self.place_addr(place)?;
                self.emit("  mov (%rax), %rax");
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::WithOverflow { op, a, b } => {
                let res = self.subplace(dest, 0);
                let flag = self.subplace(dest, 1);
                self.gen_binary(&res, *op, a, b)?;
                // Recompute the flags for the overflow condition: `seto` after
                // the operation. Reload the operands so the flags match the op.
                let (av, _) = self.eval_operand(a)?;
                if av != "%rax" {
                    self.emit(&format!("  mov {av}, %rax"));
                }
                let (bv, _) = self.eval_operand_keep_rax(b)?;
                self.emit(&format!("  {} {bv}, %rax", ovf_mnem(*op)));
                self.emit("  seto %al");
                self.emit("  movzbq %al, %rax");
                self.store_scalar(&flag, "%rax")?;
            }
            Rvalue::Unsupported(s) => {
                return Err(unsupported_note(
                    &format!("mir_codegen: unsupported rvalue `{s}`"),
                    s,
                ));
            }
        }
        Ok(())
    }

    /// A sub-place for aggregate element/field `i`.
    fn subplace(&self, dest: &Place, i: u64) -> Place {
        let mut p = dest.clone();
        // A struct/tuple element is a `Field` (so the field type and offset come
        // from the aggregate layout); an array element is an `IndexConst`. Using
        // `IndexConst` for a struct would scale by 8 and type the element `i64`,
        // which loses a multi-word field (e.g. a `format_args!` `Argument`).
        p.proj.push(self.aggregate_elem_proj(dest, i));
        p
    }

    /// The projection that selects element `i` of `dest`: `Field` for a
    /// struct/tuple, `IndexConst` for an array or anything else.
    fn aggregate_elem_proj(&self, dest: &Place, i: u64) -> Proj {
        match self.place_ty(dest) {
            Ty::Struct(_) | Ty::Tuple(_) => Proj::Field(i as u32),
            _ => Proj::IndexConst(i),
        }
    }

    /// Store the scalar in `src` (a register, immediate, or frame slot) at
    /// `dest`.
    fn store_scalar(&mut self, dest: &Place, src: &str) -> Result<(), String> {
        if dest.proj.is_empty() && self.promo.contains_key(&dest.local) {
            let dst = self.promo[&dest.local].reg;
            if src == dst {
                return Ok(());
            }
            self.emit(&format!("  mov {src}, {dst}"));
            return Ok(());
        }
        // A whole-local destination is one frame slot, so store into it
        // directly instead of materialising its address with an `lea` first.
        // `mov mem, mem` is not encodable, so a memory source still goes
        // through `%rcx`.
        if dest.proj.is_empty() {
            let dst = self.off(self.local_base(dest.local));
            // A wide immediate has no `mov $imm, mem` encoding, so it goes
            // through `%rcx` like a memory source.
            if src.contains('(') || is_wide_imm(src) {
                self.emit(&format!("  mov {src}, %rcx"));
                self.emit(&format!("  mov %rcx, {dst}"));
            } else if src.starts_with('$') {
                // An unsuffixed `mov $imm, mem` is a 32-bit store, which would
                // leave the high half of this 8-byte slot stale for a negative
                // value; `movq` forces the sign-extending 64-bit form.
                self.emit(&format!("  movq {src}, {dst}"));
            } else {
                self.emit(&format!("  mov {src}, {dst}"));
            }
            return Ok(());
        }
        // A memory source cannot be the source of a memory destination, and
        // `%rax` is clobbered by `place_addr`, so both go through `%rcx`. A
        // wide immediate has no `mov $imm, mem` encoding either.
        let is_mem = src.contains('(');
        let src = if src == "%rax" || is_mem || is_wide_imm(src) {
            self.emit(&format!("  mov {src}, %rcx"));
            "%rcx"
        } else {
            src
        };
        self.place_addr(dest)?;
        self.emit(&format!("  mov {src}, (%rax)"));
        Ok(())
    }

    fn place_ty(&self, p: &Place) -> Ty {
        let mut ty = self.local_ty(p.local);
        let mut variant_ctx: Option<String> = None;
        for proj in &p.proj {
            ty = match proj {
                Proj::Field(k) => {
                    if let Some(en) = &variant_ctx {
                        if let Ty::Struct(name) = &ty {
                            let (name, en) = (name.clone(), en.clone());
                            variant_ctx = None;
                            self.enum_field_ty(&name, &en, *k as u64)
                        } else {
                            variant_ctx = None;
                            self.field_ty(&ty, *k as u64)
                        }
                    } else {
                        self.field_ty(&ty, *k as u64)
                    }
                }
                Proj::Variant(v) => {
                    variant_ctx = Some(v.clone());
                    ty.clone()
                }
                Proj::IndexConst(_) | Proj::IndexLocal(_) => match &ty {
                    Ty::Array(elem, _) => (**elem).clone(),
                    _ => Ty::I64,
                },
                Proj::Deref => match &ty {
                    Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                    _ => Ty::I64,
                },
                Proj::RawRef => ty.clone(),
            };
        }
        ty
    }

    fn place_size(&self, p: &Place) -> i64 {
        self.size_of_ty(&self.place_ty(p))
    }

    fn array_len(&self, p: &Place) -> u64 {
        let mut ty = self.local_ty(p.local);
        for proj in &p.proj {
            if let Proj::Deref = proj {
                ty = match &ty {
                    Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                    _ => ty.clone(),
                };
            }
        }
        match ty {
            Ty::Array(_, n) => n,
            _ => 0,
        }
    }

    fn gen_binary(
        &mut self,
        dest: &Place,
        op: BinOp,
        a: &Operand,
        b: &Operand,
    ) -> Result<(), String> {
        // When the destination is a promoted register and the operator maps to
        // a single ALU instruction, update the register in place. This is the
        // accumulator pattern (`_acc = Add(_acc, k)`), which copy propagation
        // exposes by rewriting `_acc = Add(_tmp, k); _tmp = copy _acc` into a
        // self-update. It saves the value's trip through `%rax`.
        if dest.proj.is_empty()
            && matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
            )
            && self.promo.contains_key(&dest.local)
        {
            let reg = self.promo[&dest.local].reg.to_string();
            if !operand_is_local(a, dest.local) {
                let (av, _) = self.eval_operand(a)?;
                if av != reg {
                    self.emit(&format!("  mov {av}, {reg}"));
                }
            }
            let (bv, _) = self.eval_operand_keep_rax(b)?;
            let bv = self.wide_imm(&bv, "%rcx");
            self.emit(&format!("  {} {bv}, {reg}", alu_mnem(op)));
            if self.opts.overflow_checks && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
                self.emit("  jo __rmc_overflow");
            }
            return Ok(());
        }
        match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.emit_cmp(op, a, b)?;
                self.emit(&format!("  {} %al", setcc(op)));
                self.emit("  movzbl %al, %eax");
                self.store_scalar(dest, "%rax")?;
            }
            BinOp::Cmp => return Err("mir_codegen: three-way Cmp unsupported".into()),
            BinOp::Offset | BinOp::PtrOffset => {
                // `a + b*8` — a pointer offset by b elements.
                let (av, _) = self.eval_operand(a)?;
                if av != "%rax" {
                    self.emit(&format!("  mov {av}, %rax"));
                }
                let (bv, _) = self.eval_operand(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                self.emit("  lea (%rax,%rcx,8), %rax");
                self.store_scalar(dest, "%rax")?;
            }
            _ => {
                self.emit_arith(op, a, b)?;
                self.store_scalar(dest, "%rax")?;
            }
        }
        Ok(())
    }

    /// Emit `a op b` into `%rax`.
    fn emit_arith(&mut self, op: BinOp, a: &Operand, b: &Operand) -> Result<(), String> {
        // Evaluate the left operand into `%rax` unless it is a bare simple
        // operand, which needs no code and can be read later.
        let (av, a_mem) = self.eval_operand(a)?;
        if av != "%rax" {
            self.emit(&format!("  mov {av}, %rax"));
        }
        let _ = a_mem;
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                let (bv, _) = self.eval_operand_keep_rax(b)?;
                let m = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "imul",
                    BinOp::BitAnd => "and",
                    BinOp::BitOr => "or",
                    _ => "xor",
                };
                // An immediate is sign-extended from i32, so a wider one is
                // materialised in a register first.
                let bv = self.wide_imm(&bv, "%rcx");
                self.emit(&format!("  {m} {bv}, %rax"));
                if self.opts.overflow_checks && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
                    self.emit("  jo __rmc_overflow");
                }
            }
            BinOp::Shl | BinOp::Shr => {
                // The shift count must be in `%cl`. Evaluating it may make a
                // call, which clobbers every scratch register, so the value is
                // parked in a frame slot rather than `%rdx` while the count is
                // computed, then reloaded once the count is in `%rcx`.
                let tmp = self.alloc_slot();
                self.emit(&format!("  mov %rax, {}", self.off(tmp)));
                let (bv, _) = self.eval_operand(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                self.emit(&format!("  mov {}, %rax", self.off(tmp)));
                let m = if op == BinOp::Shl { "sal" } else { "sar" };
                self.emit(&format!("  {m} %cl, %rax"));
            }
            BinOp::Div | BinOp::Rem => {
                if self.gen_div_const(op, b)? {
                    return Ok(());
                }
                // `%rax` holds the dividend; the divisor must be materialised
                // without clobbering it, so a complex operand goes to `%rcx`.
                let (bv, _) = self.eval_operand_keep_rax(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                // Fast path: when both operands are non-negative and below 2^32,
                // the 32-bit unsigned `div` is correct and roughly half the
                // latency of the 64-bit `idiv`. `dividend | divisor` has a zero
                // high half exactly in that case, so `or`/`shr`/`jnz` selects the
                // path; a negative operand, a value past 2^32, or the
                // `i64::MIN / -1` overflow all take the slow path, which the
                // surrounding `assert`s already guard.
                let slow = self.new_label();
                let done = self.new_label();
                self.emit("  mov %rax, %rdx");
                self.emit("  or %rcx, %rdx");
                self.emit("  shr $32, %rdx");
                self.emit(&format!("  jnz {slow}"));
                self.emit("  div %ecx");
                if op == BinOp::Rem {
                    self.emit("  mov %rdx, %rax");
                } else {
                    self.emit("  mov %eax, %eax");
                }
                self.emit(&format!("  jmp {done}"));
                self.emit(&format!("{slow}:"));
                self.emit("  cqto");
                self.emit("  idiv %rcx");
                if op == BinOp::Rem {
                    self.emit("  mov %rdx, %rax");
                }
                self.emit(&format!("{done}:"));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Division/modulo by a constant becomes a multiply and an arithmetic shift
    /// instead of a ~30-cycle `idiv`. Returns `false` for a variable divisor, a
    /// zero divisor, or a non-`i64` literal, so the caller falls back to `idiv`.
    ///
    /// The dividend is in `%rax` on entry; `%rax` holds the result on return.
    /// `%rcx`, `%rdx`, `%r10` and `%r11` are scratch; `%r10` holds a copy of the
    /// dividend and `%r11` the divisor.
    fn gen_div_const(&mut self, op: BinOp, b: &Operand) -> Result<bool, String> {
        let d = match b {
            Operand::Const(Const::Int(v)) => *v,
            _ => return Ok(false),
        };
        if d == 0 {
            return Ok(false);
        }
        self.emit("  mov %rax, %r10");
        if d == 1 {
            // The dividend is already in `%rax`.
        } else if d == -1 {
            self.emit("  neg %rax");
            if self.opts.overflow_checks {
                self.emit("  jo __rmc_overflow");
            }
        } else {
            let mag = d.unsigned_abs();
            if mag.is_power_of_two() {
                // `d` is ±2^k. An arithmetic shift rounds toward -inf, so a
                // negative dividend is biased by 2^k-1 to round toward zero.
                let k = mag.trailing_zeros();
                self.emit("  mov %rax, %rdx");
                self.emit("  sar $63, %rdx");
                self.emit(&format!("  shr ${}, %rdx", 64 - k));
                self.emit("  add %rdx, %rax");
                self.emit(&format!("  sar ${k}, %rax"));
                if d < 0 {
                    self.emit("  neg %rax");
                }
            } else {
                let (m, s, add) = sdiv_magic(mag);
                self.emit(&format!("  movabs ${m}, %rcx"));
                self.emit("  imul %rcx");
                if add {
                    self.emit("  add %r10, %rdx");
                }
                if s > 0 {
                    self.emit(&format!("  sar ${s}, %rdx"));
                }
                self.emit("  mov %r10, %rcx");
                self.emit("  shr $63, %rcx");
                self.emit("  add %rcx, %rdx");
                if d < 0 {
                    self.emit("  neg %rdx");
                }
                if op == BinOp::Rem {
                    // The quotient is in `%rdx`; the remainder is `x - q*d`.
                    self.emit(&mov_imm_str(d, "%r11"));
                    self.emit("  imul %r11, %rdx");
                    self.emit("  sub %rdx, %r10");
                    self.emit("  mov %r10, %rax");
                    return Ok(true);
                }
                self.emit("  mov %rdx, %rax");
            }
        }
        if op == BinOp::Rem {
            self.emit("  mov %rax, %rdx");
            self.emit(&mov_imm_str(d, "%r11"));
            self.emit("  imul %r11, %rdx");
            self.emit("  sub %rdx, %r10");
            self.emit("  mov %r10, %rax");
        }
        Ok(true)
    }

    /// Emit `cmp` so that the flags read as `a op b` (i.e. `a - b`).
    fn emit_cmp(&mut self, _op: BinOp, a: &Operand, b: &Operand) -> Result<(), String> {
        // x86 has no `cmp imm, reg` form: an immediate may only be the source.
        // The fast path would emit exactly that when `a` is a constant, so an
        // immediate on the left is routed through the general path, which loads
        // it into `%rax` first.
        if !matches!(a, Operand::Const(_)) && self.operand_is_simple(a) && self.operand_is_simple(b) {
            let (av, _) = self.eval_operand(a)?;
            let (bv, _) = self.eval_operand(b)?;
            let bv = self.wide_imm(&bv, "%rcx");
            self.emit(&format!("  cmp {bv}, {av}"));
            return Ok(());
        }
        // Load `a` into `%rax`, keep it while `b` is evaluated into `%rcx`,
        // then `cmp %rcx, %rax`.
        let (av, _) = self.eval_operand(a)?;
        if av != "%rax" {
            self.emit(&format!("  mov {av}, %rax"));
        }
        let (bv, _) = self.eval_operand_keep_rax(b)?;
        let bv = self.wide_imm(&bv, "%rcx");
        self.emit(&format!("  cmp {bv}, %rax"));
        Ok(())
    }

    // --- calls -------------------------------------------------------------

    fn gen_call(&mut self, dest: &Place, func: &CallTarget, args: &[Operand]) -> Result<(), String> {
        let name = match func {
            CallTarget::Direct(n) => n.clone(),
            CallTarget::Indirect => {
                return Err("mir_codegen: indirect call (fn pointer) unsupported".into())
            }
        };
        if name == "alloc" {
            // rustc models `alloc` as an unsafe fn with an `Option`-like ABI;
            // the program's own `alloc` is rmc's builtin, so call it directly.
        }
        // Evaluate arguments left to right. An aggregate is passed by address;
        // a scalar in a frame slot or an immediate is stable and used directly;
        // only a value that landed in `%rax`/`%rcx` is stashed in a spill slot,
        // because evaluating a later argument may clobber it.
        // An argument that is already in `%rax`/`%rcx` need not be stashed when
        // nothing after it clobbers the register: only the *last* argument that
        // needs evaluation can leave its value in a register, because every
        // argument before it is followed by an evaluation that may overwrite
        // it. This is the common case for a call whose argument is a freshly
        // computed scalar, and it removes a store/reload pair from every call.
        let is_stable = |a: &Operand| {
            !self.operand_ty(a).is_aggregate() && self.operand_is_simple(a)
        };
        let last_unstable = args.iter().rposition(|a| !is_stable(a));
        let mut reg_ops: Vec<(usize, String)> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if i >= ARG_REGS.len() {
                return Err(format!("mir_codegen: call to {name} with more than 6 arguments"));
            }
            if self.operand_ty(arg).is_aggregate() {
                self.eval_aggregate_addr(arg)?;
                if Some(i) == last_unstable {
                    reg_ops.push((i, "%rax".to_string()));
                } else {
                    let tmp = self.alloc_slot();
                    self.emit(&format!("  mov %rax, {}", self.off(tmp)));
                    reg_ops.push((i, self.off(tmp)));
                }
            } else {
                let (v, _) = self.eval_operand(arg)?;
                let stable = v.starts_with('$') || v.contains('(');
                if stable || Some(i) == last_unstable {
                    reg_ops.push((i, v));
                } else {
                    let tmp = self.alloc_slot();
                    self.emit(&format!("  mov {v}, {}", self.off(tmp)));
                    reg_ops.push((i, self.off(tmp)));
                }
            }
        }
        // Spill the promoted caller-saved registers before the argument moves,
        // which overwrite them, and restore afterwards. `%rax` carries the
        // result, so the spills use memory operands directly.
        let mut caller_promos: Vec<(u32, &'static str)> = self
            .promo
            .iter()
            .filter(|(_, p)| p.caller_saved)
            .map(|(l, p)| (*l, p.reg))
            .collect();
        caller_promos.sort_unstable();
        for (l, reg) in &caller_promos {
            let slot = self.off(self.caller_spill[l]);
            self.emit(&format!("  mov {reg}, {slot}"));
        }
        for (i, v) in &reg_ops {
            if ARG_REGS[*i] != *v {
                self.emit(&format!("  mov {v}, {}", ARG_REGS[*i]));
            }
        }
        self.emit(&format!("  call {name}"));
        for (l, reg) in &caller_promos {
            let slot = self.off(self.caller_spill[l]);
            self.emit(&format!("  mov {slot}, {reg}"));
        }
        if self.place_ty(dest).is_aggregate() {
            // Aggregates return by hidden pointer in %rdi; rmc's builtins do
            // not, so this only appears for functions this backend owns.
            return Err("mir_codegen: aggregate return unsupported".into());
        }
        if !matches!(dest.proj.as_slice(), []) || self.local_ty(dest.local) != Ty::Unit {
            // A `()` destination still has a slot; storing is harmless.
            self.store_scalar(dest, "%rax")?;
        }
        Ok(())
    }

    fn operand_ty(&self, op: &Operand) -> Ty {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.place_ty(p),
            Operand::Const(Const::Bool(_)) => Ty::Bool,
            Operand::Const(Const::Str(_)) => Ty::Ptr(Box::new(Ty::Unit)),
            Operand::Const(_) => Ty::I64,
            Operand::Fmt { .. } => Ty::Ptr(Box::new(Ty::Unit)),
        }
    }
}

/// Collapse a scratch-register round trip into the register it came from.
///
/// A value that lives in a promoted register is copied into `%rax` to be
/// updated (`mov %r8, %rax; add -128(%rbp), %rax; add -136(%rbp), %rax; mov
/// %rax, %r8`), because the generator uses `%rax` as its accumulator. When the
/// instructions in between touch only the scratch register, the copy back is
/// unnecessary and every use can name the promoted register directly, turning
/// five instructions into three.
///
/// The search stops at a label, a jump, or a call, so the rewrite never crosses
/// a point where the scratch register could hold something else.
fn coalesce_scratch(body: &str) -> String {
    const SCRATCH: &[&str] = &["%rax", "%rcx", "%rdx", "%r10", "%r11"];
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    'outer: while i < lines.len() {
        if let Some((a, t)) = mov_reg_reg(lines[i]) {
            // `mov %a, %t` copies a value into `%t`. When exactly one of the
            // two is a scratch register, the copy exists only to park the value
            // somewhere the other register is not reused: carry it in the
            // non-scratch register (`keep`) and rename the scratch one's uses
            // (`dead`). A call result is the common case -- `%rax` holds it and
            // the generator copies it into a callee-saved register so the next
            // call can overwrite `%rax`.
            let (keep, dead) = if SCRATCH.contains(&t) && !SCRATCH.contains(&a) {
                (a, t)
            } else if SCRATCH.contains(&a) && !SCRATCH.contains(&t) {
                (t, a)
            } else {
                out.push_str(lines[i]);
                out.push('\n');
                i += 1;
                continue;
            };
            // The region over which the copy carries the value: it ends at the
            // first control transfer, when `dead` is overwritten (the value is
            // lost), or *after* the instruction that overwrites `keep` (the
            // value's last use). Block labels do not end it -- the value is
            // still live across a fall-through.
            let mut k = i + 1;
            let mut used = false;
            while k < lines.len() {
                let lt = lines[k].trim();
                let op = lt.split_whitespace().next().unwrap_or("");
                if lt.ends_with(':') || lt.starts_with('.') {
                    k += 1;
                    continue;
                }
                if op.starts_with('j')
                    || matches!(op, "ret" | "call" | "syscall" | "ud2" | "hlt")
                {
                    break;
                }
                // The instruction that overwrites `dead` is still part of the
                // region when it reads the value (a `lea` that rebases it, say),
                // so note the use before ending the region there.
                if reg_mentions(lines[k], keep) || reg_mentions(lines[k], dead) {
                    used = true;
                }
                let writes = written_regs(op, lt);
                k += 1;
                if writes.iter().any(|r| r == dead) || writes.iter().any(|r| r == keep) {
                    break;
                }
            }
            // Only rewrite when the value is actually read in the region, and
            // only when `keep` carries nothing past it -- otherwise renaming
            // would change a value a later block still reads.
            if used && dead_after(&lines, i, k, keep) {
                for m in &lines[i + 1..k] {
                    out.push_str(&replace_reg(m, keep, dead));
                    out.push('\n');
                }
                i = k;
                continue 'outer;
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// True when `reg` is mentioned outside `lines[start..end]` only by a `push` or
/// a `pop`, i.e. it carries nothing across the region.
fn dead_after(lines: &[&str], start: usize, end: usize, reg: &str) -> bool {
    for (idx, line) in lines.iter().enumerate() {
        if (idx >= start + 1 && idx < end) || idx == start {
            continue;
        }
        if !reg_mentions(line, reg) {
            continue;
        }
        let op = line.trim().split_whitespace().next().unwrap_or("");
        if op != "push" && op != "pop" {
            return false;
        }
    }
    true
}

/// Fold `mov %src, %t; add $k, %t; mov ...(%t)...` into a single instruction
/// that uses `%src` as the base with `k` as the displacement.
///
/// A field or element access is generated as "copy the base pointer into a
/// scratch register, add the byte offset, then load or store through it". On
/// x86-64 the offset belongs in the addressing mode: `mov 8(%r13), %rax` does in
/// one instruction what the three did.
///
/// The fold is safe when the third instruction overwrites `%t` (the address is
/// dead either way), or when `%t` occurs exactly three times in the function, so
/// the folded add cannot be observed elsewhere.
fn fold_addresses(body: &str) -> String {
    const SCRATCH: &[&str] = &["%rax", "%rcx", "%rdx", "%r10", "%r11"];
    let lines: Vec<&str> = body.lines().collect();
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for line in &lines {
        for r in SCRATCH {
            if reg_mentions(line, r) {
                *uses.entry(r).or_insert(0) += 1;
            }
        }
    }
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i + 2 < lines.len() {
        if let Some((src, t)) = mov_reg_reg(lines[i]) {
            if SCRATCH.contains(&t) && src != t {
                if let Some(("add", k, arith_dst)) = addsub_imm(lines[i + 1]) {
                    if arith_dst == t && k.unsigned_abs() < (1 << 31) {
                        if let Some((rewritten, writes_t)) = fold_base(lines[i + 2], t, src, k) {
                            if writes_t || uses.get(t).copied() == Some(3) {
                                out.push_str(&rewritten);
                                out.push('\n');
                                i += 3;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    while i < lines.len() {
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// Replace every whole-operand occurrence of `from` with `to`.
fn replace_reg(line: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(p) = rest.find(from) {
        let before = &rest[..p];
        let after = &rest[p + from.len()..];
        let left_ok = before
            .chars()
            .last()
            .map(|c| !(c.is_ascii_alphanumeric() || c == '%' || c == '_'))
            .unwrap_or(true);
        let right_ok = after
            .chars()
            .next()
            .map(|c| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(true);
        out.push_str(before);
        if left_ok && right_ok {
            out.push_str(to);
        } else {
            out.push_str(from);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Rewrite a memory operand `(%t)` as `<k>(%src)`, reporting whether the
/// instruction writes `%t`. Returns `None` unless `(%t)` is `%t`'s only use.
fn fold_base(line: &str, t: &str, src: &str, k: i64) -> Option<(String, bool)> {
    let old = format!("({t})");
    if !line.contains(&old) {
        return None;
    }
    // `%t` must not appear anywhere else (e.g. as the source of a store).
    if replace_reg(line, t, "\u{1}").contains(t) {
        return None;
    }
    let disp = if k == 0 { format!("({src})") } else { format!("{k}({src})") };
    let rewritten = line.replacen(&old, &disp, 1);
    let dst = line.trim().split_once(',').map(|(_, d)| d.trim());
    let writes_t = dst == Some(t);
    Some((rewritten, writes_t))
}

/// Fold a constant materialised in a scratch register into the arithmetic that
/// uses it: `mov $k, %t; add %t, %d` becomes `add $k, %d`, and
/// `mov $k, %t; add %s, %t` becomes `lea k(%s), %t`. The generator loads every
/// constant into a register, so a loop that multiplies by one pays an extra
/// instruction on each iteration.
fn fuse_imm(body: &str) -> String {
    const SCRATCH: &[&str] = &["%rax", "%rcx", "%rdx", "%r10", "%r11"];
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i + 1 < lines.len() {
        if let Some((k, t)) = mov_imm(lines[i]) {
            if SCRATCH.contains(&t) {
                if let Some((op, a, d)) = binop_reg(lines[i + 1]) {
                    // `mov $k, %t; imul %s, %t` for a small `k` is a scaled
                    // `lea`, which has one-cycle latency instead of `imul`'s
                    // three. A loop-carried multiply by a small constant sits on
                    // the critical path, so this is worth two cycles an
                    // iteration even though it is the same instruction count.
                    if op == "imul" && d == t && a != t {
                        if let Some(lea) = scaled_lea(k, a, t) {
                            out.push_str(&lea);
                            out.push('\n');
                            i += 2;
                            continue;
                        }
                    }
                    // `mov $k, %t; add %s, %t` -> `lea k(%s), %t`. The result
                    // stays in `%t`, so the fold does not care whether `%t` is
                    // read later; only `%s` has to be a register.
                    if d == t && a != t && (op == "add" || op == "sub") {
                        let disp = if op == "add" { k } else { -k };
                        if disp == 0 {
                            out.push_str(&format!("  mov {a}, {t}\n"));
                            i += 2;
                            continue;
                        }
                        if (-2_147_483_648..=2_147_483_647).contains(&disp) {
                            out.push_str(&format!("  lea {disp}({a}), {t}\n"));
                            i += 2;
                            continue;
                        }
                    }
                    // `mov $k, %t; <op> %t, %d` -> `<op> $k, %d`, which drops
                    // `%t`, so it must be dead afterwards. `imul` needs the
                    // three-operand encoding and its immediate is sign
                    // extended, so only a value that fits `i32` is equivalent.
                    let foldable = matches!(op, "add" | "sub" | "and" | "or" | "xor" | "cmp");
                    let fits = (-2_147_483_648..=2_147_483_647).contains(&k);
                    if a == t && d != t && fits && (foldable || op == "imul")
                        && scratch_dead_after(&lines, i + 2, t)
                    {
                        if op == "imul" {
                            out.push_str(&format!("  imul ${k}, {d}, {d}\n"));
                        } else {
                            out.push_str(&format!("  {op} ${k}, {d}\n"));
                        }
                        i += 2;
                        continue;
                    }
                }
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    while i < lines.len() {
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// `lea` for `k * s` when `k` is small enough for an x86 scale factor.
///
/// The scale factor is 1, 2, 4 or 8, so only the multipliers expressible as
/// `(1 + scale) * s` or `scale * s` qualify: 2, 3, 4, 5, 8 and 9. Anything else
/// has to go through `imul`.
fn scaled_lea(k: i64, s: &str, t: &str) -> Option<String> {
    let body = match k {
        2 => format!("({s},{s},1)"),
        3 => format!("({s},{s},2)"),
        4 => format!("0(,{s},4)"),
        5 => format!("({s},{s},4)"),
        8 => format!("0(,{s},8)"),
        9 => format!("({s},{s},8)"),
        _ => return None,
    };
    Some(format!("  lea {body}, {t}"))
}

/// `mov $k, %t` (or `movabs $k, %t`), returning the value and destination.
fn mov_imm(line: &str) -> Option<(i64, &str)> {
    let t = line.trim();
    let rest = t.strip_prefix("mov ").or_else(|| t.strip_prefix("movabs "))?;
    let (imm, dst) = rest.split_once(',')?;
    let k: i64 = imm.trim().strip_prefix('$')?.parse().ok()?;
    let dst = dst.trim();
    is_reg(dst).then_some((k, dst))
}

/// `<op> <a>, <d>` with two register operands.
fn binop_reg(line: &str) -> Option<(&str, &str, &str)> {
    let t = line.trim();
    let (op, rest) = t.split_once(' ')?;
    let (a, d) = rest.split_once(',')?;
    let (a, d) = (a.trim(), d.trim());
    (is_reg(a) && is_reg(d)).then_some((op, a, d))
}

/// Fold `mov %a, %t; add|sub $k, %t; ...; mov %t, %d` into `lea ±k(%a), %d`.
///
/// The generator materialises every offset in a scratch register (`mov %rbx,
/// %rax; sub $1, %rax; mov %rax, %rdi`), which costs three instructions for what
/// `lea -1(%rbx), %rdi` does in one. This runs before the register forwarding so
/// the source register is still named directly.
///
/// The pattern only fires when the scratch register appears exactly three times
/// in the whole function — the two `mov`s and the `add`/`sub` — so it is provably
/// dead afterwards and the rewrite cannot lose a value. Any instruction between
/// the arithmetic and the final `mov` must not read `%a` or `%d`, or the `lea`
/// could not be moved down past it.
fn fuse_lea(body: &str) -> String {
    const SCRATCH: &[&str] = &["%rax", "%rcx", "%rdx", "%r10", "%r11"];
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i + 1 < lines.len() {
        // `mov %a, %t; mov %t, %d` -> `mov %a, %d`, when `%t` is a scratch
        // register that is dead afterwards. The generator parks a result in a
        // scratch register and then copies it into the local that owns it.
        if let Some((a, t)) = mov_reg_reg(lines[i]) {
            if SCRATCH.contains(&t) && a != t && scratch_dead_after(&lines, i + 2, t) {
                if let Some((t2, d)) = mov_reg_reg(lines[i + 1]) {
                    if t2 == t && d != t {
                        out.push_str(&format!("  mov {a}, {d}\n"));
                        i += 2;
                        continue;
                    }
                }
                // `mov %a, %t; lea k(%t), %d` -> `lea k(%a), %d`: the copy only
                // rebased the `lea`.
                if let Some((k, base, d)) = lea_disp(lines[i + 1]) {
                    if base == t && d != t {
                        out.push_str(&format!("  lea {k}({a}), {d}\n"));
                        i += 2;
                        continue;
                    }
                }
            }
        }
        if let Some((a, t)) = mov_reg_reg(lines[i]) {
            if SCRATCH.contains(&t) {
                if let Some((op, k, arith_dst)) = addsub_imm(lines[i + 1]) {
                    if arith_dst == t && a != t && k.unsigned_abs() < (1 << 31) {
                        if let Some(j) = (i + 2..lines.len()).find(|j| reg_mentions(lines[*j], t)) {
                            if let Some((t2, d)) = mov_reg_reg(lines[j]) {
                                // Nothing between may touch `a` or `d`; the
                                // scratch value must be dead once it reaches `d`,
                                // or dropping the write would lose it.
                                if t2 == t
                                    && !writes_or_reads(&lines[i + 1..j], a, d)
                                    && scratch_dead_after(&lines, j + 1, t)
                                {
                                    let disp = if op == "add" { k } else { -k };
                                    if disp == 0 {
                                        out.push_str(&format!("  mov {a}, {d}\n"));
                                    } else {
                                        out.push_str(&format!("  lea {disp}({a}), {d}\n"));
                                    }
                                    // The instructions between the arithmetic and
                                    // the final move are independent of both
                                    // registers, so they run unchanged after it.
                                    for mid in &lines[i + 2..j] {
                                        out.push_str(mid);
                                        out.push('\n');
                                    }
                                    i = j + 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    if i < lines.len() {
        out.push_str(lines[i]);
        out.push('\n');
    }
    out
}

/// True when `t` is not read again before it is overwritten, scanning forward
/// from `start` and stopping at any control transfer.
fn scratch_dead_after(lines: &[&str], start: usize, t: &str) -> bool {
    for line in &lines[start.min(lines.len())..] {
        let lt = line.trim();
        let op = lt.split_whitespace().next().unwrap_or("");
        if lt.ends_with(':')
            || lt.starts_with('.')
            || op.starts_with('j')
            || op == "ret"
            || op == "call"
            || op == "syscall"
        {
            return true;
        }
        if reads_reg(line, t) {
            return false;
        }
        if written_regs(op, line).iter().any(|r| r == t) {
            return true;
        }
    }
    true
}

/// True when `line` reads `r` (as opposed to merely writing it).
///
/// For the destination-only forms (`mov`, `lea`, `pop`, the `mov`-extend
/// family) the last operand is written and not read; every other mention of a
/// register reads it. Used to prove a scratch register dead after a rewrite:
/// a register with no remaining reads does not need its old value.
fn reads_reg(line: &str, r: &str) -> bool {
    let t = line.trim();
    let Some((mnem, rest)) = t.split_once(char::is_whitespace) else {
        return reg_mentions(t, r);
    };
    // The one-operand multiply/divide forms read the accumulator implicitly:
    // `imul r/m64` reads `%rax` and writes `%rax:%rdx`; `idiv`/`div` read
    // `%rdx:%rax`. Missing this would let a pass treat `%rax` as dead across
    // them and delete the `mov` that sets up the dividend.
    if !rest.contains(',') {
        match mnem {
            "imul" | "mul" => return r == "%rax" || reg_mentions(rest, r),
            "idiv" | "div" => return r == "%rax" || r == "%rdx" || reg_mentions(rest, r),
            _ => {}
        }
    }
    let write_only_dst = matches!(
        mnem,
        "mov" | "lea" | "movabs" | "pop" | "sete" | "setne" | "setl" | "setg"
            | "setle" | "setge" | "setb" | "seta" | "movzbl" | "movsbl" | "movslq"
    );
    if write_only_dst {
        match rest.rsplit_once(',') {
            Some((src, _dst)) => reg_mentions(src, r),
            None => reg_mentions(rest, r),
        }
    } else {
        reg_mentions(rest, r)
    }
}

fn reg_mentions(line: &str, r: &str) -> bool {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '%' || c == '_'))
        .any(|tok| tok == r)
}

/// Every frame slot the body reads or writes, from the `-N(%rbp)` operands in
/// its text. `off` numbers slots so a reference is `-(slot + 1) * 8` at
/// `saved_regs == 0`, which the dry pass runs at.
fn rbp_slots(body: &str) -> HashSet<i64> {
    let mut set = HashSet::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while let Some(p) = body[i..].find("(%rbp)") {
        let at = i + p;
        let mut j = at;
        while j > 0 && (bytes[j - 1].is_ascii_digit() || bytes[j - 1] == b'-') {
            j -= 1;
        }
        if j < at {
            if let Ok(v) = body[j..at].parse::<i64>() {
                if v < 0 {
                    let slot = (-v) / 8 - 1;
                    if slot >= 0 {
                        set.insert(slot);
                    }
                }
            }
        }
        i = at + "(%rbp)".len();
    }
    set
}

/// Frame slots the body reads, i.e. every `-N(%rbp)` operand that is not the
/// sole destination of a `mov`. A slot written and never read is dead.
fn read_slots(body: &str) -> HashSet<i64> {
    let mut set = HashSet::new();
    for line in body.lines() {
        let slots = rbp_slots(line);
        if slots.is_empty() {
            continue;
        }
        // `mov <non-slot>, <slot>` writes the slot without reading it.
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("mov ").or_else(|| t.strip_prefix("movq ")) {
            if let Some((src, dst)) = rest.split_once(',') {
                if !src.contains("(%rbp)") && dst.trim().ends_with("(%rbp)") {
                    continue;
                }
            }
        }
        set.extend(slots);
    }
    set
}

/// `mov %a, %b` where both operands are registers.
fn mov_reg_reg(line: &str) -> Option<(&str, &str)> {
    let t = line.trim();
    let rest = t.strip_prefix("mov ")?;
    let (a, b) = rest.split_once(',')?;
    let (a, b) = (a.trim(), b.trim());
    (is_reg(a) && is_reg(b)).then_some((a, b))
}

/// `lea <disp>(%base), %dst`, returning the displacement, base and destination.
fn lea_disp(line: &str) -> Option<(i64, &str, &str)> {
    let t = line.trim();
    let rest = t.strip_prefix("lea ")?;
    let (mem, dst) = rest.split_once(',')?;
    let mem = mem.trim();
    let open = mem.find('(')?;
    let close = mem.find(')')?;
    let disp: i64 = mem[..open].trim().parse().ok()?;
    let base = mem[open + 1..close].trim();
    let dst = dst.trim();
    (is_reg(base) && is_reg(dst)).then_some((disp, base, dst))
}

/// `add $k, %t` or `sub $k, %t`.
fn addsub_imm(line: &str) -> Option<(&str, i64, &str)> {
    let t = line.trim();
    let (op, rest) = t.split_once(' ')?;
    if op != "add" && op != "sub" {
        return None;
    }
    let (imm, dst) = rest.split_once(',')?;
    let k: i64 = imm.trim().strip_prefix('$')?.parse().ok()?;
    Some((op, k, dst.trim()))
}

/// True when any line reads or writes either register.
fn writes_or_reads(lines: &[&str], a: &str, b: &str) -> bool {
    lines.iter().any(|l| reg_mentions(l, a) || reg_mentions(l, b))
}

/// Forward a store to a frame slot into a later load of that slot.
///
/// The generator writes a scalar to its slot and later reads it back, because
/// the value has to survive other statements. When the register that was stored
/// still holds the value at the load, the load can read the register instead,
/// removing a memory round-trip. A slot's register is forgotten as soon as
/// anything writes that register or the slot, and the map is cleared at every
/// label and control transfer, so the forwarding never crosses a place where
/// the two could diverge.
fn forward_stores(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut holding: HashMap<String, String> = HashMap::new();
    for line in body.lines() {
        let t = line.trim();
        let op = t.split_whitespace().next().unwrap_or("");
        if t.is_empty() || t.starts_with('.') || t.ends_with(':') || t.starts_with('#') {
            holding.clear();
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if op == "call" || op == "ret" || op == "syscall" || op.starts_with('j') {
            holding.clear();
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !KNOWN_OPS.contains(&op) {
            holding.clear();
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let rest = t[op.len()..].trim();
        let pair = rest.split_once(',').map(|(a, b)| (a.trim(), b.trim()));
        // `cmp`/`test` read both operands, so the second one is not a write: a
        // slot there can be forwarded to the register that still holds it (the
        // bounds check compares an index slot the code just stored), and the
        // slot must stay valid in `holding` afterwards.
        let read_only = op == "cmp" || op == "test";
        let mut rewritten: String = line.to_string();
        if let Some((src, dst)) = pair {
            // A store of a register into a slot records the register.
            if is_reg(src) && is_slot(dst) && (op == "mov" || op == "movq") {
                holding.retain(|_, r| r != dst);
                holding.insert(dst.to_string(), src.to_string());
                out.push_str(line);
                out.push('\n');
                continue;
            }
            // A read of a slot whose value is still in a register: read the
            // register instead. This covers both a plain load and a memory
            // operand (`add -8(%rbp), %rax`), and once the slot has no readers
            // left the store into it is dropped as dead.
            let mut a = src;
            let mut b = dst;
            // `lea`'s source is an addressing expression, not a memory read:
            // `lea -8(%rbp), %rax` computes an address, so the slot there must
            // not be replaced by the register that happens to hold its value.
            if op != "lea" && is_slot(a) {
                if let Some(r) = holding.get(a) {
                    if (op == "mov" || op == "movq") && r == b {
                        continue;
                    }
                    a = r;
                }
            }
            if read_only && is_slot(b) {
                if let Some(r) = holding.get(b) {
                    b = r;
                }
            }
            if a != src || b != dst {
                rewritten = format!("  {op} {a}, {b}");
            }
            // Any write to a slot (other than the read-only second operand of
            // `cmp`/`test`) invalidates the value it held.
            if !read_only && is_slot(dst) {
                holding.remove(dst);
            }
        }
        for r in written_regs(op, &rewritten) {
            drop_holding(&mut holding, &r);
        }
        out.push_str(&rewritten);
        out.push('\n');
    }
    out
}

/// Instructions the forwarding pass understands well enough not to clear its
/// map. Anything else may write a register in a way the pass does not model.
const KNOWN_OPS: &[&str] = &[
    "mov", "movq", "movabs", "movzbl", "movzbq", "movslq", "add", "addq", "sub", "subq", "imul",
    "and", "or", "xor", "cmp", "test", "lea", "sar", "shr", "shl", "neg", "inc", "dec", "sete",
    "setne", "setl", "setle", "setg", "setge", "seta", "setae", "setb", "setbe", "cqto", "cltd",
    "idiv", "div", "push", "pop", "nop", "ud2", "hlt",
];

fn is_reg(s: &str) -> bool {
    s.starts_with('%')
}

fn is_slot(s: &str) -> bool {
    s.ends_with("(%rbp)") || s.ends_with("(%rip)")
}

/// The canonical 64-bit register a possibly-narrower name refers to, so that a
/// write to `%al` invalidates a value held in `%rax`.
fn canon_reg(r: &str) -> String {
    const PARENTS: &[(&str, &str)] = &[
        ("%rax", "%rax"), ("%eax", "%rax"), ("%ax", "%rax"), ("%al", "%rax"),
        ("%rbx", "%rbx"), ("%ebx", "%rbx"), ("%bx", "%rbx"), ("%bl", "%rbx"),
        ("%rcx", "%rcx"), ("%ecx", "%rcx"), ("%cx", "%rcx"), ("%cl", "%rcx"),
        ("%rdx", "%rdx"), ("%edx", "%rdx"), ("%dx", "%rdx"), ("%dl", "%rdx"),
    ];
    for (k, v) in PARENTS {
        if r == *k {
            return (*v).to_string();
        }
    }
    r.to_string()
}

fn drop_holding(holding: &mut HashMap<String, String>, reg: &str) {
    holding.retain(|_, r| canon_reg(r) != reg);
}

/// Registers an instruction definitely writes. The destination operand covers
/// most forms; the multiply/divide forms write `%rax`/`%rdx` implicitly.
fn written_regs(op: &str, line: &str) -> Vec<String> {
    let mut w = Vec::new();
    match op {
        "cmp" | "test" | "push" | "nop" => return w,
        "cqto" | "cltd" => {
            w.push("%rdx".to_string());
            return w;
        }
        "idiv" | "div" | "mul" => {
            w.push("%rax".to_string());
            w.push("%rdx".to_string());
            return w;
        }
        "imul" if !line.contains(',') => {
            w.push("%rax".to_string());
            w.push("%rdx".to_string());
            return w;
        }
        _ => {}
    }
    if let Some((_, dst)) = line.split_once(',') {
        let dst = dst.trim();
        if is_reg(dst) {
            w.push(canon_reg(dst));
        }
    }
    w
}

/// Drop a store to a frame slot that the function never reads.
///
/// The generator stores every scalar result to its destination slot, but a
/// temporary that is only read back through the register that still holds it
/// (the very next statement) never has its slot read. Such a slot is mentioned
/// exactly once in the whole body — the store itself — so removing it is safe
/// without any liveness analysis: a slot read anywhere (a load, an ALU memory
/// operand, an address computation) would be mentioned again.
fn dead_stores(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for line in &lines {
        for slot in slots_in(line) {
            *counts.entry(slot).or_insert(0) += 1;
        }
    }
    let mut out = String::with_capacity(body.len());
    for line in &lines {
        if let Some(slot) = store_slot(line) {
            if counts.get(slot).copied().unwrap_or(0) <= 1 {
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Drop every `mov` store to a slot the body never reads. The generator stores
/// each local to its own frame slot as it is produced, even when the value is
/// then only read out of a scratch slot or a register; once the peephole passes
/// rewrite the reads, those stores have no reader left.
fn drop_unread_stores(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut read: HashSet<String> = HashSet::new();
    for line in &lines {
        let t = line.trim();
        let Some((_, rest)) = t.split_once(char::is_whitespace) else { continue };
        // A `mov`'s destination is write-only; every other operand is a read.
        match rest.split_once(',') {
            Some((src, dst)) => {
                for slot in slots_in(src) {
                    read.insert(slot.to_string());
                }
                if !dst.trim().ends_with("(%rbp)") || store_slot(line).is_none() {
                    for slot in slots_in(dst) {
                        read.insert(slot.to_string());
                    }
                }
            }
            None => {
                for slot in slots_in(rest) {
                    read.insert(slot.to_string());
                }
            }
        }
    }
    let mut out = String::with_capacity(body.len());
    for line in &lines {
        if line.trim().starts_with("mov") {
            if let Some(slot) = store_slot(line) {
                if !read.contains(slot) {
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Every `-N(%rbp)` slot mentioned on `line`, as borrowed slices of it.
fn slots_in(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            // Walk back over the digits and the sign to the start of the offset.
            let mut j = i;
            while j > 0 && (bytes[j - 1].is_ascii_digit() || bytes[j - 1] == b'-') {
                j -= 1;
            }
            if let Some(end) = line[i..].find(')') {
                found.push(&line[j..i + end + 1]);
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    found
}

/// The slot a line stores to, when it is a plain store (`mov <src>, <slot>` or
/// an ALU op whose destination is memory). Returns `None` for a load.
fn store_slot(line: &str) -> Option<&str> {
    let t = line.trim();
    let (_, dst) = t.split_once(',')?;
    let dst = dst.trim();
    if !dst.ends_with("(%rbp)") {
        return None;
    }
    let op = t.split_whitespace().next()?;
    // `mov`, `add`, `sub`, `imul`, `and`, `or`, `xor` write their destination;
    // a load (`mov <slot>, %reg`) has a register destination and is excluded
    // above by the `%rbp` suffix check.
    matches!(op, "mov" | "movq" | "add" | "sub" | "imul" | "and" | "or" | "xor")
        .then_some(dst)
}

/// Drop a register store that the very next instruction reloads into the same
/// register: the value is already there, so the reload is dead.
///
/// The generator stores almost every scalar result to its frame slot and then
/// loads it back for the next statement (`mov %rax, -216(%rbp)` / `mov
/// -216(%rbp), %rax`). Those pairs are adjacent by construction and removing
/// them is purely local: a store to memory is not observable, so skipping a
/// load of a value that is already in the register cannot change behaviour.
/// The peephole pipeline, run to a fixed point. Each pass exposes work for the
/// others: forwarding a store to its reader lets the store be dropped, which
/// lets the arithmetic that fed it fuse into a `lea`.
fn optimize(body: &str) -> String {
    let mut body = peephole(body);
    body = drop_dead_jumps(&body);
    for _ in 0..2 {
        body = fuse_imm(&body);
        body = fuse_lea(&body);
        body = coalesce_scratch(&body);
        body = forward_stores(&body);
        body = fold_addresses(&body);
        body = dead_stores(&body);
        body = drop_unread_stores(&body);
    }
    body
}

/// Drop a `jmp` whose target is the very next label. The return path always
/// jumps to the epilogue, which the block emitter places immediately after it.
fn drop_dead_jumps(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    for (i, line) in lines.iter().enumerate() {
        if let Some(target) = line.trim().strip_prefix("jmp ") {
            if let Some(next) = lines.get(i + 1) {
                if next.trim() == format!("{target}:") {
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn peephole(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((reg, slot)) = store_pair(lines[i]) {
            if i + 1 < lines.len() {
                if let Some((lsrc, ldst)) = load_pair(lines[i + 1]) {
                    if lsrc == slot {
                        // The value just stored is still in `reg`, so the load
                        // is a memory round-trip for nothing: read `reg` instead
                        // (and drop the load when it targets the same register).
                        out.push_str(lines[i]);
                        out.push('\n');
                        if ldst != reg {
                            out.push_str(&format!("  mov {reg}, {ldst}\n"));
                        }
                        i += 2;
                        continue;
                    }
                }
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// `mov <reg>, <slot>` with a register source, returning `(reg, slot)`.
fn store_pair(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("mov ")?;
    let (src, dst) = rest.split_once(',')?;
    let (src, dst) = (src.trim(), dst.trim());
    if !src.starts_with('%') || src.contains('(') || !dst.contains('(') {
        return None;
    }
    Some((src, dst))
}

/// `mov <slot>, <reg>` with a memory source, returning `(slot, reg)`.
fn load_pair(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("mov ")?;
    let (src, dst) = rest.split_once(',')?;
    let (src, dst) = (src.trim(), dst.trim());
    if !src.contains('(') || !dst.starts_with('%') {
        return None;
    }
    Some((src, dst))
}

fn align_up(v: i64, a: i64) -> i64 {
    (v + a - 1) / a * a
}

/// `mov $v, reg`, using `movabs` only when `v` does not fit a sign-extended
/// 32-bit immediate (the shorter encoding for the common small divisor).
fn mov_imm_str(v: i64, reg: &str) -> String {
    if (-2_147_483_648..=2_147_483_647).contains(&v) {
        format!("  mov ${v}, {reg}")
    } else {
        format!("  movabs ${v}, {reg}")
    }
}

/// Magic multiplier, shift and add-flag for signed division by `d > 1`, from
/// Hacker's Delight via LLVM's `SignedDivisionByConstantInfo`. For a dividend
/// `x`: `t = high64(signed(m) * x); if add { t += x } t >>= s; q = t + (x >> 63)`.
fn sdiv_magic(d: u64) -> (i64, u32, bool) {
    const N: u32 = 64;
    let smin: u128 = 1u128 << (N - 1);
    let ad = d as u128;
    // NC is the largest dividend whose remainder modulo `d` is `d - 1`.
    let anc = smin - 1 - (smin % ad);
    let (mut q1, mut r1) = (smin / anc, smin % anc);
    let (mut q2, mut r2) = (smin / ad, smin % ad);
    let mut p = N - 1;
    loop {
        p += 1;
        q1 <<= 1;
        r1 <<= 1;
        if r1 >= anc {
            q1 += 1;
            r1 -= anc;
        }
        q2 <<= 1;
        r2 <<= 1;
        if r2 >= ad {
            q2 += 1;
            r2 -= ad;
        }
        let delta = ad - r2;
        if !(q1 < delta || (q1 == delta && r1 == 0)) {
            break;
        }
    }
    let m = q2 + 1;
    let add = m > i64::MAX as u128;
    (m as i64, p - N, add)
}

/// The value of an `$imm` operand string, or `None` when it is not an immediate.
fn imm_value(op: &str) -> Option<i64> {
    op.trim().strip_prefix('$')?.parse::<i64>().ok()
}

/// Whether `op` is an immediate that does not fit a sign-extended i32 field, so
/// it cannot be encoded as an ALU or memory-store operand.
fn is_wide_imm(op: &str) -> bool {
    imm_value(op).map_or(false, |v| !(i32::MIN as i64..=i32::MAX as i64).contains(&v))
}

fn ty_size(ty: &Ty) -> i64 {
    match ty {
        Ty::I64 | Ty::Bool | Ty::Int | Ty::Ptr(_) | Ty::Ref(_) | Ty::Other | Ty::Unit => 8,
        Ty::Struct(_) => 8, // refined by the aggregate's own length
        Ty::Array(elem, n) => ty_size(elem) * (*n as i64).max(1),
        Ty::Tuple(tys) => tys.iter().map(|t| ty_size(t)).sum::<i64>().max(8),
        _ => 8,
    }
}

/// The element type of an array, or the pointee of a reference; anything else
/// is treated as an 8-byte scalar.
fn elem_ty(ty: &Ty) -> Ty {
    match ty {
        Ty::Array(elem, _) => (**elem).clone(),
        Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
        _ => Ty::I64,
    }
}

fn is_compare(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// The ALU mnemonic for a binary op that maps to one instruction.
fn alu_mnem(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "imul",
        BinOp::BitAnd => "and",
        BinOp::BitOr => "or",
        BinOp::BitXor => "xor",
        _ => unreachable!(),
    }
}

/// True when `op` is a bare read of `local` (so an in-place update can skip the
/// `mov reg, reg` that would load it into its own destination).
fn operand_is_local(op: &Operand, local: u32) -> bool {
    match op {
        Operand::Copy(p) | Operand::Move(p) => p.local == local && p.proj.is_empty(),
        Operand::Const(_) | Operand::Fmt { .. } => false,
    }
}

/// The ALU mnemonic whose `OF` flag is the overflow condition of a
/// `*WithOverflow` rvalue.
fn ovf_mnem(op: BinOp) -> &'static str {
    match op {
        BinOp::Sub => "sub",
        BinOp::Mul => "imul",
        _ => "add",
    }
}

fn jcc(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "je",
        BinOp::Ne => "jne",
        BinOp::Lt => "jl",
        BinOp::Le => "jle",
        BinOp::Gt => "jg",
        BinOp::Ge => "jge",
        _ => "je",
    }
}

/// The complement of a condition, so a test can jump on `!cond` directly.
fn negate_cc(op: BinOp) -> BinOp {
    match op {
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        BinOp::Lt => BinOp::Ge,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        BinOp::Ge => BinOp::Lt,
        _ => op,
    }
}

/// The local whose value is the discriminant of a `switchInt(Lt(..))`.
fn operand_place(op: &Operand) -> Option<&Place> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => Some(p),
        _ => None,
    }
}

fn setcc(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "sete",
        BinOp::Ne => "setne",
        BinOp::Lt => "setl",
        BinOp::Le => "setle",
        BinOp::Gt => "setg",
        BinOp::Ge => "setge",
        _ => "sete",
    }
}

/// Count how often each local is read, and note which locals are projected or
/// have their address taken (both disqualify promotion).
fn scan_rv(
    rv: &Rvalue,
    counts: &mut HashMap<u32, u32>,
    projected: &mut HashSet<u32>,
    addr_taken: &mut HashSet<u32>,
) {
    match rv {
        Rvalue::Use(op) => scan_operand(op, counts, projected),
        Rvalue::Binary(_, a, b) => {
            scan_operand(a, counts, projected);
            scan_operand(b, counts, projected);
        }
        Rvalue::Unary(_, a) => scan_operand(a, counts, projected),
        Rvalue::Cast { op, .. } => scan_operand(op, counts, projected),
        Rvalue::Repeat { op, .. } => scan_operand(op, counts, projected),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            for op in ops {
                scan_operand(op, counts, projected);
            }
        }
        Rvalue::Ref { place, .. } | Rvalue::RawRef { place, .. } => {
            bump(place, counts, projected);
            addr_taken.insert(place.local);
        }
        Rvalue::Len(p) => {
            bump(p, counts, projected);
            addr_taken.insert(p.local);
        }
        Rvalue::Discriminant(p) => {
            bump(p, counts, projected);
            addr_taken.insert(p.local);
        }
        Rvalue::WithOverflow { a, b, .. } => {
            scan_operand(a, counts, projected);
            scan_operand(b, counts, projected);
        }
        Rvalue::Unsupported(_) => {}
    }
}

fn scan_operand(op: &Operand, counts: &mut HashMap<u32, u32>, projected: &mut HashSet<u32>) {
    if let Operand::Copy(p) | Operand::Move(p) = op {
        // An index local inside a place (`_1[_7]`) is read as often as the place
        // is, so count it toward promotion.
        for proj in &p.proj {
            if let Proj::IndexLocal(i) = proj {
                *counts.entry(*i).or_insert(0) += 1;
            }
        }
        bump(p, counts, projected);
    }
}

fn scan_term(
    term: &Terminator,
    counts: &mut HashMap<u32, u32>,
    projected: &mut HashSet<u32>,
    _addr_taken: &mut HashSet<u32>,
) {
    match term {
        Terminator::SwitchInt { discr, .. } => scan_operand(discr, counts, projected),
        Terminator::Call { dest, args, .. } => {
            bump(dest, counts, projected);
            for a in args {
                scan_operand(a, counts, projected);
            }
        }
        _ => {}
    }
}

fn bump(p: &Place, counts: &mut HashMap<u32, u32>, projected: &mut HashSet<u32>) {
    *counts.entry(p.local).or_insert(0) += 1;
    // A projection that starts with `Deref` reads the local's *value* (a
    // pointer) and then dereferences it, so the local can still be promoted: a
    // promoted pointer is just its register. A projection with no leading
    // `Deref` (`_2.0`, `_2[3]`) reads out of the local's own storage, which a
    // promoted scalar no longer has, so it disqualifies promotion.
    if !p.proj.is_empty() && !matches!(p.proj.first(), Some(Proj::Deref)) {
        projected.insert(p.local);
    }
}

