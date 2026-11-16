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

/// A loop whose body holds at least this many MIR statements is pinned to a
/// 16-byte boundary (`loop_bodies_to_align`). Below it the body is short enough
/// to sit inside one fetch window already, and pinning it only adds padding.
const LOOP_ALIGN_MIN_STMTS: usize = 10;

pub struct Options {
    /// Promote hot locals into callee-saved registers (on by default).
    ///
    /// There is no overflow-check switch: on this path the trap is an `assert`
    /// rustc already put in the MIR (`AddWithOverflow` + `assert` for checked
    /// arithmetic, a bare `Add` for the wrapping forms), so the back end must
    /// not add one of its own.
    pub promote: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { promote: true }
    }
}

/// A promoted local: which register holds it, and whether that register has to
/// be saved/restored across a call.
#[derive(Clone, Copy)]
/// Where a place's bytes are, when one x86 addressing mode reaches them: a
/// base (a frame local, or a pointer local), a constant displacement, and at
/// most one index local scaled by 1, 2, 4 or 8.
struct MemPlan {
    /// `None`: the frame (`%rbp` plus the local's offset); `Some(l)`: the
    /// pointer held in local `l`.
    ptr: Option<u32>,
    frame_local: u32,
    disp: i64,
    index: Option<(u32, i64)>,
}

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
    /// True while emitting a destructor tree. The tree makes calls that clobber
    /// the caller-saved registers, and the promoted caller-saved locals are
    /// spilled once at its start, so a read inside the tree must take the value
    /// from the spill slot rather than the (by then clobbered) register.
    in_drop: bool,
    /// Frame slot holding the caller-supplied address of the return place, when
    /// this function returns an aggregate (`-1` otherwise). The address arrives
    /// in `%rdi` under the SysV ABI, the same convention rustc uses, so the
    /// arguments of such a function are shifted one register along.
    sret_slot: i64,
    /// Locals that are an operand of a loop's counted comparison (the counter
    /// and its bound), promoted first.
    loop_operands: Vec<u32>,
    /// Wide constants that recur in the body, hoisted into spare callee-saved
    /// registers: immediate value -> register. A 64-bit immediate needs a
    /// `movabs` per use, so a constant named in a loop is materialised once at
    /// entry instead. Only callee-saved registers are used, so no call can
    /// clobber the value.
    const_regs: HashMap<i64, &'static str>,
    /// Wide constants hoisted into *caller-saved* registers once the callee-saved
    /// pool is exhausted. A call clobbers these, so each gets a spill slot and is
    /// saved and restored around every call.
    const_caller_regs: HashMap<i64, &'static str>,
    /// Hoisted-constant value -> its caller-saved spill slot.
    const_spill: HashMap<i64, i64>,
    /// Callee-saved registers saved below `%rbp`; frame slots start past them.
    saved_regs: i64,
    /// Per block, the locals live at its entry. A promoted caller-saved local is
    /// spilled around a call only when it is live after it, so a value that is
    /// dead (or not yet defined) at the call costs no store/reload pair.
    live_in: Vec<HashSet<u32>>,
    /// Locals that may not share a register with another local: their address
    /// is taken, so a later read through the pointer expects the value to stay.
    no_share: HashSet<u32>,
    /// Interference: for each local, the locals live at the same time (or read
    /// by the statement that defines it). Two locals that do not interfere may
    /// hold the same register at different times.
    interf: HashMap<u32, HashSet<u32>>,
    /// Register -> the promoted locals assigned to it.
    reg_owners: HashMap<&'static str, Vec<u32>>,
    /// Copy-related locals (`d = copy s`, `d = s as T`): giving both the same
    /// register turns the copy into nothing, so each is tried in the other's
    /// register first.
    affinity: HashMap<u32, Vec<u32>>,
    /// Whether the body jumped to the shared panic exit (`trap_label`).
    trap_used: bool,
    /// Loop depth of each block (`loop_depths`), for branch layout.
    block_depth: Vec<u32>,
    /// Dry-pass slot index -> compacted slot index. `None` until the dry pass
    /// has revealed which slots the body actually touches.
    slot_map: Option<Vec<i64>>,
    next_slot: i64,
    /// Slot spans that must stay contiguous in the packed frame. `gen_unsize`
    /// writes a slice record through a base pointer plus byte offsets, so only
    /// the base appears in the emitted text; without this the frame packer
    /// would drop the interior slots and the elements would land on dead frame.
    pinned: Vec<(i64, i64)>,
    /// Deref trampolines for `println!("{}", ref)`-style placeholders, keyed by
    /// `(depth, label)`; appended after the function body. See
    /// [`FnGen::deref_fmt_label`].
    deref_tramps: Vec<(String, String)>,
    out: String,
    label: u64,
    errors: Vec<String>,
}

/// Generate x86-64 assembly for `prog`, ready to concatenate with the runtime.
pub fn compile(prog: &Program, opts: &Options) -> Result<String, String> {
    let mut asm = String::new();
    asm.push_str(".text\n");

    // String literals become `.rodata` labels, emitted once.
    let (strings, string_ids) = collect_strings(prog);
    // `format_args!` templates are byte-string literals: raw bytes, no length
    // word, because the runtime walks them and reads the length from the first
    // byte of each piece.
    let (templates, template_ids) = collect_templates(prog);

    // Every static allocation becomes a `.rodata` blob: its bytes, plus a
    // `.quad` for each interior pointer relocation. `alloc1` (string data) and
    // `alloc2` (the promoted `static` pointer) are both reached this way.
    let alloc_labels: HashMap<u32, String> =
        prog.allocs.keys().map(|id| (*id, format!("__rmc_alloc{id}"))).collect();

    // Enums first: `collect_structs` consults the variant tables to tell a
    // path-qualified struct constant (`std::ops::Range::<usize> { .. }`) from an
    // enum-variant constant (`Option::Some(..)`).
    let enums = collect_enums(prog);
    let structs = collect_structs(prog, &enums);

    // Emit only functions the program can reach from `main` (dead-function
    // elimination keeps the artifact small and the compile fast). `drop` glue
    // needs the struct layouts, so this runs after `collect_structs`.
    let live = reachable(prog, &structs, &enums);
    let order: Vec<&Function> = prog.funcs.iter().filter(|f| live.contains(&f.name)).collect();

    // Deref trampolines for reference placeholders are module-scoped and emitted
    // once, after every function body (see `FnGen::deref_fmt_label`).
    let mut deref_names: HashSet<String> = HashSet::new();
    let mut deref_tramps: Vec<(String, String)> = Vec::new();

    for f in &order {
        if is_unsupported_builtin(&f.name) {
            continue;
        }
        let mut g = FnGen::new(prog, f, opts, &string_ids, &template_ids, &alloc_labels, &structs, &enums);
        let body = g.gen_fn()?;
        asm.push_str(&body);
        for (name, text) in g.deref_tramps {
            // Module-scoped, so two functions that both format a `&i64` share
            // one trampoline rather than emitting a duplicate label.
            if deref_names.insert(name.clone()) {
                deref_tramps.push((name, text));
            }
        }
        if !g.errors.is_empty() {
            return Err(g.errors.join("\n"));
        }
    }

    for (_, text) in &deref_tramps {
        asm.push_str(text);
    }

    let mut rodata = String::new();
    if !strings.is_empty() || !templates.is_empty() {
        rodata.push_str("\n.section .rodata\n");
    }
    if !strings.is_empty() {
        for s in &strings {
            let label = &string_ids[s];
            // A `&str`/`&[u8]` literal is a `{len: i64, bytes..}` record, the
            // same shape the AST path uses and the one `print_str` and
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
fn reachable(
    prog: &Program,
    structs: &HashMap<String, Vec<Ty>>,
    enums: &HashMap<String, EnumLayout>,
) -> HashSet<String> {
    let mut live = HashSet::new();
    let Some(main) = prog.func("main") else { return live };
    let mut stack = vec![main.name.clone()];
    while let Some(name) = stack.pop() {
        if !live.insert(name.clone()) {
            continue;
        }
        if let Some(f) = prog.func(&name) {
            for b in &f.blocks {
                for s in &b.stmts {
                    if let Stmt::Assign(_, rv) = s {
                        for op in rv_all_operands(rv) {
                            if let Operand::Fn(n) = op {
                                if prog.func(n).is_some() {
                                    stack.push(n.clone());
                                }
                            }
                        }
                    }
                }
                match &b.term {
                    Terminator::Call { func: CallTarget::Direct(callee), .. } => {
                        if prog.func(callee).is_some() {
                            stack.push(callee.clone());
                        }
                    }
                    // An indirect call whose callee is a named function still
                    // needs that function emitted; a computed callee is a value,
                    // not a name, and names no function on its own.
                    Terminator::Call { func: CallTarget::Indirect(Some(Operand::Fn(callee))), .. } => {
                        if prog.func(callee).is_some() {
                            stack.push(callee.clone());
                        }
                    }
                    // A `drop(_N)` terminator runs the destructor of `_N`'s
                    // type, which may be a nested field's impl; none of them are
                    // named by a `Call`, so collect them all here. The base
                    // local's type covers a projected place too (`drop((_2.1: G))`
                    // needs `_2`'s field impls), and copy propagation often turns
                    // a plain drop into a projected one, so the projection must
                    // not be required to be empty.
                    Terminator::Drop { place, .. } => {
                        if let Some(ty) = f.locals.get(place.local as usize).map(|l| &*l.ty) {
                            let mut syms = Vec::new();
                            collect_drop_symbols(prog, structs, enums, ty, &mut syms);
                            stack.extend(syms);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    live
}

/// Every `__rmc_drop_<T>` symbol a destructor of `ty` needs: its own impl, or
/// (for an aggregate without one) the impls of its droppable fields.
fn collect_drop_symbols(
    prog: &Program,
    structs: &HashMap<String, Vec<Ty>>,
    enums: &HashMap<String, EnumLayout>,
    ty: &Ty,
    out: &mut Vec<String>,
) {
    match ty {
        Ty::Struct(name) => {
            // A type with its own impl still drops its fields afterwards, so
            // record the impl *and* recurse, exactly as `gen_drop_glue` emits.
            if let Some(sym) = prog.drop_impls.get(name) {
                out.push(sym.clone());
            }
            if let Some(layout) = enums.get(name) {
                // An enum's payload fields are variant-dependent; collect every
                // variant's droppable fields' symbols.
                for tys in layout.fields.values() {
                    for f in tys {
                        collect_drop_symbols(prog, structs, enums, f, out);
                    }
                }
            } else if let Some(fields) = structs.get(name) {
                for f in fields {
                    collect_drop_symbols(prog, structs, enums, f, out);
                }
            }
        }
        Ty::Array(elem, _) => collect_drop_symbols(prog, structs, enums, elem, out),
        Ty::Tuple(tys) => {
            for t in tys {
                collect_drop_symbols(prog, structs, enums, t, out);
            }
        }
        _ => {}
    }
}

/// Whether `name` can reach a call to `callee` through direct calls.
/// A rustc-internal shim rmc cannot compile. A user function qualified by its
/// module (`a::helper`) or an impl method keyed by receiver (`a::A__value`) also
/// contains `::`, so it must *not* be skipped here: it is emitted normally and
/// reached by the same symbol its call sites use.
fn is_unsupported_builtin(name: &str) -> bool {
    name.starts_with('<') || name == "rust_begin_unwind"
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

/// How many leading `&` (or `&mut `) a type text carries: `&i64` is 1,
/// `&&str` is 2. Used to pick the deref trampoline a reference placeholder
/// needs. `str` itself is a slice header, not a pointer, so `&str` is 0 here.
fn ref_depth(ty_name: &str) -> usize {
    let mut s = ty_name.trim();
    let mut n = 0;
    while let Some(r) = s.strip_prefix('&') {
        s = r.trim_start_matches("mut ").trim_start();
        n += 1;
    }
    n
}

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
    // A float has its own decimal renderer per width. It must be checked before
    // `int_kind`, which would otherwise read an `f64` as a signed word and print
    // its bit pattern as a huge integer.
    if base == "f64" {
        return "__rmc_fmt_f64";
    }
    if base == "f32" {
        return "__rmc_fmt_f32";
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
        true if bits == 8 => "__rmc_fmt_i8",
        true if bits == 32 => "__rmc_fmt_i32",
        true => "__rmc_fmt_i64",
        false if bits == 8 => "__rmc_fmt_u8",
        false if bits == 32 => "__rmc_fmt_u32",
        false => "__rmc_fmt_u64",
    }
}

/// Whether a type name is a scalar the runtime has a formatter for: an integer
/// of any width, a float, `bool`, `char` or `str`. A reference is not stripped
/// here; the caller does that.
fn is_scalar_ty(name: &str) -> bool {
    let n = name.trim();
    if matches!(n, "bool" | "char" | "str") {
        return true;
    }
    // An integer or float: `i`/`u` + digits, or `f32`/`f64`.
    let (sigil, rest) = n.split_at(1.min(n.len()));
    if sigil == "f" {
        return matches!(rest, "32" | "64");
    }
    if sigil == "i" || sigil == "u" {
        return rest == "size" || rest.parse::<u32>().is_ok();
    }
    false
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
            // A string literal can also be a *terminator* operand: a call
            // argument (`"x".contains("ell")` passes `"ell"` straight to the
            // callee), a `switchInt` discriminant, or an `assert` condition.
            // Those never appear in a statement, so without this scan
            // `str_label` falls back to emitting the raw text as a symbol name.
            let mut term_ops: Vec<&Operand> = Vec::new();
            match &b.term {
                Terminator::Call { args, .. } => term_ops.extend(args.iter()),
                Terminator::SwitchInt { discr, .. } => term_ops.push(discr),
                Terminator::Assert { cond, .. } => term_ops.push(cond),
                _ => {}
            }
            for op in term_ops {
                if let Operand::Const(Const::Str(s)) = op {
                    if !map.contains_key(s) {
                        map.insert(s.clone(), format!("__rmc_lit{}", map.len()));
                        order.push(s.clone());
                    }
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

/// Struct name -> field types as the back end lays them out, for the MIR
/// passes. Enums and structs whose shape was never seen are left out.
pub(crate) fn struct_shapes(prog: &Program) -> HashMap<String, Vec<Ty>> {
    let enums = collect_enums(prog);
    let mut shapes = collect_structs(prog, &enums);
    shapes.retain(|name, fields| !enums.contains_key(name) && !fields.is_empty());
    shapes
}

/// Struct name -> field types, learned from aggregate constructions.
fn collect_structs(prog: &Program, enums: &HashMap<String, EnumLayout>) -> HashMap<String, Vec<Ty>> {
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
                    // field list, so learn the struct's shape from it too. A
                    // path-qualified name (`std::ops::Range::<usize> { .. }`) is
                    // registered under its last segment (`Range`) so it matches
                    // the key `materialize_const_agg` looks up; a variant is not,
                    // or it would shadow the enum's own layout.
                    let mut aggs = Vec::new();
                    rv_const_aggs(rv, &mut aggs);
                    for (name, ops) in aggs {
                        if name.is_empty() || is_variant_const(&name, &enums) {
                            continue;
                        }
                        let key = struct_path_name(&name).unwrap_or_else(|| name.clone());
                        let tys = ops.iter().map(|op| rv_operand_ty(f, op)).collect();
                        structs.insert(key, tys);
                    }
                }
            }
        }
    }
    // A `Ty::Struct` keeps only the last path segment of a type's name, so two
    // distinct types can share one key (`std::...::HashMap` and
    // `hashbrown::...::HashMap` both key `HashMap`). When their field lists then
    // point at each other, the layout reads as a self-referential value, which
    // no Rust struct can be without indirection. Every walk over the layouts
    // (`size_of_ty`, `struct_layout`, drop glue) would then recurse forever, so
    // drop the cyclic layouts; a walker that misses one falls back to a scalar.
    let cyclic: Vec<String> = structs
        .keys()
        .filter(|n| struct_on_cycle(n, &structs))
        .cloned()
        .collect();
    for n in cyclic {
        structs.remove(&n);
    }
    structs
}

/// The struct names a type mentions, at any depth through `Tuple`/`Array`.
fn ty_struct_names(ty: &Ty) -> Vec<String> {
    fn walk(ty: &Ty, out: &mut Vec<String>) {
        match ty {
            Ty::Struct(n) => out.push(n.clone()),
            Ty::Array(e, _) => walk(e, out),
            Ty::Tuple(ts) => ts.iter().for_each(|t| walk(t, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(ty, &mut out);
    out
}

/// Whether `name` can reach itself through one or more field edges.
fn struct_on_cycle(name: &str, structs: &HashMap<String, Vec<Ty>>) -> bool {
    let Some(fields) = structs.get(name) else {
        return false;
    };
    let mut stack: Vec<String> = fields.iter().flat_map(ty_struct_names).collect();
    let mut seen = HashSet::new();
    while let Some(n) = stack.pop() {
        if n == name {
            return true;
        }
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Some(fs) = structs.get(&n) {
            stack.extend(fs.iter().flat_map(ty_struct_names));
        }
    }
    false
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

/// An assembler-safe spelling of an IR symbol. A user type keeps its module
/// path (`a::A__value`) so two same-named types stay distinct, but `::` is the
/// section separator in assembler syntax and would split the label. `$` is a
/// valid symbol character in GNU `as`, the in-process assembler and the ELF
/// symbol table, and cannot appear in a Rust identifier, so it is a safe
/// stand-in. A name with no `::` is returned unchanged, so every symbol rmc
/// emits today is byte-for-byte identical.
fn asm_sym(name: &str) -> String {
    if name.contains("::") {
        name.replace("::", "$")
    } else {
        name.to_string()
    }
}

/// Split `Enum::Variant` or `Option::<i64>::Some` into the enum and variant
/// names, dropping any generic arguments.
fn split_ev(name: &str) -> (String, String) {
    // A bare variant (`Some(1)`, the enum dropped because the type is
    // inferred) has no `::`; the whole name is the variant and the enum is
    // unknown here. `is_variant_const` resolves it against the variant tables.
    let Some(idx) = name.rfind("::") else {
        return (String::new(), name.trim().to_string());
    };
    let variant = name[idx + 2..].trim().to_string();
    let head = &name[..idx];
    let head = head.split("::<").next().unwrap_or(head);
    // Keep the enum's whole path, matching the identity `parse_ty` gave the
    // enum type; `canonicalize_types` has reconciled the two spellings.
    (head.trim().to_string(), variant)
}

/// The struct name a path-qualified aggregate constant names, if the path's
/// last segment is a plausible Rust *type* name. The whole pre-generic path is
/// kept (`std::ops::Range`, `a::A`) so the key matches the identity `parse_ty`
/// gave the same type; `canonicalize_types` has already shortened a path whose
/// last segment is unambiguous, so a type the printer spelled bare in its own
/// module resolves to the one qualified spelling.
///
/// A variant is a CamelCase identifier and so is a type, so this cannot
/// separate `Option::Some` from `Range` on shape alone. The caller resolves the
/// ambiguity with the variant table `collect_enums` already built.
fn struct_path_name(name: &str) -> Option<String> {
    let head = name.split("::<").next().unwrap_or(name);
    let last = head.rsplit("::").next().unwrap_or(head).trim();
    let ok = !last.is_empty()
        && last.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| crate::mir::struct_const_identity(name))
}

/// Whether a constant named `name` is an enum variant rather than a struct,
/// given the variant tables `collect_enums` has already built. A bare `Some(1)`
/// (the printer drops the enum when the type is inferred) has no `::`, so its
/// head is the variant itself; a path-qualified name is a variant only if its
/// head names a known enum that has that variant.
fn is_variant_const(name: &str, enums: &HashMap<String, EnumLayout>) -> bool {
    if name.contains("::") {
        let (e, v) = split_ev(name);
        // A path-qualified constant is a variant only if the segment before it
        // is a type name. `b::E::B` has `E`; a struct constant qualified by its
        // module (`a::A { .. }`) has the lowercase module there, so it stays a
        // struct rather than gaining a phantom tag word.
        let last = e.rsplit("::").next().unwrap_or(&e).trim();
        last.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && enums.get(&e).is_some_and(|l| l.fields.contains_key(&v))
    } else {
        enums.values().any(|l| l.fields.contains_key(name.trim()))
    }
}

/// Whether `s` is a Rust identifier, so a `split_ev` "variant" is a real
/// variant name and not a generic argument. `std::ops::Range::<usize>` splits
/// to variant `"<usize>"`, which is not an identifier, and so is a struct, not
/// an enum.
fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
                        // `std::ops::Range::<usize> { start, end }` splits to a
                        // variant that is not an identifier, and `a::A { v: .. }`
                        // to one whose "enum" is the lowercase module `a`: both
                        // are structs the printer spelled with a path, not enum
                        // variants, and registering them would give the struct a
                        // phantom tag word. Skip them; `collect_structs` claims
                        // them instead.
                        if !is_ident(&v) {
                            continue;
                        }
                        let enum_seg = e.rsplit("::").next().unwrap_or(&e).trim();
                        if !enum_seg.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                            continue;
                        }
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

    // `Option` and `Result` use rustc's *declaration-order* discriminants
    // (`None`=0, `Some`=1; `Ok`=0, `Err`=1), and rustc folds a `discriminant`
    // comparison on a niche-optimised enum (`Option<&T>`, `Result<T,&E>`) to a
    // literal with those values - `is_some` is `discriminant(_x) == 1`. The tag
    // evidence above only comes from a `switchInt` on the discriminant, which a
    // folded comparison never emits, so a niche enum would otherwise take its
    // tags from *construction* order (`Some` = 0) and `is_some` would read
    // false. Seed the declaration order; real `switchInt` evidence still wins,
    // because only a missing tag is filled.
    for (e, vs) in [
        ("Option", ["None", "Some"]),
        ("Result", ["Ok", "Err"]),
    ] {
        let ord = order.entry(e.to_string()).or_default();
        for v in vs {
            if !ord.iter().any(|x| x == v) {
                ord.push(v.to_string());
            }
        }
        let tm = tags.entry(e.to_string()).or_default();
        for (i, v) in vs.iter().enumerate() {
            tm.entry(v.to_string()).or_insert(i as u64);
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
        // The widest variant decides the enum's size: the tag plus one word per
        // payload field. A `Rect { w, h }`-shaped variant needs two payload
        // words, so a fixed `tag + 1` would undersize the enum and an array of
        // it would overlap its neighbour. This holds for a float payload too,
        // which is one word here: `size_of_ty` must still reserve the right
        // frame bytes whether or not the payload is otherwise "modelled".
        let max_fields = variants.values().map(|tys| tys.len()).max().unwrap_or(0) as i64;
        let (payload_off, size) = if fieldless {
            (0, 8)
        } else {
            (8, 8 + max_fields * 8)
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
            in_drop: false,
            sret_slot: -1,
            loop_operands: Vec::new(),
            const_regs: HashMap::new(),
            const_caller_regs: HashMap::new(),
            const_spill: HashMap::new(),
            saved_regs: 0,
            live_in: Vec::new(),
            no_share: HashSet::new(),
            interf: HashMap::new(),
            reg_owners: HashMap::new(),
            affinity: HashMap::new(),
            trap_used: false,
            block_depth: Vec::new(),
            slot_map: None,
            next_slot: 0,
            pinned: Vec::new(),
            deref_tramps: Vec::new(),
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
        // The counter restarts in every function (both codegen passes must
        // number identically), so the label carries the function's name like
        // the block labels do. A bare `.Lrmc{n}` was defined once per function:
        // GNU `as` rejected the file, and the in-process assembler resolved
        // every jump to the *last* definition, so a loop in one function
        // jumped into another (the `vm` and `textparse` segfaults).
        format!(".Lrmc_{}_{}", self.label_tag(), self.label)
    }

    /// The function name, reduced to label-safe characters.
    fn label_tag(&self) -> String {
        self.func
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .collect()
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

    fn local_ty(&self, local: u32) -> Ty {
        self.ty.get(local as usize).cloned().unwrap_or(Ty::I64)
    }

    /// A scalar local's home: its promoted register or its frame slot.
    fn local_operand(&self, local: u32) -> String {
        match self.promo.get(&local) {
            Some(p) => {
                // Inside a destructor tree the caller-saved registers have been
                // clobbered by the calls already made, but the local's value was
                // spilled to its caller-spill slot at the tree's start. Read the
                // slot there instead of the stale register.
                if self.in_drop && p.caller_saved {
                    if let Some(slot) = self.caller_spill.get(&local) {
                        return self.off(*slot);
                    }
                }
                p.reg.to_string()
            }
            None => self.off(self.local_base(local)),
        }
    }

    /// True when a scalar local is held in a register (so `p.proj` is empty and
    /// reading it needs no memory access).
    // --- analysis ----------------------------------------------------------

    /// Compute each local's size and type, and find the operands of counted
    /// loop tests (`switchInt(Lt(counter, bound))`).
    fn analyze(&mut self) {
        // A local whose declared type the backend does not model (`Ty::Other`)
        // may still be built from a multi-field aggregate literal: a
        // `std::slice::Iter` is `Ty::Other` but holds two words. Sizing such a
        // local from its type alone would give it one word, so its second field
        // would land in a neighbouring local's frame. The region is widened to
        // the widest aggregate stored into it.
        let mut agg_size = vec![0i64; self.func.locals.len()];
        for b in &self.func.blocks {
            for s in &b.stmts {
                let Stmt::Assign(dest, rv) = s else { continue };
                if !dest.proj.is_empty() {
                    continue;
                }
                let extent = match rv {
                    Rvalue::Aggregate(ops) | Rvalue::Array(ops) => ops
                        .iter()
                        .map(|op| self.size_of_ty(&rv_operand_ty(self.func, op)))
                        .sum::<i64>(),
                    Rvalue::Enum { name, ops } => {
                        let en = split_ev(name).0;
                        match self.enums.get(&en) {
                            Some(l) => l.size.max(8),
                            None => 8 + ops.len() as i64 * 8,
                        }
                    }
                    _ => 0,
                };
                let idx = dest.local as usize;
                if let Some(slot) = agg_size.get_mut(idx) {
                    *slot = (*slot).max(extent);
                }
            }
        }
        // Give every local its own region sized to its type: an aggregate such
        // as `[i64; 200000]` needs 1.6 MB of frame, not one 8-byte slot. A
        // scalar takes one slot. Regions are laid out first so `local_base` can
        // be indexed directly.
        let mut slot = 0i64;
        for (i, l) in self.func.locals.iter().enumerate() {
            let size = self.size_of_ty(&l.ty).max(agg_size[i]);
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
        // One priority order for every candidate: each scalar local that is
        // not projected or address-taken, and each wide constant worth keeping
        // in a register. A use counts 8^depth, so a use in an inner loop
        // outweighs any number in the loop around it -- a linear weight let
        // the counters of outer loops, which an inner loop never touches, hold
        // registers the inner loop's values needed (`bfs` ran 1.7x slower).
        //
        // Locals take a register through the interference graph, so two whose
        // lifetimes never overlap share it (`promote_shared`). A constant is
        // materialised once at entry and stays for the whole function, so it
        // takes a register nothing else owns.
        let depths = self.loop_depths();
        let freq = |bi: usize| -> u32 { 8u32.pow(depths.get(bi).copied().unwrap_or(0).min(5)) };
        let mut weighted: HashMap<u32, u32> = HashMap::new();
        let mut projected: HashSet<u32> = HashSet::new();
        let mut addr_taken: HashSet<u32> = HashSet::new();
        let mut index_locals: HashSet<u32> = HashSet::new();
        let mut in_loop: HashSet<u32> = HashSet::new();
        for (bi, b) in self.func.blocks.iter().enumerate() {
            let w = freq(bi);
            let mut local_counts: HashMap<u32, u32> = HashMap::new();
            for s in &b.stmts {
                if let Stmt::Assign(p, rv) = s {
                    // A written place reads its index locals and, behind a
                    // leading `Deref`, its base pointer, exactly as a read of
                    // the place does. Counted only as `projected`, the index
                    // of every store (`a[i] = x`) had no weight and stayed in
                    // the frame, stored and reloaded on each iteration
                    // (`life`), and a pointer written through never got a
                    // register.
                    if !p.proj.is_empty() {
                        scan_operand(&Operand::Copy(p.clone()), &mut local_counts, &mut projected, &mut index_locals);
                    }
                    scan_rv(rv, &mut local_counts, &mut projected, &mut addr_taken, &mut index_locals);
                }
            }
            scan_term(&b.term, &mut local_counts, &mut projected, &mut addr_taken, &mut index_locals);
            for (l, c) in local_counts {
                *weighted.entry(l).or_insert(0) += c * w;
                if w > 1 {
                    in_loop.insert(l);
                }
            }
        }
        enum Cand {
            Local(u32),
            Const(i64),
        }
        let mut cands: Vec<(u32, u8, i64, Cand)> = weighted
            .iter()
            .filter(|(l, _)| !projected.contains(l) && !addr_taken.contains(l))
            .filter(|(l, _)| !self.local_ty(**l).is_aggregate())
            .map(|(l, w)| (*w, 0u8, *l as i64, Cand::Local(*l)))
            .collect();
        for (v, w) in self.const_candidates(&freq) {
            cands.push((w, 1u8, v, Cand::Const(v)));
        }
        // Every local before any constant, each group hottest first (then by
        // number, so the order is deterministic). A local shares its register
        // with whatever it never overlaps; a constant holds one for the whole
        // function, so a constant used in one loop would take a register from
        // every other loop's values. It gets what the locals leave: inline, a
        // constant costs one `movabs` per use, while a loop value in the frame
        // costs a load and a store on a dependency chain.
        cands.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)).then(a.2.cmp(&b.2)));
        let crossers = self.call_crossers();
        // `RMC_TRACE_ALLOC=1`: every candidate in priority order and where it
        // went, to see why a hot value is in the frame.
        let trace = std::env::var_os("RMC_TRACE_ALLOC").is_some();
        if trace {
            eprintln!("--- alloc: {}", self.func.name);
        }
        for (w, _, _, c) in cands {
            match c {
                Cand::Local(l) => {
                    let got = self.promote_shared(l, crossers.contains(&l), in_loop.contains(&l));
                    if trace && !got {
                        // Which owner of each register it overlaps.
                        let empty = HashSet::new();
                        let mine = self.interf.get(&l).unwrap_or(&empty);
                        let mut why = String::new();
                        for r in PROMOTE_REGS.iter().chain(CALLER_REGS.iter()) {
                            let owners = self.reg_owners.get(r).cloned().unwrap_or_default();
                            let hit: Vec<String> = owners
                                .iter()
                                .filter(|o| mine.contains(o) || self.no_share.contains(o))
                                .map(|o| format!("_{o}"))
                                .collect();
                            let c = self.const_regs.values().chain(self.const_caller_regs.values()).any(|x| x == r);
                            why.push_str(&format!(" {r}:{}{}", hit.join("/"), if c { "const" } else { "" }));
                        }
                        eprintln!("        blocked by{why}");
                    }
                    if trace {
                        let home = if got { self.promo[&l].reg } else { "frame" };
                        let name = self.func.locals[l as usize].name.clone().unwrap_or_default();
                        eprintln!(
                            "  _{l:<4} w={w:<8} {home:6} crosses_call={} loop={} {name}",
                            crossers.contains(&l),
                            in_loop.contains(&l)
                        );
                    }
                }
                Cand::Const(v) => {
                    self.hoist_const(v);
                    if trace {
                        let home = self
                            .const_regs
                            .get(&v)
                            .or_else(|| self.const_caller_regs.get(&v))
                            .copied()
                            .unwrap_or("inline");
                        eprintln!("  const {v} w={w} {home}");
                    }
                }
            }
        }
    }

    /// Wide immediates worth a register, with their weighted use counts: each
    /// constant outside the `i32` range an instruction can encode, and the
    /// magic multiplier a signed division by a constant lowers to (generated,
    /// not named in the source). Two uses break even -- one `movabs` per use
    /// against one at entry and a register read per use -- and a single use in
    /// a loop clears the bar.
    fn const_candidates(&self, freq: &dyn Fn(usize) -> u32) -> Vec<(i64, u32)> {
        let mut counts: HashMap<i64, u32> = HashMap::new();
        for (bi, b) in self.func.blocks.iter().enumerate() {
            let w = freq(bi);
            for s in &b.stmts {
                let mut count_op = |op: &Operand| {
                    if let Operand::Const(Const::Int(v)) = op {
                        if !(-2_147_483_648..=2_147_483_647).contains(v) {
                            *counts.entry(*v).or_insert(0) += w;
                        }
                    }
                };
                scan_operands(s, &mut count_op);
                if let Stmt::Assign(_, Rvalue::Binary(op, _, rhs)) = s {
                    if matches!(op, BinOp::Div | BinOp::Rem) {
                        if let Operand::Const(Const::Int(d)) = rhs {
                            let mag = (*d).unsigned_abs();
                            if *d != 0 && mag > 1 && !mag.is_power_of_two() {
                                let (m, _, _) = sdiv_magic(mag);
                                *counts.entry(m).or_insert(0) += w;
                            }
                        }
                    }
                }
            }
        }
        counts.into_iter().filter(|(_, c)| *c >= 2).collect()
    }

    /// Keep the constant `v` in a register nothing else owns for the whole
    /// function: a callee-saved one first (saved once at entry), else a
    /// caller-saved one, saved and restored around each call
    /// (`spill_caller_saved`). LLVM keeps `stats`'s `% 1000003` multiplier in
    /// a caller-saved register for the whole loop.
    fn hoist_const(&mut self, v: i64) {
        let free = |r: &&'static str, me: &Self| {
            me.reg_owners.get(*r).map(|o| o.is_empty()).unwrap_or(true)
                && !me.const_regs.values().any(|c| *c == *r)
                && !me.const_caller_regs.values().any(|c| *c == *r)
        };
        if let Some(reg) = PROMOTE_REGS.iter().find(|r| free(r, self)).copied() {
            self.const_regs.insert(v, reg);
        } else if let Some(reg) = CALLER_REGS.iter().find(|r| free(r, self)).copied() {
            self.const_caller_regs.insert(v, reg);
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
        loop_depths_of(&succ)
    }

    /// The loop-body entry block of each natural loop, to pin to a 16-byte
    /// boundary. A loop's body is the hot region, and its speed is otherwise an
    /// alignment lottery: a one-byte shift anywhere earlier in the function
    /// moves it relative to the fetch windows, which measured up to 1.4x on
    /// `stats`'s division loop. The body entry is the header's successor that
    /// belongs to the loop; that is the block the loop runs first, whether the
    /// header falls through to it or jumps.
    ///
    /// A back edge here is an edge `latch -> header` with `header` before
    /// `latch` in block order and a cycle back from `header` to `latch`. Block
    /// order is the emission order, so a lower index means an earlier address
    /// and the edge really does jump backwards. Dominance would be the textbook
    /// test, but `stats` reaches its latch from a block outside the loop as
    /// well, so its header does not strictly dominate the latch; the index
    /// order identifies that loop where dominance would miss it.
    fn loop_bodies_to_align(&self) -> HashSet<u32> {
        let n = self.func.blocks.len();
        let succ: Vec<Vec<u32>> = self
            .func
            .blocks
            .iter()
            .map(|b| successors(&b.term).into_iter().filter(|s| (*s as usize) < n).collect())
            .collect();
        let reach = block_reachability(&succ);
        let mut bodies: HashSet<u32> = HashSet::new();
        for (latch, s) in succ.iter().enumerate() {
            for &header in s {
                let h = header as usize;
                if h >= latch || !reach[h][latch] {
                    continue;
                }
                let body: Vec<u32> = (0..n)
                    .filter(|x| *x == h || (reach[h][*x] && reach[*x][latch]))
                    .map(|x| x as u32)
                    .collect();
                let stmts: usize = body
                    .iter()
                    .map(|x| self.func.blocks[*x as usize].stmts.len())
                    .sum();
                if stmts < LOOP_ALIGN_MIN_STMTS {
                    continue;
                }
                let members: HashSet<u32> = body.iter().copied().collect();
                let entry = successors(&self.func.blocks[h].term)
                    .into_iter()
                    .filter(|s| members.contains(s))
                    .min()
                    .or_else(|| body.iter().copied().filter(|x| *x != header).min());
                if let Some(entry) = entry {
                    bodies.insert(entry);
                }
            }
        }
        bodies
    }

    /// Which locals are live at each block's entry, by a backwards fixpoint over
    /// the standard `use`/`def` sets. Only the promoted caller-saved locals and
    /// hoisted caller-saved constants are queried, so a coarse but conservative
    /// answer is enough: a local counts as live if any path from the block reads
    /// it before redefining it. Spilling around a call is then needed only when
    /// the register holds a value the code after the call still reads, which is
    /// what removes the store/reload pair a call in a non-looping region
    /// otherwise pays.
    fn compute_liveness(&mut self) {
        let n = self.func.blocks.len();
        let succ: Vec<Vec<u32>> = self
            .func
            .blocks
            .iter()
            .map(|b| successors(&b.term).into_iter().filter(|s| (*s as usize) < n).collect())
            .collect();
        let mut live_out: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        let mut live_in: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        // `use` and `def` per block, computed once.
        let mut uses: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        let mut defs: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        for (bi, b) in self.func.blocks.iter().enumerate() {
            let mut u: HashSet<u32> = HashSet::new();
            let mut d: HashSet<u32> = HashSet::new();
            for s in &b.stmts {
                if let Stmt::Assign(p, rv) = s {
                    let mut r: HashSet<u32> = HashSet::new();
                    scan_rv_uses(rv, &mut r);
                    for l in r {
                        if !d.contains(&l) {
                            u.insert(l);
                        }
                    }
                    // A destination with a projection only writes part of the
                    // local (or writes through it), so the local stays live; it
                    // and any index local in the projection are reads.
                    if p.proj.is_empty() {
                        d.insert(p.local);
                    } else {
                        let mut r: HashSet<u32> = HashSet::new();
                        written_place_reads(p, &mut r);
                        for l in r {
                            if !d.contains(&l) {
                                u.insert(l);
                            }
                        }
                    }
                }
            }
            let mut r: HashSet<u32> = HashSet::new();
            scan_term_uses(&b.term, &mut r);
            term_extra_reads(&b.term, &mut r);
            for l in r {
                if !d.contains(&l) {
                    u.insert(l);
                }
            }
            if let Terminator::Call { dest, .. } = &b.term {
                if dest.proj.is_empty() {
                    d.insert(dest.local);
                }
            }
            uses[bi] = u;
            defs[bi] = d;
        }
        loop {
            let mut changed = false;
            for bi in (0..n).rev() {
                let mut out: HashSet<u32> = HashSet::new();
                for s in &succ[bi] {
                    out.extend(live_in[*s as usize].iter().copied());
                }
                let mut inn = uses[bi].clone();
                for l in &out {
                    if !defs[bi].contains(l) {
                        inn.insert(*l);
                    }
                }
                if out != live_out[bi] || inn != live_in[bi] {
                    live_out[bi] = out;
                    live_in[bi] = inn;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.live_in = live_in;
    }

    /// Build the interference relation from statement-level liveness.
    ///
    /// Walking each block backwards from its live-out set, a definition of `d`
    /// interferes with every local live after it *and* with every local the
    /// defining statement reads (so a statement that writes its destination
    /// register before reading an operand can never clobber that operand). The
    /// operands of a comparison that `compare_branch_skip` folds into the
    /// terminator are treated as read at the terminator, where the fold
    /// evaluates them. The arguments are all defined at entry.
    fn compute_interference(&mut self) {
        let n = self.func.blocks.len();
        let mut affinity: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut interf: HashMap<u32, HashSet<u32>> = HashMap::new();
        let edge = |a: u32, b: u32, m: &mut HashMap<u32, HashSet<u32>>| {
            if a != b {
                m.entry(a).or_default().insert(b);
                m.entry(b).or_default().insert(a);
            }
        };
        for b in &self.func.blocks {
            let mut live: HashSet<u32> = HashSet::new();
            for sc in successors(&b.term) {
                if (sc as usize) < n {
                    live.extend(self.live_in[sc as usize].iter().copied());
                }
            }
            // Terminator.
            let mut tu: HashSet<u32> = HashSet::new();
            scan_term_uses(&b.term, &mut tu);
            term_extra_reads(&b.term, &mut tu);
            // A comparison folded into the branch is evaluated at the
            // terminator, from its operands, so they are read there: those of
            // the statement that computes the condition, and, when that is the
            // `and` of an overflow check, those of the two comparisons it
            // joins. (Not every binary statement in the block: that kept every
            // operand of an unrelated `a - b` alive to the end of a loop's
            // entry block, across the loop's own initialisers.)
            let cond = match &b.term {
                Terminator::SwitchInt { discr, .. } => operand_place(discr),
                Terminator::Assert { cond, .. } => operand_place(cond),
                _ => None,
            };
            if let Some(c) = cond.filter(|p| p.proj.is_empty()) {
                let mut conds: HashSet<u32> = HashSet::new();
                conds.insert(c.local);
                for st in &b.stmts {
                    if let Stmt::Assign(d, Rvalue::Binary(BinOp::BitAnd | BinOp::BitOr, x, y)) = st {
                        if d.proj.is_empty() && d.local == c.local {
                            for o in [x, y] {
                                if let Some(q) = operand_place(o).filter(|q| q.proj.is_empty()) {
                                    conds.insert(q.local);
                                }
                            }
                        }
                    }
                }
                for st in &b.stmts {
                    if let Stmt::Assign(d, rv @ Rvalue::Binary(..)) = st {
                        if d.proj.is_empty() && conds.contains(&d.local) {
                            scan_rv_uses(rv, &mut tu);
                        }
                    }
                }
            }
            if let Terminator::Call { dest, .. } = &b.term {
                if dest.proj.is_empty() {
                    for &o in live.iter().chain(tu.iter()) {
                        edge(dest.local, o, &mut interf);
                    }
                    live.remove(&dest.local);
                }
            }
            live.extend(tu.iter().copied());
            // Statements, last to first.
            for st in b.stmts.iter().rev() {
                let Stmt::Assign(p, rv) = st else { continue };
                let mut su: HashSet<u32> = HashSet::new();
                scan_rv_uses(rv, &mut su);
                if p.proj.is_empty() {
                    // A copy or a cast reads its one operand in full before it
                    // writes the destination, so the two may share a register
                    // (the move is then a no-op): the operand is not an
                    // interference by being read here. If it is also live
                    // *after* the statement, sharing still holds for a copy
                    // that leaves both with the same value, but not for a cast
                    // that changes it (`x as u8` would destroy `x`).
                    let int8 = |t: &Ty| matches!(t.int_width_sign(), Some((8, _)));
                    let copy_of: Option<(u32, bool)> = match rv {
                        Rvalue::Use(Operand::Copy(q) | Operand::Move(q)) if q.proj.is_empty() => {
                            Some((q.local, true))
                        }
                        Rvalue::Cast { op: Operand::Copy(q) | Operand::Move(q), to, .. }
                            if q.proj.is_empty()
                                && !to.is_float()
                                && !self.local_ty(q.local).is_float()
                                && !self.local_ty(q.local).is_aggregate() =>
                        {
                            Some((q.local, int8(to) && int8(&self.local_ty(q.local))))
                        }
                        _ => None,
                    };
                    for &o in su.iter() {
                        if copy_of.map(|(q, _)| q) != Some(o) {
                            edge(p.local, o, &mut interf);
                        }
                    }
                    for &o in live.iter() {
                        if copy_of != Some((o, true)) {
                            edge(p.local, o, &mut interf);
                        }
                    }
                    if let Some((q, _)) = copy_of {
                        if q != p.local {
                            affinity.entry(p.local).or_default().push(q);
                            affinity.entry(q).or_default().push(p.local);
                        }
                    }
                    live.remove(&p.local);
                } else {
                    written_place_reads(p, &mut su);
                }
                // `&x` makes `x` addressable; `&*p` or `&(*p).f` only reads the
                // pointer `p`.
                if let Rvalue::Ref { place, .. } | Rvalue::RawRef { place, .. } = rv {
                    if !matches!(place.proj.first(), Some(Proj::Deref)) {
                        self.no_share.insert(place.local);
                    }
                }
                live.extend(su);
            }
        }
        // Every argument is written at entry, together.
        let args: Vec<u32> = (0..self.func.locals.len() as u32)
            .filter(|l| self.func.locals[*l as usize].is_arg)
            .collect();
        let entry_live: Vec<u32> = self.live_in.first().map(|s| s.iter().copied().collect()).unwrap_or_default();
        for &a in &args {
            for &o in args.iter().chain(entry_live.iter()) {
                edge(a, o, &mut interf);
            }
        }
        self.interf = interf;
        self.affinity = affinity;
    }

    /// The ABI register argument `local` arrives in, if any. A function that
    /// returns an aggregate receives the return pointer in `%rdi`, so its
    /// arguments start at `%rsi` (see `gen_body`).
    fn incoming_reg(&self, local: u32) -> Option<&'static str> {
        let mut args: Vec<u32> = (0..self.func.locals.len() as u32)
            .filter(|i| self.func.locals[*i as usize].is_arg)
            .collect();
        args.sort_unstable();
        let ord = args.iter().position(|a| *a == local)?;
        let shift = if self.local_ty(0).is_aggregate() { 1 } else { 0 };
        ARG_REGS.get(ord + shift).copied()
    }

    /// The locals live across a call or a destructor: a caller-saved register
    /// holding one is saved and restored around it.
    fn call_crossers(&self) -> HashSet<u32> {
        let mut out = HashSet::new();
        for b in &self.func.blocks {
            let (target, dest) = match &b.term {
                Terminator::Call { target, dest, .. } => (*target, Some(dest)),
                Terminator::Drop { target, .. } => (Some(*target), None),
                _ => continue,
            };
            let Some(t) = target else { continue };
            if let Some(live) = self.live_in.get(t as usize) {
                for &l in live {
                    if dest.map(|d| d.local == l && d.proj.is_empty()).unwrap_or(false) {
                        continue;
                    }
                    out.insert(l);
                }
            }
        }
        out
    }

    /// Promote `local` into a register it can share: one whose current owners
    /// are never live at the same time as `local`. Registers holding a hoisted
    /// constant are never shared. Returns whether a register was found.
    ///
    /// The order of preference keeps the cost of the register below the cost of
    /// the frame slot it replaces. A callee-saved register costs a push and a
    /// pop per call of the function the first time it is used, and nothing
    /// after; a caller-saved one costs a save and a restore around every call
    /// the local is live across. So a local live across a call prefers a
    /// callee-saved register the function already saves, and a local that is
    /// not prefers a caller-saved register. A callee-saved register nobody
    /// uses yet is taken only for a local used in a loop, where the per-call
    /// push/pop is paid back; a recursive function's one-shot temporaries
    /// otherwise turn every call into extra pushes (`fib`).
    fn promote_shared(&mut self, local: u32, crosses_call: bool, in_loop: bool) -> bool {
        if self.promo.contains_key(&local) || self.no_share.contains(&local) {
            return false;
        }
        let ty = self.ty.get(local as usize).cloned().unwrap_or(Ty::I64);
        if ty.is_aggregate() {
            return false;
        }
        let is_arg = self.func.locals.get(local as usize).map(|l| l.is_arg).unwrap_or(false);
        let callee_used: Vec<&'static str> = PROMOTE_REGS
            .iter()
            .copied()
            .filter(|r| {
                self.reg_owners.get(r).map(|v| !v.is_empty()).unwrap_or(false)
                    || self.const_regs.values().any(|c| c == r)
            })
            .collect();
        let callee_fresh: Vec<&'static str> = if in_loop {
            PROMOTE_REGS.iter().copied().filter(|r| !callee_used.contains(r)).collect()
        } else {
            Vec::new()
        };
        let mut pool: Vec<&'static str> = Vec::new();
        if is_arg {
            // An argument is copied out of its ABI register at entry, in
            // parallel with the others, so it may not land in a register that
            // still holds a later argument -- except its own: staying in the
            // register it arrived in makes its entry move a no-op. That is the
            // best home for an argument no call outlives (no save at entry, no
            // load per use); one live across a call prefers a callee-saved
            // register.
            let own = self.incoming_reg(local).filter(|r| CALLER_REGS.contains(r));
            if !crosses_call {
                pool.extend(own);
            }
            pool.extend(&callee_used);
            pool.extend(&callee_fresh);
            if crosses_call {
                pool.extend(own);
            }
        } else if crosses_call {
            // Not a caller-saved register: it would be stored before and
            // reloaded after every call the local is live across, which costs
            // as much as leaving it in its slot and measured slower in a
            // recursive loop (`nqueens`'s running count: 47 ms in `%r8`, 44 ms
            // in the frame).
            pool.extend(&callee_used);
            pool.extend(&callee_fresh);
        } else {
            pool.extend_from_slice(&CALLER_REGS);
            pool.extend(&callee_used);
            pool.extend(&callee_fresh);
        }
        // A copy partner's register first: sharing it deletes the copy.
        let mut order: Vec<&'static str> = Vec::with_capacity(pool.len());
        if let Some(partners) = self.affinity.get(&local) {
            for q in partners {
                if let Some(pr) = self.promo.get(q) {
                    if pool.contains(&pr.reg) && !order.contains(&pr.reg) {
                        order.push(pr.reg);
                    }
                }
            }
        }
        for r in pool {
            if !order.contains(&r) {
                order.push(r);
            }
        }
        let empty = HashSet::new();
        let mine = self.interf.get(&local).unwrap_or(&empty);
        for reg in order {
            if self.const_regs.values().any(|r| *r == reg)
                || self.const_caller_regs.values().any(|r| *r == reg)
            {
                continue;
            }
            let owners = self.reg_owners.get(reg).map(|v| v.as_slice()).unwrap_or(&[]);
            if owners.iter().any(|o| self.no_share.contains(o) || mine.contains(o)) {
                continue;
            }
            self.reg_owners.entry(reg).or_default().push(local);
            self.promo.insert(local, Promo { reg, caller_saved: CALLER_REGS.contains(&reg) });
            return true;
        }
        false
    }

    /// Whether `local` is live after a call in `bb`: that is, whether the
    /// call's successor reads the value before redefining it. A local held in a
    /// caller-saved register must be spilled around the call exactly then.
    fn live_across_call(&self, bb: u32, local: u32) -> bool {
        let Some(block) = self.func.blocks.get(bb as usize) else { return true };
        let Terminator::Call { target, dest, .. } = &block.term else { return true };
        // If the call writes the local itself, the old value is dead.
        if dest.local == local && dest.proj.is_empty() {
            return false;
        }
        let Some(t) = target else { return false };
        self.live_in.get(*t as usize).map(|s| s.contains(&local)).unwrap_or(true)
    }

    // --- function ----------------------------------------------------------

    fn gen_fn(&mut self) -> Result<String, String> {
        self.analyze();
        self.compute_liveness();
        self.compute_interference();
        self.plan_promotion();
        // A function that returns an aggregate receives a hidden pointer to the
        // caller's return place in `%rdi` and stores `_0` through it. The
        // address lives in a frame slot for the whole call, so it must be taken
        // before the argument registers are overwritten -- the arguments of
        // such a function start at `%rsi`.
        if self.local_ty(0).is_aggregate() {
            self.sret_slot = self.alloc_slot();
        }
        // A shadow slot for each promoted local, so taking its address has a
        // stable home. Allocated once, before the frame is sized.
        for l in 0..self.func.locals.len() as u32 {
            if self.promo.contains_key(&l) {
                let s = self.alloc_slot();
                self.shadow.insert(l, s);
            }
        }
        // A promoted caller-saved local needs a home across a call. Locals that
        // share a register share the home: a spill saves the register, whichever
        // of its owners is live, and a read inside a destructor tree (which
        // reads the home, see `local_operand`) then finds that value.
        let mut reg_slot: HashMap<&'static str, i64> = HashMap::new();
        for l in 0..self.func.locals.len() as u32 {
            let Some(p) = self.promo.get(&l) else { continue };
            if p.caller_saved {
                let reg = p.reg;
                let s = match reg_slot.get(reg) {
                    Some(s) => *s,
                    None => {
                        let s = self.alloc_slot();
                        reg_slot.insert(reg, s);
                        s
                    }
                };
                self.caller_spill.insert(l, s);
            }
        }
        // A caller-saved hoisted constant is clobbered by a call too, so it
        // needs the same home.
        let mut const_caller_vals: Vec<i64> = self.const_caller_regs.keys().copied().collect();
        const_caller_vals.sort_unstable();
        for v in const_caller_vals {
            let s = self.alloc_slot();
            self.const_spill.insert(v, s);
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
        // A slice record built by `gen_unsize` is written through `base + k`,
        // so pin every slot of its run the same way.
        for &(first, base) in &self.pinned {
            let lo = (first.max(0) as usize).min(used.len().saturating_sub(1));
            let hi = (base.max(0) as usize).min(used.len().saturating_sub(1));
            if lo <= hi {
                used[lo..=hi].fill(true);
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
        // requires it), plus any that hold a hoisted constant even if the body
        // does not name them -- the value must survive a call to the function
        // that follows. A caller-saved register is preserved by the caller, so
        // it is spilled only around a call (see `gen_call`).
        let saved: Vec<&'static str> = PROMOTE_REGS
            .iter()
            .copied()
            .filter(|r| {
                reg_mentions(&opt, r) || self.const_regs.values().any(|cr| cr == r)
            })
            .collect();
        // The saved registers sit below `%rbp`, so every frame slot is shifted
        // past them; `off` reads this.
        self.saved_regs = saved.len() as i64;

        self.gen_body()?;
        let body = optimize(&std::mem::take(&mut self.out));

        // A function whose final body never names `%rbp` needs no frame pointer:
        // the `push %rbp; mov %rsp, %rbp; ...; leave` sequence is then pure
        // overhead on every call, which a recursive function pays per level.
        // Decide from the *optimised* body: the folding passes rewrite the store
        // and reload of a call result into a register move, so a slot the raw
        // body named can be gone by the time the frame is laid out.
        let omit_frame = rbp_slots(&body).is_empty();
        let saved_bytes = self.saved_regs * 8;
        // A function with no `%rbp` operand touches no frame slot, so the only
        // stack it needs is the padding that keeps `%rsp` 16-byte aligned at a
        // call. rmc emits no aligned SSE and makes no external ABI call (its
        // runtime manages its own stack), so that padding is pure per-call
        // overhead -- and a recursive function pays it on every level. Only a
        // body that actually addresses the frame gets the padding.
        let mut frame_bytes = if omit_frame { 0 } else { packed_slots * 8 };
        while !omit_frame && (8 + saved_bytes + 8 + frame_bytes) % 16 != 0 {
            frame_bytes += 8;
        }

        let mut out = String::new();
        // Align the entry to a 16-byte boundary: a call-heavy program (every
        // recursive call, every `main` -> helper jump) fetches the first bytes
        // of the callee, and an unaligned entry splits that fetch across two
        // cache lines. LLVM aligns every function this way.
        out.push_str("  .p2align 4\n");
        let fname = asm_sym(&self.func.name);
        let _ = writeln!(out, ".globl {fname}");
        let _ = writeln!(out, "{fname}:");
        if !omit_frame {
            out.push_str("  push %rbp\n  mov %rsp, %rbp\n");
        }
        for r in &saved {
            let _ = writeln!(out, "  push {r}");
        }
        if frame_bytes > 0 {
            let _ = writeln!(out, "  sub ${frame_bytes}, %rsp");
        }
        // Materialise each hoisted constant once, after the frame is set up so
        // the value is not clobbered by the frame arithmetic. Sorted so the
        // output is byte-for-byte reproducible.
        let mut consts: Vec<(&i64, &&str)> = self.const_regs.iter().collect();
        consts.sort_by_key(|(v, _)| **v);
        for (v, reg) in consts {
            let _ = writeln!(out, "  movabs ${v}, {reg}");
        }
        out.push_str(&body);

        // Epilogue: restore the saved registers and return `%rax`. The saved
        // registers live directly below `%rbp`, but `%rsp` is still at the
        // bottom of the frame, so it has to be moved back up first.
        let _ = writeln!(out, ".Lret_{}:", asm_sym(&self.func.name));
        // `_start` passes `main`'s `%rax` straight to `exit(2)`, so a `main`
        // that returns `()` must define it. The ABI leaves `%rax` unspecified
        // for a unit return, so without this the process exits with whatever
        // the last statement happened to leave in the register.
        if self.func.name == "main" && self.local_ty(0) == Ty::Unit {
            out.push_str("  xor %eax, %eax\n");
        }
        if omit_frame {
            if frame_bytes > 0 {
                let _ = writeln!(out, "  add ${frame_bytes}, %rsp");
            }
        } else if saved_bytes > 0 {
            let _ = writeln!(out, "  lea -{saved_bytes}(%rbp), %rsp");
        }
        for r in saved.iter().rev() {
            let _ = writeln!(out, "  pop {r}");
        }
        if omit_frame {
            out.push_str("  ret\n");
        } else {
            out.push_str("  leave\n  ret\n");
        }
        Ok(finish_function(&optimize(&out)))
    }

    /// Argument setup plus every basic block. Run twice (dry, then real).
    fn gen_body(&mut self) -> Result<(), String> {
        self.pinned.clear();
        self.trap_used = false;
        self.block_depth = self.loop_depths();
        // A hidden return pointer arrives in `%rdi` and is stashed in its frame
        // slot before the argument moves, which would overwrite `%rdi`. The
        // arguments of an aggregate-returning function therefore start at
        // `%rsi`.
        let arg_shift = if self.sret_slot >= 0 { 1 } else { 0 };
        if arg_shift == 1 {
            let slot = self.off(self.sret_slot);
            self.emit(&format!("  mov %rdi, {slot}"));
        }
        // MIR numbers arguments from `_1`, so the register is chosen by the
        // argument's ordinal, not by its local index.
        let mut args: Vec<u32> = (0..self.func.locals.len() as u32)
            .filter(|i| self.func.locals[*i as usize].is_arg)
            .collect();
        args.sort_unstable();
        for (ord, li) in args.into_iter().enumerate() {
            if ord + arg_shift >= ARG_REGS.len() {
                return Err(format!("mir_codegen: {} takes more than 6 arguments", self.func.name));
            }
            let dst = self.local_operand(li);
            // An argument promoted to the register it arrived in stays put.
            if dst != ARG_REGS[ord + arg_shift] {
                let _ = writeln!(self.out, "  mov {}, {}", ARG_REGS[ord + arg_shift], dst);
            }
        }
        // Materialise caller-saved hoisted constants after the argument moves:
        // until the last move the argument registers still hold incoming
        // arguments, so a `movabs` into one before that would be overwritten.
        let mut caller_consts: Vec<(&i64, &&str)> = self.const_caller_regs.iter().collect();
        caller_consts.sort_by_key(|(v, _)| **v);
        for (v, reg) in caller_consts {
            let _ = writeln!(self.out, "  movabs ${v}, {reg}");
        }
        let bodies = self.loop_bodies_to_align();
        for i in 0..self.func.blocks.len() as u32 {
            // Pin a loop's body entry to a 16-byte fetch boundary, as the `.rms`
            // path does. The body is where the loop spends its iterations, and
            // without the pin a one-byte shift earlier in the function moves it
            // relative to the fetch windows -- up to 1.4x on `stats`'s division
            // loop. The header is left where the layout put it: pinning the
            // header instead costs padding on the preheader's fall-through and
            // measured a net loss on short loops.
            if bodies.contains(&i) {
                let _ = writeln!(self.out, "  .p2align 4");
            }
            let _ = writeln!(self.out, ".Lbb{}:", self.bb_label(i));
            self.gen_block(i)?;
        }
        // The failure path of every assertion that falls through on success:
        // out of line, so a bounds check costs a not-taken branch.
        if self.trap_used {
            let t = self.trap_label();
            let _ = writeln!(self.out, "{t}:\n  mov $101, %rdi\n  mov $60, %rax\n  syscall");
        }
        Ok(())
    }

    /// The function's shared panic exit (exit status 101, as rustc's panic).
    fn trap_label(&self) -> String {
        format!(".Lrmc_{}_trap", self.label_tag())
    }

    /// A basic-block label, qualified by the function name so that labels from
    /// different functions cannot collide (assembler-local `.L` labels share
    /// one namespace across the whole translation unit).
    fn bb_label(&self, bb: u32) -> String {
        format!("{}_{bb}", self.label_tag())
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
                // Name the function, block and statement on failure: a bare
                // "aggregate constant unsupported" gives no way back to the
                // MIR line that caused it.
                self.gen_assign(p, rv).map_err(|e| {
                    format!("{e}\n  in `{}` bb{bb}, statement {i}: {p:?} = {rv:?}", self.func.name)
                })?;
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
                // Every folded statement's result must be dead elsewhere, the
                // comparisons' as well as the `and`'s: rustc's GVN reuses a
                // comparison (`_90 = Eq(x, i64::MIN)`) for a later overflow
                // check of another division by the same value, and that later
                // block then read a `bool` that was never stored.
                let cmp_local = |i: usize| match &block.stmts[i] {
                    Stmt::Assign(p, _) => Some(p.local),
                    Stmt::Nop => None,
                };
                let c_dead = [c1i, c2i].iter().all(|&i| match cmp_local(i) {
                    Some(l) => !self.local_used_elsewhere_except(bb, l, Some(ai)),
                    None => false,
                });
                if c_dead && !self.local_used_elsewhere(bb, pl.local) {
                    return vec![ai, c1i, c2i];
                }
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
        let Some(i) = last else { return Vec::new() };
        if let Stmt::Assign(_, Rvalue::Binary(_, a, b)) = &block.stmts[i] {
            if self.cmp_float(a, b) {
                return Vec::new();
            }
        }
        // Folding the comparison into the terminator leaves no `bool` in
        // memory, so a *later* block that reads the same local (rustc reuses
        // one `_15` for both the first and a second bounds check) would read a
        // stale slot. Keep the store when the local is live elsewhere.
        if self.local_used_elsewhere(bb, pl.local) {
            return Vec::new();
        }
        vec![i]
    }

    /// Whether `local` is read somewhere the fold cannot serve: a read in `bb`
    /// other than the folded comparison, or a read in another block that is not
    /// preceded by its own definition of `local`.
    ///
    /// The "own definition" test matters because rustc reuses one local for
    /// repeated tests. `fib` computes `_2 = Lt(n, 2)` in both the entry block
    /// and the loop header; a read of `_2` in the second block is served by the
    /// second block's own `_2 = ..`, not by the first block's store, so folding
    /// the first store away is safe. Treating every read anywhere as live made
    /// both folds bail, and the `bool` materialisation (`setl`/`movzbl`/store)
    /// was paid on every loop iteration.
    fn local_used_elsewhere(&self, bb: u32, local: u32) -> bool {
        self.local_used_elsewhere_except(bb, local, None)
    }

    /// `local_used_elsewhere`, not counting the reads of statement `except`
    /// of `bb` (the `and` of an overflow check reads both comparisons, and the
    /// fold serves exactly that read), and counting a read by `bb`'s own
    /// terminator when `except` is given (a comparison result the terminator
    /// reads directly is not one the `and` fold serves).
    fn local_used_elsewhere_except(&self, bb: u32, local: u32, except: Option<usize>) -> bool {
        for (b, block) in self.func.blocks.iter().enumerate() {
            // In `bb` itself the fold removes one statement, so any read of the
            // local there - before the comparison (the old value) or after it
            // (the folded value) - is a read the fold cannot serve.
            if b as u32 == bb {
                if except.is_some() && terminator_reads(&block.term, local) {
                    return true;
                }
                for (i, s) in block.stmts.iter().enumerate() {
                    if Some(i) == except {
                        continue;
                    }
                    let mut used = false;
                    scan_operands(s, &mut |op| {
                        if let Operand::Copy(p) | Operand::Move(p) = op {
                            if p.local == local {
                                used = true;
                            }
                        }
                    });
                    if used {
                        return true;
                    }
                }
                continue;
            }
            // Another block: a read is served by that block's own definition of
            // the local when one precedes it, so only a read before the first
            // definition makes the local live across the fold.
            let mut defined = false;
            for s in &block.stmts {
                let mut used = false;
                scan_operands(s, &mut |op| {
                    if let Operand::Copy(p) | Operand::Move(p) = op {
                        if p.local == local {
                            used = true;
                        }
                    }
                });
                if used && !defined {
                    return true;
                }
                if let Stmt::Assign(d, _) = s {
                    if d.local == local && d.proj.is_empty() {
                        defined = true;
                    }
                }
            }
            if terminator_reads(&block.term, local) && !defined {
                return true;
            }
        }
        false
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
        let (c1i, _, a1, b1) = cmp_of(xp.local)?;
        let (c2i, _, a2, b2) = cmp_of(yp.local)?;
        // `and_chain_cmps` refuses a float pair, so the terminator will not
        // fold it; its statements must stay.
        if self.cmp_float(&a1, &b1) || self.cmp_float(&a2, &b2) {
            return None;
        }
        Some((ai, c1i, c2i))
    }

    /// Whether a comparison of `a` with `b` is unsigned: either operand an
    /// unsigned integer or a pointer. Both sides have one type in MIR, but a
    /// constant operand reads as `i64` here, so either side decides.
    fn cmp_unsigned(&self, a: &Operand, b: &Operand) -> bool {
        let u = |t: Ty| t.is_unsigned() || matches!(t, Ty::Ref(_) | Ty::Ptr(_));
        u(self.operand_ty(a)) || u(self.operand_ty(b))
    }

    /// Whether a comparison is between floats. An integer `cmp` of the bit
    /// patterns is wrong for negative values and for NaN, so a float
    /// comparison is never folded into a branch; it is materialised by the
    /// `ucomisd` path, which handles both, and the branch tests the `bool`.
    fn cmp_float(&self, a: &Operand, b: &Operand) -> bool {
        self.operand_ty(a).is_float() || self.operand_ty(b).is_float()
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
        let (c1, c2) = (cmp_of(xp.local)?, cmp_of(yp.local)?);
        if self.cmp_float(&c1.1, &c1.2) || self.cmp_float(&c2.1, &c2.2) {
            return None;
        }
        Some((c1, c2))
    }

    /// Emit the `cmp` for the comparison that produced `cond` in `bb`, when
    /// there is one, and return the comparison's operator. `None` when `cond`
    /// is not a just-computed compare.
    fn emit_fused_compare(
        &mut self,
        bb: u32,
        cond: &Operand,
    ) -> Result<Option<(BinOp, bool)>, String> {
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
        if self.cmp_float(&a, &b) {
            return Ok(None);
        }
        self.emit_cmp(op, &a, &b)?;
        Ok(Some((op, self.cmp_unsigned(&a, &b))))
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
                if self.sret_slot >= 0 {
                    // An aggregate is returned by copy: `_0` holds the value, and
                    // the caller's return place is the pointer received in `%rdi`.
                    // Copy it word by word (`copy_words` reads `%r10`/`%r11`).
                    let size = self.place_size(&Place { local: 0, proj: Vec::new() });
                    self.place_addr(&Place { local: 0, proj: Vec::new() })?;
                    self.emit("  mov %rax, %r10");
                    let slot = self.off(self.sret_slot);
                    self.emit(&format!("  mov {slot}, %r11"));
                    self.copy_words(size);
                } else if self.local_ty(0) != Ty::Unit && !self.func.locals.is_empty() {
                    let src = self.local_operand(0);
                    if src != "%rax" {
                        self.emit(&format!("  mov {src}, %rax"));
                    }
                }
                self.emit(&format!("  jmp .Lret_{}", asm_sym(&self.func.name)));
            }
            Terminator::Unreachable => {
                self.emit(&format!("  jmp .Lret_{}", asm_sym(&self.func.name)));
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
                                let u1 = self.cmp_unsigned(&c1.1, &c1.2);
                                let u2 = self.cmp_unsigned(&c2.1, &c2.2);
                                self.emit_cmp(c1.0, &c1.1, &c1.2)?;
                                self.emit(&format!("  {} {ok}", jcc(negate_cc(c1.0), u1)));
                                self.emit_cmp(c2.0, &c2.1, &c2.2)?;
                                self.emit(&format!("  {} {ok}", jcc(negate_cc(c2.0), u2)));
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
                // `ok` jumps when the assertion holds, `fail` when it does not.
                let (ok, fail) = match self.emit_fused_compare(bb, cond)? {
                    Some((op, u)) => {
                        // `assert(!cmp, ..)` succeeds when the comparison is
                        // false, so jump on the complement of the comparison.
                        let op = if *negate { negate_cc(op) } else { op };
                        (jcc(op, u), jcc(negate_cc(op), u))
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
                        if *negate { ("je", "jne") } else { ("jne", "je") }
                    }
                };
                if *target == bb + 1 && (*target as usize) < self.func.blocks.len() {
                    // The success block is next: fall into it, and branch out
                    // of line only on failure.
                    self.trap_used = true;
                    let t = self.trap_label();
                    self.emit(&format!("  {fail} {t}"));
                } else {
                    self.emit(&format!("  {ok} .Lbb{}", self.bb_label(*target)));
                    self.emit("  mov $101, %rdi");
                    self.emit("  mov $60, %rax");
                    self.emit("  syscall");
                }
            }
            Terminator::SwitchInt { discr, targets, otherwise } => {
                self.gen_switch(bb, discr, targets, *otherwise)?;
            }
            Terminator::Drop { place, target } => {
                self.gen_drop(place)?;
                if *target != bb + 1 || *target as usize >= self.func.blocks.len() {
                    self.emit(&format!("  jmp .Lbb{}", self.bb_label(*target)));
                }
            }
            Terminator::Call { dest, func, args, target } => {
                // `cold_path` is an optimisation hint with no effect here; it
                // has no body to call, so skip it and continue.
                let cold = matches!(func, CallTarget::Direct(n) if n == "cold_path" || n.ends_with("::cold_path"));
                if !cold {
                    self.gen_call(dest, func, args, bb)?;
                }
                let t = target.unwrap_or_else(|| {
                    // A call with no return edge is `-> []` (noreturn).
                    u32::MAX
                });
                if t == u32::MAX {
                    self.emit(&format!("  jmp .Lret_{}", asm_sym(&self.func.name)));
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
        let next = bb + 1;
        if let [(val, t)] = targets {
            // The value's own target follows: fall into it and jump away only
            // when the value differs (`je next; jmp other` ran two branches).
            // Neither follows and `otherwise` is in a deeper loop: the loop
            // goes on through it, so it takes the conditional jump, as in
            // `try_branch_compare`.
            let falls = *t == next && otherwise != next;
            let loops = otherwise != next && *t != next && self.deeper(otherwise, *t);
            if falls || loops {
                let v = self.wide_imm(&format!("${val}"), "%rcx");
                let src = if is_mem {
                    self.emit(&format!("  mov {src}, %rax"));
                    "%rax".to_string()
                } else {
                    src
                };
                self.emit(&format!("  cmp {v}, {src}"));
                self.emit(&format!("  jne .Lbb{}", self.bb_label(otherwise)));
                if loops {
                    self.emit(&format!("  jmp .Lbb{}", self.bb_label(*t)));
                }
                return Ok(());
            }
        }
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
        if self.cmp_float(&a, &b) {
            return Ok(false);
        }
        let u = self.cmp_unsigned(&a, &b);
        self.emit_cmp(op, &a, &b)?;
        // Jump to `taken` when the comparison holds, else to `fallthrough`.
        // Whichever arm is the next emitted block becomes the fall-through: when
        // it is `taken` the branch is emitted on the complement, so a loop body
        // that follows its header needs no jump at all.
        let next = bb + 1;
        if taken == next {
            self.emit(&format!("  {} .Lbb{}", jcc(negate_cc(op), u), self.bb_label(fallthrough)));
        } else if fallthrough != next && self.deeper(fallthrough, taken) {
            // Neither arm follows. The one in a deeper loop is where the loop
            // goes on, so it gets the conditional jump: an iteration then takes
            // one branch, where `jcc exit; jmp body` ran two.
            self.emit(&format!("  {} .Lbb{}", jcc(negate_cc(op), u), self.bb_label(fallthrough)));
            self.emit(&format!("  jmp .Lbb{}", self.bb_label(taken)));
        } else {
            self.emit(&format!("  {} .Lbb{}", jcc(op, u), self.bb_label(taken)));
            if fallthrough != next || fallthrough as usize >= self.func.blocks.len() {
                self.emit(&format!("  jmp .Lbb{}", self.bb_label(fallthrough)));
            }
        }
        Ok(true)
    }

    /// Whether block `a` sits in a deeper loop than block `b`.
    fn deeper(&self, a: u32, b: u32) -> bool {
        let d = |x: u32| self.block_depth.get(x as usize).copied().unwrap_or(0);
        d(a) > d(b)
    }

    // --- operands and places ----------------------------------------------

    /// Evaluate a scalar operand to something usable as an instruction operand:
    /// a register, an immediate, or a direct memory slot. Returns the operand
    /// text and whether it is a memory operand. A complex operand is loaded
    /// into `%rax`.
    fn eval_operand(&mut self, op: &Operand) -> Result<(String, bool), String> {
        match op {
            Operand::Const(c) => Ok((self.const_operand(c)?, false)),
            Operand::Fn(name) => {
                // A function taken as a value is its own address; materialise
                // the symbol so it can be stored, passed, or called.
                self.emit(&format!("  lea {}(%rip), %rax", asm_sym(name)));
                Ok(("%rax".to_string(), false))
            }
            Operand::Fmt { ty_name, trait_name } => {
                // A reified formatter is the address of rmc's own runtime
                // formatter for this type/trait pair.
                let label = self.fmt_label(ty_name, trait_name)?;
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
            Operand::Fn(name) => {
                self.emit(&format!("  lea {}(%rip), %rcx", asm_sym(name)));
                Ok(("%rcx".to_string(), false))
            }
            Operand::Fmt { ty_name, trait_name } => {
                let label = self.fmt_label(ty_name, trait_name)?;
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
                    if let Some(plan) = self.mem_plan(p) {
                        if !self.plan_loads_ptr(&plan) && !self.place_ty(p).is_aggregate() {
                            let mem = self.emit_mem_plan(&plan, "%rcx");
                            self.load_mem_to(p, &mem, "%rcx");
                            return Ok(("%rcx".to_string(), false));
                        }
                    }
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
            | Operand::Fmt { .. }
            | Operand::Fn(_) => false,
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
    ///
    /// A reference type (`&T`) gets a *deref trampoline*: rustc formats a `&T`
    /// by calling `<T as Display>::fmt` with a pointer to the reference's
    /// referent, while rmc's runtime hands a formatter the slot holding the
    /// value. For a scalar referent the runtime formatter dereferences once to
    /// reach the scalar, so a `depth`-deep reference needs `depth` derefs. A
    /// `str`/`[u8]` referent is itself a pointer, and the runtime formatter
    /// dereferences once to reach its record, so the outermost `&` already
    /// names the slot it wants and only `depth - 1` derefs are needed.
    fn fmt_label(&mut self, ty_name: &str, trait_name: &str) -> Result<String, String> {
        let base = ty_name.trim().trim_start_matches('&').trim_start_matches("mut ").trim();
        let label = self.fmt_label_inner(ty_name, trait_name)?;
        let is_str = base == "str" || base.starts_with("[u8]") || base.starts_with("[u8;");
        let depth = ref_depth(ty_name).saturating_sub(is_str as usize);
        if depth > 0 {
            return Ok(self.deref_fmt_label(&label, depth));
        }
        Ok(label)
    }

    /// The label of a trampoline that dereferences its argument `depth` times
    /// before tail-calling the base formatter `base`. Deduplicated, and emitted
    /// after the function body so the trampoline cannot fall through into it.
    fn deref_fmt_label(&mut self, base: &str, depth: usize) -> String {
        let name = format!(".Lrmc_deref{depth}_{}", base.trim_start_matches("__rmc_fmt_"));
        if !self.deref_tramps.iter().any(|(n, _)| n == &name) {
            let mut text = String::new();
            let _ = writeln!(text, "{name}:");
            for _ in 0..depth {
                let _ = writeln!(text, "  mov (%rdi), %rdi");
            }
            let _ = writeln!(text, "  jmp {base}");
            self.deref_tramps.push((name.clone(), text));
        }
        name
    }

    fn fmt_label_inner(&self, ty_name: &str, trait_name: &str) -> Result<String, String> {
        // A float's `Display` is the shortest decimal that round-trips, which
        // the runtime does not yet render. Reject it here, naming the feature,
        // rather than emitting a call to a formatter that does not exist (an
        // "undefined symbol" link error) or, worse, reading the bit pattern as
        // an integer and printing a huge number.
        let base = ty_name.trim().trim_start_matches('&').trim_start_matches("mut ").trim();
        if base == "f64" || base == "f32" {
            return Err(format!(
                "mir_codegen: formatting a float (`{base}`) is not implemented on the MIR path"
            ));
        }
        // Only a scalar (or a byte slice, which formats as text) has a runtime
        // formatter. `resolve_fmt` falls through to its integer default for
        // anything else, so an aggregate - `{:?}` on an array, tuple, struct,
        // `Option`, `Result`, `Vec` - silently printed the first word as a
        // number. That is a miscompile, worse than a diagnostic: reject it.
        let byte_slice = base.starts_with("[u8]") || base.starts_with("[u8;");
        if !is_scalar_ty(base) && !byte_slice {
            return Err(format!(
                "mir_codegen: formatting `{base}` is not implemented on the MIR path \
                 (only scalars, `str` and `[u8]` have a runtime formatter)"
            ));
        }
        Ok(resolve_fmt(ty_name, trait_name).to_string())
    }

    fn const_operand(&mut self, c: &Const) -> Result<String, String> {
        Ok(match c {
            // A wide constant hoisted into a register (see `hoist_consts`) is
            // read from there instead of re-materialised with `movabs`.
            Const::Int(n) if self.const_regs.contains_key(n) => self.const_regs[n].to_string(),
            // A caller-saved hoisted constant has been clobbered once a
            // destructor tree has made its calls; read the spill slot there.
            Const::Int(n) if self.const_caller_regs.contains_key(n) => {
                if self.in_drop {
                    self.off(self.const_spill[n])
                } else {
                    self.const_caller_regs[n].to_string()
                }
            }
            Const::Int(n) => format!("${n}"),
            Const::Bool(b) => format!("${}", *b as i64),
            // A float's value as a general-purpose register is its bit pattern;
            // the float arithmetic paths move that word into an SSE register.
            // An f32 keeps its 32 bits in the low half.
            Const::Float(v, is_f32) => {
                let bits = if *is_f32 {
                    (*v as f32).to_bits() as i64
                } else {
                    v.to_bits() as i64
                };
                format!("${bits}")
            }
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
            Const::Agg { name, .. } => {
                return Err(format!("mir_codegen: aggregate constant not yet materialised: `{name}`").into());
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
    /// Plan a single addressing mode for `p`, or `None` when the place needs
    /// `place_addr`'s general walk (a deref past the first projection, two
    /// dynamic indices, a stride other than 1/2/4/8, or a bare scalar local,
    /// whose promoted register has no address). Emits nothing.
    ///
    /// Only a place reached through a pointer (`(*p)[i]`, or an aggregate
    /// argument, which is passed by address) is planned. A frame local keeps
    /// `place_addr`'s `lea N(%rbp)`: the slot peepholes (`drop_unread_stores`,
    /// `forward_stores`) recognise a slot by its `N(%rbp)` text and treat a
    /// `lea` of it as the address escaping, so an indexed `N(%rbp,%i,8)` read
    /// would be invisible to them and the stores that initialise the array
    /// would be dropped as unread.
    fn mem_plan(&self, p: &Place) -> Option<MemPlan> {
        if p.proj.is_empty() {
            return None;
        }
        let starts_deref = matches!(p.proj.first(), Some(Proj::Deref));
        let agg_arg = !starts_deref
            && self.local_ty(p.local).is_aggregate()
            && self.func.locals.get(p.local as usize).map(|l| l.is_arg).unwrap_or(false);
        if !starts_deref && !agg_arg {
            return None;
        }
        let mut ty = self.local_ty(p.local);
        if starts_deref {
            ty = match &ty {
                Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                _ => Ty::I64,
            };
        }
        let rest = if starts_deref { &p.proj[1..] } else { &p.proj[..] };
        let mut disp = 0i64;
        let mut index: Option<(u32, i64)> = None;
        let mut variant_ctx: Option<(String, String)> = None;
        for proj in rest {
            match proj {
                Proj::Field(k) => {
                    if let Some((en, v)) = &variant_ctx {
                        disp += *k as i64 * 8;
                        ty = self.enum_field_ty(en, v, *k as u64);
                        variant_ctx = None;
                        continue;
                    }
                    disp += self.field_offset(&ty, *k as u64);
                    ty = self.field_ty(&ty, *k as u64);
                }
                Proj::Variant(v) => {
                    if let Ty::Struct(en) = &ty {
                        disp += self.enum_field_offset(en, v, 0);
                        variant_ctx = Some((en.clone(), v.clone()));
                    }
                }
                Proj::IndexConst(i) => {
                    let stride = self.size_of_ty(&elem_ty(&ty)).max(1);
                    disp += *i as i64 * stride + slice_header(&ty);
                    ty = elem_ty(&ty);
                }
                Proj::IndexLocal(l) => {
                    let stride = self.size_of_ty(&elem_ty(&ty)).max(1);
                    if index.is_some() || ![1, 2, 4, 8].contains(&stride) {
                        return None;
                    }
                    index = Some((*l, stride));
                    disp += slice_header(&ty);
                    ty = elem_ty(&ty);
                }
                Proj::Deref => return None,
                Proj::RawRef => {}
            }
        }
        if disp.unsigned_abs() > i32::MAX as u64 / 2 {
            return None;
        }
        let ptr = if starts_deref || agg_arg { Some(p.local) } else { None };
        Some(MemPlan { ptr, frame_local: p.local, disp, index })
    }

    /// Whether emitting `plan` must load its pointer into a scratch register
    /// (the pointer local lives in a frame slot, not a register).
    fn plan_loads_ptr(&self, plan: &MemPlan) -> bool {
        match plan.ptr {
            Some(l) => !self.local_operand(l).starts_with('%'),
            None => false,
        }
    }

    /// Emit what `plan` needs (the pointer into `ptr_reg` when it is in memory,
    /// the index into `%rcx` when it is in memory) and return the operand.
    fn emit_mem_plan(&mut self, plan: &MemPlan, ptr_reg: &str) -> String {
        let (base, disp) = match plan.ptr {
            Some(l) => {
                let src = self.local_operand(l);
                if src.starts_with('%') {
                    (src, plan.disp)
                } else {
                    self.emit(&format!("  mov {src}, {ptr_reg}"));
                    (ptr_reg.to_string(), plan.disp)
                }
            }
            None => {
                let o = self.off(self.local_base(plan.frame_local));
                let n: i64 = o.trim_end_matches("(%rbp)").parse().unwrap_or(0);
                ("%rbp".to_string(), n + plan.disp)
            }
        };
        let d = if disp == 0 && base != "%rbp" { String::new() } else { disp.to_string() };
        match plan.index {
            None => format!("{d}({base})"),
            Some((l, stride)) => {
                let src = self.local_operand(l);
                let idx = if src.starts_with('%') {
                    src
                } else {
                    self.emit(&format!("  mov {src}, %rcx"));
                    "%rcx".to_string()
                };
                format!("{d}({base},{idx},{stride})")
            }
        }
    }

    /// A memory operand for `p`: one addressing mode when `mem_plan` finds
    /// one, else `(%rax)` after `place_addr`. Clobbers at most `%rax`/`%rcx`,
    /// as `place_addr` does.
    fn place_mem(&mut self, p: &Place) -> Result<String, String> {
        if let Some(plan) = self.mem_plan(p) {
            return Ok(self.emit_mem_plan(&plan, "%rax"));
        }
        self.place_addr(p)?;
        Ok("(%rax)".to_string())
    }

    fn place_addr(&mut self, p: &Place) -> Result<(), String> {
        let starts_deref = matches!(p.proj.first(), Some(Proj::Deref));
        // An aggregate argument is passed by address: its frame slot holds a
        // pointer to the value, not the value itself (see `gen_call`, which
        // evaluates every aggregate argument to an address). Every read of such
        // a local, and every projection off it, is relative to that pointer.
        let agg_arg = !starts_deref
            && self.local_ty(p.local).is_aggregate()
            && self.func.locals.get(p.local as usize).map(|l| l.is_arg).unwrap_or(false);
        if starts_deref {
            let src = self.local_operand(p.local);
            self.emit(&format!("  mov {src}, %rax"));
        } else if agg_arg {
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
                    // An element is at least a byte; a `[u8; N]` strides 1.
                    let stride = self.size_of_ty(&elem_ty(&ty)).max(1);
                    self.emit(&format!("  add ${}, %rax", *i as i64 * stride + slice_header(&ty)));
                    ty = elem_ty(&ty);
                }
                Proj::IndexLocal(l) => {
                    let stride = self.size_of_ty(&elem_ty(&ty)).max(1);
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
                    // A slice is the address of a `{len: i64, data..}` record
                    // (the shape a `&str`/`&[u8]` literal already has), so its
                    // first element sits one word past the record base.
                    let header = slice_header(&ty);
                    if header != 0 {
                        self.emit(&format!("  add ${header}, %rax"));
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
            Ty::U8 => 1,
            // A one-byte signed integer occupies one byte like `u8`; the
            // extension on load differs.
            Ty::Int { width: 1, .. } => 1,
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

    /// Load the scalar value at `place` into `reg` (which may be `%rax`).
    fn load_place_to(&mut self, p: &Place, reg: &str) -> Result<(), String> {
        let mem = self.place_mem(p)?;
        self.load_mem_to(p, &mem, reg);
        Ok(())
    }

    /// Load the scalar at `mem` (the operand for place `p`) into `reg`.
    fn load_mem_to(&mut self, p: &Place, mem: &str, reg: &str) {
        // A `u8`/`i8` is one byte, so a whole-word load would pull in the seven
        // bytes after it. The byte is zero-extended for `u8` and sign-extended
        // for `i8`, so the value in the slot matches the type's semantics.
        match self.place_ty(p).int_width_sign() {
            Some((1, true)) => {
                self.emit(&format!("  movsbq {mem}, {reg}"));
                return;
            }
            Some((1, false)) => {
                self.emit(&format!("  movzbl {mem}, {}", dword_reg(reg)));
                return;
            }
            _ => {}
        }
        self.emit(&format!("  mov {mem}, {reg}"));
    }

    /// Canonicalise `%rax` to the value of an integer type: truncate to its
    /// width, then sign- or zero-extend back to 64 bits, so every later use
    /// reads the number rustc's `as` would produce. An `i64`/`u64` needs no
    /// work. Used after an `as` cast and by a narrow load.
    fn canonicalize_int(&mut self, ty: &Ty) -> Result<(), String> {
        let Some((width, signed)) = ty.int_width_sign() else {
            return Ok(());
        };
        match (width, signed) {
            (1, true) => self.emit("  movsbq %al, %rax"),
            (1, false) => self.emit("  movzbl %al, %eax"),
            // The encoder has no 16-bit register operand, so extend by shifting
            // the low half into the top and back.
            (2, true) => {
                self.emit("  shl $48, %rax");
                self.emit("  sar $48, %rax");
            }
            (2, false) => {
                self.emit("  shl $48, %rax");
                self.emit("  shr $48, %rax");
            }
            (4, true) => self.emit("  movslq %eax, %rax"),
            (4, false) => self.emit("  mov %eax, %eax"),
            // A 64-bit (or wider, unmodelled) value already fills the slot.
            _ => {}
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
            Operand::Fn(_) => {
                Err("mir_codegen: a function pointer is not an aggregate".into())
            }
        }
    }

    /// Store a full 64-bit word into a frame slot. An unsuffixed
    /// `mov $imm, mem` assembles to a 32-bit store, which zero-extends a
    /// negative immediate (`$-3` becomes `0xFFFFFFFD`); `movq` forces the
    /// sign-extending 64-bit form, and a wide immediate has no memory form at
    /// all and must go through `%rcx`.
    fn store_word(&mut self, src: &str, dst: &str) {
        if is_wide_imm(src) || src.contains('(') {
            self.emit(&format!("  mov {src}, %rcx"));
            self.emit(&format!("  mov %rcx, {dst}"));
        } else if src.starts_with('$') {
            self.emit(&format!("  movq {src}, {dst}"));
        } else {
            self.emit(&format!("  mov {src}, {dst}"));
        }
    }

    /// The field offsets, field types, total size, and (for an enum variant
    /// constant) the tag, of an aggregate constant. A path-qualified name is an
    /// enum only when its head names one the variant tables know:
    /// `Option::Some(1)` is a variant, while `std::ops::Range::<usize> { start,
    /// end }` is a struct the printer spelled out with its full path. Treating
    /// that struct as an enum reserved a tag word and shifted `start`/`end`, so
    /// `r.start` read the tag (`0`) instead of `1`.
    fn const_agg_layout(
        &self,
        c: &Const,
    ) -> Result<(Vec<i64>, Vec<Ty>, i64, Option<i64>), String> {
        let Const::Agg { name, ops } = c else {
            return Err(format!("mir_codegen: aggregate constant unsupported: {c:?}"));
        };
        if is_variant_const(name, &self.enums) {
            let (en, v) = split_ev(name);
            let Some(l) = self.enums.get(&en) else {
                return Err(format!("mir_codegen: unknown enum `{en}`"));
            };
            // Word 0 is the tag; a payload variant's fields follow it.
            let mut offs = Vec::new();
            let mut tys = Vec::new();
            if !l.fieldless {
                for k in 0..ops.len() {
                    offs.push(self.enum_field_offset(&en, &v, k as u64));
                    tys.push(self.enum_field_ty(&en, &v, k as u64));
                }
            }
            let tag = self.variant_tag(&en, &v) as i64;
            Ok((offs, tys, l.size.max(8), Some(tag)))
        } else if name.is_empty() {
            // A tuple or array constant is printed without a name; lay its
            // fields out by their real sizes, as the struct branch does.
            let tys: Vec<Ty> = ops.iter().map(|op| rv_operand_ty(&self.func, op)).collect();
            let mut offs = Vec::new();
            let mut acc = 0i64;
            for t in &tys {
                offs.push(acc);
                acc += self.size_of_ty(t);
            }
            Ok((offs, tys, acc.max(8), None))
        } else {
            let key = struct_path_name(name).unwrap_or_else(|| name.clone());
            let tys = self.structs.get(&key).cloned().unwrap_or_default();
            let mut offs = Vec::new();
            let mut acc = 0i64;
            for t in &tys {
                offs.push(acc);
                acc += self.size_of_ty(t);
            }
            Ok((offs, tys, acc.max(8), None))
        }
    }

    /// Write an aggregate constant into a fresh frame region and leave its
    /// address in `%rax`.
    fn materialize_const_agg(&mut self, c: &Const) -> Result<(), String> {
        let (_offs, _tys, size, _tag) = self.const_agg_layout(c)?;
        let slots = ((size + 7) / 8).max(1);
        let first = self.next_slot;
        for _ in 0..slots {
            self.alloc_slot();
        }
        // `off` grows the address as the slot number falls, so the region's
        // lowest address (byte offset 0) is its highest slot, `base`.
        let base = first + slots - 1;
        // Fields are written through `base + k`; pin the whole run so the frame
        // packer keeps it contiguous.
        self.pinned.push((first, base));
        self.write_const_agg(c, base)?;
        self.emit(&format!("  lea {}, %rax", self.off(base)));
        Ok(())
    }

    /// Write `c`'s bytes into the already-allocated region based at `base`,
    /// recursing into nested aggregate constants so a field that is itself a
    /// struct (or tuple, or array) is laid out rather than stored as a pointer.
    fn write_const_agg(&mut self, c: &Const, base: i64) -> Result<(), String> {
        let Const::Agg { ops, .. } = c else {
            return Err(format!("mir_codegen: aggregate constant unsupported: {c:?}"));
        };
        let (offs, tys, _size, tag) = self.const_agg_layout(c)?;
        if let Some(tag) = tag {
            self.emit_region_addr(base, 0, "%rax");
            self.store_word(&format!("${tag}"), "(%rax)");
        }
        for (k, op) in ops.iter().enumerate() {
            // A zero-sized field stores nothing and takes no word.
            if matches!(rv_operand_ty(&self.func, op), Ty::Unit) {
                continue;
            }
            let off = offs.get(k).copied().unwrap_or(k as i64 * 8);
            let fty = tys.get(k).cloned().unwrap_or(Ty::I64);
            if let Operand::Const(nested @ Const::Agg { .. }) = op {
                // A byte offset of 8*k is slot `base - k`; a nested aggregate is
                // always word-aligned, so this is exact.
                self.write_const_agg(nested, base - off / 8)?;
            } else if fty.is_aggregate() {
                // An enum payload holding a struct is copied in, not stored as
                // a pointer.
                self.eval_aggregate_addr(op)?;
                self.emit("  mov %rax, %r10");
                self.emit_region_addr(base, off, "%r11");
                self.copy_words(self.size_of_ty(&fty));
            } else {
                let (src, _) = self.eval_operand(op)?;
                self.emit_region_addr(base, off, "%rax");
                self.store_word(&src, "(%rax)");
            }
        }
        Ok(())
    }

    /// Emit the address of `base + off` bytes of a contiguous, pinned frame
    /// region into `reg`.
    ///
    /// The write goes through a computed address rather than a direct slot
    /// store: the frame optimiser's dead-store passes only track direct
    /// `(%rbp)` stores, so an interior field written as `movq $v, slot(%rbp)`
    /// looks unread and is deleted, silently zeroing the field.
    fn emit_region_addr(&mut self, base: i64, off: i64, reg: &str) {
        self.emit(&format!("  lea {}, {reg}", self.off(base)));
        if off != 0 {
            self.emit(&format!("  add ${off}, {reg}"));
        }
    }

    /// Materialise the `{len: i64, data..}` record a `&[T]` points at, for the
    /// array-to-slice unsizing `&[T; N] -> &[T]`. `op` is a reference to a
    /// fixed-size array; the record (length word, then a copy of the elements)
    /// is built in the frame and its address left in `%rax`. rmc's slices are
    /// thin pointers to such a record, so binding one in a local keeps the
    /// record alive for as long as the slice is usable.
    fn gen_unsize(&mut self, op: &Operand) -> Result<(), String> {
        let (arr_ref, _) = self.eval_operand(op)?;
        let (elem, n) = match self.operand_ty(op) {
            Ty::Ref(t) | Ty::Ptr(t) => match *t {
                Ty::Array(e, n) => (*e, n),
                _ => return Err("mir_codegen: unsize source is not an array".into()),
            },
            _ => return Err("mir_codegen: unsize source is not a reference".into()),
        };
        let total = (n as i64) * self.size_of_ty(&elem).max(1);
        // The record is the length word plus the element bytes; the slots run
        // from `base` (lowest address, the length word) upward.
        let data_slots = ((total + 7) / 8).max(1);
        let slots = data_slots + 1;
        let first = self.next_slot;
        for _ in 0..slots {
            self.alloc_slot();
        }
        let base = first + slots - 1;
        // The record spans `slots` frame words but is only ever named by its
        // base, so pin the whole run for the frame packer.
        self.pinned.push((first, base));
        self.emit(&format!("  mov ${n}, %rcx"));
        self.emit(&format!("  mov %rcx, {}", self.off(base)));
        if total > 0 {
            self.emit(&format!("  mov {arr_ref}, %r10"));
            self.emit(&format!("  lea {}, %r11", self.off(base - 1)));
            self.copy_words(total);
        }
        self.emit(&format!("  lea {}, %rax", self.off(base)));
        Ok(())
    }

    /// Copy `size` bytes from the address in `%r10` to the address in `%r11`.
    /// `%r10`/`%r11` are never promoted (they are scratch for `div`/`rem`), so
    /// an aggregate copy cannot clobber a promoted local the way `%rsi`/`%rdi`
    /// would when they hold a caller-saved promotion.
    fn copy_words(&mut self, size: i64) {
        if size <= 0 {
            return;
        }
        if size <= 64 {
            let n = size / 8;
            for i in 0..n {
                let d = i * 8;
                self.emit(&format!("  mov {d}(%r10), %rcx"));
                self.emit(&format!("  mov %rcx, {d}(%r11)"));
            }
            // A trailing partial word (a `[u8; N]` slice whose length is not a
            // multiple of eight) is copied byte by byte.
            for i in (n * 8)..size {
                self.emit(&format!("  movb {i}(%r10), %cl"));
                self.emit(&format!("  movb %cl, {i}(%r11)"));
            }
        } else {
            self.emit(&format!("  mov ${}, %rcx", size / 8));
            let loop_l = self.new_label();
            self.emit(&format!("{loop_l}:"));
            self.emit("  mov (%r10), %rax");
            self.emit("  mov %rax, (%r11)");
            self.emit("  add $8, %r10");
            self.emit("  add $8, %r11");
            self.emit("  dec %rcx");
            self.emit(&format!("  jnz {loop_l}"));
            // `%r10`/`%r11` now point past the whole words; copy any tail bytes.
            for i in 0..(size - (size / 8) * 8) {
                self.emit(&format!("  movb {i}(%r10), %cl"));
                self.emit(&format!("  movb %cl, {i}(%r11)"));
            }
        }
    }

    /// `[agg; count]`: copy the `esize`-byte element `count` times. `copy_words`
    /// uses `%rax`, `%rcx`, `%r10` and `%r11`, so the source address, the
    /// running destination and the remaining count live in frame slots.
    fn gen_repeat_aggregate(
        &mut self,
        dest: &Place,
        op: &Operand,
        count: u64,
        esize: i64,
    ) -> Result<(), String> {
        let src_slot = self.alloc_slot();
        let cur_slot = self.alloc_slot();
        let n_slot = self.alloc_slot();
        self.eval_aggregate_addr(op)?;
        let src_off = self.off(src_slot);
        self.emit(&format!("  mov %rax, {src_off}"));
        self.place_addr(dest)?;
        let cur_off = self.off(cur_slot);
        self.emit(&format!("  mov %rax, {cur_off}"));
        let n_off = self.off(n_slot);
        self.emit(&format!("  mov ${count}, %rdx"));
        self.emit(&format!("  mov %rdx, {n_off}"));
        let loop_lbl = self.new_label();
        self.emit(&format!("{loop_lbl}:"));
        self.emit(&format!("  mov {src_off}, %r10"));
        self.emit(&format!("  mov {cur_off}, %r11"));
        self.copy_words(esize);
        self.emit(&format!("  mov {cur_off}, %r11"));
        self.emit(&format!("  add ${esize}, %r11"));
        self.emit(&format!("  mov %r11, {cur_off}"));
        self.emit(&format!("  mov {n_off}, %rdx"));
        self.emit("  dec %rdx");
        self.emit(&format!("  mov %rdx, {n_off}"));
        self.emit(&format!("  jne {loop_lbl}"));
        Ok(())
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
                    self.emit("  mov %rax, %r10");
                    self.place_addr(dest)?;
                    self.emit("  mov %rax, %r11");
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
                    // MIR spells both boolean negation and integer bitwise
                    // complement `Not`. `xor $1` is the boolean inversion (a
                    // normalized `0`/`1`); an integer needs every bit flipped,
                    // or `!2_i64` yields `3` instead of `-3`. The operand type
                    // picks the form.
                    UnOp::Not => {
                        if matches!(self.operand_ty(a), Ty::Bool) {
                            self.emit("  xor $1, %rax");
                        } else {
                            self.emit("  not %rax");
                        }
                    }
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
            Rvalue::Cast { op, to, kind, .. } => {
                // A bit-preserving reinterpretation (`f64::to_bits`,
                // `f32::to_bits`) copies the bits; it is not a value conversion,
                // so it must not reach the numeric float-cast path (which would
                // round `1.5` to `1` instead of reinterpreting `0x3ff8..`).
                if matches!(kind, CastKind::Transmute) {
                    let (src, _) = self.eval_operand(op)?;
                    self.store_scalar(dest, &src)?;
                    return Ok(());
                }
                // A float is involved when either side is one. The source type
                // is resolved at codegen time, not from the `from` field, which
                // the parser cannot know from a local.
                let src_ty = self.operand_ty(op);
                if src_ty.is_float() || to.is_float() {
                    self.gen_float_cast(dest, op, &src_ty, to)?;
                    return Ok(());
                }
                // rmc has no real slice representation: a `&[T]`/`&str` value
                // is a *thin* pointer to a `{len: i64, data..}` record. Only a
                // string literal (and the `b".."` template bytes, which are the
                // same shape) actually has that record. An array unsized to a
                // slice points straight at the elements, so indexing it or
                // reading `PtrMetadata` would walk the length word as data and
                // miscompile. Reject it rather than emit a wrong value.
                if matches!(to.as_ref(), Ty::Ref(t) | Ty::Ptr(t) if matches!(**t, Ty::Array(_, 0))) {
                    // An operand already of slice/`str` type (a literal, or a
                    // local that holds one) carries the record. An operand that
                    // is a reference to a fixed-size array is the genuine
                    // unsizing: build the record it should point at.
                    let unsizing = match op {
                        Operand::Const(Const::Str(_)) | Operand::Const(Const::Bytes(_)) => false,
                        _ => matches!(
                            self.operand_ty(op),
                            Ty::Ref(t) | Ty::Ptr(t)
                                if matches!(*t, Ty::Array(_, n) if n > 0)
                        ),
                    };
                    if unsizing {
                        self.gen_unsize(op)?;
                        self.store_scalar(dest, "%rax")?;
                        return Ok(());
                    }
                }
                // The mirror image: a slice pointer (`*const [T]`/`&[T]`, which
                // rmc holds as the address of a `{len: i64, data..}` record)
                // cast to an element pointer (`*const T`/`&T`) is the
                // data-pointer extraction. The result must point at the
                // elements, one word past the length, so the cast cannot be the
                // no-op the generic path makes it: `Iter::new` does exactly this
                // (`*mut [f64]` -> `*mut f64`), and without the skip the first
                // element read is the length word, shifting every element by
                // one. `[T]` is spelled `Array(T, 0)`.
                let from_ty = self.operand_ty(op);
                if let (Ty::Ptr(f) | Ty::Ref(f), Ty::Ptr(t) | Ty::Ref(t)) =
                    (&from_ty, to.as_ref())
                {
                    if matches!(**f, Ty::Array(_, 0)) && !matches!(**t, Ty::Array(_, 0)) {
                        let (src, _) = self.eval_operand(op)?;
                        if src != "%rax" {
                            self.emit(&format!("  mov {src}, %rax"));
                        }
                        self.emit("  add $8, %rax");
                        self.store_scalar(dest, "%rax")?;
                        return Ok(());
                    }
                }
                let (src, _) = self.eval_operand(op)?;
                if src != "%rax" {
                    self.emit(&format!("  mov {src}, %rax"));
                }
                // A cast reads its source at the source's width and signedness
                // first: `i8 as i64` sign-extends, `u8 as i64` zero-extends.
                // Then `as` to a narrower or unsigned type truncates to the
                // target. Without both steps `-1_i64 as u32` stays `-1` and
                // `x as i8 as i64` keeps the stale high bytes.
                self.canonicalize_int(&from_ty)?;
                self.canonicalize_int(to)?;
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Ref { place, .. } | Rvalue::RawRef { place, .. } => {
                self.place_addr(place)?;
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Repeat { op, count } => {
                // `[v; n]` stores `v` into each of `n` elements of the element's
                // real size. The loop used to store one 8-byte word with an
                // 8-byte stride whatever the element was: `[0u8; 1 << 20]`
                // wrote 8 MiB into a 1 MiB array and ran off the stack (the
                // `textparse` segfault), and `[S { .. }; n]` initialised only
                // the first word of each multi-word element.
                let elem = elem_ty(&self.place_ty(dest));
                let esize = self.size_of_ty(&elem);
                if *count == 0 || esize == 0 {
                    // Nothing to store.
                } else if elem.is_aggregate() {
                    self.gen_repeat_aggregate(dest, op, *count, esize)?;
                } else {
                    // Materialise the value once; it must survive `place_addr`
                    // (which uses `%rax`), so it lives in a scratch slot. The
                    // destination address is kept in `%rcx`, the count in `%rdx`.
                    let (src, _) = self.eval_operand(op)?;
                    let tmp = self.alloc_slot();
                    let off = self.off(tmp);
                    self.store_word(&src, &off);
                    self.place_addr(dest)?;
                    self.emit("  mov %rax, %rcx");
                    let val = self.off(tmp);
                    let loop_lbl = self.new_label();
                    self.emit(&format!("  mov ${count}, %rdx"));
                    self.emit(&format!("  mov {val}, %rax"));
                    self.emit(&format!("{loop_lbl}:"));
                    if esize == 1 {
                        self.emit("  mov %al, (%rcx)");
                        self.emit("  add $1, %rcx");
                    } else {
                        self.emit("  mov %rax, (%rcx)");
                        self.emit(&format!("  add ${esize}, %rcx"));
                    }
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
                    // A one-word aggregate field (`NonNull<T>`, a newtype) fed
                    // a scalar constant is just that word: `ArgumentType::Count`
                    // stores a null `value`, and a constant has no address to
                    // copy from.
                    let scalar_const = matches!(op, Operand::Const(c) if !matches!(c, Const::Agg { .. }))
                        && self.place_size(&target) == 8;
                    if ty.is_aggregate() && !scalar_const {
                        let size = self.place_size(&target);
                        self.eval_aggregate_addr(op)?;
                        self.emit("  mov %rax, %r10");
                        self.place_addr(&target)?;
                        self.emit("  mov %rax, %r11");
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
        // A `u8`/`i8` destination is one byte: a whole-word store would clobber
        // the seven bytes after it. `%al` holds the low byte of the source.
        let byte = matches!(self.place_ty(dest).int_width_sign(), Some((1, _)));
        if dest.proj.is_empty() {
            let dst = self.off(self.local_base(dest.local));
            if byte {
                let src = if src.starts_with('$') {
                    src.to_string()
                } else if src.contains('(') {
                    self.emit(&format!("  mov {src}, %rcx"));
                    "%cl".to_string()
                } else {
                    byte_reg(src).to_string()
                };
                self.emit(&format!("  movb {src}, {dst}"));
                return Ok(());
            }
            self.store_word(src, &dst);
            return Ok(());
        }
        if byte {
            // A memory source cannot feed `mov mem, reg8` directly when it is a
            // wide slot; load it into `%rdx` first. An immediate needs the `b`
            // suffix, or the assembler would pick a 32-bit store.
            //
            // `%rdx`, not `%rcx`: `place_addr` below computes a dynamic index
            // through `%rcx`, so a value parked there is overwritten by the
            // index and the element receives the index (`arr[i] = src[i]`).
            let is_mem = src.contains('(');
            let (mnem, src) = if src == "%rax" || is_mem || is_wide_imm(src) {
                self.emit(&format!("  mov {src}, %rdx"));
                ("mov", "%dl")
            } else if src.starts_with('$') {
                ("movb", src)
            } else {
                ("mov", byte_reg(src))
            };
            let mem = self.place_mem(dest)?;
            self.emit(&format!("  {mnem} {src}, {mem}"));
            return Ok(());
        }
        // A memory source cannot be the source of a memory destination, and
        // `%rax` is clobbered by `place_addr`, so both go through `%rdx`. A
        // wide immediate has no `mov $imm, mem` encoding either. `place_addr`
        // uses `%rcx` for a dynamic index, so `%rdx` is the one scratch it
        // leaves alone.
        let is_mem = src.contains('(');
        let src = if src == "%rax" || src == "%rcx" || is_mem || is_wide_imm(src) {
            self.emit(&format!("  mov {src}, %rdx"));
            "%rdx"
        } else {
            src
        };
        let mem = self.place_mem(dest)?;
        // An unsuffixed `mov $imm, (mem)` is a 32-bit store and zero-extends a
        // negative immediate; `movq` is the sign-extending 64-bit form.
        let mnem = if src.starts_with('$') { "movq" } else { "mov" };
        self.emit(&format!("  {mnem} {src}, {mem}"));
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
        // A float operand routes to SSE: the arithmetic mnemonics differ, the
        // comparison is `ucomisd` rather than `cmp`, and both sides are moved
        // through `%xmm` registers. A pointer offset is never float, so only
        // the arithmetic and comparison operators need this.
        if self.operand_ty(a).is_float() || self.operand_ty(b).is_float() {
            return self.gen_float_binary(dest, op, a, b);
        }
        // `k * x` is `x * k`, so the constant forms of `mul_by_const` see it.
        let (a, b) = match (op, a, b) {
            (BinOp::Mul, Operand::Const(Const::Int(_)), Operand::Copy(_) | Operand::Move(_)) => (b, a),
            _ => (a, b),
        };
        // When the destination is a promoted register and the operator maps to
        // a single ALU instruction, update the register in place. This is the
        // accumulator pattern (`_acc = Add(_acc, k)`), which copy propagation
        // exposes by rewriting `_acc = Add(_tmp, k); _tmp = copy _acc` into a
        // self-update. It saves the value's trip through `%rax`.
        //
        // The destination can be *either* operand: rustc prints `x = x + y` as
        // `Add(copy y, copy x)` in some pipelines and `Add(copy x, copy y)` in
        // others, so `_acc` may be the second operand. Writing `a` into `reg`
        // first would then overwrite the accumulator before it is read
        // (`_acc = Add(x, _acc)` became `x + x`), so the operand that is the
        // destination decides the order: a commutative operator with `_acc` on
        // the right is folded right-to-left, and `Sub` — whose accumulator is
        // always the left operand — computes `a` first and reads `_acc` after.
        if dest.proj.is_empty()
            && matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
            )
            && self.promo.contains_key(&dest.local)
        {
            let reg = self.promo[&dest.local].reg.to_string();
            let acc_left = operand_is_local(a, dest.local);
            let acc_right = operand_is_local(b, dest.local);
            if acc_right && op != BinOp::Sub {
                // Commutative, accumulator on the right: `_acc = x op _acc`
                // is `_acc op= x`. `keep_rax` leaves `reg` (the accumulator)
                // live, so the old value is not lost.
                let (xv, _) = self.eval_operand_keep_rax(a)?;
                let xv = self.wide_imm(&xv, "%rcx");
                self.emit(&format!("  {} {xv}, {reg}", alu_mnem(op)));
                return Ok(());
            }
            if acc_right {
                // `_acc = x - _acc`: compute `x` before reading `_acc`, since
                // the subtraction is not commutative.
                let (xv, _) = self.eval_operand(a)?;
                if xv != "%rax" {
                    self.emit(&format!("  mov {xv}, %rax"));
                }
                let (acc, _) = self.eval_operand_keep_rax(b)?;
                self.emit(&format!("  sub {acc}, %rax"));
                self.emit(&format!("  mov %rax, {reg}"));
                return Ok(());
            }
            if !acc_left {
                let (av, _) = self.eval_operand(a)?;
                if av != reg {
                    self.emit(&format!("  mov {av}, {reg}"));
                }
            }
            if let (BinOp::Mul, Some(seq)) = (op, mul_by_const(b)) {
                for insn in seq {
                    self.emit(&insn.replace("%rax", &reg));
                }
                return Ok(());
            }
            let (bv, _) = self.eval_operand_keep_rax(b)?;
            let bv = self.wide_imm(&bv, "%rcx");
            self.emit(&format!("  {} {bv}, {reg}", alu_mnem(op)));
            return Ok(());
        }
        match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.emit_cmp(op, a, b)?;
                // A `<`/`<=`/`>`/`>=` on an unsigned integer must compare with
                // the carry flag (`setb`/`setbe`/`seta`/`setae`), not the sign
                // flag, or a value with the top bit set reads as negative.
                let cc = if self.cmp_unsigned(a, b) {
                    match op {
                        BinOp::Lt => "setb",
                        BinOp::Le => "setbe",
                        BinOp::Gt => "seta",
                        BinOp::Ge => "setae",
                        other => setcc(other),
                    }
                } else {
                    setcc(op)
                };
                self.emit(&format!("  {cc} %al"));
                self.emit("  movzbl %al, %eax");
                self.store_scalar(dest, "%rax")?;
            }
            BinOp::Cmp => return Err("mir_codegen: three-way Cmp unsupported".into()),
            BinOp::Offset | BinOp::PtrOffset => {
                // `a + b*scale` — a pointer offset by `b` elements. The scale is
                // the pointee's size, not a fixed 8: `*const u8` advances one
                // byte and `*mut i32` four, so a hardcoded 8 would walk off the
                // end of the allocation.
                let scale = match self.operand_ty(a) {
                    Ty::Ptr(t) | Ty::Ref(t) => self.size_of_ty(&t).max(1),
                    // A `&[T]` slice constant's pointer is to `T`; `Array` is
                    // how the parser spells the pointee.
                    Ty::Array(t, _) => self.size_of_ty(&t).max(1),
                    _ => 8,
                };
                let (av, _) = self.eval_operand(a)?;
                if av != "%rax" {
                    self.emit(&format!("  mov {av}, %rax"));
                }
                let (bv, _) = self.eval_operand(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                match scale {
                    1 => self.emit("  add %rcx, %rax"),
                    2 => self.emit("  lea (%rax,%rcx,2), %rax"),
                    4 => self.emit("  lea (%rax,%rcx,4), %rax"),
                    8 => self.emit("  lea (%rax,%rcx,8), %rax"),
                    s => {
                        self.emit(&format!("  imul ${s}, %rcx, %rcx"));
                        self.emit("  add %rcx, %rax");
                    }
                }
                self.store_scalar(dest, "%rax")?;
            }
            _ => {
                self.emit_arith(op, a, b)?;
                self.store_scalar(dest, "%rax")?;
            }
        }
        Ok(())
    }

    /// Emit a floating-point `a op b` into `%rax`.
    ///
    /// A float operand is a one-word value whose bit pattern is moved into
    /// `%xmm0`/`%xmm1` to be operated on and back to `%rax` afterwards, so the
    /// rest of the back end (a frame slot, a promoted register, an argument
    /// register) sees an ordinary scalar. The width picks the `sd`/`ss`
    /// mnemonic: an `f32` operates on the low 32 bits of the `%xmm` register,
    /// which is also how it is stored, so no packing is needed.
    fn gen_float_binary(
        &mut self,
        dest: &Place,
        op: BinOp,
        a: &Operand,
        b: &Operand,
    ) -> Result<(), String> {
        let f32 = matches!(self.operand_ty(a), Ty::F32)
            || matches!(self.operand_ty(b), Ty::F32);
        let suf = if f32 { "ss" } else { "sd" };
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                self.load_float(a, "%xmm0")?;
                self.load_float_keep_xmm0(b, "%xmm1")?;
                let m = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "mul",
                    _ => "div",
                };
                self.emit(&format!("  {m}{suf} %xmm1, %xmm0"));
                self.emit("  movq %xmm0, %rax");
                if f32 {
                    // An `f32` value lives in the low 32 bits, so the upper half
                    // of the word must be cleared for the bit pattern to be
                    // exactly the float and for a `ucomiss`/store to agree.
                    self.emit("  mov %eax, %eax");
                }
                self.store_scalar(dest, "%rax")?;
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.load_float(a, "%xmm0")?;
                self.load_float_keep_xmm0(b, "%xmm1")?;
                self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
                // `ucomiss`/`ucomisd` set CF/ZF/PF as an *unsigned* compare, so
                // the mnemonic is the unsigned one (`setb`, not `setl`), and an
                // unordered result (a NaN operand) sets ZF=PF=CF=1. Rust's
                // comparisons are all false when a NaN is involved except `!=`,
                // which is true, so `==`/`<`/`<=` must clear the parity-bit
                // result and `!=` must set it. `>` and `>=` are already false
                // when unordered (their CF=0 precondition fails), so they need
                // no guard.
                let (cc, guard) = match op {
                    BinOp::Eq => ("sete", Guard::Ordered),
                    BinOp::Ne => ("setne", Guard::Unordered),
                    BinOp::Lt => ("setb", Guard::Ordered),
                    BinOp::Le => ("setbe", Guard::Ordered),
                    BinOp::Gt => ("seta", Guard::None),
                    _ => ("setae", Guard::None),
                };
                self.emit(&format!("  {cc} %al"));
                match guard {
                    Guard::Ordered => {
                        self.emit("  setnp %cl");
                        self.emit("  and %cl, %al");
                    }
                    Guard::Unordered => {
                        self.emit("  setp %cl");
                        self.emit("  or %cl, %al");
                    }
                    Guard::None => {}
                }
                self.emit("  movzbl %al, %eax");
                self.store_scalar(dest, "%rax")?;
            }
            _ => {
                // Route through the shared classifier so the user gets the same
                // `note:` as every other unsupported construct (and so the note
                // lives in one place, `classify_unsupported`, rather than being
                // duplicated here).
                let tyname = if f32 { "f32" } else { "f64" };
                return Err(unsupported_note(
                    &format!("mir_codegen: float operator {op:?} is not implemented"),
                    &format!("{op:?} {tyname}"),
                ));
            }
        }
        Ok(())
    }

    /// Load a float operand's bit pattern into an `%xmm` register. The operand
    /// is a scalar (an immediate bit pattern, a frame slot, or a promoted
    /// register), so it goes through `%rax` first, except for a direct memory
    /// operand which `movsd`/`movss` can read without the trip.
    fn load_float(&mut self, op: &Operand, xmm: &str) -> Result<(), String> {
        let f32 = matches!(self.operand_ty(op), Ty::F32);
        let (v, mem) = self.eval_operand(op)?;
        if mem && !f32 {
            // `movsd` reads a double straight from memory.
            self.emit(&format!("  movsd {v}, {xmm}"));
            return Ok(());
        }
        if mem && f32 {
            self.emit(&format!("  movss {v}, {xmm}"));
            return Ok(());
        }
        if v != "%rax" {
            self.emit(&format!("  mov {v}, %rax"));
        }
        if f32 {
            self.emit(&format!("  movd %eax, {xmm}"));
        } else {
            self.emit(&format!("  movq %rax, {xmm}"));
        }
        Ok(())
    }

    /// Load the right-hand float operand without clobbering `%xmm0`, mirroring
    /// `eval_operand_keep_rax` for the SSE side.
    fn load_float_keep_xmm0(&mut self, op: &Operand, xmm: &str) -> Result<(), String> {
        self.load_float(op, xmm)
    }

    /// Load a compile-time float constant into an `%xmm` register.
    fn load_float_imm(&mut self, v: f64, f32: bool, xmm: &str) {
        let bits = if f32 {
            (v as f32).to_bits() as i64
        } else {
            v.to_bits() as i64
        };
        self.emit(&format!("  movabs ${bits}, %rax"));
        if f32 {
            self.emit(&format!("  movd %eax, {xmm}"));
        } else {
            self.emit(&format!("  movq %rax, {xmm}"));
        }
    }

    /// Convert the float in `%xmm0` to the integer type `to`, saturating and
    /// mapping NaN to zero the way Rust's `as` does. `cvttsd2si` only saturates
    /// at the *signed* 64-bit limits (and returns `i64::MIN` for a positive
    /// overflow), so a narrower, unsigned, or `u64` target needs an explicit
    /// clamp against the target's bounds. Result lands in `%rax`.
    fn gen_float_to_int(&mut self, src_f32: bool, to: &Ty) -> Result<(), String> {
        let Some((width, signed)) = to.int_width_sign() else {
            // A float-to-float or float-to-pointer cast does not reach here.
            return Ok(());
        };
        let suf = if src_f32 { "ss" } else { "sd" };
        let hi_lbl = self.new_label();
        let lo_lbl = self.new_label();
        let nan_lbl = self.new_label();
        let done_lbl = self.new_label();
        // NaN is the only value not equal to itself; catch it before the range
        // compares, which would otherwise fall through to the conversion.
        self.emit(&format!("  ucomi{suf} %xmm0, %xmm0"));
        self.emit(&format!("  jp {nan_lbl}"));

        if signed {
            let bits = width * 8;
            let hi: i64 = if bits >= 64 { i64::MAX } else { (1i64 << (bits - 1)) - 1 };
            let lo: i64 = if bits >= 64 { i64::MIN } else { -(1i64 << (bits - 1)) };
            // `2^(bits-1)` is exactly representable as a float; `>=` it clamps
            // to the maximum.
            let hi_f = 2f64.powi((bits - 1) as i32);
            let lo_f = lo as f64;
            self.load_float_imm(hi_f, src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jae {hi_lbl}"));
            self.load_float_imm(lo_f, src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jb {lo_lbl}"));
            self.emit_float_trunc(src_f32)?;
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{hi_lbl}:"));
            self.emit(&format!("  movabs ${hi}, %rax"));
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{lo_lbl}:"));
            self.emit(&format!("  movabs ${lo}, %rax"));
        } else if width < 8 {
            let bits = width * 8;
            let hi: i64 = (1i64 << bits) - 1;
            let hi_f = 2f64.powi(bits as i32);
            self.load_float_imm(hi_f, src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jae {hi_lbl}"));
            self.load_float_imm(0.0, src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jb {lo_lbl}"));
            self.emit_float_trunc(src_f32)?;
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{hi_lbl}:"));
            self.emit(&format!("  movabs ${hi}, %rax"));
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{lo_lbl}:"));
            self.emit("  xor %eax, %eax");
        } else {
            // `u64`: [0, 2^63) converts as a signed value; [2^63, 2^64) is
            // converted after subtracting 2^63 and re-biasing; >= 2^64 clamps.
            let big_lbl = self.new_label();
            self.load_float_imm(0.0, src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jb {lo_lbl}"));
            self.load_float_imm(2f64.powi(64), src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jae {hi_lbl}"));
            self.load_float_imm(2f64.powi(63), src_f32, "%xmm1");
            self.emit(&format!("  ucomi{suf} %xmm1, %xmm0"));
            self.emit(&format!("  jae {big_lbl}"));
            self.emit_float_trunc(src_f32)?;
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{big_lbl}:"));
            self.load_float_imm(2f64.powi(63), src_f32, "%xmm1");
            self.emit(&format!("  sub{suf} %xmm1, %xmm0"));
            self.emit_float_trunc(src_f32)?;
            self.emit("  movabs $-9223372036854775808, %rcx");
            self.emit("  add %rcx, %rax");
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{hi_lbl}:"));
            self.emit("  movabs $-1, %rax");
            self.emit(&format!("  jmp {done_lbl}"));
            self.emit(&format!("{lo_lbl}:"));
            self.emit("  xor %eax, %eax");
        }
        self.emit(&format!("  jmp {done_lbl}"));
        self.emit(&format!("{nan_lbl}:"));
        self.emit("  xor %eax, %eax");
        self.emit(&format!("{done_lbl}:"));
        Ok(())
    }

    /// `cvttsd2si`/`cvttss2si` from `%xmm0` into `%rax`.
    fn emit_float_trunc(&mut self, src_f32: bool) -> Result<(), String> {
        if src_f32 {
            self.emit("  cvttss2si %xmm0, %rax");
        } else {
            self.emit("  cvttsd2si %xmm0, %rax");
        }
        Ok(())
    }

    /// A cast with a float on one side: `IntToFloat`, `FloatToInt`, and the
    /// `f32`/`f64` widening and narrowing. The integer side is a signed 64-bit
    /// value, which is what the back end stores for every integer width.
    fn gen_float_cast(
        &mut self,
        dest: &Place,
        op: &Operand,
        src: &Ty,
        to: &Ty,
    ) -> Result<(), String> {
        let to_f32 = matches!(to, Ty::F32);
        match (src.is_float(), to.is_float()) {
            // Integer -> float: `cvtsi2ss`/`cvtsi2sd` read a signed 64-bit
            // general-purpose register and write the `%xmm` register.
            (false, true) => {
                let (v, _) = self.eval_operand(op)?;
                if v != "%rax" {
                    self.emit(&format!("  mov {v}, %rax"));
                }
                if to_f32 {
                    self.emit("  cvtsi2ss %rax, %xmm0");
                    self.emit("  movd %xmm0, %eax");
                } else {
                    self.emit("  cvtsi2sd %rax, %xmm0");
                    self.emit("  movq %xmm0, %rax");
                }
                self.store_scalar(dest, "%rax")?;
            }
            // Float -> integer: `cvttsd2si` truncates toward zero, which is
            // Rust's `as` semantics. A value out of range (or a NaN) leaves
            // `0x8000000000000000`, matching rustc's saturating cast for the
            // signed 64-bit case.
            (true, false) => {
                self.load_float(op, "%xmm0")?;
                self.gen_float_to_int(matches!(src, Ty::F32), to)?;
                self.store_scalar(dest, "%rax")?;
            }
            // Float -> float: `cvtss2sd` widens, `cvtsd2ss` narrows (rounding
            // to nearest-even, which matches rustc).
            (true, true) => {
                self.load_float(op, "%xmm0")?;
                if matches!(src, Ty::F32) && !to_f32 {
                    self.emit("  cvtss2sd %xmm0, %xmm0");
                    self.emit("  movq %xmm0, %rax");
                } else if matches!(src, Ty::F64) && to_f32 {
                    self.emit("  cvtsd2ss %xmm0, %xmm0");
                    self.emit("  movd %xmm0, %eax");
                } else if to_f32 {
                    self.emit("  movd %xmm0, %eax");
                } else {
                    self.emit("  movq %xmm0, %rax");
                }
                self.store_scalar(dest, "%rax")?;
            }
            (false, false) => unreachable!("gen_float_cast called with no float side"),
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
            BinOp::Mul if mul_by_const(b).is_some() => {
                for insn in mul_by_const(b).unwrap_or_default() {
                    self.emit(&insn);
                }
            }
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
            }
            BinOp::Shl | BinOp::Shr => {
                let m = match op {
                    BinOp::Shl => "sal",
                    // An unsigned shift fills with zeros, a signed one with the
                    // sign bit. MIR's `Shr` covers both; the operand's type
                    // decides, or `0x8000000000000000u64 >> 60` fills with ones
                    // and reads as a huge negative instead of 8.
                    _ => {
                        if self.operand_ty(a).is_unsigned() {
                            "shr"
                        } else {
                            "sar"
                        }
                    }
                };
                // A constant count is an immediate. The hardware masks a
                // 64-bit shift's count to six bits whether it comes from `%cl`
                // or an immediate, so masking here keeps the two forms equal.
                if let Operand::Const(Const::Int(k)) = b {
                    self.emit(&format!("  {m} ${}, %rax", k & 63));
                    return Ok(());
                }
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
                self.emit(&format!("  {m} %cl, %rax"));
            }
            BinOp::Div | BinOp::Rem => {
                let unsigned = self.div_is_unsigned(a, b);
                if !unsigned && self.gen_div_const(op, b)? {
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
                if unsigned {
                    // Zero-extend the dividend into `%rdx:%rax`; `div` divides
                    // the 128-bit pair by `%rcx` as unsigned.
                    self.emit("  xor %edx, %edx");
                    self.emit("  div %rcx");
                } else {
                    self.emit("  cqto");
                    self.emit("  idiv %rcx");
                }
                if op == BinOp::Rem {
                    self.emit("  mov %rdx, %rax");
                }
                self.emit(&format!("{done}:"));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Whether a `/`/`%` should use the unsigned `div` instruction. The
    /// dividend's type decides; when it is an untyped constant the divisor's
    /// type is the fallback.
    fn div_is_unsigned(&self, a: &Operand, b: &Operand) -> bool {
        let ta = self.operand_ty(a);
        if ta.is_int() {
            return ta.is_unsigned();
        }
        self.operand_ty(b).is_unsigned()
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
                if let Some(reg) = self.const_regs.get(&m) {
                    self.emit(&format!("  mov {reg}, %rcx"));
                } else if let Some(reg) = self.const_caller_regs.get(&m).copied() {
                    if self.in_drop {
                        let slot = self.off(self.const_spill[&m]);
                        self.emit(&format!("  mov {slot}, %rcx"));
                    } else {
                        self.emit(&format!("  mov {reg}, %rcx"));
                    }
                } else {
                    self.emit(&format!("  movabs ${m}, %rcx"));
                }
                self.emit("  imul %rcx");
                if add {
                    // `m` does not fit a positive `i64`, so `imul` multiplied by
                    // its negative two's-complement value and the product does
                    // not carry the dividend's sign. Fold in the bias first, then
                    // take the correction term from the dividend copy in `%r10`.
                    self.emit("  add %r10, %rdx");
                    if s > 0 {
                        self.emit(&format!("  sar ${s}, %rdx"));
                    }
                    self.emit("  mov %r10, %rcx");
                    self.emit("  shr $63, %rcx");
                    self.emit("  add %rcx, %rdx");
                } else {
                    // `m` is positive, so the product has the dividend's sign,
                    // which is `%rdx >> 63`; taking the correction term from
                    // `%rdx` straight after the `imul` keeps the `%r10` copy off
                    // the critical path (only the `add` above reads it, and only
                    // when `add` is set, which is not this branch).
                    self.emit("  mov %rdx, %rcx");
                    self.emit("  shr $63, %rcx");
                    if s > 0 {
                        self.emit(&format!("  sar ${s}, %rdx"));
                    }
                    self.emit("  add %rcx, %rdx");
                }
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
            let (av, a_mem) = self.eval_operand(a)?;
            let (bv, b_mem) = self.eval_operand(b)?;
            let bv = self.wide_imm(&bv, "%rcx");
            // x86 has no memory-to-memory `cmp`. Two unpromoted locals are both
            // frame slots, which a function with more live values than
            // registers produces (an interpreter loop, a parser); emitting
            // `cmp -848(%rbp), -904(%rbp)` failed to assemble, or, once a
            // peephole rewrote a neighbour, assembled into the wrong compare.
            if a_mem && b_mem {
                self.emit(&format!("  mov {av}, %rax"));
                self.emit(&format!("  cmp {bv}, %rax"));
            } else {
                self.emit(&format!("  cmp {bv}, {av}"));
            }
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

    /// Spill every promoted caller-saved local to its frame slot, returning the
    /// `(local, register)` list `restore_caller_saved` reloads. `%rdi`, `%rsi`,
    /// `%r8` and `%r9` are all clobbered by a call, so a local that lives in one
    /// must be saved around it. `gen_call` and the drop glue both call this.
    ///
    /// When `bb` is a call block, a local is saved only when it is live after
    /// the call (`live_across_call`): a register whose value the code never reads
    /// again does not need a store/reload pair. The drop-glue path passes `None`
    /// and saves every promoted caller-saved local, because its calls are spread
    /// across a tree with no single block to query.
    fn spill_caller_saved(&mut self, bb: Option<u32>) -> Vec<(u32, &'static str)> {
        let mut caller_promos: Vec<(u32, &'static str)> = self
            .promo
            .iter()
            .filter(|(_, p)| p.caller_saved)
            .map(|(l, p)| (*l, p.reg))
            .collect();
        caller_promos.sort_unstable();
        if let Some(bb) = bb {
            caller_promos.retain(|(l, _)| self.live_across_call(bb, *l));
        }
        // Locals sharing a register share its spill slot: save and restore
        // each register once, or a restore of a dead owner could land last.
        let mut seen: HashSet<&'static str> = HashSet::new();
        caller_promos.retain(|(_, r)| seen.insert(*r));
        for (l, reg) in &caller_promos {
            let slot = self.off(self.caller_spill[l]);
            self.emit(&format!("  mov {reg}, {slot}"));
        }
        // Caller-saved hoisted constants are clobbered by the call too.
        let mut consts: Vec<(i64, &'static str)> = self
            .const_caller_regs
            .iter()
            .map(|(v, r)| (*v, *r))
            .collect();
        consts.sort_unstable();
        for (v, reg) in consts {
            let slot = self.off(self.const_spill[&v]);
            self.emit(&format!("  mov {reg}, {slot}"));
        }
        caller_promos
    }

    /// Reload the promoted caller-saved locals `spill_caller_saved` saved.
    fn restore_caller_saved(&mut self, saved: &[(u32, &'static str)]) {
        for (l, reg) in saved {
            let slot = self.off(self.caller_spill[l]);
            self.emit(&format!("  mov {slot}, {reg}"));
        }
        let mut consts: Vec<(i64, &'static str)> = self
            .const_caller_regs
            .iter()
            .map(|(v, r)| (*v, *r))
            .collect();
        consts.sort_unstable();
        for (v, reg) in consts {
            let slot = self.off(self.const_spill[&v]);
            self.emit(&format!("  mov {slot}, {reg}"));
        }
    }

    /// Emit `place`'s destructor: call its type's `Drop::drop` impl with the
    /// value's address, or, when the type has no impl of its own but has fields
    /// that do, run each field's destructor in place.
    fn gen_drop(&mut self, place: &Place) -> Result<(), String> {
        let ty = self.place_ty(place);
        if self.ty_needs_drop(&ty) {
            // The destructor tree makes calls that clobber the caller-saved
            // argument registers, and a promoted caller-saved local may live in
            // one. Spill them once here, before any address is materialised
            // into `%rdi` (which would overwrite a promoted `%rdi`), and restore
            // after the whole tree. The spill cannot live inside the nested
            // glue: every level shares one slot per local, so an inner spill
            // would overwrite the outer's saved value with an argument register
            // that already holds an address.
            let saved = self.spill_caller_saved(None);
            let was_in_drop = self.in_drop;
            self.in_drop = true;
            self.gen_drop_glue(place, &ty)?;
            self.in_drop = was_in_drop;
            self.restore_caller_saved(&saved);
        }
        Ok(())
    }

    /// Run the destructor of the value at `place`: its type's own `Drop` impl,
    /// then, as rustc's `drop_in_place` does, the destructor of every droppable
    /// field. A type without an impl has only the fields to drop.
    fn gen_drop_glue(&mut self, place: &Place, ty: &Ty) -> Result<(), String> {
        // An enum's payload is variant-dependent, so it cannot be walked by
        // field index. Hand it to the address-based helper, which runs the
        // enum's own `Drop` impl (if any) and then dispatches on the tag. This
        // must happen *before* the own-impl call below, or the impl would run
        // twice (once here, once inside the helper).
        if let Ty::Struct(name) = ty {
            if self.enums.contains_key(name) {
                // The enum's own `Drop` impl runs first, then the active
                // variant's fields. Hand the whole thing to the address helper,
                // which runs the impl and dispatches on the tag; `gen_drop`
                // already spilled the promoted caller-saved locals across it.
                self.place_addr(place)?;
                self.emit("  mov %rax, %rdi");
                self.gen_drop_glue_at_reg(ty)?;
                return Ok(());
            }
        }
        if let Some(sym) = self.drop_symbol_for_ty(ty) {
            self.place_addr(place)?;
            self.emit("  mov %rax, %rdi");
            self.emit(&format!("  call {}", asm_sym(&sym)));
        }
        match ty {
            Ty::Struct(name) => {
                let fields = self.structs.get(name).cloned().unwrap_or_default();
                for (i, fty) in fields.iter().enumerate() {
                    if !self.ty_needs_drop(fty) {
                        continue;
                    }
                    let sub = self.subplace(place, i as u64);
                    self.gen_drop_glue(&sub, fty)?;
                }
            }
            Ty::Tuple(tys) => {
                for (i, fty) in tys.iter().enumerate() {
                    if !self.ty_needs_drop(fty) {
                        continue;
                    }
                    let sub = self.subplace(place, i as u64);
                    self.gen_drop_glue(&sub, fty)?;
                }
            }
            Ty::Array(elem, n) => {
                if *n == 0 || !self.ty_needs_drop(elem) {
                    return Ok(());
                }
                let size = self.size_of_ty(elem).max(1);
                let ptr = self.alloc_slot();
                let i = self.alloc_slot();
                let loop_lbl = self.new_label();
                let done = self.new_label();
                // Evaluate the base address once: the element destructor is a
                // call that clobbers the registers `place_addr` would otherwise
                // recompute it from, and a projected base may live in one.
                self.place_addr(place)?;
                self.emit(&format!("  mov %rax, {}", self.off(ptr)));
                self.emit(&format!("  movq $0, {}", self.off(i)));
                self.emit(&format!("{loop_lbl}:"));
                self.emit(&format!("  cmpq ${n}, {}", self.off(i)));
                self.emit(&format!("  jge {done}"));
                self.emit(&format!("  mov {}, %rdi", self.off(ptr)));
                self.emit(&format!("  mov {}, %rdx", self.off(i)));
                if size != 1 {
                    self.emit(&format!("  imul ${size}, %rdx"));
                }
                self.emit("  add %rdx, %rdi");
                // The element's destructor may itself be glue, so recurse
                // through the element address rather than inlining the math.
                self.gen_drop_glue_at_reg(elem)?;
                self.emit(&format!("  addq $1, {}", self.off(i)));
                self.emit(&format!("  jmp {loop_lbl}"));
                self.emit(&format!("{done}:"));
            }
            _ => {}
        }
        Ok(())
    }

    /// Glue for an aggregate whose address is already in `%rdi` (an array
    /// element, or a field reached by pointer). Emits the type's own impl, then
    /// its fields' destructors at their offsets, recursing for nested
    /// aggregates.
    fn gen_drop_glue_at_reg(&mut self, ty: &Ty) -> Result<(), String> {
        // Save the aggregate's address before the own-impl call, which clobbers
        // `%rdi`; the field loops below need it to reach each field.
        let fields: Vec<Ty> = match ty {
            Ty::Struct(name) => self.structs.get(name).cloned().unwrap_or_default(),
            Ty::Tuple(tys) => tys.iter().map(|t| (**t).clone()).collect(),
            _ => Vec::new(),
        };
        let needs_fields = match ty {
            Ty::Array(elem, n) => *n != 0 && self.ty_needs_drop(elem),
            Ty::Struct(name) if self.enums.contains_key(name) => self.enum_needs_drop(name),
            _ => fields.iter().any(|f| self.ty_needs_drop(f)),
        };
        let ptr = if needs_fields { Some(self.alloc_slot()) } else { None };
        if let Some(p) = ptr {
            self.emit(&format!("  mov %rdi, {}", self.off(p)));
        }
        if let Some(sym) = self.drop_symbol_for_ty(ty) {
            self.emit(&format!("  call {}", asm_sym(&sym)));
        }
        match ty {
            Ty::Array(elem, n) => {
                if *n == 0 || !self.ty_needs_drop(elem) {
                    return Ok(());
                }
                let size = self.size_of_ty(elem).max(1);
                let ptr = ptr.expect("array glue allocated its pointer slot");
                let i = self.alloc_slot();
                let loop_lbl = self.new_label();
                let done = self.new_label();
                self.emit(&format!("  movq $0, {}", self.off(i)));
                self.emit(&format!("{loop_lbl}:"));
                self.emit(&format!("  cmpq ${n}, {}", self.off(i)));
                self.emit(&format!("  jge {done}"));
                self.emit(&format!("  mov {}, %rdi", self.off(ptr)));
                self.emit(&format!("  mov {}, %rdx", self.off(i)));
                if size != 1 {
                    self.emit(&format!("  imul ${size}, %rdx"));
                }
                self.emit("  add %rdx, %rdi");
                self.gen_drop_glue_at_reg(elem)?;
                self.emit(&format!("  addq $1, {}", self.off(i)));
                self.emit(&format!("  jmp {loop_lbl}"));
                self.emit(&format!("{done}:"));
            }
            Ty::Struct(name) if self.enums.contains_key(name) => {
                let Some(ptr) = ptr else { return Ok(()) };
                let layout = self.enums.get(name).cloned().unwrap_or_default();
                // For each variant, compare the tag and, on a match, drop each
                // droppable payload field. `switchInt` on the discriminant is
                // exactly what the value's own `drop_in_place` lowers to.
                let variants: Vec<(String, Vec<Ty>)> = layout
                    .tags
                    .iter()
                    .map(|(v, _)| (v.clone(), layout.fields.get(v).cloned().unwrap_or_default()))
                    .collect();
                let done = self.new_label();
                for (v, tys) in variants {
                    if !tys.iter().any(|t| self.ty_needs_drop(t)) {
                        continue;
                    }
                    let next = self.new_label();
                    let tag = self.variant_tag(name, &v);
                    self.emit(&format!("  mov {}, %rax", self.off(ptr)));
                    self.emit(&format!("  mov {}, %rdx", self.off(ptr)));
                    self.emit("  mov (%rdx), %rdx");
                    self.emit(&format!("  cmp ${tag}, %rdx"));
                    self.emit(&format!("  jne {next}"));
                    for (k, fty) in tys.iter().enumerate() {
                        if !self.ty_needs_drop(fty) {
                            continue;
                        }
                        let off = layout.payload_off + k as i64 * 8;
                        self.emit(&format!("  mov {}, %rdi", self.off(ptr)));
                        if off != 0 {
                            self.emit(&format!("  add ${off}, %rdi"));
                        }
                        self.gen_drop_glue_at_reg(fty)?;
                    }
                    self.emit(&format!("  jmp {done}"));
                    self.emit(&format!("{next}:"));
                }
                self.emit(&format!("{done}:"));
            }
            Ty::Struct(_) | Ty::Tuple(_) => {
                // No droppable field: the own impl (already emitted) is all.
                let Some(ptr) = ptr else { return Ok(()) };
                let mut off = 0i64;
                for fty in fields.iter() {
                    if self.ty_needs_drop(fty) {
                        self.emit(&format!("  mov {}, %rdi", self.off(ptr)));
                        if off != 0 {
                            self.emit(&format!("  add ${off}, %rdi"));
                        }
                        self.gen_drop_glue_at_reg(fty)?;
                    }
                    off += self.size_of_ty(fty);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The `__rmc_drop_<T>` symbol for a type that has a `Drop` impl of its own.
    fn drop_symbol_for_ty(&self, ty: &Ty) -> Option<String> {
        let Ty::Struct(name) = ty else { return None };
        self.prog.drop_impls.get(name).cloned()
    }

    /// Whether a value of `ty` has a destructor: a `Drop` impl of its own, or a
    /// field that does.
    fn ty_needs_drop(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Struct(name) => {
                if self.prog.drop_impls.contains_key(name) {
                    return true;
                }
                if self.enums.contains_key(name) {
                    return self.enum_needs_drop(name);
                }
                self.structs
                    .get(name)
                    .map(|f| f.iter().any(|t| self.ty_needs_drop(t)))
                    .unwrap_or(false)
            }
            Ty::Array(elem, _) => self.ty_needs_drop(elem),
            Ty::Tuple(tys) => tys.iter().any(|t| self.ty_needs_drop(t)),
            _ => false,
        }
    }

    /// Whether any variant of enum `name` carries a payload field that needs a
    /// destructor.
    fn enum_needs_drop(&self, name: &str) -> bool {
        self.enums
            .get(name)
            .map(|l| {
                l.fields
                    .values()
                    .flatten()
                    .any(|t| self.ty_needs_drop(t))
            })
            .unwrap_or(false)
    }

    fn gen_call(&mut self, dest: &Place, func: &CallTarget, args: &[Operand], bb: u32) -> Result<(), String> {
        // A call through a computed callee: evaluate the callee's address into a
        // fresh frame slot, then use the same argument-marshalling path as a
        // direct call and emit `call *slot(%rbp)`. A frame slot (not `%r10`/
        // `%r11`) is used because those are scratch for `div`/`rem` and the
        // block copy, which an aggregate argument's evaluation may run. The
        // callee is computed *first* so a callee expression that reads an
        // argument local sees the pre-call values (as MIR does).
        let mut indirect: Option<String> = None;
        let name = match func {
            CallTarget::Direct(n) => n.clone(),
            CallTarget::Indirect(op) => {
                let op = op.as_ref().ok_or_else(|| {
                    "mir_codegen: indirect call (fn pointer) unsupported".to_string()
                })?;
                // A direct reference to a named function is a plain direct call.
                if let Operand::Fn(n) = op {
                    n.clone()
                } else {
                    let (v, _) = self.eval_operand(op)?;
                    let slot = self.alloc_slot();
                    let off = self.off(slot);
                    self.store_word(&v, &off);
                    indirect = Some(off);
                    "<fn ptr>".to_string()
                }
            }
        };
        if name == "alloc" {
            // rustc models `alloc` as an unsafe fn with an `Option`-like ABI;
            // the program's own `alloc` is rmc's builtin, so call it directly.
        }
        // A call that returns an aggregate takes a hidden pointer to the
        // destination in `%rdi`, so every real argument shifts one register
        // along and the pointer is computed after the argument moves. That
        // pointer setup clobbers `%rax`/`%rcx`, which the "leave the last
        // argument in its scratch register" shortcut relies on, so the shortcut
        // is disabled here and every computed argument goes through a spill slot.
        let agg_ret = self.place_ty(dest).is_aggregate();
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
        let last_unstable = if agg_ret {
            None
        } else {
            args.iter().rposition(|a| !is_stable(a))
        };
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
                // An immediate, a memory operand, or a promoted local's own
                // register: evaluating a later argument writes only scratch
                // registers, so none of these changes before the moves below.
                let stable = v.starts_with('$') || v.contains('(') || !SCRATCH_REGS.contains(&v.as_str());
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
        let caller_promos = self.spill_caller_saved(Some(bb));
        // A hidden aggregate-return pointer claims `%rdi`, so every real
        // argument moves one register along (`%rsi` for argument 0).
        let arg_shift = agg_ret as usize;
        if arg_shift == 1 && reg_ops.iter().any(|(i, _)| *i + 1 >= ARG_REGS.len()) {
            return Err(format!(
                "mir_codegen: aggregate-returning call to {name} with six arguments unsupported"
            ));
        }
        let moves: Vec<(String, &str)> =
            reg_ops.iter().map(|(i, v)| (v.clone(), ARG_REGS[*i + arg_shift])).collect();
        match parallel_moves(moves.clone()) {
            Some(ms) => {
                for m in ms {
                    self.emit(&m);
                }
            }
            None => {
                // No free scratch register to break a cycle with (not seen in
                // practice): stash every register source in a slot first.
                let mut staged = Vec::new();
                for (v, d) in moves {
                    if v.starts_with('%') && v != d {
                        let tmp = self.alloc_slot();
                        self.emit(&format!("  mov {v}, {}", self.off(tmp)));
                        staged.push((self.off(tmp), d));
                    } else {
                        staged.push((v, d));
                    }
                }
                for (v, d) in staged {
                    if v != d {
                        self.emit(&format!("  mov {v}, {d}"));
                    }
                }
            }
        }
        if agg_ret {
            // Compute the destination address last, into `%rdi`, after every
            // other register is final (`place_addr` clobbers `%rax`/`%rcx`).
            self.place_addr(dest)?;
            self.emit("  mov %rax, %rdi");
        }
        // The note tells `asmopt` how many argument registers the call reads.
        let n = reg_ops.len() + arg_shift;
        match &indirect {
            Some(reg) => self.emit(&format!("  call *{reg} #argregs {n}")),
            None => self.emit(&format!("  call {} #argregs {n}", asm_sym(&name))),
        }
        self.restore_caller_saved(&caller_promos);
        if agg_ret {
            // The aggregate was written straight into `dest` through the hidden
            // pointer, so there is nothing to store.
            return Ok(());
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
            // A float constant carries its width, so the arithmetic paths can
            // pick the single- or double-precision SSE form.
            Operand::Const(Const::Float(_, is_f32)) => {
                if *is_f32 {
                    Ty::F32
                } else {
                    Ty::F64
                }
            }
            // A string constant is a byte pointer, so `Offset` scales by 1.
            Operand::Const(Const::Str(_)) => Ty::Ptr(Box::new(Ty::U8)),
            Operand::Const(_) => Ty::I64,
            Operand::Fmt { .. } | Operand::Fn(_) => Ty::Ptr(Box::new(Ty::Unit)),
        }
    }
}

/// Fold `mov %a, %b` into the reads of `%b` that follow, when `%b` is dead
/// afterwards.
///
/// `heap`'s traversal loop copies its cursor into a second register before each
/// dereference (`mov %r15, %r14; mov %rax, %r15; mov (%rax), ...`), because the
/// register allocator works per MIR local. While `%b` still holds `%a` and
/// neither is rewritten, every read of `%b` can name `%a`; if `%b` carries
/// nothing past the region, the copy goes too.
///
/// The region stops at the first label (a branch could enter with a different
/// `%a`), the first control transfer, or the first write to either register.
/// Only registers with no implicit operand role take part: renaming a read of
/// `%rax` or `%rcx` would leave a `mul`/`div`/shift's implicit use pointing at a
/// value the dropped copy no longer put there.
fn propagate_copies(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        let Some((a, b)) = mov_reg_reg(lines[i]) else {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        };
        if !is_copy_safe_reg(a) || !is_copy_safe_reg(b) || a == b {
            out.push_str(lines[i]);
            out.push('\n');
            i += 1;
            continue;
        }
        let mut k = i + 1;
        while k < lines.len() {
            let lt = lines[k].trim();
            let op = lt.split_whitespace().next().unwrap_or("");
            if lt.ends_with(':') || lt.starts_with('.') {
                break;
            }
            if op.starts_with('j') || matches!(op, "ret" | "call" | "syscall") {
                break;
            }
            if written_regs(op, lt).iter().any(|r| r == a || r == b) {
                break;
            }
            k += 1;
        }
        if k > i + 1 && dead_after(&lines, i, k, b) {
            for m in &lines[i + 1..k] {
                out.push_str(&replace_reg(m, b, a));
                out.push('\n');
            }
            i = k;
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// True for a register the copy rename cannot disturb: it has no implicit
/// operand role in any instruction the generator emits.
fn is_copy_safe_reg(r: &str) -> bool {
    matches!(
        r,
        "%rbx" | "%r8" | "%r9" | "%r10" | "%r11" | "%r12" | "%r13" | "%r14" | "%r15"
    )
}

/// The register a memory operand is based on: `8(%r14)` -> `%r14`.
fn base_reg(slot: &str) -> Option<&str> {
    let l = slot.find('(')?;
    let r = slot[l + 1..].find(')')? + l + 1;
    let b = &slot[l + 1..r];
    is_reg(b).then_some(b)
}

/// Rewrite a memory operand's base register: `8(%rax)` -> `8(%r14)`.
fn replace_base(slot: &str, from: &str, to: &str) -> String {
    let Some(l) = slot.find('(') else { return slot.to_string() };
    let Some(rel) = slot[l + 1..].find(')') else { return slot.to_string() };
    let r = rel + l + 1;
    if &slot[l + 1..r] != from {
        return slot.to_string();
    }
    format!("{}({}){}", &slot[..l], to, &slot[r + 1..])
}

/// Fold `mov <base>, t` into the loads that immediately follow it through `t`.
///
/// The allocator parks an address in a scratch register before each
/// dereference, so `heap`'s traversal loop reads
/// `mov %r14, %rax; mov (%rax), %rax; add %rax, %rsi` where LLVM folds the base
/// straight into the load. The region is the contiguous run of loads based on
/// `t`; the run must end by writing `t` itself, so the address in `t` is dead
/// there and the dropped `mov` cannot be missed. A second load based on `t`
/// would read the *loaded* value rather than the address, so the run stops at
/// the first load that writes `t`.
fn fold_load_copies(body: &str) -> String {
    const SCRATCH: &[&str] = &["%rax", "%rcx", "%rdx", "%r10", "%r11"];
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((src, t)) = mov_reg_reg(lines[i]) {
            // `%rcx`/`%rdx` carry implicit shift and divide operands, so a
            // dropped `mov` that fed one would change the instruction's
            // meaning.
            if SCRATCH.contains(&t) && !matches!(t, "%rcx" | "%rdx") {
                let mut j = i + 1;
                let mut loads: Vec<(String, String)> = Vec::new();
                while j < lines.len() {
                    let lt = lines[j].trim();
                    let op = lt.split_whitespace().next().unwrap_or("");
                    if lt.ends_with(':') || lt.starts_with('.') || op.starts_with('j')
                        || matches!(op, "ret" | "call" | "syscall")
                    {
                        break;
                    }
                    match load_pair(lines[j]) {
                        Some((slot, dst)) if base_reg(slot) == Some(t) => {
                            let stops = dst == t;
                            loads.push((replace_base(slot, t, src), dst.to_string()));
                            j += 1;
                            if stops {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                if !loads.is_empty() && loads.last().is_some_and(|(_, d)| d == t) {
                    for (slot, dst) in &loads {
                        out.push_str(&format!("  mov {slot}, {dst}\n"));
                    }
                    i = j;
                    continue;
                }
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// Drop the copy back in a `mov %x, %y` ... `mov %y, %x` round trip.
///
/// After the first copy `%y` already holds `%x`, so the copy back leaves `%x`
/// exactly as it was. `heap`'s loops emit the pair around every allocation
/// (`mov %rax, %r13; ...; mov %r13, %rax`) with a basic-block label between the
/// two, because the block split falls between them.
///
/// Only labels that no branch targets may sit between the pair: control could
/// otherwise enter with a different value in `%x`, and the copy back would then
/// be doing real work. Unlike a general copy propagation this needs no
/// liveness reasoning and is safe for every register, including the
/// implicit-operand ones.
fn drop_round_trips(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    // Only a label a branch actually names is a possible entry point; a label
    // definition (`foo:`) mentions its own name and must not count.
    let targets: std::collections::HashSet<&str> = lines
        .iter()
        .filter(|l| l.trim_start().starts_with('j'))
        .flat_map(|l| l.split_whitespace())
        .filter(|t| t.starts_with(".L"))
        .collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((a, b)) = mov_reg_reg(lines[i]) {
            if a != b {
                let mut k = i + 1;
                let mut blocked = false;
                while k < lines.len() {
                    let lt = lines[k].trim();
                    if lt.ends_with(':') {
                        if targets.contains(lt.trim_end_matches(':')) {
                            blocked = true;
                        }
                        k += 1;
                        continue;
                    }
                    break;
                }
                if !blocked {
                    if let Some((c, d)) = lines.get(k).and_then(|l| mov_reg_reg(l)) {
                        if a == d && b == c {
                            out.push_str(lines[i]);
                            out.push('\n');
                            for m in &lines[i + 1..k] {
                                out.push_str(m);
                                out.push('\n');
                            }
                            i = k + 1;
                            continue;
                        }
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
            //
            // Only the copy *out of* a scratch register qualifies. For a copy
            // *into* one (`mov %r14, %rcx; imul %rcx`) the region names the
            // scratch register, the value lives in the source, and deleting the
            // `mov` while renaming the source to the scratch register left
            // `imul %rcx` reading whatever `%rcx` held. That fired whenever the
            // source had no other mention in the body, as a hoisted constant
            // loaded in the prologue does: `x % 1000003` in a loop returned
            // garbage.
            let (keep, dead) = if SCRATCH.contains(&a) && !SCRATCH.contains(&t) {
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
    // A `call` reads the integer argument registers without naming them, so a
    // value parked in `%rsi`/`%rdi`/`%r8`/`%r9` is live at the call even though
    // no later line mentions the register. Treating it as dead would delete the
    // `mov` that set up an argument register. The other caller-saved registers
    // (`%rax`, `%rcx`, `%rdx`, `%r10`, `%r11`) are only clobbered, never read,
    // by a call, so a value there really is dead and the rewrite stays.
    let arg_reg = matches!(reg, "%rsi" | "%rdi" | "%r8" | "%r9");
    for (idx, line) in lines.iter().enumerate() {
        if (idx >= start + 1 && idx < end) || idx == start {
            continue;
        }
        let op = line.trim().split_whitespace().next().unwrap_or("");
        if arg_reg && op == "call" {
            return false;
        }
        if !reg_mentions(line, reg) {
            continue;
        }
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

/// Fold `lea D(%rbp), %t; add $K, %t` into one `lea (D+K)(%rbp), %t`, and drop
/// an `add $0, %t` that is pure noise.
///
/// Walking a struct field-by-field materialises the base address of the local
/// and then adds each field offset to it, so a local with eight fields pays
/// eight extra instructions. The offset belongs in the `lea` displacement. Both
/// rewrites drop an `add`, which is the only instruction here that sets flags,
/// so they are skipped when a conditional jump reads those flags.
fn fold_local_offsets(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((disp, t)) = lea_local(lines[i]) {
            if let Some((k, arith_dst)) = addsub_imm_at(lines.get(i + 1).copied()) {
                if arith_dst == t && !flags_used_by(lines.get(i + 2).copied()) {
                    if k == 0 {
                        out.push_str(lines[i]);
                        out.push('\n');
                        i += 2;
                        continue;
                    }
                    if let Some(sum) = disp.checked_add(k) {
                        out.push_str(&format!("  lea {sum}(%rbp), {t}\n"));
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

/// `lea (Rbase,Ridx,S), Rdst` or `lea D(Rbase,Ridx,S), Rdst`, returning the
/// displacement, base, index, scale and destination.
fn lea_indexed(line: &str) -> Option<(i64, &str, &str, i64, &str)> {
    let t = line.trim();
    let rest = t.strip_prefix("lea ")?;
    // Split at the comma *after* the closing paren: the memory operand itself
    // contains commas (`(%rax,%rcx,8)`), so the first comma is not the separator.
    let open = rest.find('(')?;
    let close = rest.find(')')?;
    let mem = rest[..close + 1].trim();
    let dst = rest[close + 1..].trim().strip_prefix(',')?.trim();
    let disp: i64 = if open == 0 { 0 } else { mem[..open].trim().parse().ok()? };
    let parts: Vec<&str> = mem[open + 1..close].split(',').collect();
    let (base, idx, scale) = match parts.as_slice() {
        [b, i] => (b.trim(), i.trim(), 1),
        [b, i, s] => (b.trim(), i.trim(), s.trim().parse().ok()?),
        _ => return None,
    };
    if !is_reg(base) || !is_reg(idx) || !is_reg(dst) {
        return None;
    }
    Some((disp, base, idx, scale, dst))
}

/// Whether the next mention of `%rcx` is a pure write, so dropping a write to
/// it cannot leave a stale value behind.
fn rcx_next_is_write(lines: &[&str], from: usize) -> bool {
    for line in lines.iter().skip(from) {
        let t = line.trim();
        if !reg_mentions(t, "%rcx") {
            continue;
        }
        if let Some((src, dst)) = split_ops(t) {
            let op = t.split_whitespace().next().unwrap_or("");
            if dst.trim() == "%rcx" && !src.contains("%rcx") && matches!(op, "mov" | "movq" | "lea") {
                return true;
            }
        }
        return false;
    }
    true
}

/// Fuse `lea D(%B), %R1; mov %I, %rcx; lea (%R1,%rcx,S), %R1` into
/// `lea D(%B,%I,S), %R1`, dropping the copy of the index into the scratch
/// register. The generator materialises an array base and its index into
/// separate registers before combining them; on x86-64 the base, the scaled
/// index and the displacement all fit one addressing mode, which is a load or
/// store fewer per element access.
fn fold_lea_terms(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        if i + 2 < lines.len() {
            if let Some((disp, bbase, r1)) = lea_disp(lines[i]) {
                if let Some((idx, t)) = mov_reg_reg(lines[i + 1]) {
                    if let Some((0, cbase, cidx, scale, cdst)) = lea_indexed(lines[i + 2]) {
                        if cbase == r1
                            && cidx == t
                            && cdst == r1
                            && t == "%rcx"
                            && rcx_next_is_write(&lines, i + 3)
                        {
                            out.push_str(&format!("  lea {disp}({bbase}, {idx}, {scale}), {r1}\n"));
                            i += 3;
                            continue;
                        }
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

/// `lea <disp>(%rbp), <reg>`, returning the displacement and the register.
fn lea_local(line: &str) -> Option<(i64, &str)> {
    let t = line.trim();
    let rest = t.strip_prefix("lea ")?;
    let (mem, dst) = split_ops(rest)?;
    let (mem, dst) = (mem.trim(), dst.trim());
    let disp = mem.strip_suffix("(%rbp)")?;
    let disp: i64 = disp.parse().ok()?;
    is_reg(dst).then_some((disp, dst))
}

/// `add|sub $k, <reg>` on any line (not just the head of a pair).
fn addsub_imm_at(line: Option<&str>) -> Option<(i64, &str)> {
    let (op, k, dst) = addsub_imm(line?)?;
    let k = if op == "sub" { -k } else { k };
    Some((k, dst))
}

/// Whether an instruction reads the flags a preceding `add` would set.
/// Whether the flags may be read at or after `lines[from]` before something
/// sets them again. A label or a jump ends the scan as "may be read" (code
/// reached there is not visible here); a `call`/`ret` as "not read" (the ABI
/// does not preserve flags).
fn flags_live_from(lines: &[&str], from: usize) -> bool {
    for line in &lines[from.min(lines.len())..] {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.ends_with(':') || t.starts_with('.') {
            return true;
        }
        if flags_used_by(Some(t)) {
            return true;
        }
        let op = t.split_whitespace().next().unwrap_or("");
        if matches!(op, "call" | "ret" | "syscall") {
            return false;
        }
        if matches!(
            op,
            "add" | "addq" | "sub" | "subq" | "and" | "andq" | "or" | "orq" | "xor" | "xorq"
                | "cmp" | "cmpq" | "test" | "testq" | "neg" | "imul" | "sal" | "shl" | "sar"
                | "shr"
        ) {
            return false;
        }
    }
    false
}

fn flags_used_by(line: Option<&str>) -> bool {
    let Some(line) = line else { return false };
    let t = line.trim();
    let op = t.split_whitespace().next().unwrap_or("");
    // `jmp` reads no flags, but treating it as a reader only blocks a fold.
    op.starts_with('j')
        || op.starts_with("set")
        || op.starts_with("cmov")
        || matches!(op, "adc" | "sbb" | "pushf" | "pushfq" | "lahf")
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
    let dst = split_ops(line.trim()).map(|(_, d)| d.trim());
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
                    let flags_read = flags_used_by(lines.get(i + 2).copied());
                    if op == "imul" && d == t && a != t && !flags_read {
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
                    //
                    // Only `add` qualifies. `mov $k, %t; sub %s, %t` computes
                    // `k - s`, which no displacement expresses: folding it to
                    // `lea -k(%s)` computed `s - k`, so `5 - x` returned `x - 5`
                    // (and `0 - x`, i.e. `wrapping_neg`, returned `x`).
                    if d == t && a != t && op == "add" && !flags_read {
                        let disp = k;
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
    let (imm, dst) = split_ops(rest)?;
    let k: i64 = imm.trim().strip_prefix('$')?.parse().ok()?;
    let dst = dst.trim();
    is_reg(dst).then_some((k, dst))
}

/// `<op> <a>, <d>` with two register operands.
fn binop_reg(line: &str) -> Option<(&str, &str, &str)> {
    let t = line.trim();
    let (op, rest) = t.split_once(' ')?;
    let (a, d) = split_ops(rest)?;
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
        // `mov %a, %d; add|sub $k, %d` -> `lea ±k(%a), %d` when `%d` is a
        // promoted register: a temporary that lives in a register of its own
        // (`n - 1` passed as an argument) is computed in place, so the
        // scratch-register pattern below never sees it. `%d` ends up with the
        // same value either way; only the flags differ, so they must be dead.
        // (`mov %a, %d; sub $k, %d` is `a - k`, which a displacement does
        // express; it is `mov $k, %d; sub %a, %d`, `k - a`, that never folds.)
        if let Some((a, d)) = mov_reg_reg(lines[i]) {
            if a != d && !SCRATCH.contains(&d) {
                if let Some((op, k, dst)) = addsub_imm(lines[i + 1]) {
                    if dst == d && k.unsigned_abs() < (1 << 31) && !flags_live_from(&lines, i + 2) {
                        let disp = if op == "add" { k } else { -k };
                        out.push_str(&format!("  lea {disp}({a}), {d}\n"));
                        i += 2;
                        continue;
                    }
                }
            }
        }
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
                    if arith_dst == t
                        && a != t
                        && k.unsigned_abs() < (1 << 31)
                        && !flags_used_by(lines.get(i + 2).copied())
                    {
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
        match split_ops(rest) {
            Some((src, _dst)) => reg_mentions(src, r),
            None => reg_mentions(rest, r),
        }
    } else {
        reg_mentions(rest, r)
    }
}

/// `%rax *= k` as a shift and a `lea` when that is quicker than `imul`, whose
/// three cycles sit on the dependency chain of an accumulation like
/// `cur = cur * 10 + d` (`textparse`): `x * 10` is `lea (x,x,4)` then
/// `sal $1`, two one-cycle steps. Neither replacement sets flags anything
/// reads: a plain `Mul` wraps, and a checked one is a `WithOverflow`, which
/// recomputes its flags with an `imul` of its own.
fn mul_by_const(b: &Operand) -> Option<Vec<String>> {
    let Operand::Const(Const::Int(k)) = *b else { return None };
    if k <= 0 {
        return None;
    }
    let pow2 = |r: i64| r & (r - 1) == 0;
    let shift = |r: i64| (r > 1).then(|| format!("  sal ${}, %rax", r.trailing_zeros()));
    if pow2(k) {
        return Some(shift(k).into_iter().collect());
    }
    for m in [9i64, 5, 3] {
        if k % m == 0 && pow2(k / m) {
            let mut v = vec![format!("  lea (%rax,%rax,{}), %rax", m - 1)];
            v.extend(shift(k / m));
            return Some(v);
        }
    }
    None
}

/// Registers the code generator uses as scratch while evaluating an operand.
/// A value in any other register is a promoted local's home, which operand
/// evaluation never writes.
const SCRATCH_REGS: [&str; 5] = ["%rax", "%rcx", "%rdx", "%r10", "%r11"];

/// The `mov`s that perform `moves` (source operand, destination register) as
/// one parallel assignment. Each move is emitted once no other pending move
/// still reads its destination (a source register, or the base or index of a
/// memory source); a cycle (`%rdi` and `%rsi` swapped) parks one source in
/// `%r11`, which is scratch and never an argument register. Done one by one in
/// argument order, a source that was an earlier argument's destination was
/// overwritten first, so every register-held argument went through a frame
/// slot instead (`nqueens` stored and reloaded one per recursive call).
fn parallel_moves(moves: Vec<(String, &str)>) -> Option<Vec<String>> {
    let mut pending: Vec<(String, &str)> = moves.into_iter().filter(|(v, d)| v != d).collect();
    let mut out = Vec::new();
    while !pending.is_empty() {
        let blocked = |k: usize, pending: &[(String, &str)]| {
            pending.iter().enumerate().any(|(j, (v, _))| j != k && reg_mentions(v, pending[k].1))
        };
        if let Some(k) = (0..pending.len()).find(|&k| !blocked(k, &pending)) {
            let (v, d) = pending.remove(k);
            out.push(format!("  mov {v}, {d}"));
            continue;
        }
        // Every destination is still read by another move: a cycle. Park a
        // source that reads a pending destination in a scratch register no
        // pending move reads, which unblocks that destination's move.
        let reads_dest = |v: &str| pending.iter().any(|(_, d)| reg_mentions(v, d));
        let k = pending.iter().position(|(v, _)| reads_dest(v))?;
        let park = ["%r11", "%r10"]
            .into_iter()
            .find(|r| !pending.iter().any(|(v, _)| reg_mentions(v, r)))?;
        let parked = pending[k].0.clone();
        out.push(format!("  mov {parked}, {park}"));
        for (v, _) in pending.iter_mut() {
            if *v == parked {
                *v = park.to_string();
            }
        }
    }
    Some(out)
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
            if let Some((src, dst)) = split_ops(rest) {
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
    let (a, b) = split_ops(rest)?;
    let (a, b) = (a.trim(), b.trim());
    (is_reg(a) && is_reg(b)).then_some((a, b))
}

/// `lea <disp>(%base), %dst`, returning the displacement, base and destination.
fn lea_disp(line: &str) -> Option<(i64, &str, &str)> {
    let t = line.trim();
    let rest = t.strip_prefix("lea ")?;
    let (mem, dst) = split_ops(rest)?;
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
    let (imm, dst) = split_ops(rest)?;
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
        let pair = split_ops(rest).map(|(a, b)| (a.trim(), b.trim()));
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
/// Split an AT&T operand list into everything before the last operand and the
/// last operand, at the last comma *outside parentheses*.
///
/// A scaled-index address carries commas of its own: `lea 0(,%r15,4), %rax`
/// split at the first comma gave the destination `%r15,4), %rax`, which is no
/// register, so `written_regs` missed the write to `%rax` and `forward_stores`
/// went on believing `%rax` held a value it had just overwritten (`a * (4*b)`
/// became `(4*b) * (4*b)`). The same address can be a destination
/// (`mov %rax, (%rcx,%rdx,8)`), where splitting at the *last* comma fails too.
fn split_ops(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut cut = None;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => cut = Some(i),
            _ => {}
        }
    }
    cut.map(|i| (&s[..i], &s[i + 1..]))
}

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
    if let Some((_, dst)) = split_ops(line) {
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
        match split_ops(rest) {
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
    let (_, dst) = split_ops(t)?;
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
    // `RMC_TRACE_PEEPHOLE=1` prints every pass that changed the body to stderr,
    // so a miscompile can be bisected to the rewrite that introduced it.
    let trace = std::env::var_os("RMC_TRACE_PEEPHOLE").is_some();
    // `RMC_SKIP_PEEPHOLE=a,b` skips the named passes, to bisect a miscompile.
    let skip = std::env::var("RMC_SKIP_PEEPHOLE").unwrap_or_default();
    // `RMC_TIME=1` also reports any peephole pass that takes over a millisecond
    // on one function, with the function's size, to find a quadratic pass.
    let timing = std::env::var_os("RMC_TIME").is_some();
    let step = |name: &str, f: fn(&str) -> String, body: String| -> String {
        if skip.split(',').any(|s| s.trim() == name) {
            return body;
        }
        let t = std::time::Instant::now();
        let out = f(&body);
        if timing && t.elapsed().as_secs_f64() > 1e-3 {
            eprintln!(
                "{:>24} {:8.1} ms  ({} lines)",
                format!("peephole:{name}"),
                t.elapsed().as_secs_f64() * 1000.0,
                body.lines().count()
            );
        }
        if trace && out != body {
            eprintln!("--- peephole: {name}\n{out}");
        }
        out
    };
    if trace {
        eprintln!("--- peephole: input\n{body}");
    }
    let mut body = step("peephole", peephole, body.to_string());
    body = step("drop_dead_jumps", drop_dead_jumps, body);
    body = step("drop_noop_arith", drop_noop_arith, body);
    for _ in 0..2 {
        body = step("fuse_imm", fuse_imm, body);
        body = step("fuse_lea", fuse_lea, body);
        body = step("coalesce_scratch", coalesce_scratch, body);
        body = step("drop_round_trips", drop_round_trips, body);
        body = step("propagate_copies", propagate_copies, body);
        body = step("fold_load_copies", fold_load_copies, body);
        body = step("forward_stores", forward_stores, body);
        body = step("fold_addresses", fold_addresses, body);
        body = step("fold_lea_terms", fold_lea_terms, body);
        body = step("fold_local_offsets", fold_local_offsets, body);
        body = step("dead_stores", dead_stores, body);
        body = step("drop_unread_stores", drop_unread_stores, body);
        body = step("drop_noop_arith", drop_noop_arith, body);
    }
    let body = step("interleave_div_pairs", interleave_div_pairs, body);
    step("suffix_imm_mem", suffix_imm_mem, body)
}

/// The last step, on a whole function (prologue, body, epilogue): the
/// register-level clean-up in `asmopt`.
///
/// It runs after every text peephole and never before one. It folds an
/// address into the access that uses it, so a frame array is then read as
/// `-48(%rbp,%rsi,8)` and written as `-48(%rbp)`, and the slot peepholes --
/// which take an `N(%rbp)` operand to be a scalar slot nothing else can
/// reach -- would forward a stale value past the indexed store, or drop the
/// element stores as unread.
fn finish_function(text: &str) -> String {
    let skip = std::env::var("RMC_SKIP_PEEPHOLE").unwrap_or_default();
    if skip.split(',').any(|s| s.trim() == "asmopt") {
        return text.to_string();
    }
    let t = std::time::Instant::now();
    let (out, gave_up) = super::asmopt::optimize_or_explain(text);
    if let (Some(line), true) = (&gave_up, std::env::var_os("RMC_TRACE_PEEPHOLE").is_some()) {
        eprintln!("--- peephole: asmopt skipped a function, not modelled: {line}");
    }
    if std::env::var_os("RMC_TIME").is_some() {
        eprintln!(
            "{:>24} {:8.1} ms  ({} lines)",
            "peephole:asmopt",
            t.elapsed().as_secs_f64() * 1000.0,
            text.lines().count()
        );
    }
    if std::env::var_os("RMC_TRACE_PEEPHOLE").is_some() && out != text {
        eprintln!("--- peephole: asmopt\n{out}");
    }
    suffix_imm_mem(&out)
}

/// Give every immediate-to-memory ALU instruction an explicit 64-bit suffix.
///
/// `cmp $32, -472(%rbp)` names no register, so GNU `as` cannot infer a width
/// and picks 32 bits (`cmpl`), while the in-process encoder encodes it with
/// REX.W. The two link paths then disagreed: under `--binutils` a bounds check
/// compared only the low half of a 64-bit index. The peepholes create these
/// forms (folding a register load into the instruction), so the suffix is added
/// once, after every rewrite has run, rather than taught to each pass.
/// `mov $imm, mem` is left alone: its unsuffixed width is 32 bits in both
/// assemblers, and the generator sizes those stores itself.
fn suffix_imm_mem(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 64);
    for line in body.lines() {
        let t = line.trim_start();
        let indent = &line[..line.len() - t.len()];
        let rewritten = t.split_once(' ').and_then(|(op, rest)| {
            let alu = matches!(op, "add" | "sub" | "and" | "or" | "xor" | "cmp");
            let (src, dst) = split_ops(rest)?;
            (alu && src.trim().starts_with('$') && dst.trim().ends_with(')'))
                .then(|| format!("{indent}{op}q {rest}"))
        });
        match rewritten {
            Some(l) => out.push_str(&l),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Interleave two adjacent `mul`/`div`-by-constant units so the two long
/// `imul` latencies overlap. `mandel`'s inner loop computes `zr*zr/1000` and
/// `zi*zi/1000`, two independent units the emitter writes back to back. Each
/// unit is short, so the CPU cannot hide the second `imul` behind the first
/// unit's tail and the pair runs serially; issuing the second multiply above
/// the first division lets it run while the first chain is still in flight,
/// which the hand-built interleave measured at LLVM parity.
///
/// A unit is `mov src, %rax; imul src, %rax; mov %rax, %r10; <divide by a
/// constant>`: the product in `%rax`, saved to `%r10` as the division's
/// dividend, then the magic-multiply divide. Two adjacent units are independent
/// when they name the same constant (the same shift and magic multiplier) and
/// take their products from different registers. The transform parks the two
/// products in `%r10`/`%r11` -- scratch in this path (see `gen_div_const`) and
/// never promoted -- and runs the two divides back to back:
///
///   mov A, %rax; imul A, %rax; mov %rax, %r10; <div>   ->  mov A, %rax; imul A, %rax
///   mov B, %rax; imul B, %rax; mov %rax, %r10; <div>       mov %rax, %r10
///                                                          mov B, %rax; imul B, %rax
///                                                          mov %rax, %r11
///                                                          mov %r10, %rax; <div A>
///                                                          mov %r11, %rax; <div B>
fn interleave_div_pairs(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < lines.len() {
        if let Some((a, b)) = match_div_pair(&lines, i) {
            emit_unit_pair(&mut out, &lines, &a, &b);
            i = b.end;
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// One `mul`-then-`div`-by-constant unit: the register the product is taken
/// from, the divide's magic multiplier and shift, and its span in the body.
struct DivUnit {
    src: String,
    /// The register the divide's quotient is written to. If this is the *next*
    /// unit's `src`, the two units are dependent: the second multiply must read
    /// this quotient, but the reorder reads `src` before this store runs, so the
    /// pair must not be interleaved.
    dst: String,
    magic: String,
    shift: String,
    start: usize,
    end: usize,
}

/// The divide half of a unit: `mov %rax, %r10` (the dividend save), a magic
/// constant into `%rcx`, an `imul %rcx`, the sign-correction tail, and the
/// quotient store out of `%rdx`. The tail only reads `%r10`/`%r11`, never
/// writes them, so the caller may hold the other unit's dividend there.
fn read_div_block(lines: &[&str], start: usize) -> Option<(usize, String, String, String)> {
    if lines.get(start)?.trim() != "mov %rax, %r10" {
        return None;
    }
    let limit = (start + 20).min(lines.len());
    let (mut magic, mut shift, mut saw_imul) = (None, None, false);
    let mut j = start + 1;
    while j < limit {
        let t = lines[j].trim();
        if t.is_empty() || t.ends_with(':') || t.starts_with('.') {
            return None;
        }
        if t.starts_with("j") || t.starts_with("call") {
            return None;
        }
        // The tail must not touch the two scratch registers the caller parks
        // the products in. The `add`-biased divide reads `%r10` for its sign
        // correction, so it is rejected here; only the plain magic divide,
        // whose correction comes from `%rdx`, is safe to interleave.
        if t.contains("%r10") || t.contains("%r11") {
            return None;
        }
        if let Some(rest) = t.strip_prefix("movabs $") {
            let (m, reg) = rest.split_once(", ")?;
            if reg != "%rcx" || magic.is_some() {
                return None;
            }
            magic = Some(m.to_string());
        } else if t == "imul %rcx" {
            saw_imul = true;
        } else if let Some(rest) = t.strip_prefix("sar $") {
            if let Some((s, reg)) = rest.split_once(", ") {
                if reg == "%rdx" && shift.is_none() {
                    shift = Some(s.to_string());
                }
            }
        } else if let Some(dst) = t.strip_prefix("mov %rdx, ") {
            // The chain has two `mov %rdx, ..`: the sign correction writes the
            // scratch `%rcx`, and the final quotient store writes a real
            // register. Skip the former and end at the latter.
            if matches!(dst, "%rax" | "%rcx" | "%rdx" | "%r10" | "%r11") {
                j += 1;
                continue;
            }
            if !is_reg(dst) {
                return None;
            }
            if !saw_imul {
                return None;
            }
            return Some((
                j - start + 1,
                dst.to_string(),
                magic?,
                shift.unwrap_or_else(|| "0".to_string()),
            ));
        }
        j += 1;
    }
    None
}

/// Read a full unit at `start`: `mov src, %rax; imul src, %rax`, then the divide
/// block. `src` must be a plain register that is not one of the two scratch
/// registers the transform parks products in.
fn read_div_unit(lines: &[&str], start: usize) -> Option<DivUnit> {
    let load = lines.get(start)?.trim();
    let (src, dst) = load.strip_prefix("mov ")?.split_once(", ")?;
    if dst != "%rax" || !is_reg(src) || matches!(src, "%r10" | "%r11") {
        return None;
    }
    if lines.get(start + 1)?.trim() != format!("imul {src}, %rax") {
        return None;
    }
    let (len, dst, magic, shift) = read_div_block(lines, start + 2)?;
    Some(DivUnit { src: src.to_string(), dst, magic, shift, start, end: start + 2 + len })
}

/// Match two adjacent units that divide by the same constant from different
/// registers, so they are the two independent halves of one expression pair.
/// The pair must be genuinely independent: if the first divide writes the
/// register the second multiplies, the second product depends on the first
/// quotient, and reordering would square the register's *old* value.
fn match_div_pair(lines: &[&str], start: usize) -> Option<(DivUnit, DivUnit)> {
    let a = read_div_unit(lines, start)?;
    let b = read_div_unit(lines, a.end)?;
    if a.magic != b.magic || a.shift != b.shift || a.src == b.src || a.dst == b.src {
        return None;
    }
    Some((a, b))
}

/// Emit the two units with the second multiply hoisted above the first divide.
fn emit_unit_pair(out: &mut String, lines: &[&str], a: &DivUnit, b: &DivUnit) {
    let line = |out: &mut String, s: &str| {
        out.push_str(s);
        out.push('\n');
    };
    line(out, lines[a.start]);
    line(out, lines[a.start + 1]);
    line(out, "  mov %rax, %r10");
    line(out, lines[b.start]);
    line(out, lines[b.start + 1]);
    line(out, "  mov %rax, %r11");
    line(out, "  mov %r10, %rax");
    for l in &lines[a.start + 3..a.end] {
        line(out, l);
    }
    line(out, "  mov %r11, %rax");
    for l in &lines[b.start + 3..b.end] {
        line(out, l);
    }
}

/// Drop an arithmetic instruction that cannot change its destination: `add $0`
/// and `sub $0` leave a register untouched. A field access at offset 0 emits
/// exactly `add $0, %rax`, and the dead instruction then blocks the
/// store-forwarding peephole that would otherwise fold the pointer's round-trip
/// through its frame slot. Kept only when the next instruction does not read
/// the flags the instruction would have set.
fn drop_noop_arith(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::with_capacity(body.len());
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        let noop = t
            .strip_prefix("add $0, ")
            .or_else(|| t.strip_prefix("sub $0, "))
            .map(is_reg)
            .unwrap_or(false);
        if noop && !flags_used_by(lines.get(i + 1).copied()) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
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
    let (src, dst) = split_ops(rest)?;
    let (src, dst) = (src.trim(), dst.trim());
    if !src.starts_with('%') || src.contains('(') || !dst.contains('(') {
        return None;
    }
    Some((src, dst))
}

/// `mov <slot>, <reg>` with a memory source, returning `(slot, reg)`.
fn load_pair(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("mov ")?;
    let (src, dst) = split_ops(rest)?;
    let (src, dst) = (src.trim(), dst.trim());
    if !src.contains('(') || !dst.starts_with('%') {
        return None;
    }
    Some((src, dst))
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

/// The length word a slice's data sits behind. A `&[T]`/`*const [T]` (and the
/// `[T]` its deref yields) is the address of a `{len: i64, data..}` record, so
/// the first element is one word in. A fixed-size array (`[T; N]`, N > 0) has
/// no such word.
fn slice_header(ty: &Ty) -> i64 {
    match ty {
        Ty::Array(_, 0) => 8,
        Ty::Ref(t) | Ty::Ptr(t) => match &**t {
            Ty::Array(_, 0) => 8,
            _ => 0,
        },
        _ => 0,
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

/// The 32-bit name of a 64-bit register (a `movzbl` destination).
fn dword_reg(reg: &str) -> String {
    match reg {
        "%rax" => "%eax".into(),
        "%rbx" => "%ebx".into(),
        "%rcx" => "%ecx".into(),
        "%rdx" => "%edx".into(),
        "%rsi" => "%esi".into(),
        "%rdi" => "%edi".into(),
        "%rbp" => "%ebp".into(),
        "%rsp" => "%esp".into(),
        r => format!("{r}d"),
    }
}

/// The 8-bit register holding the low byte of a 64-bit register name.
fn byte_reg(reg: &str) -> &str {
    match reg {
        "%rax" => "%al",
        "%rbx" => "%bl",
        "%rcx" => "%cl",
        "%rdx" => "%dl",
        "%rsi" => "%sil",
        "%rdi" => "%dil",
        "%r8" => "%r8b",
        "%r9" => "%r9b",
        "%r10" => "%r10b",
        "%r11" => "%r11b",
        "%r12" => "%r12b",
        "%r13" => "%r13b",
        "%r14" => "%r14b",
        "%r15" => "%r15b",
        other => other,
    }
}

fn is_compare(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// The loop-nesting depth of each block of a control-flow graph given as
/// successor lists, block 0 the entry (see `FnGen::loop_depths`).
fn loop_depths_of(succ: &[Vec<u32>]) -> Vec<u32> {
    let n = succ.len();
    // A back edge is an edge to a block still on the depth-first stack, so
    // it closes a cycle through that block. (Index order is not enough: an
    // out-of-line `then` block jumping back to its join looks backwards
    // too. Dominance is not enough either: `stats`'s loop is entered at
    // two places.) Each header is one loop however many back edges reach
    // it. Counting every edge on a cycle as a loop, as this once did, gave
    // a block a depth of roughly the number of blocks in its loop body,
    // and with weights of 8^depth every loop of five blocks or more looked
    // equally hot, so an outer loop's values outranked an inner loop's.
    let mut latches: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut state = vec![0u8; n]; // 0 unseen, 1 on the stack, 2 done
    let mut stack: Vec<(usize, usize)> = Vec::new();
    if n > 0 {
        state[0] = 1;
        stack.push((0, 0));
    }
    while let Some(&mut (u, ref mut next)) = stack.last_mut() {
        if *next < succ[u].len() {
            let v = succ[u][*next] as usize;
            *next += 1;
            match state[v] {
                0 => {
                    state[v] = 1;
                    stack.push((v, 0));
                }
                1 => latches[v].push(u),
                _ => {}
            }
        } else {
            state[u] = 2;
            stack.pop();
        }
    }
    // The loop of header `h` is `h` plus every block that reaches one of
    // its latches without passing through `h` (walking predecessors back
    // from the latches and stopping at `h`). "Reachable from `h` and
    // reaching the latch" is not the same thing in a nest: from an inner
    // header one can leave the inner loop, go round the outer one and come
    // back, so that test put the whole outer loop inside the inner one.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, ss) in succ.iter().enumerate() {
        for &v in ss {
            preds[v as usize].push(u);
        }
    }
    let mut depths = vec![0u32; n];
    let mut mark = vec![usize::MAX; n];
    for h in 0..n {
        if latches[h].is_empty() {
            continue;
        }
        mark[h] = h;
        depths[h] += 1;
        let mut work: Vec<usize> = Vec::new();
        for &u in &latches[h] {
            if mark[u] != h {
                mark[u] = h;
                depths[u] += 1;
                work.push(u);
            }
        }
        while let Some(x) = work.pop() {
            for &p in &preds[x] {
                if mark[p] != h {
                    mark[p] = h;
                    depths[p] += 1;
                    work.push(p);
                }
            }
        }
    }
    depths
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
        Operand::Const(_) | Operand::Fmt { .. } | Operand::Fn(_) => false,
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

/// The conditional jump taken when `op` holds after `cmp b, a`. An unsigned
/// (or pointer) comparison reads the carry flag (`jb`/`jbe`/`ja`/`jae`), a
/// signed one the sign and overflow flags: `jl` on a `usize` index let
/// `-1 as usize` pass a bounds check.
fn jcc(op: BinOp, unsigned: bool) -> &'static str {
    match (op, unsigned) {
        (BinOp::Eq, _) => "je",
        (BinOp::Ne, _) => "jne",
        (BinOp::Lt, false) => "jl",
        (BinOp::Le, false) => "jle",
        (BinOp::Gt, false) => "jg",
        (BinOp::Ge, false) => "jge",
        (BinOp::Lt, true) => "jb",
        (BinOp::Le, true) => "jbe",
        (BinOp::Gt, true) => "ja",
        (BinOp::Ge, true) => "jae",
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

/// Every local an rvalue reads. Used by the liveness analysis, which needs a
/// plain set rather than the counted maps `scan_rv` builds.
fn scan_rv_uses(rv: &Rvalue, out: &mut HashSet<u32>) {
    fn add_place(p: &Place, out: &mut HashSet<u32>) {
        out.insert(p.local);
        for proj in &p.proj {
            if let Proj::IndexLocal(i) = proj {
                out.insert(*i);
            }
        }
    }
    fn add_op(op: &Operand, out: &mut HashSet<u32>) {
        if let Operand::Copy(p) | Operand::Move(p) = op {
            add_place(p, out);
        }
    }
    match rv {
        Rvalue::Use(op) | Rvalue::Unary(_, op) => add_op(op, out),
        Rvalue::Repeat { op, .. } => add_op(op, out),
        Rvalue::Cast { op, .. } => add_op(op, out),
        Rvalue::Binary(_, a, b) | Rvalue::WithOverflow { a, b, .. } => {
            add_op(a, out);
            add_op(b, out);
        }
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            for op in ops {
                add_op(op, out);
            }
        }
        Rvalue::Ref { place, .. }
        | Rvalue::RawRef { place, .. }
        | Rvalue::Len(place)
        | Rvalue::Discriminant(place) => add_place(place, out),
        Rvalue::Unsupported(_) => {}
    }
}

/// The locals a *written* place reads: the base, when the place projects into
/// it or through it (a partial write keeps the rest live; a `Deref` reads the
/// pointer), and every dynamic index local.
fn written_place_reads(p: &Place, out: &mut HashSet<u32>) {
    if p.proj.is_empty() {
        return;
    }
    out.insert(p.local);
    for proj in &p.proj {
        if let Proj::IndexLocal(i) = proj {
            out.insert(*i);
        }
    }
}

/// Reads a terminator makes that `scan_term_uses` does not report: `return`
/// reads `_0`, an indirect call reads its callee, a call into a projected
/// place reads that place's base and indices, and a drop reads its indices.
fn term_extra_reads(t: &Terminator, out: &mut HashSet<u32>) {
    match t {
        Terminator::Return => {
            out.insert(0);
        }
        Terminator::Call { dest, func, .. } => {
            if let Some(Operand::Copy(p) | Operand::Move(p)) = call_callee(func) {
                out.insert(p.local);
                for proj in &p.proj {
                    if let Proj::IndexLocal(i) = proj {
                        out.insert(*i);
                    }
                }
            }
            written_place_reads(dest, out);
        }
        Terminator::Drop { place, .. } => {
            for proj in &place.proj {
                if let Proj::IndexLocal(i) = proj {
                    out.insert(*i);
                }
            }
        }
        _ => {}
    }
}

/// Every local a terminator reads (not a call destination, which is a write).
fn scan_term_uses(t: &Terminator, out: &mut HashSet<u32>) {
    fn add_op(op: &Operand, out: &mut HashSet<u32>) {
        if let Operand::Copy(p) | Operand::Move(p) = op {
            out.insert(p.local);
            for proj in &p.proj {
                if let Proj::IndexLocal(i) = proj {
                    out.insert(*i);
                }
            }
        }
    }
    match t {
        Terminator::SwitchInt { discr, .. } => add_op(discr, out),
        Terminator::Assert { cond, .. } => add_op(cond, out),
        Terminator::Call { args, .. } => {
            for a in args {
                add_op(a, out);
            }
        }
        Terminator::Drop { place, .. } => {
            out.insert(place.local);
        }
        _ => {}
    }
}

fn scan_operands(s: &Stmt, f: &mut impl FnMut(&Operand)) {
    let Stmt::Assign(_, rv) = s else { return };
    match rv {
        Rvalue::Use(op)
        | Rvalue::Unary(_, op)
        | Rvalue::Repeat { op, .. } => f(op),
        Rvalue::Binary(_, a, b) | Rvalue::WithOverflow { a, b, .. } => {
            f(a);
            f(b);
        }
        Rvalue::Cast { op, .. } => f(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            for op in ops {
                f(op);
            }
        }
        // `&place`, `Len(place)`, `discriminant(place)` read a place, not an
        // operand; treat a non-deref read of a local as a use of that local.
        Rvalue::Ref { place, .. }
        | Rvalue::RawRef { place, .. }
        | Rvalue::Len(place)
        | Rvalue::Discriminant(place) => {
            if !matches!(place.proj.first(), Some(Proj::Deref)) {
                f(&Operand::Copy(place.clone()));
            }
        }
        Rvalue::Unsupported(_) => {}
    }
}

/// Whether a terminator reads `local`.
fn terminator_reads(t: &Terminator, local: u32) -> bool {
    let mut hit = false;
    let mut check = |op: &Operand| {
        if let Operand::Copy(p) | Operand::Move(p) = op {
            if p.local == local {
                hit = true;
            }
        }
    };
    match t {
        Terminator::SwitchInt { discr, .. } => check(discr),
        Terminator::Assert { cond, .. } => check(cond),
        Terminator::Call { args, .. } => {
            for a in args {
                check(a);
            }
        }
        Terminator::Drop { place, .. } => {
            if place.local == local {
                hit = true;
            }
        }
        _ => {}
    }
    hit
}


/// The NaN guard a float comparison needs after `ucomisd`/`ucomiss`.
enum Guard {
    /// The comparison is false when unordered: AND the result with `setnp`.
    Ordered,
    /// The comparison is true when unordered: OR the result with `setp`.
    Unordered,
    /// The flag test is already false when unordered.
    None,
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
    index_locals: &mut HashSet<u32>,
) {
    match rv {
        Rvalue::Use(op) => scan_operand(op, counts, projected, index_locals),
        Rvalue::Binary(_, a, b) => {
            scan_operand(a, counts, projected, index_locals);
            scan_operand(b, counts, projected, index_locals);
        }
        Rvalue::Unary(_, a) => scan_operand(a, counts, projected, index_locals),
        Rvalue::Cast { op, .. } => scan_operand(op, counts, projected, index_locals),
        Rvalue::Repeat { op, .. } => scan_operand(op, counts, projected, index_locals),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            for op in ops {
                scan_operand(op, counts, projected, index_locals);
            }
        }
        // `&x`, `Len(x)` and `discriminant(x)` need `x` in memory; through a
        // leading deref (`&*p`, `&(*p).f`) they only read the pointer `p`.
        Rvalue::Ref { place: p, .. }
        | Rvalue::RawRef { place: p, .. }
        | Rvalue::Len(p)
        | Rvalue::Discriminant(p) => {
            bump(p, counts, projected);
            if !matches!(p.proj.first(), Some(Proj::Deref)) {
                addr_taken.insert(p.local);
            }
        }
        Rvalue::WithOverflow { a, b, .. } => {
            scan_operand(a, counts, projected, index_locals);
            scan_operand(b, counts, projected, index_locals);
        }
        Rvalue::Unsupported(_) => {}
    }
}

fn scan_operand(
    op: &Operand,
    counts: &mut HashMap<u32, u32>,
    projected: &mut HashSet<u32>,
    index_locals: &mut HashSet<u32>,
) {
    if let Operand::Copy(p) | Operand::Move(p) = op {
        // An index local inside a place (`_1[_7]`) is read as often as the place
        // is, so count it toward promotion.
        for proj in &p.proj {
            if let Proj::IndexLocal(i) = proj {
                *counts.entry(*i).or_insert(0) += 1;
                index_locals.insert(*i);
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
    index_locals: &mut HashSet<u32>,
) {
    match term {
        Terminator::SwitchInt { discr, .. } => scan_operand(discr, counts, projected, index_locals),
        Terminator::Call { dest, args, .. } => {
            bump(dest, counts, projected);
            for a in args {
                scan_operand(a, counts, projected, index_locals);
            }
        }
        // A `drop(_N)` takes `_N`'s address, so the local must not be promoted
        // into a register it has no address for.
        Terminator::Drop { place, .. } => bump(place, counts, projected),
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


#[cfg(test)]
mod tests {
    use super::*;

    /// The assembly text of the first function named `name` in `asm`.
    fn func_asm(asm: &str, name: &str) -> String {
        let start = format!("\n{name}:\n");
        let Some(i) = asm.find(&start) else { return String::new() };
        let rest = &asm[i + start.len() - 1..];
        let mut out = String::new();
        for (k, line) in rest.lines().enumerate() {
            if k > 0 {
                // A new top-level label (a letter, not the leading `.` of a
                // local label) ends the function.
                if line.starts_with(|c: char| c.is_ascii_alphabetic()) && line.ends_with(':') {
                    break;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// A `bool` local reused by two loop tests must still fold in both blocks.
    ///
    /// rustc computes `_1 = Lt(_2, 2)` in the loop header *and* reuses the same
    /// `_1` for a second test. Folding each test into a `cmp`/`jcc` is only
    /// sound if the store of `_1` in the *other* block is not treated as making
    /// the local live: the other block recomputes `_1` before reading it. When
    /// the liveness test was too coarse the fold bailed, and the backend paid
    /// `setl`/`movzbl`/store plus a second `cmp` on every iteration, which is
    /// what put `fib` and `callrec` behind LLVM. This checks the `setl` is gone.
    /// A loop nested in another: the inner loop's blocks are at depth 2, the
    /// outer loop's own blocks at depth 1, and nothing else is in a loop. A
    /// test of "reachable from the header and reaching the latch" put the
    /// whole outer loop inside the inner one (one can leave the inner loop,
    /// go round the outer one and come back); counting every edge on a cycle
    /// as a loop gave depths of the order of the loop's size.
    #[test]
    fn loop_depths_follow_the_nest() {
        // bb0 -> bb1 (outer header) -> bb2 -> bb3 (inner header)
        // bb3 -> bb4 (inner latch, back to bb3) | bb5 (outer latch, back to bb1)
        // bb1 -> bb6 (exit)
        let succ = vec![vec![1], vec![2, 6], vec![3], vec![4, 5], vec![3], vec![1], vec![]];
        assert_eq!(loop_depths_of(&succ), vec![0, 1, 1, 2, 2, 1, 0]);
        // An out-of-line `then` block that jumps back to its join is not a
        // loop, though it jumps to a lower-numbered block.
        let succ = vec![vec![2, 1], vec![3], vec![3], vec![]];
        assert_eq!(loop_depths_of(&succ), vec![0, 0, 0, 0]);
        // Two latches into one header are one loop.
        let succ = vec![vec![1], vec![2, 3], vec![1], vec![1, 4], vec![]];
        assert_eq!(loop_depths_of(&succ), vec![0, 1, 1, 1, 0]);
    }

    /// Compile MIR text and return the named function's assembly. Only what
    /// `main` reaches is emitted, so `f` is called from a `main` that passes
    /// it a frame array and a runtime index.
    fn asm_of(src: &str, name: &str) -> String {
        let src = format!(
            "{src}
fn main() -> i64 {{
    let mut _0: i64;
    let mut _1: [i64; 8];
    let mut _2: &[i64; 8];
    let mut _3: i64;

    bb0: {{
        _1 = [const 1_i64; 8];
        _2 = &_1;
        _3 = arg_i64(const 1_i64) -> [return: bb1, unwind unreachable];
    }}

    bb1: {{
        _0 = f(copy _2, copy _3) -> [return: bb2, unwind unreachable];
    }}

    bb2: {{
        return;
    }}
}}
"
        );
        let mut prog = crate::mir::parse(&src).expect("parse");
        crate::mir::layout_blocks(&mut prog);
        let asm = compile(&prog, &Options::default()).expect("compile");
        let f = func_asm(&asm, name);
        assert!(!f.is_empty(), "{name} not emitted:\n{asm}");
        f.to_string()
    }

    /// `f(p: &[i64; 8], i: usize) -> i64 { p[i] }` with its bounds check.
    const INDEX_THROUGH_REF: &str = "\
fn f(_1: &[i64; 8], _2: usize) -> i64 {
    let mut _0: i64;
    let mut _3: bool;

    bb0: {
        _3 = Lt(copy _2, const 8_usize);
        assert(move _3, \"index out of bounds\", const 8_usize, copy _2) -> [success: bb1, unwind unreachable];
    }

    bb1: {
        _0 = copy (*_1)[_2];
        return;
    }
}
";

    /// A loop whose latch switches on an integer (`while x != 0`, which rustc
    /// prints as `switchInt(x) -> [0: exit, otherwise: body]`) with the exit
    /// next in layout: the latch is one `jne` back to the body, not `je exit;
    /// jmp body`.
    #[test]
    fn an_integer_loop_test_falls_into_its_exit() {
        let f = asm_of(
            "\
fn f(_1: &[i64; 8], _2: i64) -> i64 {
    let mut _0: i64;
    let mut _3: i64;

    bb0: {
        _0 = const 0_i64;
        _3 = copy _2;
        switchInt(copy _3) -> [0: bb2, otherwise: bb1];
    }

    bb1: {
        _3 = Shr(copy _3, const 1_i32);
        _0 = Add(copy _0, const 1_i64);
        switchInt(copy _3) -> [0: bb2, otherwise: bb1];
    }

    bb2: {
        return;
    }
}
",
            "f",
        );
        assert!(f.contains("jne .Lbbf_1"), "{f}");
        assert!(!f.contains("jmp .Lbbf_1"), "{f}");
    }

    /// Run `moves` on a register file and return the argument registers.
    fn run_moves(moves: &[(&str, &'static str)]) -> Vec<(String, i64)> {
        let mut regs: HashMap<String, i64> = HashMap::new();
        for (i, r) in ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9", "%rbx", "%r12", "%rax"].iter().enumerate() {
            regs.insert(r.to_string(), 100 + i as i64);
        }
        // Memory at `8(%reg)` reads as the register's value plus 1000.
        let read = |v: &str, regs: &HashMap<String, i64>| -> i64 {
            if let Some(k) = v.strip_prefix('$') {
                k.parse().unwrap()
            } else if let Some(r) = v.strip_prefix("8(").and_then(|r| r.strip_suffix(')')) {
                regs[r] + 1000
            } else {
                regs[v]
            }
        };
        let want: Vec<(String, i64)> = moves.iter().map(|(v, d)| (d.to_string(), read(v, &regs))).collect();
        let ms = parallel_moves(moves.iter().map(|(v, d)| (v.to_string(), *d)).collect()).expect("resolvable");
        for m in &ms {
            let (src, dst) = m.trim().strip_prefix("mov ").unwrap().rsplit_once(", ").unwrap();
            let v = read(src, &regs);
            regs.insert(dst.to_string(), v);
        }
        let got: Vec<(String, i64)> = want.iter().map(|(d, _)| (d.clone(), regs[d])).collect();
        assert_eq!(got, want, "{ms:?}");
        got
    }

    /// Argument moves are one parallel assignment: a swap, a rotation, a
    /// memory source based on a register another argument overwrites, and
    /// two independent cycles all land every value where it belongs.
    #[test]
    fn argument_moves_are_a_parallel_assignment() {
        run_moves(&[("%rsi", "%rdi"), ("%rdi", "%rsi")]);
        run_moves(&[("%rdx", "%rdi"), ("%rdi", "%rsi"), ("%rsi", "%rdx")]);
        run_moves(&[("%r12", "%rdi"), ("8(%rdi)", "%rsi"), ("$7", "%rdx"), ("%rdi", "%rcx")]);
        run_moves(&[("8(%rsi)", "%rdi"), ("%rdi", "%rsi")]);
        run_moves(&[("%rsi", "%rdi"), ("%rdi", "%rsi"), ("%r9", "%r8"), ("%r8", "%r9"), ("$3", "%rdx")]);
        run_moves(&[("%rbx", "%rdi"), ("%rbx", "%rsi"), ("%rax", "%rdx"), ("%rdi", "%rcx")]);
    }

    /// The index of a store in a loop (`a[i & 7] = x`) is a register like the
    /// index of a load. Weighted by reads only, a store's index had no weight:
    /// it was written to a frame slot and reloaded into `%rcx` for every
    /// store (`life`'s inner loop).
    #[test]
    fn a_store_index_in_a_loop_takes_a_register() {
        let f = asm_of(
            "\
fn f(_1: &[i64; 8], _2: i64) -> i64 {
    let mut _0: i64;
    let mut _3: [i64; 8];
    let mut _4: usize;
    let mut _5: i64;
    let mut _6: bool;
    let mut _7: usize;

    bb0: {
        _3 = [const 0_i64; 8];
        _5 = const 0_i64;
        goto -> bb1;
    }

    bb1: {
        _4 = copy _5 as usize (IntToInt);
        _7 = BitAnd(copy _4, const 7_usize);
        _3[_7] = copy _2;
        _5 = Add(copy _5, const 1_i64);
        _6 = Lt(copy _5, const 100_i64);
        switchInt(move _6) -> [0: bb2, otherwise: bb1];
    }

    bb2: {
        _0 = copy _3[_4];
        return;
    }
}
",
            "f",
        );
        let store = f
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("mov %") && l.contains("(%rbp,%") && l.ends_with(",8)"))
            .unwrap_or_else(|| panic!("no indexed store:\n{f}"));
        assert!(!store.contains("%rcx,8)"), "index reloaded into a scratch register:\n{f}");
        let reload = f.lines().map(str::trim).any(|l| l.starts_with("mov -") && l.ends_with("(%rbp), %rcx"));
        assert!(!reload, "index reloaded from the frame:\n{f}");
    }

    /// A shift by a constant is an immediate, not a count loaded into `%cl`
    /// (which also cost a frame slot to park the shifted value).
    #[test]
    fn a_constant_shift_count_is_an_immediate() {
        let f = asm_of(
            "\
fn f(_1: &[i64; 8], _2: i64) -> i64 {
    let mut _0: i64;

    bb0: {
        _0 = Shl(copy _2, const 13_i32);
        return;
    }
}
",
            "f",
        );
        assert!(f.contains("sal $13,"), "{f}");
        assert!(!f.contains("%cl"), "{f}");
    }

    /// An element read through a reference is one instruction with a scaled
    /// index, not `lea`/`mov`/`lea`/`mov` through `%rax` and `%rcx`.
    #[test]
    fn an_element_through_a_reference_is_one_addressing_mode() {
        let f = asm_of(INDEX_THROUGH_REF, "f");
        // (The pointer argument itself may sit in a frame slot: outside a
        // loop a callee-saved register costs as much as the slot.)
        let one = f.lines().any(|l| {
            let t = l.trim();
            t.starts_with("mov (%") && t.contains(",8), ")
        });
        assert!(one, "no scaled-index load:\n{f}");
        assert!(!f.contains("lea (%rax,%rcx,8)") && !f.contains("%rcx,8)"), "{f}");
    }

    /// An argument no call outlives stays in the register it arrived in: no
    /// move into a callee-saved register (and its push/pop), no frame slot.
    #[test]
    fn an_argument_stays_in_its_incoming_register() {
        let f = asm_of(
            "\
fn f(_1: &[i64; 8], _2: i64) -> i64 {
    let mut _0: i64;
    let mut _3: i64;

    bb0: {
        _3 = Add(copy _2, const 1_i64);
        _0 = Mul(copy _3, copy _2);
        return;
    }
}
",
            "f",
        );
        assert!(!f.contains("push %r") && !f.contains("push %rbx"), "{f}");
        assert!(!f.contains("mov %rsi, -") && !f.contains("mov %rsi, %r1"), "{f}");
        assert!(f.contains("%rsi"), "{f}");
    }

    /// `mov %a, %d; sub $k, %d` into a promoted register is `lea -k(%a), %d`
    /// when nothing reads the flags; a flag reader keeps the `sub`.
    #[test]
    fn a_register_minus_a_constant_into_a_promoted_register_is_a_lea() {
        let body = "  mov %rbx, %rdi\n  sub $1, %rdi\n  call f\n";
        assert_eq!(fuse_lea(body), "  lea -1(%rbx), %rdi\n  call f\n");
        let body = "  mov %rbx, %rdi\n  add $3, %rdi\n  mov %rdi, %rsi\n  cmp $0, %rsi\n";
        assert_eq!(fuse_lea(body), "  lea 3(%rbx), %rdi\n  mov %rdi, %rsi\n  cmp $0, %rsi\n");
        let body = "  mov %rbx, %rdi\n  sub $1, %rdi\n  jne .L1\n";
        assert_eq!(fuse_lea(body), body, "the jne reads the sub's flags");
        let body = "  mov %rbx, %rdi\n  sub $1, %rdi\n.L2:\n  jne .L1\n";
        assert_eq!(fuse_lea(body), body, "flags may be read past a label");
    }

    /// A bounds check that falls through on success branches to one shared,
    /// out-of-line panic exit on failure, and compares unsigned.
    #[test]
    fn a_bounds_check_branches_out_of_line_on_failure() {
        let f = asm_of(INDEX_THROUGH_REF, "f");
        assert!(f.contains("jae .Lrmc_f_trap"), "{f}");
        let check = f.find("jae .Lrmc_f_trap").unwrap();
        let trap = f.find(".Lrmc_f_trap:").expect("trap block emitted");
        assert!(trap > check, "{f}");
        // The success path reaches the load without passing an exit syscall.
        let load = f[check..].find("mov (%").map(|o| o + check).expect("load");
        assert!(!f[check..load].contains("syscall"), "{f}");
    }

    /// A copy *into* a scratch register is not coalesced: the value lives in
    /// the source, so deleting the copy leaves the scratch register stale.
    #[test]
    fn a_copy_into_a_scratch_register_is_kept() {
        let body = "  mov %rdi, %rax\n  mov %r14, %rcx\n  imul %rcx\n  mov %rdx, %rax\n";
        assert_eq!(coalesce_scratch(body), body);
    }

    /// A copy *out of* a scratch register still is: `%rax` holds the value
    /// already, so the region can read it there.
    #[test]
    fn a_copy_out_of_a_scratch_register_is_coalesced() {
        let body = "  mov %rax, %rbx\n  mov %rbx, %rdi\n  mov $0, %rax\n  call f\n";
        let out = coalesce_scratch(body);
        assert_eq!(out, "  mov %rax, %rdi\n  mov $0, %rax\n  call f\n");
    }

    #[test]
    fn reused_bool_local_still_folds() {
        let src = "\
fn main() -> i64 {
    let mut _0: i64;
    let mut _1: bool;
    let mut _2: i64;

    bb0: {
        _2 = const 0_i64;
        goto -> bb1;
    }

    bb1: {
        _1 = Lt(copy _2, const 2_i64);
        switchInt(move _1) -> [0: bb2, otherwise: bb4];
    }

    bb2: {
        _2 = Add(copy _2, const 1_i64);
        goto -> bb3;
    }

    bb3: {
        _1 = Lt(copy _2, const 2_i64);
        switchInt(move _1) -> [0: bb2, otherwise: bb4];
    }

    bb4: {
        _0 = copy _2;
        return;
    }
}
";
        let mut prog = crate::mir::parse(src).expect("parse");
        crate::mir::layout_blocks(&mut prog);
        let asm = compile(&prog, &Options::default()).expect("compile");
        let main = func_asm(&asm, "main");
        assert!(!main.is_empty(), "main not emitted:\n{asm}");
        assert!(
            !main.contains("setl"),
            "the reused bool was materialised instead of folded:\n{main}"
        );
        assert!(main.matches("cmp").count() >= 2, "both tests should branch:\n{main}");
    }

    /// The fold must not drop a `bool` store that a *later* block reads without
    /// recomputing it: that is the `reusedcheck` miscompile shape, caught here
    /// at the unit level too.
    ///
    /// `bb3` reads `_1` and never defines it, so the fold in `bb0` cannot serve
    /// that read and must leave the store behind. The materialisation shows up
    /// as a `setl` (the original test read `_1` nowhere else, so its `cmp`
    /// assertion held whether the store survived or not).
    #[test]
    fn external_bool_read_blocks_fold() {
        let src = "\
fn main() -> i64 {
    let mut _0: i64;
    let mut _1: bool;
    let mut _2: i64;

    bb0: {
        _1 = Lt(const 1_i64, const 2_i64);
        switchInt(move _1) -> [0: bb1, otherwise: bb2];
    }

    bb1: {
        _2 = const 0_i64;
        goto -> bb3;
    }

    bb2: {
        _2 = const 1_i64;
        goto -> bb3;
    }

    bb3: {
        switchInt(copy _1) -> [0: bb4, otherwise: bb5];
    }

    bb4: {
        _0 = copy _2;
        return;
    }

    bb5: {
        _0 = copy _2;
        return;
    }
}
";
        let mut prog = crate::mir::parse(src).expect("parse");
        crate::mir::layout_blocks(&mut prog);
        let asm = compile(&prog, &Options::default()).expect("compile");
        let main = func_asm(&asm, "main");
        assert!(!main.is_empty(), "main not emitted:\n{asm}");
        // `_1` is read in `bb3` with no definition there, so the `bb0` fold is
        // blocked and the `bool` is materialised (`setl`); folding it away would
        // leave `bb3` reading a stale or undefined value.
        assert!(
            main.contains("setl"),
            "a cross-block read must keep the bool store:\n{main}"
        );
    }

    /// A struct layout whose field list names its own struct must not send the
    /// layout walkers into unbounded recursion.
    ///
    /// `Ty::Struct` keeps only a type name's last path segment, so two distinct
    /// types (`A` and `x::A`) share one layout key. When that shared key's field
    /// list then names itself — as in the `HashMap`/`hashbrown::HashMap`
    /// collision the MIR fuzzer found — `size_of_ty`, `struct_layout` and the
    /// drop glue each walk the layout forever and overflow the stack.
    /// `collect_structs` now drops cyclic layouts; this checks the compile
    /// terminates instead of crashing.
    #[test]
    fn self_referential_layout_terminates() {
        let src = "\
fn main() -> () {
    let mut _0: ();
    let mut _1: A;
    let mut _2: A;
    bb0: {
        _2 = A { f: const 0_i64 };
        _1 = A { f: move _2 };
        drop(_1) -> [return: bb1, unwind unreachable];
    }
    bb1: { return; }
}
";
        let mut prog = crate::mir::parse(src).expect("parse");
        crate::mir::layout_blocks(&mut prog);
        let asm = compile(&prog, &Options::default()).expect("compile");
        assert!(asm.contains("main:"), "main not emitted:\n{asm}");
    }

    /// An immediate-to-memory ALU op names no register, so `as` would pick a
    /// 32-bit width; the in-process encoder uses 64. The suffix makes both
    /// agree. Register forms and `mov` are untouched.
    #[test]
    fn an_immediate_to_memory_alu_op_gets_a_q_suffix() {
        let body = "  cmp $32, -472(%rbp)\n  add $8, (%rax)\n  cmp $3, %rax\n  mov $0, -8(%rbp)\n";
        assert_eq!(
            suffix_imm_mem(body),
            "  cmpq $32, -472(%rbp)\n  addq $8, (%rax)\n  cmp $3, %rax\n  mov $0, -8(%rbp)\n"
        );
    }

    /// Operands split at the last comma outside parentheses, so a scaled-index
    /// address on either side stays whole.
    #[test]
    fn operand_split_respects_scaled_index_addresses() {
        assert_eq!(split_ops("0(,%r15,4), %rax"), Some(("0(,%r15,4)", " %rax")));
        assert_eq!(split_ops("%rax, (%rcx,%rdx,8)"), Some(("%rax", " (%rcx,%rdx,8)")));
        assert_eq!(split_ops("$3, %rdx, %rdx"), Some(("$3, %rdx", " %rdx")));
        assert_eq!(split_ops("%rax"), None);
    }

    /// The write of an `lea` with a scaled index, and of a three-operand `imul`,
    /// is seen; both were missed when the destination was read off the first
    /// comma.
    #[test]
    fn writes_through_scaled_lea_and_three_operand_imul_are_seen() {
        assert_eq!(written_regs("lea", "  lea 0(,%r15,4), %rax"), vec!["%rax".to_string()]);
        assert_eq!(written_regs("imul", "  imul $3, %rdx, %rdx"), vec!["%rdx".to_string()]);
    }

    /// The fuzz-found miscompile: `%rax` is stored, overwritten by a scaled
    /// `lea`, then reloaded. The reload must survive, or `a * (4*b)` becomes
    /// `(4*b) * (4*b)`.
    #[test]
    fn forward_stores_sees_a_scaled_lea_overwrite_the_register() {
        let body = "  mov %rax, -240(%rbp)\n  lea 0(,%r15,4), %rax\n  mov %rax, -280(%rbp)\n  mov -240(%rbp), %rax\n  imul -280(%rbp), %rax\n";
        let out = forward_stores(body);
        assert!(out.contains("mov -240(%rbp), %rax"), "the reload of the stored value was dropped:\n{out}");
    }

    /// `mov $k, %t; sub %s, %t` is `k - s`. No `lea` displacement expresses
    /// that, so the pair must survive; folding it to `lea -k(%s)` computed
    /// `s - k` (`5 - x` printed `x - 5`, `wrapping_neg` printed `x`).
    #[test]
    fn a_constant_minus_a_register_is_not_an_lea() {
        let body = "  mov $5, %rax\n  sub %rbx, %rax\n  mov %rax, %rdi\n";
        assert_eq!(fuse_imm(body), body);
        let zero = "  mov $0, %rax\n  sub %rbx, %rax\n";
        assert_eq!(fuse_imm(zero), zero);
    }

    /// `mov $k, %t; add %s, %t` is `s + k` and folds to `lea k(%s), %t`, but
    /// `lea` sets no flags: when the next instruction reads them (the `seto`
    /// of an overflow-checked add) the `add` must stay.
    #[test]
    fn a_constant_plus_a_register_folds_unless_its_flags_are_read() {
        let out = fuse_imm("  mov $5, %rax\n  add %rbx, %rax\n  mov %rax, %rdi\n");
        assert!(out.contains("lea 5(%rbx), %rax"), "{out}");
        let checked = "  mov $5, %rax\n  add %rbx, %rax\n  seto %al\n";
        assert_eq!(fuse_imm(checked), checked);
        let scaled = "  mov $3, %rax\n  imul %rbx, %rax\n  jo .Lovf\n";
        assert_eq!(fuse_imm(scaled), scaled);
    }

    /// `fuse_lea` drops the `add`/`sub` that set the flags; a following
    /// conditional jump or `set`/`cmov` still needs them.
    #[test]
    fn fuse_lea_keeps_an_arithmetic_whose_flags_are_read() {
        let body = "  mov %rbx, %rax\n  sub $1, %rax\n  seto %cl\n  mov %rax, %rdi\n";
        assert_eq!(fuse_lea(body), body);
    }

    /// Every conditional move reads the flags, not only the four spellings the
    /// first version listed.
    #[test]
    fn every_conditional_move_reads_the_flags() {
        for op in ["cmovg", "cmovl", "cmovge", "cmovle", "cmova", "cmovae", "cmovs", "cmovo"] {
            assert!(flags_used_by(Some(&format!("  {op} %rcx, %rax"))), "{op}");
        }
        assert!(!flags_used_by(Some("  mov %rcx, %rax")));
    }

    /// `add $0, %reg` cannot change its destination and is dropped; the same
    /// instruction must survive when the next one reads the flags it sets.
    #[test]
    fn noop_arithmetic_is_dropped_but_flags_are_kept() {
        let body = "  mov %rax, %rcx\n  add $0, %rcx\n  mov %rcx, %rdx\n";
        let out = drop_noop_arith(body);
        assert!(!out.contains("add $0"), "no-op add not dropped:\n{out}");
        let flags = "  add $0, %rcx\n  jle .Lx\n";
        let out = drop_noop_arith(flags);
        assert!(out.contains("add $0"), "flagged add wrongly dropped:\n{out}");
    }

    /// Two adjacent `mul`/`div`-by-constant units are interleaved: the second
    /// multiply is hoisted above the first divide so the two latencies overlap.
    #[test]
    fn adjacent_division_units_are_interleaved() {
        let body = "\
  mov %rsi, %rax
  imul %rsi, %rax
  mov %rax, %r10
  movabs $2361183241434822607, %rcx
  imul %rcx
  mov %rdx, %rcx
  shr $63, %rcx
  sar $7, %rdx
  add %rcx, %rdx
  mov %rdx, %r8
  mov %rdi, %rax
  imul %rdi, %rax
  mov %rax, %r10
  movabs $2361183241434822607, %rcx
  imul %rcx
  mov %rdx, %rcx
  shr $63, %rcx
  sar $7, %rdx
  add %rcx, %rdx
  mov %rdx, %r9
";
        let out = interleave_div_pairs(body);
        // Both multiplies appear before the first divide's `movabs`.
        let first_movabs = out.find("movabs").expect("a movabs");
        let second_imul = out.match_indices("imul %rsi, %rax")
            .chain(out.match_indices("imul %rdi, %rax"))
            .map(|(i, _)| i)
            .max()
            .expect("an imul");
        assert!(
            second_imul < first_movabs,
            "the second multiply must precede the first divide:\n{out}"
        );
        assert_eq!(
            out.matches("movabs").count(),
            2,
            "both divides survive:\n{out}"
        );
    }

    /// A lone division unit (no adjacent partner) is left untouched, so the
    /// pass never rewrites an unpaired divide.
    #[test]
    fn a_lone_division_unit_is_not_rewritten() {
        let body = "\
  mov %rsi, %rax
  imul %rsi, %rax
  mov %rax, %r10
  movabs $2361183241434822607, %rcx
  imul %rcx
  mov %rdx, %rcx
  shr $63, %rcx
  sar $7, %rdx
  add %rcx, %rdx
  mov %rdx, %r8
  add %r8, %r9
";
        assert_eq!(interleave_div_pairs(body), body, "an unpaired divide moved");
    }

    /// Two division units whose quotient feeds the next unit's multiply are
    /// *dependent*: the second product must square the first quotient, so the
    /// pair must be left alone. Interleaving would read the register before the
    /// first quotient is written and square its stale value.
    #[test]
    fn a_dependent_division_pair_is_not_interleaved() {
        let body = "\
  mov %rbx, %rax
  imul %rbx, %rax
  mov %rax, %r10
  movabs $2361183241434822607, %rcx
  imul %rcx
  mov %rdx, %rcx
  shr $63, %rcx
  sar $7, %rdx
  add %rcx, %rdx
  mov %rdx, %r12
  mov %r12, %rax
  imul %r12, %rax
  mov %rax, %r10
  movabs $2361183241434822607, %rcx
  imul %rcx
  mov %rdx, %rcx
  shr $63, %rcx
  sar $7, %rdx
  add %rcx, %rdx
  mov %rdx, %r13
";
        assert_eq!(
            interleave_div_pairs(body),
            body,
            "a dependent pair was reordered, so the second multiply reads a stale quotient"
        );
    }

    /// The array-index address chain folds into one addressing mode: the base
    /// `lea`, the index copy into `%rcx`, and the indexed `lea` become a single
    /// `lea` naming the base register, the index and the scale.
    #[test]
    fn an_indexed_address_chain_folds_to_one_lea() {
        let body = "\
  lea -2096(%rbp), %rax
  mov %rsi, %rcx
  lea (%rax,%rcx,8), %rax
  mov (%rax), %rax
";
        let out = fold_lea_terms(body);
        assert_eq!(out, "  lea -2096(%rbp, %rsi, 8), %rax\n  mov (%rax), %rax\n");
    }

    /// The fold is refused when the scratch register still holds a live value:
    /// here the following instruction reads `%rcx`, so dropping the index copy
    /// would leave it stale.
    #[test]
    fn an_indexed_chain_whose_scratch_is_still_live_is_not_folded() {
        let body = "\
  lea -2096(%rbp), %rax
  mov %rsi, %rcx
  lea (%rax,%rcx,8), %rax
  mov (%rax), %rdx
  add %rcx, %rdx
";
        assert_eq!(fold_lea_terms(body), body, "a live scratch value was dropped");
    }
}
