//! Semantic analysis: resolves variables to stack slots, lays out structs, and
//! infers and checks types.
//!
//! Every local (parameters included) gets a unique slot range in the function
//! frame. `Expr::Var` is rewritten in place with its slot and type, and
//! `Expr::Field` with its byte offset, so codegen never does a name lookup.
//!
//! Struct layout is declaration order with no padding: field `k` starts at the
//! sum of the sizes of fields `0..k`. Every field is 8 bytes, so a struct is
//! always a multiple of 8 and every field is naturally aligned.

use crate::ast::*;
use crate::consts::{const_binop, const_binop_checked, const_i64};
use std::collections::HashMap;

#[derive(Clone)]
pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
    /// A method receiver: such a function may only be called as `recv.name(..)`.
    pub takes_self: bool,
    /// A `&self` / `&mut self` receiver: `params[0]` is a reference to the impl
    /// type, and a method call passes the receiver's address, not a copy.
    pub ref_self: bool,
    /// True when the receiver was `&mut self`, so the synthesized parameter is
    /// `&mut T` and matches the address the call site builds.
    pub ref_self_mut: bool,
}

/// A laid-out struct: each field's name, resolved type and byte offset, plus the
/// total size.
#[derive(Clone)]
pub struct StructInfo {
    pub fields: Vec<(String, Type, usize)>,
    pub size: usize,
}

/// An enum's variants and its representation. A unit enum has `size == 8` and
/// holds only a tag; a payload enum is a tagged union of `size` bytes, laid out
/// as `{ i64 tag, <payload> }` where the payload starts at offset 8 and is as
/// wide as the widest variant.
#[derive(Clone)]
pub struct EnumInfo {
    /// Per variant: its name, its fields as `(type, offset within the payload)`,
    /// and the payload width that variant uses.
    pub variants: Vec<(String, Vec<(Type, usize)>, usize)>,
    pub size: usize,
    pub has_payload: bool,
}

impl EnumInfo {
    fn tag_of(&self, variant: &str) -> Option<i64> {
        self.variants.iter().position(|(n, _, _)| n == variant).map(|i| i as i64)
    }

    fn variant(&self, name: &str) -> Option<&(String, Vec<(Type, usize)>, usize)> {
        self.variants.iter().find(|(n, _, _)| n == name)
    }
}

pub struct Sema {
    sigs: HashMap<String, Sig>,
    structs: HashMap<String, StructInfo>,
    enums: HashMap<String, EnumInfo>,
    next_slot: i32,
    loop_depth: u32,
    /// One entry per enclosing `while`/`for` loop, holding the scope length at
    /// the loop's entry. Such a body may run again, so a write inside it to a
    /// non-`mut` binding that already existed before the loop is a second write
    /// on the first iteration (rustc's `cannot assign twice`); a binding
    /// declared *inside* the body is fresh each iteration and is exempt. A
    /// `loop` runs at least once, so it pushes nothing. The paired
    /// `loop_repeats` flag marks a `loop` whose body can reach the top again.
    loop_marks: Vec<usize>,
    /// Per enclosing loop, whether a write to a binding that existed before the
    /// loop can repeat. A `while`/`for` always may; a `loop` only when its body
    /// can reach the top again. `check_block` rewrites the top entry per
    /// statement to "the rest of the body from here can reach the top", so a
    /// write is a second write only when its own suffix can loop back.
    loop_repeats: Vec<bool>,
    /// Per enclosing loop, the definite-initialization state captured at each
    /// `break` that leaves it. A `loop` runs its body at least once, so a
    /// binding the loop initializes stays initialized afterwards — but only if
    /// *every* exit leaves it initialized. `while`/`for` may run zero times, so
    /// their entry is recorded but the merge is not used.
    loop_exits: Vec<Vec<Vec<bool>>>,
    /// Parallel to `loop_repeats`: at the current point, whether control can
    /// still reach the top of that loop. Inside a `loop` body this is the
    /// per-statement repeat flag. A `continue` in a branch restores it to `true`
    /// while that branch is checked.
    loop_reach: Vec<bool>,
    /// Parallel to `loop_reach`: the reach of the code *after* the statement
    /// currently being checked. A nested block (an `if` arm, a `{}` expression)
    /// falls through to this point, so its own reach analysis is seeded with it
    /// rather than with the statement's own reach.
    loop_cont: Vec<bool>,
    /// Per enclosing loop, the may-assigned state accumulated over every `break`
    /// that leaves it, so a write on a break path counts as the first write for
    /// a later statement.
    break_assigned: Vec<Vec<bool>>,
    ret_ty: Type,
    /// While checking the `else` chain of a desugared `match`: whether the
    /// whole chain names every variant of one enum. Decided once at the chain's
    /// first `if` and carried down, so the final `if` (which has no `else`) may
    /// stand for a value. Cleared while checking a condition or an arm body.
    match_total: bool,
    /// Whether the `if` being checked is a non-root step of such a chain, so it
    /// inherits `match_total` instead of recomputing it.
    match_chain: bool,
    /// The constant value of each local, by slot, if it is known. A local is
    /// entered only when rustc's constant propagation would know it: its
    /// initializer is constant and it is never written or mutably borrowed
    /// anywhere in the function (see `collect_written`). Used to fold an index
    /// or arithmetic operand that is a constant *local* (`let i = -1; a[i]`),
    /// which rustc folds and rmc otherwise missed.
    const_locals: HashMap<i32, i64>,
    /// How many times each binding name is assigned by an `=` statement in the
    /// current function (the `let` initializer is not counted). rustc folds a
    /// local only when it is assigned at most once in total, so a name with any
    /// `=` assignment is folded only if it was declared without an initializer
    /// and this is that one assignment. Keyed by name because the scan runs
    /// before sema assigns slots.
    assign_count: HashMap<String, u32>,
    /// Binding names taken by `&mut` anywhere in the function; such a local is
    /// never folded, since the callee may write it.
    mut_borrowed: std::collections::HashSet<String>,
    /// Names of the program's `const` items. A local binding may not reuse one
    /// (rustc E0530), so a `let`, a parameter or a `for` variable is checked
    /// against this set.
    const_names: std::collections::HashSet<String>,
}

/// A lexical scope entry. `mutable` records whether the binding was declared
/// `mut`, so an assignment to a non-`mut` binding can be rejected the way rustc
/// rejects it. `init` records whether the binding is definitely initialized at
/// the current program point, so a read of a `let x: T;` that was never written
/// is rejected like rustc's "used binding isn't initialized".
struct Binding {
    name: String,
    slot: i32,
    ty: Type,
    mutable: bool,
    init: bool,
    /// True for a function/method parameter, which rustc reports as an
    /// "immutable argument" rather than a twice-assigned variable.
    param: bool,
    /// True once the binding has been written on some path: its declaration
    /// initialiser, or an assignment. A plain `let` without an initialiser may
    /// be written exactly once, so a second write is rejected the way rustc
    /// rejects `cannot assign twice to immutable variable`.
    assigned: bool,
}

/// A lexical scope: a stack of bindings, innermost last.
type Scope = Vec<Binding>;

pub fn analyze(prog: &mut Program) -> Result<(), String> {
    let (structs, enums) = resolve_layouts(&prog.structs, &prog.enums)?;

    // Validate the `const` items. Their values were folded by the parser (which
    // needs them for array lengths and patterns); here each is checked for a
    // duplicate name, a usable type, and an initializer that actually folds.
    let const_names = check_consts(prog, &structs, &enums)?;

    let mut sigs: HashMap<String, Sig> = HashMap::new();
    // Runtime helpers provided by the freestanding assembly runtime. They are
    // linked in automatically, so they do not appear in the source program.
    let builtins: [(&str, &[Type], Type); 7] = [
        ("print_i64", &[Type::I64], Type::Unit),
        ("print_str", &[Type::Str], Type::Unit),
        ("print_bool", &[Type::Bool], Type::Unit),
        ("putchar", &[Type::I64], Type::Unit),
        ("alloc", &[Type::I64], Type::I64),
        ("free", &[Type::I64], Type::Unit),
        ("arg_i64", &[Type::I64], Type::I64),
    ];
    for (name, params, ret) in builtins {
        sigs.insert(
            name.to_string(),
            Sig {
                params: params.to_vec(),
                ret,
                takes_self: false,
                ref_self: false,
                ref_self_mut: false,
            },
        );
    }
    // A method's receiver is its first parameter, with the impl's struct type.
    // Give it to every method up front so its signature (and later its frame)
    // includes `self` from the start.
    for f in &mut prog.fns {
        if let Some(ty) = f.self_ty.clone() {
            let rt = if f.ref_self {
                Type::Ref(Box::new(Type::Named(ty)), f.ref_self_mut)
            } else {
                Type::Named(ty)
            };
            f.params.insert(0, ("self".to_string(), rt));
            f.param_mut.insert(0, false);
        }
    }
    for f in &prog.fns {
        if sigs.contains_key(&f.name) {
            return Err(format!("function `{}` is defined more than once", f.name));
        }
        let params: Result<Vec<Type>, String> =
            f.params.iter().map(|(_, t)| resolve_in(t, &structs, &enums)).collect();
        let params = params?;
        let ret = resolve_in(&f.ret, &structs, &enums)?;
        // The System V calling convention passes the first six integer
        // arguments in registers; this compiler does not implement stack
        // arguments, so a seventh parameter could never receive a value.
        if params.len() > 6 {
            return Err(format!(
                "function `{}` has {} parameters, but at most 6 are supported",
                f.name,
                params.len()
            ));
        }
        // Aggregate parameters and returns are passed by pointer, so any type
        // is allowed here; the calling convention handles the rest.
        sigs.insert(
            f.name.clone(),
            Sig {
                params,
                ret,
                takes_self: f.takes_self,
                ref_self: f.ref_self,
                ref_self_mut: f.ref_self_mut,
            },
        );
    }
    // `main` is the entry point: it takes no parameters (rustc E0580). rmc
    // additionally lets it return `i64`, which becomes the exit status.
    if let Some(main) = prog.fns.iter().find(|f| f.name == "main") {
        if !main.params.is_empty() {
            return Err(format!(
                "`main` takes no parameters, but {} were declared",
                main.params.len()
            ));
        }
    }

    let mut sema = Sema {
        sigs,
        structs,
        enums,
        next_slot: 0,
        loop_depth: 0,
        loop_marks: Vec::new(),
        loop_repeats: Vec::new(),
        loop_exits: Vec::new(),
        loop_reach: Vec::new(),
        loop_cont: Vec::new(),
        break_assigned: Vec::new(),
        ret_ty: Type::Unit,
        match_total: false,
        match_chain: false,
        const_locals: HashMap::new(),
        assign_count: HashMap::new(),
        mut_borrowed: std::collections::HashSet::new(),
        const_names,
    };
    for f in &mut prog.fns {
        sema.check_fn(f)?;
    }
    Ok(())
}

/// Resolve the layouts of every struct and enum together, since they may refer
/// to each other: a struct may have an enum field and an enum variant may have a
/// struct payload. Returns both tables.
fn resolve_layouts(
    structs: &[StructDef],
    enums: &[EnumDef],
) -> Result<(HashMap<String, StructInfo>, HashMap<String, EnumInfo>), String> {
    let raw_s: HashMap<&str, &StructDef> = structs.iter().map(|d| (d.name.as_str(), d)).collect();
    if raw_s.len() != structs.len() {
        return Err("a struct is defined more than once".into());
    }
    let raw_e: HashMap<&str, &EnumDef> = enums.iter().map(|d| (d.name.as_str(), d)).collect();
    if raw_e.len() != enums.len() {
        return Err("an enum is defined more than once".into());
    }
    for e in enums {
        if raw_s.contains_key(e.name.as_str()) {
            return Err(format!("`{}` is both a struct and an enum", e.name));
        }
        for (i, v) in e.variants.iter().enumerate() {
            if e.variants[..i].iter().any(|w| w.name == v.name) {
                return Err(format!("enum `{}`: duplicate variant `{}`", e.name, v.name));
            }
        }
    }
    let mut so: HashMap<String, StructInfo> = HashMap::new();
    let mut eo: HashMap<String, EnumInfo> = HashMap::new();
    for d in structs {
        layout_struct(&d.name, &raw_s, &raw_e, &mut so, &mut eo, &mut Vec::new())?;
    }
    for d in enums {
        layout_enum(&d.name, &raw_s, &raw_e, &mut so, &mut eo, &mut Vec::new())?;
    }
    Ok((so, eo))
}

/// Lay out a struct: field `k` at the sum of the sizes of fields `0..k`, no
/// padding. A struct may refer to a struct or enum declared later, but not to
/// itself, directly or indirectly.
fn layout_struct(
    name: &str,
    raw_s: &HashMap<&str, &StructDef>,
    raw_e: &HashMap<&str, &EnumDef>,
    so: &mut HashMap<String, StructInfo>,
    eo: &mut HashMap<String, EnumInfo>,
    visiting: &mut Vec<String>,
) -> Result<(), String> {
    if so.contains_key(name) {
        return Ok(());
    }
    if visiting.iter().any(|n| n == name) {
        return Err(format!("`{name}` contains itself"));
    }
    let def = raw_s.get(name).ok_or_else(|| format!("unknown type `{name}`"))?;
    visiting.push(name.to_string());
    let mut fields: Vec<(String, Type, usize)> = Vec::new();
    let mut size = 0usize;
    for (fname, fty) in &def.fields {
        let rty = resolve_ty_rec(fty, raw_s, raw_e, so, eo, visiting)?;
        if fields.iter().any(|(n, _, _)| n == fname) {
            return Err(format!("struct `{name}`: duplicate field `{fname}`"));
        }
        let off = size;
        size += rty.size();
        fields.push((fname.clone(), rty, off));
    }
    visiting.pop();
    so.insert(name.to_string(), StructInfo { fields, size });
    Ok(())
}

/// Lay out an enum. A payload enum is `{ i64 tag, <widest variant payload> }`:
/// the payload starts at 8 and the enum is `8 + max_payload` bytes, rounded up
/// to a multiple of 8.
fn layout_enum(
    name: &str,
    raw_s: &HashMap<&str, &StructDef>,
    raw_e: &HashMap<&str, &EnumDef>,
    so: &mut HashMap<String, StructInfo>,
    eo: &mut HashMap<String, EnumInfo>,
    visiting: &mut Vec<String>,
) -> Result<(), String> {
    if eo.contains_key(name) {
        return Ok(());
    }
    if visiting.iter().any(|n| n == name) {
        return Err(format!("`{name}` contains itself"));
    }
    let def = raw_e.get(name).ok_or_else(|| format!("unknown type `{name}`"))?;
    visiting.push(name.to_string());
    let mut variants: Vec<(String, Vec<(Type, usize)>, usize)> = Vec::new();
    let mut max_payload = 0usize;
    let mut has_payload = false;
    for v in &def.variants {
        let mut fields: Vec<(Type, usize)> = Vec::new();
        let mut off = 0usize;
        for fty in &v.fields {
            let rty = resolve_ty_rec(fty, raw_s, raw_e, so, eo, visiting)?;
            fields.push((rty.clone(), off));
            off += rty.size();
        }
        if !v.fields.is_empty() {
            has_payload = true;
        }
        max_payload = max_payload.max(off);
        variants.push((v.name.clone(), fields, off));
    }
    visiting.pop();
    let size = if has_payload { 8 + ((max_payload + 7) / 8) * 8 } else { 8 };
    eo.insert(name.to_string(), EnumInfo { variants, size, has_payload });
    Ok(())
}

/// Resolve a type, laying out any struct or enum it names first. An enum name
/// becomes `Type::Enum` (a unit enum) or `Type::EnumData` (a payload enum); a
/// struct name becomes `Type::Struct`. Array elements stay `i64`/`bool`.
fn resolve_ty_rec(
    t: &Type,
    raw_s: &HashMap<&str, &StructDef>,
    raw_e: &HashMap<&str, &EnumDef>,
    so: &mut HashMap<String, StructInfo>,
    eo: &mut HashMap<String, EnumInfo>,
    visiting: &mut Vec<String>,
) -> Result<Type, String> {
    match t {
        Type::Named(n) => {
            if raw_e.contains_key(n.as_str()) {
                layout_enum(n, raw_s, raw_e, so, eo, visiting)?;
                let info = &eo[n];
                if info.has_payload {
                    Ok(Type::EnumData(n.clone(), info.size))
                } else {
                    Ok(Type::Enum(n.clone()))
                }
            } else {
                layout_struct(n, raw_s, raw_e, so, eo, visiting)?;
                Ok(Type::Struct(n.clone(), so[n].size))
            }
        }
        Type::Array(elem, n) => {
            let e = resolve_ty_rec(elem, raw_s, raw_e, so, eo, visiting)?;
            if !e.is_int_or_bool() {
                return Err(format!("array elements must be `i64`, `bool` or a unit enum, not {e:?}"));
            }
            Ok(Type::Array(Box::new(e), *n))
        }
        Type::Ref(inner, m) => {
            let t = resolve_ty_rec(inner, raw_s, raw_e, so, eo, visiting)?;
            Ok(Type::Ref(Box::new(t), *m))
        }
        other => Ok(other.clone()),
    }
}

/// Resolve a type against finished tables, for callers that run after layout
/// (signature building). Mirrors `resolve_ty_rec` without the recursion.
fn resolve_in(
    t: &Type,
    structs: &HashMap<String, StructInfo>,
    enums: &HashMap<String, EnumInfo>,
) -> Result<Type, String> {
    match t {
        Type::Named(n) => {
            if let Some(info) = enums.get(n) {
                return Ok(if info.has_payload {
                    Type::EnumData(n.clone(), info.size)
                } else {
                    Type::Enum(n.clone())
                });
            }
            match structs.get(n) {
                Some(info) => Ok(Type::Struct(n.clone(), info.size)),
                None => Err(format!("unknown type `{n}`")),
            }
        }
        Type::Array(elem, n) => {
            let e = resolve_in(elem, structs, enums)?;
            if !e.is_int_or_bool() {
                return Err(format!("array elements must be `i64`, `bool` or a unit enum, not {e:?}"));
            }
            Ok(Type::Array(Box::new(e), *n))
        }
        Type::Ref(inner, m) => Ok(Type::Ref(Box::new(resolve_in(inner, structs, enums)?), *m)),
        other => Ok(other.clone()),
    }
}

/// Validate every `const` item: unique name, a type rmc can fold (`i64` or
/// `bool`), and an initializer that evaluates to that type. Returns the set of
/// constant names so local bindings can be checked against it. The parser
/// already folded the values; this only checks them, so a constant whose
/// initializer does not fold (a call, a cycle, an out-of-range arithmetic) is
/// reported here rather than silently dropped.
fn check_consts(
    prog: &Program,
    structs: &HashMap<String, StructInfo>,
    enums: &HashMap<String, EnumInfo>,
) -> Result<std::collections::HashSet<String>, String> {
    let mut names = std::collections::HashSet::new();
    // A `const` and a function share a namespace in rustc (E0428).
    let fn_names: std::collections::HashSet<&str> =
        prog.fns.iter().map(|f| f.name.as_str()).collect();
    let items: Vec<(String, Type, Expr)> = prog
        .consts
        .iter()
        .map(|c| (c.name.clone(), c.ty.clone(), c.value.clone()))
        .collect();
    let resolved = crate::consts::resolve(&items);
    for c in &prog.consts {
        if !names.insert(c.name.clone()) {
            return Err(format!("the name `{}` is defined multiple times", c.name));
        }
        if fn_names.contains(c.name.as_str()) {
            return Err(format!("the name `{}` is defined multiple times", c.name));
        }
        let ty = resolve_in(&c.ty, structs, enums)?;
        match ty {
            Type::I64 | Type::Bool => {}
            other => {
                return Err(format!(
                    "constant `{}` has type {other:?}; rmc only folds `i64` and `bool` constants",
                    c.name
                ))
            }
        }
        // `resolved` holds only items whose initializer folded, so a missing
        // name means the initializer is not a constant expression (or is part
        // of a cycle).
        match (ty, resolved.get(&c.name)) {
            (Type::I64, Some(crate::consts::ConstVal::Int(_))) => {}
            (Type::Bool, Some(crate::consts::ConstVal::Bool(_))) => {}
            (Type::I64, _) => {
                return Err(format!(
                    "constant `{}` is not a constant `i64` expression",
                    c.name
                ))
            }
            (Type::Bool, _) => {
                return Err(format!(
                    "constant `{}` is not a constant `bool` expression",
                    c.name
                ))
            }
            _ => unreachable!(),
        }
    }
    Ok(names)
}

impl Sema {
    /// Every type site goes through here, so an enum name resolves wherever a
    /// type may appear.
    fn resolve_ty(&self, t: &Type) -> Result<Type, String> {
        resolve_in(t, &self.structs, &self.enums)
    }

    fn check_fn(&mut self, f: &mut FnDef) -> Result<(), String> {
        // A parameter name must be unique within its function (rustc E0415).
        // `self` is inserted first, so this also rejects a second `self`.
        for (i, (name, _)) in f.params.iter().enumerate() {
            if f.params[..i].iter().any(|(n, _)| n == name) {
                return Err(format!(
                    "identifier `{name}` is bound more than once in this parameter list"
                ));
            }
            // A parameter may not shadow a constant (rustc E0530). `self` is
            // never a constant, so it needs no special case.
            if self.const_names.contains(name) {
                return Err(format!(
                    "identifier `{name}` shadows a constant of the same name"
                ));
            }
        }
        self.next_slot = 0;
        self.loop_depth = 0;
        self.const_locals.clear();
        self.assign_count.clear();
        self.mut_borrowed.clear();
        Self::collect_assigns(&f.body, &mut self.assign_count, &mut self.mut_borrowed);
        self.ret_ty = self.resolve_ty(&f.ret)?;
        f.ret = self.ret_ty.clone();
        // An aggregate return is written through a hidden pointer, so reserve a
        // slot to hold that pointer for the whole body.
        f.ret_slot = if self.ret_ty.is_aggregate() {
            self.alloc_slot()
        } else {
            -1
        };

        let mut scope: Scope = Vec::with_capacity(f.params.len() + 8);
        f.param_slots.clear();
        for (i, (name, ty)) in f.params.iter_mut().enumerate() {
            let rt = self.resolve_ty(ty)?;
            *ty = rt.clone();
            // An aggregate parameter owns a frame buffer big enough for a copy;
            // a scalar (including a reference) owns a single slot.
            let slot = self.alloc_slots((rt.size() / 8) as i32);
            f.param_slots.push(slot);
            // A parameter is a `let` binding; it is immutable unless declared
            // `mut`, and it is always initialized by the caller.
            let mutable = f.param_mut.get(i).copied().unwrap_or(false);
            scope.push(Binding { name: name.clone(), slot, ty: rt, mutable, init: true, param: true, assigned: true });
        }

        let body_ty = self.check_block(&mut f.body, &mut scope)?;
        // A body whose tail is a value must match the declared return type.
        // A body ending in `return` (or `break`) has a `Unit` tail and is fine;
        // a completely empty body is not, for a non-unit function.
        if body_ty != Type::Unit && body_ty != self.ret_ty {
            return Err(format!(
                "function `{}` returns {body_ty:?} but is declared to return {:?}",
                f.name, self.ret_ty
            ));
        }
        // A body whose tail yields no value (a `Unit` body: no tail, or a tail
        // like an `if` without an `else`) falls off its end, returning zero.
        // That is fine for a unit function, but for a value-returning one every
        // path must `return` (or diverge) so the end is never reached.
        if self.ret_ty != Type::Unit && body_ty == Type::Unit && !block_diverges(&f.body) {
            return Err(format!(
                "function `{}` is declared to return {:?} but its body can fall off the end",
                f.name, self.ret_ty
            ));
        }
        f.nslots = self.next_slot;
        Ok(())
    }

    fn alloc_slot(&mut self) -> i32 {
        let s = self.next_slot;
        self.next_slot += 1;
        s
    }

    fn alloc_slots(&mut self, n: i32) -> i32 {
        let s = self.next_slot;
        self.next_slot += n.max(1);
        s
    }

    fn lookup(scope: &Scope, name: &str) -> Option<(i32, Type, bool, bool)> {
        scope
            .iter()
            .rev()
            .find(|b| b.name == name)
            .map(|b| (b.slot, b.ty.clone(), b.mutable, b.init))
    }

    /// Index of the innermost binding of `name`, for mutating its state.
    fn binding_index(scope: &Scope, name: &str) -> Option<usize> {
        scope.iter().rposition(|b| b.name == name)
    }

    /// Snapshot the initialization state of each binding, to compare against
    /// after checking a branch or loop body.
    fn init_snapshot(scope: &Scope) -> Vec<bool> {
        scope.iter().map(|b| b.init).collect()
    }

    /// After a loop body, a variable written inside is not definitely
    /// initialized: the loop may run zero times.
    fn init_restore(scope: &mut Scope, snap: &[bool]) {
        for (b, &before) in scope.iter_mut().zip(snap) {
            b.init = before;
        }
    }

    /// Snapshot the "already written" state of each binding, so a branch can be
    /// checked from the same state its sibling saw.
    fn assigned_snapshot(scope: &Scope) -> Vec<bool> {
        scope.iter().map(|b| b.assigned).collect()
    }

    fn assigned_restore(scope: &mut Scope, snap: &[bool]) {
        if scope.len() == snap.len() {
            for (b, &s) in scope.iter_mut().zip(snap) {
                b.assigned = s;
            }
        }
    }

    /// Merge two paths: a binding counts as written when *either* path wrote it,
    /// since a later write only needs to know it *may* already have happened.
    fn assigned_merge(scope: &mut Scope, a: &[bool], b: &[bool]) {
        if scope.len() == a.len() && scope.len() == b.len() {
            for (i, binding) in scope.iter_mut().enumerate() {
                binding.assigned = a[i] || b[i];
            }
        }
    }

    /// Type-check a `let` initialiser. A `[v; n]` written directly here is kept
    /// as an `ArrayLit` so codegen splats one value across the array, rather
    /// than materialising it element by element. Any other initialiser (and any
    /// repeat literal nested inside one) goes through `check_expr`, which
    /// rewrites a repeat literal to a block so it has an address.
    fn check_let_init(&mut self, e: &mut Expr, scope: &mut Scope) -> Result<Type, String> {
        if let Expr::ArrayLit(val, n) = e {
            let t = self.check_expr(val, scope)?;
            if !t.is_int_or_bool() {
                return Err(format!("array elements must be `i64`, `bool` or a unit enum, found {t:?}"));
            }
            return Ok(Type::Array(Box::new(t), *n));
        }
        self.check_expr(e, scope)
    }

    /// Whether a desugared `match` `if`-chain is provably total: it tests every
    /// variant of one enum exactly once, or both values of one `bool`, and no
    /// other condition.
    ///
    /// A `match` with no `_` arm folds to `if c1 { a1 } else { if c2 { a2 } ... }`
    /// with no final `else`, so a value that matches no arm falls through to
    /// unit. That is exactly right when the match is non-exhaustive, but when
    /// the arms name *every* variant the fall-through is unreachable and the
    /// chain may be used as a value. rustc accepts that; rmc rejected it with a
    /// spurious "`if` without `else`" error.
    ///
    /// `VariantTest` and `BoolTest` nodes are produced only by `match`
    /// desugaring, so a chain whose conditions are all of one kind, test the
    /// same enum (or the same bool variable), and whose tested variants (or
    /// bool values) are pairwise distinct and complete is exhaustive. Anything
    /// else (an integer arm, a duplicate, a missing case, a hand-written `if`
    /// chain) is not, and keeps the old error.
    ///
    /// The comparison is by variant *name*, not tag: the nested `VariantTest`s
    /// have not been visited by `check_expr` yet, so their tags are still -1.
    fn if_chain_is_exhaustive(&self, cond: &Expr, els: &Option<Block>) -> bool {
        // Every condition must test the same thing: all the variants of one
        // enum, or both values of one `bool`. A chain that mixes the two, or
        // tests two different enums, is never exhaustive. The `BoolTest` case
        // is why the node exists: `match b { true => .., false => .. }` is
        // exhaustive, but the equivalent hand-written `if b == true { .. }
        // else if b == false { .. }` (a plain `Bin(Eq, ..)`, not a `BoolTest`)
        // is not, and rustc rejects it in value position.
        enum Kind<'a> {
            Enum(&'a str),
            Bool(&'a str),
        }
        let mut kind: Option<Kind> = None;
        let mut seen_variants: Vec<&str> = Vec::new();
        let mut seen_bools: Vec<bool> = Vec::new();
        let mut cur_cond = cond;
        let mut cur_els = els;
        loop {
            match cur_cond {
                Expr::VariantTest { enum_name, variant, .. } => {
                    match kind {
                        Some(Kind::Enum(e)) if e == enum_name.as_str() => {}
                        None => kind = Some(Kind::Enum(enum_name.as_str())),
                        _ => return false,
                    }
                    if seen_variants.contains(&variant.as_str()) {
                        return false;
                    }
                    seen_variants.push(variant.as_str());
                }
                Expr::BoolTest { scrutinee, value } => {
                    // `desugar_match` only tests a `Var` (the temporary it bound
                    // the scrutinee to, or a bool variable); anything else cannot
                    // be proven the same across arms.
                    let name = match scrutinee.as_ref() {
                        Expr::Var { name, .. } => name.as_str(),
                        _ => return false,
                    };
                    match kind {
                        Some(Kind::Bool(s)) if s == name => {}
                        None => kind = Some(Kind::Bool(name)),
                        _ => return false,
                    }
                    if seen_bools.contains(value) {
                        return false;
                    }
                    seen_bools.push(*value);
                }
                _ => return false,
            }
            match cur_els {
                None => break,
                Some(block) => {
                    // An arm body is `{ body }`; only the nested `else` chains.
                    if !block.stmts.is_empty() {
                        return false;
                    }
                    match block.tail.as_deref() {
                        Some(Expr::If(cond, _, els)) => {
                            cur_cond = cond;
                            cur_els = els;
                        }
                        _ => return false,
                    }
                }
            }
        }
        match kind {
            Some(Kind::Enum(name)) => {
                self.enums.get(name).map_or(false, |i| i.variants.len() == seen_variants.len())
            }
            // Both `true` and `false` tested, pairwise distinct.
            Some(Kind::Bool(_)) => seen_bools.len() == 2,
            None => false,
        }
    }

    /// Whether a desugared `match` (the `MatchBoundary` block `b`) names every
    /// variant of one enum. Unlike `if_chain_is_exhaustive`, this takes the
    /// boundary block so it can look through the binding-arm wrapper, and it is
    /// called before the chain is checked, so a nested `match`'s own
    /// `MatchBoundary` (not a bare `If`) still delimits the walk.
    fn match_chain_is_exhaustive(&self, b: &Block) -> bool {
        match b.tail.as_deref() {
            Some(Expr::If(cond, _, els)) => self.if_chain_is_exhaustive(cond, els),
            _ => false,
        }
    }

    fn check_block(&mut self, b: &mut Block, scope: &mut Scope) -> Result<Type, String> {
        self.check_block_inner(b, scope, false)
    }

    /// Check a block. When `loop_body` is set the block is a loop body, whose
    /// normal completion loops back to the top (seed `true`); any other block
    /// falls through to the code after its statement (`loop_cont`). Each
    /// statement's reach and continuation come from the block's suffix walk, so
    /// a nested `if`/block sees the continuation that really follows it.
    /// `loop_reach`/`loop_cont` are restored on exit.
    fn check_block_inner(
        &mut self,
        b: &mut Block,
        scope: &mut Scope,
        loop_body: bool,
    ) -> Result<Type, String> {
        let mark = scope.len();
        let saved_reach = self.loop_reach.last().copied().unwrap_or(false);
        let saved_cont = self.loop_cont.last().copied().unwrap_or(false);
        let in_loop = !self.loop_reach.is_empty();
        let seed = if loop_body { true } else { saved_cont };
        let active = in_loop && (!loop_body || self.loop_repeats.last().copied().unwrap_or(false));
        let suffixes = if active {
            Some(block_repeat_suffixes(b, seed))
        } else {
            None
        };
        let n = b.stmts.len();
        for (i, st) in b.stmts.iter_mut().enumerate() {
            let reach = match &suffixes {
                Some(sfx) => sfx[i],
                None => false,
            };
            let cont = match &suffixes {
                Some(sfx) => sfx[i + 1],
                None => saved_cont,
            };
            if let Some(l) = self.loop_reach.last_mut() {
                *l = reach;
            }
            if let Some(l) = self.loop_cont.last_mut() {
                *l = cont;
            }
            self.check_stmt(st, scope)?;
        }
        let ty = match &mut b.tail {
            Some(e) => {
                let reach = match &suffixes {
                    Some(sfx) => sfx[n],
                    None => false,
                };
                if let Some(l) = self.loop_reach.last_mut() {
                    *l = reach;
                }
                if let Some(l) = self.loop_cont.last_mut() {
                    *l = seed;
                }
                self.check_expr(e, scope)?
            }
            None => Type::Unit,
        };
        if let Some(l) = self.loop_reach.last_mut() {
            *l = saved_reach;
        }
        if let Some(l) = self.loop_cont.last_mut() {
            *l = saved_cont;
        }
        scope.truncate(mark);
        Ok(ty)
    }

    fn check_stmt(&mut self, st: &mut Stmt, scope: &mut Scope) -> Result<(), String> {
        match st {
            Stmt::Let { name, ty, init, slot, mutable } => {
                // A `let` may not shadow a constant (rustc E0530). Checked
                // before the initializer so the error names the binding.
                if self.const_names.contains(name) {
                    return Err(format!(
                        "identifier `{name}` shadows a constant of the same name"
                    ));
                }
                let declared = match ty {
                    Some(t) => Some(self.resolve_ty(t)?),
                    None => None,
                };
                let init_ty = match init {
                    Some(e) => Some(self.check_let_init(e, scope)?),
                    None => None,
                };
                let final_ty = match (declared, init_ty) {
                    (Some(d), Some(i)) => {
                        if d != i && !ref_coerces(&i, &d) {
                            return Err(format!(
                                "variable `{name}`: declared type {d:?} does not match initializer type {i:?}"
                            ));
                        }
                        d
                    }
                    (Some(d), None) => d,
                    (None, Some(i)) => i,
                    (None, None) => {
                        return Err(format!(
                            "variable `{name}`: needs a type annotation or an initializer"
                        ))
                    }
                };
                // Persist the resolved type so codegen takes the aggregate path
                // even when the source omitted the annotation (`let a = [7; 2]`).
                *ty = Some(final_ty.clone());
                let s = self.alloc_slots((final_ty.size() / 8) as i32);
                *slot = s;
                let initialized = init.is_some();
                scope.push(Binding {
                    name: name.clone(),
                    slot: s,
                    ty: final_ty,
                    mutable: *mutable,
                    init: initialized,
                    param: false,
                    assigned: init.is_some(),
                });
                // Track an `i64` local whose initializer is constant and that
                // is never written or mutably borrowed anywhere in the function
                // (`written`), so a later `a[i]` or `x + i` folds it the way
                // rustc's constant propagation does. The whole-function test
                // matters: `let mut x = i64::MAX; x = x + 1;` is accepted by
                // rustc because `x` is assigned twice, so its constant is not
                // propagated — folding the first `x` here would wrongly reject
                // the program.
                let assigned = self.assign_count.get(name).copied().unwrap_or(0);
                if init.is_some() && assigned == 0 && !self.mut_borrowed.contains(name) {
                    if let Some(e) = init.as_ref() {
                        if let Some(k) = self.const_of(e, &self.const_locals) {
                            self.const_locals.insert(s, k);
                        }
                    }
                }
                Ok(())
            }
            Stmt::Assign { target, value } => {
                // The destination must be a mutable place. Checked before the
                // value so the error names the binding rather than a type.
                self.place_writable(target, scope, true)?;
                // The value is checked first, so a self-referential initializer
                // (`let x: i64; x = x + 1;`) still reads `x` as uninitialized.
                let vty = self.check_expr(value, scope)?;
                // The write initializes a whole binding; mark it before
                // checking the target as a place, so `let x: i64; x = 7;` does
                // not read `x` back as uninitialized.
                let mut deferred_init = None;
                if let Expr::Var { name, .. } = target {
                    if let Some(i) = Self::binding_index(scope, name) {
                        let was_uninit = !scope[i].init;
                        scope[i].init = true;
                        if was_uninit {
                            deferred_init = Some(name.clone());
                        }
                    }
                }
                let tty = self.check_lvalue(target, scope)?;
                // A deferred single init (`let i; i = -1;`) is the one
                // assignment rustc folds; a second assignment would make the
                // count 2 and the gate fail. This runs after `check_lvalue`,
                // which is what resolves the target's slot.
                if let Some(name) = deferred_init {
                    if let Expr::Var { slot, .. } = target {
                        let assigned = self.assign_count.get(&name).copied().unwrap_or(0);
                        if assigned == 1
                            && !self.mut_borrowed.contains(&name)
                            && *slot >= 0
                        {
                            if let Some(k) = self.const_of(value, &self.const_locals) {
                                self.const_locals.insert(*slot, k);
                            }
                        }
                    }
                }
                if vty != tty && !ref_coerces(&vty, &tty) {
                    return Err(format!("assignment type mismatch: target is {tty:?}, value is {vty:?}"));
                }
                // Record the write so a later assignment to a non-`mut` binding
                // is the `cannot assign twice` error. The root name is used for
                // a field/index target (`a.f`, `a[i]`).
                let root = Self::place_root_name(target);
                if let Some(name) = root {
                    if let Some(i) = Self::binding_index(scope, &name) {
                        scope[i].assigned = true;
                    }
                }
                Ok(())
            }
            Stmt::Expr(e) => {
                self.check_expr(e, scope)?;
                Ok(())
            }
            Stmt::While(cond, body) => {
                let ct = self.check_expr(cond, scope)?;
                if ct != Type::Bool {
                    return Err(format!("`while` condition must be `bool`, found {ct:?}"));
                }
                let init_before = Self::init_snapshot(scope);
                self.loop_depth += 1;
                // The body may run again, so a non-`mut` binding that already
                // existed before the loop cannot be written inside it.
                self.loop_marks.push(scope.len());
                self.loop_repeats.push(true);
                self.loop_exits.push(Vec::new());
                self.loop_reach.push(false);
                self.break_assigned.push(Self::assigned_snapshot(scope));
                self.check_block_inner(body, scope, true)?;
                let breaks = self.break_assigned.pop().unwrap();
                self.loop_exits.pop();
                self.loop_reach.pop();
                self.loop_repeats.pop();
                self.loop_marks.pop();
                for (i, b) in scope.iter_mut().enumerate() {
                    if breaks.get(i).copied().unwrap_or(false) {
                        b.assigned = true;
                    }
                }
                self.loop_depth -= 1;
                // The loop may run zero times, so nothing it initializes is
                // guaranteed afterwards.
                Self::init_restore(scope, &init_before);
                Ok(())
            }
            Stmt::Loop(body) => {
                // A `loop` runs at least once, so writes to outer bindings inside
                // it behave like straight-line writes unless the body can reach
                // the top again; then it is a second write on the next pass and
                // rustc rejects it, exactly like a `while` body. A binding is
                // definitely initialized after the loop only if it was before, or
                // every `break` that leaves the loop initializes it.
                let init_before = Self::init_snapshot(scope);
                self.loop_depth += 1;
                self.loop_marks.push(scope.len());
                self.loop_repeats.push(block_reaches_top(body));
                self.loop_exits.push(Vec::new());
                self.loop_reach.push(false);
                self.break_assigned.push(Self::assigned_snapshot(scope));
                self.check_block_inner(body, scope, true)?;
                let breaks = self.break_assigned.pop().unwrap();
                let exits = self.loop_exits.pop().unwrap();
                self.loop_reach.pop();
                self.loop_repeats.pop();
                self.loop_marks.pop();
                for (i, b) in scope.iter_mut().enumerate() {
                    if breaks.get(i).copied().unwrap_or(false) {
                        b.assigned = true;
                    }
                }
                self.loop_depth -= 1;
                // The loop diverges when no `break` can leave it: nothing after
                // is reachable, so the state does not matter.
                if block_has_break(body) {
                    for (i, b) in scope.iter_mut().enumerate() {
                        b.init = init_before.get(i).copied().unwrap_or(false)
                            || exits.iter().all(|e| e.get(i).copied().unwrap_or(false));
                    }
                }
                Ok(())
            }
            Stmt::For { var, var_slot, start, end, end_slot, body } => {
                // The loop variable may not shadow a constant (rustc E0530).
                if self.const_names.contains(var) {
                    return Err(format!(
                        "identifier `{var}` shadows a constant of the same name"
                    ));
                }
                let ts = self.check_expr(start, scope)?;
                let te = self.check_expr(end, scope)?;
                if ts != Type::I64 || te != Type::I64 {
                    return Err(format!(
                        "`for` range bounds must be `i64`, found {ts:?} and {te:?}"
                    ));
                }
                // The end bound is evaluated once, before the loop starts.
                *end_slot = self.alloc_slot();
                let s = self.alloc_slot();
                *var_slot = s;
                let mark = scope.len();
                let init_before = Self::init_snapshot(scope);
                // The loop variable is a fresh immutable binding each iteration.
                scope.push(Binding {
                    name: var.clone(),
                    slot: s,
                    ty: Type::I64,
                    mutable: false,
                    init: true,
                    param: false,
                    assigned: true,
                });
                self.loop_depth += 1;
                self.loop_marks.push(mark);
                self.loop_repeats.push(true);
                self.loop_exits.push(Vec::new());
                self.loop_reach.push(false);
                self.break_assigned.push(Self::assigned_snapshot(scope));
                self.check_block_inner(body, scope, true)?;
                let breaks = self.break_assigned.pop().unwrap();
                self.loop_exits.pop();
                self.loop_reach.pop();
                self.loop_repeats.pop();
                self.loop_marks.pop();
                for (i, b) in scope.iter_mut().enumerate() {
                    if breaks.get(i).copied().unwrap_or(false) {
                        b.assigned = true;
                    }
                }
                self.loop_depth -= 1;
                scope.truncate(mark);
                Self::init_restore(scope, &init_before);
                Ok(())
            }
            Stmt::Return(v) => {
                match v {
                    Some(e) => {
                        let t = self.check_expr(e, scope)?;
                        if t != self.ret_ty {
                            return Err(format!(
                                "`return` type mismatch: function returns {:?}, value is {:?}",
                                self.ret_ty, t
                            ));
                        }
                    }
                    None => {
                        if self.ret_ty != Type::Unit {
                            return Err(format!(
                                "`return` without a value in a function returning {:?}",
                                self.ret_ty
                            ));
                        }
                    }
                }
                Ok(())
            }
            Stmt::Break => {
                if self.loop_depth == 0 {
                    return Err("`break` outside of a loop".into());
                }
                // Record the initialization state this exit leaves behind, so
                // the loop can require every exit to initialize a binding before
                // treating it as initialized after the loop.
                let snap = Self::init_snapshot(scope);
                self.loop_exits.last_mut().unwrap().push(snap);
                // The branch's writes are seen by the code after the loop, so
                // merge this state into the loop's break-exit state before the
                // branch's local state is discarded.
                if let Some(acc) = self.break_assigned.last_mut() {
                    let cur = Self::assigned_snapshot(scope);
                    if acc.len() == cur.len() {
                        for (a, b) in acc.iter_mut().zip(cur.iter()) {
                            *a |= *b;
                        }
                    }
                }
                Ok(())
            }
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    return Err("`continue` outside of a loop".into());
                }
                // The back edge is taken: from here control reaches the loop's
                // top again, so a binding declared before the loop is written
                // once per iteration even if the rest of the body would exit.
                if let Some(last) = self.loop_reach.last_mut() {
                    *last = true;
                }
                Ok(())
            }
        }
    }

    /// The root binding of a place must be writable: a `mut` binding, or a
    /// reference whose referent is mutable. A shared reference (`&T`) cannot be
    /// written through. With `allow_uninit`, a not-yet-initialized binding is
    /// writable too — an assignment may initialize it, whole or by field/element
    /// (rustc allows a partial initializer and only rejects reading it). A
    /// `&mut place` borrow passes `false`: borrowing an uninitialized place is
    /// itself E0381. Mirrors rustc's E0594/E0596 for the cases rmc can express.
    fn place_writable(
        &mut self,
        e: &mut Expr,
        scope: &mut Scope,
        allow_uninit: bool,
    ) -> Result<(), String> {
        match e {
            Expr::Var { name, .. } => {
                let idx = Self::binding_index(scope, name)
                    .ok_or_else(|| format!("unknown variable `{name}`"))?;
                let b = &scope[idx];
                // Reassigning the binding itself needs `mut`; writing *through*
                // a reference is decided by the field/index/deref arm, so a
                // `&mut T` binding need not itself be `mut` for `r.x = ..`.
                if b.mutable {
                    return Ok(());
                }
                // A mutable borrow (`&mut place`, `allow_uninit == false`) needs
                // the binding itself to be `mut`.
                if !allow_uninit {
                    return Err(format!(
                        "cannot assign to immutable variable `{name}` (add `mut`)"
                    ));
                }
                // A plain `let` may be written exactly once: its initialiser, or
                // its first assignment. A write inside a repeating loop to a
                // binding declared before that loop runs once per iteration, so
                // it is a second write even on the first pass. A `loop` that
                // runs at most once is exempt (its flag is `false`).
                if b.assigned
                    || self
                        .loop_marks
                        .iter()
                        .zip(self.loop_reach.iter())
                        .any(|(&m, &reach)| reach && idx < m)
                {
                    return Err(if b.param {
                        format!("cannot assign to immutable argument `{name}` (add `mut`)")
                    } else {
                        format!("cannot assign twice to immutable variable `{name}` (add `mut`)")
                    });
                }
                Ok(())
            }
            Expr::Field { base, .. } | Expr::Index(base, _) => {
                // A write into a component goes through the base's address when
                // the base is a reference, so the referent's mutability decides;
                // a shared `&T` cannot be written through at all.
                match self.check_expr(base, scope)? {
                    Type::Ref(_, true) => Ok(()),
                    Type::Ref(_, false) => {
                        Err("cannot assign through a shared reference; use `&mut`".to_string())
                    }
                    _ => self.place_writable(base, scope, allow_uninit),
                }
            }
            Expr::Deref { expr, .. } => match self.check_expr(expr, scope)? {
                Type::Ref(_, true) => Ok(()),
                Type::Ref(_, false) => {
                    Err("cannot assign through a shared reference; use `&mut`".to_string())
                }
                other => Err(format!("cannot dereference a value of type {other:?}")),
            },
            _ => Err("invalid assignment target".into()),
        }
    }

    /// Type-check an assignment target (variable, array element or field).
    /// Check a cast `from as to`, mirroring rustc's allowed conversions plus
    /// rmc's two intentional extensions. Both extensions are documented: the
    /// raw pointer cast `i64 as &T` (rustc would require `as *const T` first;
    /// the heap benchmark builds a linked list with it) and `i64 as bool`
    /// (rustc requires `!= 0`; `boolops.rms` uses it). Everything else rmc
    /// accepts here is valid Rust, so no accidental reinterpretation slips
    /// through — in particular a reference may only become a reference to the
    /// same referent, and may never gain mutability or become a scalar.
    fn check_cast(&self, from: &Type, to: &Type) -> Result<(), String> {
        match (from, to) {
            (Type::Ref(a, a_mut), Type::Ref(b, b_mut)) => {
                if a != b {
                    return Err(format!(
                        "cannot cast a `{a:?}` reference to a `{b:?}` reference"
                    ));
                }
                if *b_mut && !*a_mut {
                    return Err("cannot cast a shared reference `&T` to `&mut T`".into());
                }
                Ok(())
            }
            // A reference cannot become a scalar: `&T as i64` would expose the
            // address as an integer, which rustc forbids (E0606).
            (Type::Ref(..), _) => {
                Err(format!("cannot cast a reference `{from:?}` to `{to:?}`"))
            }
            // The raw pointer cast extension: an integer becomes a reference.
            (Type::I64, Type::Ref(..)) => Ok(()),
            // Any other scalar may not become a reference (rustc E0605).
            (_, Type::Ref(..)) => {
                Err(format!("cannot cast `{from:?}` to a reference `{to:?}`"))
            }
            // A string literal is not castable in rmc.
            (Type::Str, _) | (_, Type::Str) => {
                Err(format!("cannot cast `{from:?}` to `{to:?}`"))
            }
            // `i64`, `bool` and an enum tag convert among themselves.
            _ => Ok(()),
        }
    }

    /// Count the `=` assignments to each binding name in a function and record
    /// the names taken by `&mut`. rustc's constant propagation folds a local
    /// only when it is assigned at most once in total, so a name with a second
    /// assignment must not be folded even when a fold would be correct on the
    /// path actually taken. The scan is whole-function and order-independent:
    /// `let mut x = i64::MAX; x = x + 1;` is accepted by rustc because `x` is
    /// assigned twice, so its constant is not propagated. Keyed by name because
    /// it runs before sema assigns slots.
    fn collect_assigns(
        block: &Block,
        a: &mut HashMap<String, u32>,
        mb: &mut std::collections::HashSet<String>,
    ) {
        for st in &block.stmts {
            Self::collect_assigns_stmt(st, a, mb);
        }
        if let Some(t) = &block.tail {
            Self::collect_mut_borrows(t, a, mb);
        }
    }

    fn collect_assigns_stmt(
        st: &Stmt,
        a: &mut HashMap<String, u32>,
        mb: &mut std::collections::HashSet<String>,
    ) {
        match st {
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    Self::collect_mut_borrows(e, a, mb);
                }
            }
            Stmt::Assign { target, value } => {
                if let Some(n) = Self::place_root_name(target) {
                    *a.entry(n).or_insert(0) += 1;
                }
                Self::collect_mut_borrows(value, a, mb);
            }
            Stmt::Expr(e) => Self::collect_mut_borrows(e, a, mb),
            Stmt::While(c, b) => {
                Self::collect_mut_borrows(c, a, mb);
                Self::collect_assigns(b, a, mb);
            }
            Stmt::Loop(b) => Self::collect_assigns(b, a, mb),
            Stmt::For { start, end, body, .. } => {
                Self::collect_mut_borrows(start, a, mb);
                Self::collect_mut_borrows(end, a, mb);
                Self::collect_assigns(body, a, mb);
            }
            Stmt::Return(v) => {
                if let Some(e) = v {
                    Self::collect_mut_borrows(e, a, mb);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    /// The root name of a place expression, if it names a local.
    fn place_root_name(e: &Expr) -> Option<String> {
        match e {
            Expr::Var { name, .. } => Some(name.clone()),
            Expr::Deref { expr, .. } | Expr::Field { base: expr, .. } => Self::place_root_name(expr),
            Expr::Index(base, _) => Self::place_root_name(base),
            _ => None,
        }
    }

    /// Record every `&mut place` name, and recurse into nested blocks so their
    /// assignments are counted too.
    fn collect_mut_borrows(
        e: &Expr,
        a: &mut HashMap<String, u32>,
        mb: &mut std::collections::HashSet<String>,
    ) {
        match e {
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
            Expr::Bin(_, x, y) => {
                Self::collect_mut_borrows(x, a, mb);
                Self::collect_mut_borrows(y, a, mb);
            }
            Expr::Un(_, x) => Self::collect_mut_borrows(x, a, mb),
            Expr::Call { args, .. } => {
                for x in args {
                    Self::collect_mut_borrows(x, a, mb);
                }
            }
            Expr::If(c, t, el) => {
                Self::collect_mut_borrows(c, a, mb);
                Self::collect_assigns(t, a, mb);
                if let Some(b) = el {
                    Self::collect_assigns(b, a, mb);
                }
            }
            Expr::MatchBoundary(b, _) | Expr::Block(b) => Self::collect_assigns(b, a, mb),
            Expr::Index(base, idx) => {
                Self::collect_mut_borrows(base, a, mb);
                Self::collect_mut_borrows(idx, a, mb);
            }
            Expr::Field { base, .. } => Self::collect_mut_borrows(base, a, mb),
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    Self::collect_mut_borrows(&f.value, a, mb);
                }
            }
            Expr::ArrayLit(v, _) => Self::collect_mut_borrows(v, a, mb),
            Expr::ArrayList(vs) => {
                for v in vs {
                    Self::collect_mut_borrows(v, a, mb);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                Self::collect_mut_borrows(recv, a, mb);
                for x in args {
                    Self::collect_mut_borrows(x, a, mb);
                }
            }
            Expr::EnumCtor { args, .. } => {
                for x in args {
                    Self::collect_mut_borrows(x, a, mb);
                }
            }
            Expr::VariantTest { scrutinee, .. } => Self::collect_mut_borrows(scrutinee, a, mb),
            Expr::BoolTest { scrutinee, .. } => Self::collect_mut_borrows(scrutinee, a, mb),
            Expr::EnumPayload { recv, .. } => Self::collect_mut_borrows(recv, a, mb),
            Expr::Cast(inner, _) => Self::collect_mut_borrows(inner, a, mb),
            Expr::AddrOf { expr, mutable, .. } => {
                if *mutable {
                    if let Some(n) = Self::place_root_name(expr) {
                        mb.insert(n);
                    }
                }
                Self::collect_mut_borrows(expr, a, mb);
            }
            Expr::Deref { expr, .. } => Self::collect_mut_borrows(expr, a, mb),
        }
    }

    /// The constant value of an expression: a literal (with `const_i64`'s
    /// arithmetic folding), a constant local read, or an operator applied to
    /// two constants. `snap` is the current `const_locals`, passed so this can
    /// be a pure function over an immutable snapshot (it is called while the
    /// AST is mutably borrowed).
    fn const_of(&self, e: &Expr, snap: &HashMap<i32, i64>) -> Option<i64> {
        const_i64(e).or_else(|| match e {
            Expr::Var { slot, ty, .. } if *ty == Type::I64 && *slot >= 0 => snap.get(slot).copied(),
            Expr::Bin(op, a, b) => {
                let x = self.const_of(a, snap)?;
                let y = self.const_of(b, snap)?;
                const_binop_checked(*op, x, y)
            }
            _ => None,
        })
    }

    fn check_lvalue(&mut self, e: &mut Expr, scope: &mut Scope) -> Result<Type, String> {
        match e {
            Expr::Var { .. } | Expr::Field { .. } => self.check_expr(e, scope),
            Expr::Deref { .. } => self.check_expr(e, scope),
            Expr::Index(base, idx) => {
                let bt = deref_ty(self.check_expr(base, scope)?);
                let it = self.check_expr(idx, scope)?;
                if it != Type::I64 {
                    return Err(format!("array index must be `i64`, found {it:?}"));
                }
                match bt {
                    Type::Array(elem, n) => {
                        // A constant index outside the array is a compile error,
                        // exactly as rustc rejects it. A non-constant index is
                        // checked at runtime (when bounds checks are on), so it
                        // stays a runtime concern here.
                        if let Some(k) = self.const_of(idx, &self.const_locals) {
                            if k < 0 || k as usize >= n {
                                return Err(format!(
                                    "index {k} is out of bounds for an array of length {n}"
                                ));
                            }
                        }
                        Ok(*elem)
                    }
                    other => Err(format!("cannot index a value of type {other:?}")),
                }
            }
            _ => Err("invalid assignment target".into()),
        }
    }

    fn check_expr(&mut self, e: &mut Expr, scope: &mut Scope) -> Result<Type, String> {
        match e {
            Expr::Int(_) => Ok(Type::I64),
            Expr::Bool(_) => Ok(Type::Bool),
            // A string literal has its own type, so it cannot be added to an
            // integer or passed where an `i64` is expected.
            Expr::Str(_) => Ok(Type::Str),
            Expr::Var { name, slot, ty } => match Self::lookup(scope, name) {
                Some((s, t, _mut, init)) => {
                    if !init {
                        return Err(format!(
                            "use of possibly-uninitialized variable `{name}`"
                        ));
                    }
                    *slot = s;
                    *ty = t.clone();
                    Ok(t)
                }
                None => Err(format!("unknown variable `{name}`")),
            },
            Expr::Un(op, inner) => {
                let t = self.check_expr(inner, scope)?;
                match op {
                    UnOp::Neg => {
                        if t != Type::I64 {
                            return Err(format!("unary `-` expects `i64`, found {t:?}"));
                        }
                        // Negating `i64::MIN` overflows. rustc's
                        // `arithmetic_overflow` deny lint catches it when the
                        // operand folds to a constant, so rmc does too; the
                        // `i64::MIN` literal itself is parsed as `Int(i64::MIN)`,
                        // not a negation, so only a written `-` reaches here.
                        if self.const_of(inner, &self.const_locals) == Some(i64::MIN) {
                            return Err("this arithmetic operation overflows".into());
                        }
                        Ok(Type::I64)
                    }
                    UnOp::Not => match t {
                        // `!` is logical not on `bool` and bitwise not on an
                        // integer, matching Rust. It yields its operand's type.
                        Type::Bool | Type::I64 => Ok(t),
                        _ => Err(format!("unary `!` expects `bool` or `i64`, found {t:?}")),
                    },
                }
            }
            Expr::Bin(op, a, b) => {
                let ta = self.check_expr(a, scope)?;
                let tb = self.check_expr(b, scope)?;
                use BinOp::*;
                match op {
                    Add | Sub | Mul | Div | Mod => {
                        if ta != Type::I64 || tb != Type::I64 {
                            return Err(format!("arithmetic expects `i64` operands, found {ta:?} and {tb:?}"));
                        }
                        if let (Some(x), Some(y)) = (self.const_of(a, &self.const_locals), self.const_of(b, &self.const_locals)) {
                            const_binop(*op, x, y)?;
                        }
                        Ok(Type::I64)
                    }
                    Eq | Ne => {
                        if ta != tb {
                            return Err(format!("comparison between different types: {ta:?} and {tb:?}"));
                        }
                        // `is_scalar` here, so a matching enum compares but an
                        // aggregate (array/struct) does not.
                        if !ta.is_scalar() {
                            return Err(format!(
                                "comparison expects `i64`, `bool` or a matching enum, found {ta:?}"
                            ));
                        }
                        Ok(Type::Bool)
                    }
                    Lt | Le | Gt | Ge => {
                        if ta != Type::I64 || tb != Type::I64 {
                            return Err(format!("ordering comparison expects `i64`, found {ta:?} and {tb:?}"));
                        }
                        Ok(Type::Bool)
                    }
                    And | Or => {
                        if ta != Type::Bool || tb != Type::Bool {
                            return Err(format!("logical operator expects `bool`, found {ta:?} and {tb:?}"));
                        }
                        Ok(Type::Bool)
                    }
                    BitAnd | BitOr | BitXor | Shl | Shr => {
                        if ta != Type::I64 || tb != Type::I64 {
                            return Err(format!("bitwise operator expects `i64` operands, found {ta:?} and {tb:?}"));
                        }
                        // rustc rejects a shift whose count is a known constant
                        // outside 0..=63 even when the shifted value is a
                        // runtime value (`x << 81` is a compile error, not a
                        // wrapped result), so the count is checked on its own
                        // rather than only when both sides are constant.
                        if matches!(op, Shl | Shr) {
                            if let Some(y) = self.const_of(b, &self.const_locals) {
                                if !(0..64).contains(&y) {
                                    return Err("shift amount is out of range for `i64`".into());
                                }
                            }
                        }
                        if let (Some(x), Some(y)) = (self.const_of(a, &self.const_locals), self.const_of(b, &self.const_locals)) {
                            const_binop(*op, x, y)?;
                        }
                        Ok(Type::I64)
                    }
                }
            }
            Expr::Call { name, args, arg_slots, ret_slot, ret_ty } => {
                let sig = self
                    .sigs
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("call to unknown function `{name}`"))?;
                if args.len() != sig.params.len() {
                    return Err(format!(
                        "function `{name}` expects {} argument(s), got {}",
                        sig.params.len(),
                        args.len()
                    ));
                }
                arg_slots.clear();
                for (a, p) in args.iter_mut().zip(sig.params.iter()) {
                    let t = self.check_expr(a, scope)?;
                    if &t != p && !ref_coerces(&t, p) {
                        return Err(format!("function `{name}`: argument type mismatch, expected {p:?}, found {t:?}"));
                    }
                    // An aggregate argument is passed by pointer, so give it a
                    // hidden frame slot to hold its address for the call.
                    arg_slots.push(if p.is_aggregate() {
                        let n = p.size() / 8;
                        (self.alloc_slots(n as i32), n)
                    } else {
                        (-1, 0)
                    });
                }
                *ret_ty = sig.ret.clone();
                *ret_slot = if sig.ret.is_aggregate() {
                    self.alloc_slots((sig.ret.size() / 8) as i32)
                } else {
                    -1
                };
                Ok(sig.ret)
            }
            Expr::If(cond, then, els) => {
                // A desugared `match` is a chain of `if`s. The first step is an
                // ordinary `if`; the steps in its `else` chain are flagged with
                // `match_chain`, and inherit `match_total`, which records
                // whether the whole chain names every variant of one enum. A
                // total chain may stand for a value even though its last step
                // has no `else`.
                let is_root = !self.match_chain;
                let total = self.match_total;
                self.match_chain = false;
                self.match_total = false;
                let ct = self.check_expr(cond, scope)?;
                if ct != Type::Bool {
                    return Err(format!("`if` condition must be `bool`, found {ct:?}"));
                }
                let total = if is_root {
                    self.if_chain_is_exhaustive(cond, els)
                } else {
                    total
                };
                // Track definite initialization across the branches: a binding
                // is initialized afterwards only if it was before, or on both
                // paths. `init_before` is the state on entry to the branch.
                let init_before = Self::init_snapshot(scope);
                let assigned_before = Self::assigned_snapshot(scope);
                let tt = self.check_block(then, scope)?;
                let then_init = Self::init_snapshot(scope);
                let then_assigned = Self::assigned_snapshot(scope);
                match els {
                    Some(e) => {
                        // Only a total chain's `else` continues the chain: an
                        // all-variant `match` has no wildcard body, so the else
                        // block is always the next arm's test. A non-total chain
                        // may end in a wildcard body that is itself an `if`, and
                        // that nested `if` must be checked as an ordinary root.
                        self.match_chain = total;
                        self.match_total = total;
                        // Restore the entry state before the else, so the else
                        // sees the same scope the then saw.
                        Self::init_restore(scope, &init_before);
                        Self::assigned_restore(scope, &assigned_before);
                        let et = self.check_block(e, scope)?;
                        self.match_chain = false;
                        self.match_total = false;
                        if tt != et {
                            return Err(format!("`if` branches have different types: {tt:?} and {et:?}"));
                        }
                        // Keep only what every path that reaches the code
                        // after the `if` initialized. An arm that
                        // returns/breaks/continues cannot reach it, so its
                        // state must not be ANDed in: only fall-through arms
                        // decide the merged state.
                        let else_init = Self::init_snapshot(scope);
                        let then_div = block_diverges(then);
                        let else_div = block_diverges(e);
                        for (i, b) in scope.iter_mut().enumerate() {
                            let t = then_init.get(i).copied().unwrap_or(false);
                            let el = else_init.get(i).copied().unwrap_or(false);
                            b.init = match (then_div, else_div) {
                                (false, false) => t && el,
                                (false, true) => t,
                                (true, false) => el,
                                // Both arms diverge: nothing after is reachable.
                                (true, true) => false,
                            };
                        }
                        // A binding counts as written if a branch that can reach
                        // the next statement wrote it. An arm that
                        // returns/breaks/continues cannot, so its write must not
                        // count: only fall-through arms decide the merged state.
                        let else_assigned = Self::assigned_snapshot(scope);
                        match (then_div, else_div) {
                            (false, false) => {
                                Self::assigned_merge(scope, &then_assigned, &else_assigned)
                            }
                            (true, false) => Self::assigned_restore(scope, &else_assigned),
                            (false, true) => Self::assigned_restore(scope, &then_assigned),
                            // Both arms diverge: nothing after the `if` is reachable.
                            (true, true) => {}
                        }
                        Ok(tt)
                    }
                    None => {
                        if tt != Type::Unit && !total {
                            return Err(format!(
                                "`if` without `else` must evaluate to `()`, but the branch is {tt:?}"
                            ));
                        }
                        // No else: the then branch may not run. If it can fall
                        // through, its writes count (a may-analysis); if it
                        // diverges, the fall-through keeps the incoming state.
                        Self::init_restore(scope, &init_before);
                        if block_diverges(then) {
                            Self::assigned_restore(scope, &assigned_before);
                        } else {
                            Self::assigned_merge(scope, &then_assigned, &assigned_before);
                        }
                        Ok(tt)
                    }
                }
            }
            // The start of a desugared `match`. It is transparent: check the
            // chain inside as an ordinary expression, but clear any inherited
            // chain context first so the inner chain is judged on its own. This
            // is what stops a wildcard arm's nested `match` from borrowing the
            // outer match's exhaustiveness.
            Expr::MatchBoundary(b, has_catch_all) => {
                let outer_chain = self.match_chain;
                let outer_total = self.match_total;
                self.match_chain = false;
                self.match_total = false;
                // Every `match` must be exhaustive, in any position: rustc
                // rejects a non-exhaustive one with E0004, even in statement
                // position where the fall-through to `()` is harmless. A `_` or
                // binding arm is a catch-all; otherwise the arms must name every
                // variant of one enum. Decide it before checking the body, while
                // the `VariantTest` tags are still unset, so the walk compares
                // variant *names*.
                let exhaustive = *has_catch_all || self.match_chain_is_exhaustive(b);
                let t = self.check_block(b, scope);
                self.match_chain = outer_chain;
                self.match_total = outer_total;
                // A non-exhaustive match in *value* position already failed with
                // the "`if` without `else`" error inside `check_block`; that
                // message is more specific, so let it win. This only adds the
                // statement-position case, where the arms are unit-typed and the
                // chain checked clean.
                let t = t?;
                if !exhaustive {
                    return Err(
                        "non-exhaustive `match`: add the missing variant arm or a `_` arm"
                            .to_string(),
                    );
                }
                Ok(t)
            }
            Expr::Block(b) => self.check_block(b, scope),
            Expr::Index(base, idx) => {
                let bt = deref_ty(self.check_expr(base, scope)?);
                let it = self.check_expr(idx, scope)?;
                if it != Type::I64 {
                    return Err(format!("array index must be `i64`, found {it:?}"));
                }
                match bt {
                    Type::Array(elem, n) => {
                        // A constant index outside the array is a compile error,
                        // exactly as rustc rejects it. A non-constant index is
                        // checked at runtime (when bounds checks are on), so it
                        // stays a runtime concern here.
                        if let Some(k) = self.const_of(idx, &self.const_locals) {
                            if k < 0 || k as usize >= n {
                                return Err(format!(
                                    "index {k} is out of bounds for an array of length {n}"
                                ));
                            }
                        }
                        Ok(*elem)
                    }
                    other => Err(format!("cannot index a value of type {other:?}")),
                }
            }
            Expr::Field { base, name, offset, ty } => {
                let bt = deref_ty(self.check_expr(base, scope)?);
                match bt {
                    Type::Struct(sname, _) => {
                        let info = self.structs.get(&sname).cloned().unwrap();
                        match info.fields.iter().find(|(f, _, _)| f == name) {
                            Some((_, fty, off)) => {
                                *offset = *off as i32;
                                *ty = fty.clone();
                                Ok(fty.clone())
                            }
                            None => Err(format!("struct `{sname}` has no field `{name}`")),
                        }
                    }
                    other => Err(format!("field access on a value of type {other:?}")),
                }
            }
            Expr::StructLit { name, fields, slot } => {
                let info = self
                    .structs
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown struct `{name}`"))?;
                if fields.len() != info.fields.len() {
                    return Err(format!(
                        "struct `{name}` has {} field(s), but {} were given",
                        info.fields.len(),
                        fields.len()
                    ));
                }
                // Type-check each field and attach its layout offset, but keep
                // the fields in source order: initializer side effects must run
                // in the order written, not in declaration order.
                let mut seen = vec![false; info.fields.len()];
                for fi in fields.iter_mut() {
                    let idx = info
                        .fields
                        .iter()
                        .position(|(f, _, _)| *f == fi.name)
                        .ok_or_else(|| format!("struct `{name}` has no field `{}`", fi.name))?;
                    if seen[idx] {
                        return Err(format!("struct `{name}`: field `{}` given twice", fi.name));
                    }
                    seen[idx] = true;
                    let want = info.fields[idx].1.clone();
                    let got = self.check_expr(&mut fi.value, scope)?;
                    if got != want {
                        return Err(format!(
                            "struct `{name}` field `{}`: expected {want:?}, found {got:?}",
                            fi.name
                        ));
                    }
                    fi.offset = info.fields[idx].2 as i32;
                    fi.ty = want;
                }
                // A frame buffer so the literal has an address in any context.
                *slot = self.alloc_slots((info.size / 8) as i32);
                Ok(Type::Struct(name.clone(), info.size))
            }
            Expr::ArrayLit(val, n) => {
                let t = self.check_expr(val, scope)?;
                if !t.is_int_or_bool() {
                    return Err(format!("array elements must be `i64`, `bool` or a unit enum, found {t:?}"));
                }
                let ty = Type::Array(Box::new(t), *n);
                // Value position: `[v; n]` has no address of its own, so rewrite
                // it to a block that builds a local array and yields it. This
                // reuses the `let`-initialiser splat path (a single fill loop
                // for a large `n`), and both slots are resolved when the block
                // is re-checked below. Without this a repeat literal used as an
                // index base, argument, return or field would be emitted as a
                // bare scalar and dereferenced as an address.
                let arr_slot = self.alloc_slots(*n as i32);
                let name = "\u{0}rep".to_string();
                let var = Expr::Var { name: name.clone(), slot: arr_slot, ty: ty.clone() };
                let stmts = vec![Stmt::Let {
                    name: name.clone(),
                    ty: Some(ty.clone()),
                    init: Some(Expr::ArrayLit(val.clone(), *n)),
                    slot: arr_slot,
                    mutable: false,
                }];
                *e = Expr::Block(Block { stmts, tail: Some(Box::new(var)) });
                // Re-check the rewritten block so its statements are typed and
                // slotted like any other.
                self.check_expr(e, scope)
            }
            Expr::ArrayList(elems) => {
                // The parser never produces an empty list: it requires at least
                // one element before the `]`, so `[]` fails there.
                let elem_ty = self.check_expr(&mut elems[0], scope)?;
                if !elem_ty.is_int_or_bool() {
                    return Err(format!(
                        "array elements must be `i64`, `bool` or a unit enum, found {elem_ty:?}"
                    ));
                }
                for e in elems.iter_mut().skip(1) {
                    let t = self.check_expr(e, scope)?;
                    if t != elem_ty {
                        return Err(format!(
                            "array literal: element type mismatch, expected {elem_ty:?}, found {t:?}"
                        ));
                    }
                }
                let n = elems.len();
                let arr_ty = Type::Array(Box::new(elem_ty), n);
                // Build `{ let mut __list: [T; n]; __list[0] = e0; ...; __list }`.
                // The `let` is left uninitialised on purpose: every element is
                // written by an assignment, and an uninitialised local needs no
                // default value (which an aggregate element type has no way to
                // spell). Codegen zero-fills a scalar array anyway.
                let arr_slot = self.alloc_slots(n as i32);
                let name = "\u{0}list".to_string();
                let mut stmts = vec![Stmt::Let {
                    name: name.clone(),
                    ty: Some(arr_ty.clone()),
                    init: None,
                    slot: arr_slot,
                    mutable: true,
                }];
                for (i, e) in elems.drain(..).enumerate() {
                    stmts.push(Stmt::Assign {
                        target: Expr::Index(
                            Box::new(Expr::Var {
                                name: name.clone(),
                                slot: arr_slot,
                                ty: arr_ty.clone(),
                            }),
                            Box::new(Expr::Int(i as i64)),
                        ),
                        value: e,
                    });
                }
                *e = Expr::Block(Block {
                    stmts,
                    tail: Some(Box::new(Expr::Var {
                        name,
                        slot: arr_slot,
                        ty: arr_ty.clone(),
                    })),
                });
                // Every piece is already resolved: the elements were checked
                // above, and the `let` and assignments were built with their
                // slots. Re-checking the block would allocate a *new* slot for
                // the `let` and leave the assignments pointing at the old one.
                Ok(arr_ty)
            }
            Expr::MethodCall { recv, name, args } => {
                // Resolve the receiver's type, then look the method up as the
                // mangled free function `Type$method` the parser produced.
                // The receiver is checked twice — once here for its type, once
                // again as the first argument of the rewritten call. Snapshot the
                // assignment flags so the second check sees the receiver's writes
                // exactly once, not twice.
                let assigned_before = Self::assigned_snapshot(scope);
                let rt = self.check_expr(recv, scope)?;
                // Reading a method off a reference auto-derefs, so `r.m()`
                // resolves against the referent's type.
                let via_ref = matches!(rt, Type::Ref(..));
                let base_ty = deref_ty(rt.clone());
                // A method lives on a struct or an enum; both give the mangled
                // prefix `Type$method`.
                let tyname = match &base_ty {
                    Type::Struct(n, _) | Type::Enum(n) | Type::EnumData(n, _) => n.clone(),
                    other => {
                        return Err(format!(
                            "method `{name}` called on a value of type {other:?}, which is not a struct or enum"
                        ))
                    }
                };
                let mangled = format!("{tyname}${name}");
                let sig = match self.sigs.get(&mangled).cloned() {
                    Some(s) => s,
                    None => return Err(format!("no method `{name}` on `{tyname}`")),
                };
                // Rewrite into a call with the receiver as the first argument.
                Self::assigned_restore(scope, &assigned_before);
                let recv = std::mem::replace(recv.as_mut(), Expr::Int(0));
                // Match the receiver's form to the method's. A `&self` method
                // takes an address, so a large receiver is not copied; `self`
                // takes the value. A receiver that is already a reference is
                // passed through for `&self`, and dereferenced (copied) for
                // `self`.
                let recv = match (sig.ref_self, via_ref) {
                    (true, false) => Expr::AddrOf {
                        expr: Box::new(recv),
                        ty: Type::Named(String::new()),
                        mutable: sig.ref_self_mut,
                        slot: -1,
                    },
                    (false, true) => Expr::Deref { expr: Box::new(recv), ty: Type::Named(String::new()) },
                    _ => recv,
                };
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(recv);
                call_args.append(args);
                *e = Expr::Call {
                    name: mangled,
                    args: call_args,
                    arg_slots: Vec::new(),
                    ret_slot: -1,
                    ret_ty: Type::I64,
                };
                self.check_expr(e, scope)
            }
            Expr::EnumVariant { enum_name, variant, tag } => {
                let enum_name = enum_name.clone();
                let variant = variant.clone();
                let info = self
                    .enums
                    .get(&enum_name)
                    .ok_or_else(|| format!("unknown enum `{enum_name}`"))?
                    .clone();
                let (_, fields, _) = info
                    .variant(&variant)
                    .ok_or_else(|| format!("enum `{enum_name}` has no variant `{variant}`"))?;
                if !fields.is_empty() {
                    return Err(format!(
                        "variant `{enum_name}::{variant}` takes {} argument(s)",
                        fields.len()
                    ));
                }
                // Record the tag in the node rather than rewriting to an
                // `Int`: keeping the node keeps its enum type, so re-checking
                // it (as a method receiver is) stays consistent.
                let t = info.tag_of(&variant).unwrap();
                *tag = t;
                if info.has_payload {
                    // A payload enum has no bare-variant value: every value
                    // needs a frame buffer, since it may carry a payload. Rewrite
                    // to a zero-argument constructor, which materialises one.
                    let size = info.size;
                    let slot = self.alloc_slots((size / 8) as i32);
                    *e = Expr::EnumCtor {
                        enum_name: enum_name.clone(),
                        variant,
                        tag: t,
                        args: Vec::new(),
                        arg_offsets: Vec::new(),
                        slot,
                        size,
                    };
                    Ok(Type::EnumData(enum_name, size))
                } else {
                    Ok(Type::Enum(enum_name))
                }
            }
            Expr::EnumCtor { enum_name, variant, tag, args, arg_offsets, slot, size } => {
                let enum_name = enum_name.clone();
                let variant = variant.clone();
                let info = self
                    .enums
                    .get(&enum_name)
                    .ok_or_else(|| format!("unknown enum `{enum_name}`"))?
                    .clone();
                let (_, fields, _) = info
                    .variant(&variant)
                    .ok_or_else(|| format!("enum `{enum_name}` has no variant `{variant}`"))?
                    .clone();
                if args.len() != fields.len() {
                    return Err(format!(
                        "variant `{enum_name}::{variant}` takes {} argument(s), got {}",
                        fields.len(),
                        args.len()
                    ));
                }
                // A payload enum is a tagged union; a variant constructor is
                // only meaningful on one, and only ever builds it.
                if !info.has_payload {
                    return Err(format!("enum `{enum_name}` has no payload variants"));
                }
                // Type-check the arguments in source order (side effects must
                // run as written) and record each field's offset in the payload.
                let mut offsets = Vec::with_capacity(fields.len());
                for (i, arg) in args.iter_mut().enumerate() {
                    let got = self.check_expr(arg, scope)?;
                    let (want, off) = fields[i].clone();
                    if got != want {
                        return Err(format!(
                            "variant `{enum_name}::{variant}` field {i}: expected {want:?}, found {got:?}"
                        ));
                    }
                    offsets.push(off as i32);
                }
                *tag = info.tag_of(&variant).unwrap();
                *arg_offsets = offsets;
                *size = info.size;
                // A frame buffer for the value, like a struct literal, so it has
                // an address in any context.
                *slot = self.alloc_slots((info.size / 8) as i32);
                Ok(Type::EnumData(enum_name, info.size))
            }
            Expr::BoolTest { scrutinee, value } => {
                let st = self.check_expr(scrutinee, scope)?;
                if st != Type::Bool {
                    return Err(format!(
                        "`{value}` pattern cannot match a value of type {st:?}"
                    ));
                }
                Ok(Type::Bool)
            }
            Expr::VariantTest { scrutinee, enum_name, variant, tag, n_binds } => {
                let n_binds = *n_binds;
                let st = self.check_expr(scrutinee, scope)?;
                let ename = match &st {
                    Type::Enum(n) | Type::EnumData(n, _) => n.clone(),
                    other => {
                        return Err(format!(
                            "`{enum_name}::{variant}` pattern cannot match a value of type {other:?}"
                        ))
                    }
                };
                if ename != *enum_name {
                    return Err(format!(
                        "`{enum_name}::{variant}` pattern cannot match a value of enum `{ename}`"
                    ));
                }
                let info = self.enums.get(&ename).unwrap().clone();
                let (_, fields, _) = info
                    .variant(variant)
                    .ok_or_else(|| format!("enum `{enum_name}` has no variant `{variant}`"))?;
                // A variant that carries a payload must be destructured, so the
                // pattern must name its fields; and it must name all of them.
                if fields.is_empty() {
                    if n_binds > 0 {
                        return Err(format!(
                            "variant `{enum_name}::{variant}` takes no arguments"
                        ));
                    }
                } else if n_binds < 0 {
                    return Err(format!(
                        "variant `{enum_name}::{variant}` has a payload; bind it with `{enum_name}::{variant}(..)`"
                    ));
                } else if n_binds as usize != fields.len() {
                    return Err(format!(
                        "variant `{enum_name}::{variant}` has {} field(s), but the pattern binds {n_binds}",
                        fields.len()
                    ));
                }
                *tag = info.tag_of(variant).unwrap();
                Ok(Type::Bool)
            }
            Expr::EnumPayload { recv, enum_name, variant, idx, offset, ty } => {
                let enum_name = enum_name.clone();
                let variant = variant.clone();
                let idx = *idx;
                let rt = self.check_expr(recv, scope)?;
                let info = self
                    .enums
                    .get(&enum_name)
                    .ok_or_else(|| format!("unknown enum `{enum_name}`"))?
                    .clone();
                if !matches!(&rt, Type::EnumData(n, _) if *n == enum_name) {
                    return Err(format!(
                        "`{enum_name}::{variant}` payload read on a value of type {rt:?}"
                    ));
                }
                let (_, fields, _) = info
                    .variant(&variant)
                    .ok_or_else(|| format!("enum `{enum_name}` has no variant `{variant}`"))?;
                let (fty, foff) = fields
                    .get(idx)
                    .ok_or_else(|| {
                        format!(
                            "variant `{enum_name}::{variant}` has {} field(s), but field {idx} was read",
                            fields.len()
                        )
                    })?
                    .clone();
                // The payload sits after the tag word.
                *offset = foff as i32 + 8;
                *ty = fty.clone();
                Ok(fty)
            }
            Expr::AddrOf { expr, ty, mutable, slot } => {
                // A reference is one word, whether it points at a scalar or an
                // aggregate. An lvalue is addressed directly; anything else has
                // no address of its own, so it is checked as a value and given a
                // hidden frame slot to be materialised into.
                let t = if is_lvalue(expr) {
                    // `&mut place` needs a mutable place; rustc rejects
                    // `&mut x` on an immutable binding (E0596). A non-lvalue
                    // operand is materialised into a temporary, which is a
                    // fresh mutable slot, so it is always borrowable.
                    if *mutable {
                        self.place_writable(expr, scope, false)?;
                    }
                    self.check_lvalue(expr, scope)?
                } else {
                    let t = self.check_expr(expr, scope)?;
                    *slot = self.alloc_slots((t.size() / 8).max(1) as i32);
                    t
                };
                *ty = t.clone();
                Ok(Type::Ref(Box::new(t), *mutable))
            }
            Expr::Deref { expr, ty } => {
                let it = self.check_expr(expr, scope)?;
                match it {
                    Type::Ref(inner, _) => {
                        *ty = (*inner).clone();
                        Ok(*inner)
                    }
                    other => Err(format!("cannot dereference a value of type {other:?}")),
                }
            }
            Expr::Cast(inner, to) => {
                let it = self.check_expr(inner, scope)?;
                // An enum may be cast to `i64` (its tag), but not the reverse:
                // rustc rejects `i64 as Enum`, and rmc is a subset of Rust, so
                // it rejects it too rather than inventing a conversion.
                if let Type::Named(n) = to.clone() {
                    if self.enums.contains_key(&n) {
                        return Err(format!(
                            "cannot cast to enum `{n}`; cast the other way (`e as i64`) or use `match`"
                        ));
                    }
                }
                let rt = self.resolve_ty(to)?;
                // Only scalars can be cast. Casting an integer to an aggregate
                // would let codegen reinterpret the integer as an address and
                // read arbitrary memory.
                if !it.is_scalar() || !rt.is_scalar() {
                    return Err(format!("cannot cast {it:?} to {rt:?}"));
                }
                self.check_cast(&it, &rt)?;
                *to = rt.clone();
                Ok(rt)
            }
        }
    }
}

/// True when a value of type `found` may be used where `expected` is wanted.
/// The only coercion rmc performs is `&mut T` to `&T`: both are one pointer, and
/// Rust allows it, so a `&mut` receiver can be read by a `&self` method.
fn ref_coerces(found: &Type, expected: &Type) -> bool {
    match (found, expected) {
        (Type::Ref(f, true), Type::Ref(e, false)) => f == e,
        _ => false,
    }
}

/// The type a reference points at, or the type itself when it is not a
/// reference. Reading through a reference (`r.x`, `r[i]`) does not need an
/// explicit `*r`, so every base that reaches a place goes through here.
fn deref_ty(t: Type) -> Type {
    match t {
        Type::Ref(inner, _) => *inner,
        other => other,
    }
}

/// True when `e` denotes a place with an address: a variable, a field of a
/// place, an element of a place, or a dereference. `&e` on such an expression is
/// its address; anything else (a literal, a call, arithmetic) has no address and
/// must be materialised into a temporary first.
fn is_lvalue(e: &Expr) -> bool {
    match e {
        Expr::Var { .. } | Expr::Deref { .. } => true,
        Expr::Field { base, .. } => is_lvalue(base),
        Expr::Index(base, _) => is_lvalue(base),
        _ => false,
    }
}

/// True when control cannot reach the end of `b`: its tail diverges, or its
/// last statement does. Used to accept a value-returning function whose body
/// ends in `return` on every path, and reject one that can fall off the end.
fn block_diverges(b: &Block) -> bool {
    // Walk in execution order: a statement that unconditionally diverges ends
    // the block, so everything after it (including the tail) is unreachable and
    // must not decide the result. `fn f() -> i64 { return 7; print_i64(9); }`
    // diverges at the `return`.
    for s in &b.stmts {
        if stmt_diverges(s) {
            return true;
        }
    }
    b.tail.as_deref().map_or(false, expr_diverges)
}

fn stmt_diverges(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => true,
        // Only reachable inside a loop, so it never ends a function body, but
        // treating it as diverging keeps the walk total.
        Stmt::Break | Stmt::Continue => true,
        Stmt::Expr(e) => expr_diverges(e),
        // A `let`/assignment whose initialiser diverges never completes, so the
        // following statements are unreachable.
        Stmt::Let { init: Some(e), .. } => expr_diverges(e),
        Stmt::Assign { value, .. } => expr_diverges(value),
        // A loop is not a guaranteed return: it may break or never run.
        _ => false,
    }
}

fn expr_diverges(e: &Expr) -> bool {
    match e {
        Expr::Block(b) => block_diverges(b),
        Expr::MatchBoundary(b, _) => block_diverges(b),
        // Both arms must diverge for the `if` to diverge.
        Expr::If(_, t, els) => {
            block_diverges(t) && els.as_ref().map_or(false, |b| block_diverges(b))
        }
        _ => false,
    }
}

/// True when a `loop` body can reach its own top again, so a write to a binding
/// declared before the loop repeats and rustc rejects it. That happens when some
/// path completes the body normally (which loops back) or hits a `continue` for
/// this loop. A body whose every path `break`s or `return`s runs at most once.
fn block_reaches_top(b: &Block) -> bool {
    block_reaches_top_from(b, true)
}

/// `reach[i]` is true when some path from statement `i` onward (then the tail,
/// then the continuation `after`) can reach the loop's top.
fn block_repeat_suffixes(b: &Block, after: bool) -> Vec<bool> {
    let n = b.stmts.len();
    let mut out = vec![false; n + 1];
    // The block's own tail is the last thing evaluated before the continuation,
    // so the reach after the statements is the tail's reach, not `after`.
    let mut reach = match &b.tail {
        Some(t) => expr_reaches_top(t, after),
        None => after,
    };
    out[n] = reach;
    for i in (0..n).rev() {
        reach = stmt_reaches_top(&b.stmts[i]) || (stmt_can_fall(&b.stmts[i]) && reach);
        out[i] = reach;
    }
    out
}

/// True when some path from the start of `b` reaches the loop's top, given the
/// reach state `after` that holds once the block completes.
fn block_reaches_top_from(b: &Block, after: bool) -> bool {
    let mut reach = match &b.tail {
        Some(t) => expr_reaches_top(t, after),
        None => after,
    };
    for i in (0..b.stmts.len()).rev() {
        reach = stmt_reaches_top(&b.stmts[i]) || (stmt_can_fall(&b.stmts[i]) && reach);
    }
    reach
}

/// True when evaluating `e` can reach the loop's top: it contains a `continue`,
/// or it can complete normally and `after` reaches the top.
fn expr_reaches_top(e: &Expr, after: bool) -> bool {
    expr_contains_continue(e, None, 0) || (expr_can_fall(e) && after)
}

/// True when `e` can complete normally (produce a value) on some path, i.e. it
/// does not always `break`/`return`/`continue` first.
fn expr_can_fall(e: &Expr) -> bool {
    match e {
        Expr::Block(b) => block_can_fall(b),
        Expr::MatchBoundary(b, _) => block_can_fall(b),
        Expr::If(cond, t, els) => {
            expr_can_fall(cond)
                && (block_can_fall(t) || els.as_ref().map_or(true, |e| block_can_fall(e)))
        }
        Expr::Bin(op, a, b) => {
            use BinOp::*;
            match op {
                And | Or => expr_can_fall(a),
                _ => expr_can_fall(a) && expr_can_fall(b),
            }
        }
        Expr::Un(_, a) => expr_can_fall(a),
        Expr::Index(a, b) => expr_can_fall(a) && expr_can_fall(b),
        Expr::Field { base, .. } => expr_can_fall(base),
        Expr::Cast(a, _) => expr_can_fall(a),
        Expr::AddrOf { expr, .. } => expr_can_fall(expr),
        Expr::Deref { expr, .. } => expr_can_fall(expr),
        Expr::BoolTest { scrutinee, .. } => expr_can_fall(scrutinee),
        Expr::Call { args, .. } => args.iter().all(expr_can_fall),
        Expr::StructLit { fields, .. } => fields.iter().all(|f| expr_can_fall(&f.value)),
        Expr::ArrayLit(v, _) => expr_can_fall(v),
        Expr::ArrayList(xs) => xs.iter().all(expr_can_fall),
        Expr::MethodCall { recv, args, .. } => expr_can_fall(recv) && args.iter().all(expr_can_fall),
        Expr::EnumCtor { args, .. } => args.iter().all(expr_can_fall),
        Expr::VariantTest { scrutinee, .. } => expr_can_fall(scrutinee),
        Expr::EnumPayload { recv, .. } => expr_can_fall(recv),
        _ => true,
    }
}

/// True when every path through `b` reaches the end of the block.
fn block_can_fall(b: &Block) -> bool {
    for s in &b.stmts {
        if !stmt_can_fall(s) {
            return false;
        }
    }
    b.tail.as_deref().map_or(true, expr_can_fall)
}

/// True when control from `s` can reach the statement after it.
fn stmt_can_fall(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => false,
        Stmt::Break | Stmt::Continue => false,
        Stmt::Expr(e) => expr_can_fall(e),
        Stmt::Let { init: Some(e), .. } => expr_can_fall(e),
        Stmt::Assign { value, .. } => expr_can_fall(value),
        _ => true,
    }
}

/// True when `s` can reach the loop's top directly: it contains a `continue` for
/// this loop, or a `return` whose value continues back to the top.
fn stmt_reaches_top(s: &Stmt) -> bool {
    match s {
        Stmt::Continue => true,
        Stmt::Return(Some(e)) => expr_contains_continue(e, None, 0),
        Stmt::Expr(e) => expr_contains_continue(e, None, 0),
        Stmt::Let { init: Some(e), .. } => expr_contains_continue(e, None, 0),
        Stmt::Assign { value, .. } => expr_contains_continue(value, None, 0),
        _ => false,
    }
}

/// True when `b` contains a `continue` that reaches the top of the loop `depth`
/// levels out (0 = the innermost loop) with the given label. A block that merely
/// falls through its end does not count here: only the loop body's own end loops
/// back. `break`/`return` end the path first, so a statement after them cannot
/// contribute.
fn block_contains_continue(b: &Block, label: Option<&str>, depth: usize) -> bool {
    for s in &b.stmts {
        if stmt_contains_continue(s, label, depth) {
            return true;
        }
        if stmt_diverges(s) {
            return false;
        }
    }
    b.tail
        .as_deref()
        .map_or(false, |t| expr_contains_continue(t, label, depth))
}

fn stmt_contains_continue(s: &Stmt, label: Option<&str>, depth: usize) -> bool {
    match s {
        // A `continue` reaches the top of the loop `depth` levels out when it is
        // unlabelled (`depth == 0`) or its label names that loop.
        Stmt::Continue => depth == 0 && label.is_none(),
        // A nested loop hides its own unlabelled `continue`s from the enclosing
        // loop; a labelled `continue` inside it can still target the outer one.
        Stmt::While(..) | Stmt::For { .. } | Stmt::Loop(_) => depth == 0 && label.is_some(),
        Stmt::Break => false,
        Stmt::Expr(e) => expr_contains_continue(e, label, depth),
        // A `return` evaluates its value before leaving the function, and that
        // value can `continue` back to the loop instead.
        Stmt::Return(Some(e)) => expr_contains_continue(e, label, depth),
        Stmt::Let { init: Some(e), .. } => expr_contains_continue(e, label, depth),
        Stmt::Assign { value, .. } => expr_contains_continue(value, label, depth),
        _ => false,
    }
}

fn expr_contains_continue(e: &Expr, label: Option<&str>, depth: usize) -> bool {
    match e {
        Expr::Block(b) => block_contains_continue(b, label, depth),
        Expr::MatchBoundary(b, _) => block_contains_continue(b, label, depth),
        Expr::If(c, t, els) => {
            expr_contains_continue(c, label, depth)
                || block_contains_continue(t, label, depth)
                || els
                    .as_ref()
                    .map_or(false, |b| block_contains_continue(b, label, depth))
        }
        Expr::Bin(_, a, b) => {
            expr_contains_continue(a, label, depth) || expr_contains_continue(b, label, depth)
        }
        Expr::Un(_, a) => expr_contains_continue(a, label, depth),
        Expr::Index(a, b) => {
            expr_contains_continue(a, label, depth) || expr_contains_continue(b, label, depth)
        }
        Expr::Field { base, .. } => expr_contains_continue(base, label, depth),
        Expr::Cast(a, _) => expr_contains_continue(a, label, depth),
        Expr::AddrOf { expr, .. } => expr_contains_continue(expr, label, depth),
        Expr::Deref { expr, .. } => expr_contains_continue(expr, label, depth),
        Expr::BoolTest { scrutinee, .. } => expr_contains_continue(scrutinee, label, depth),
        Expr::Call { args, .. } => args.iter().any(|a| expr_contains_continue(a, label, depth)),
        Expr::StructLit { fields, .. } => fields
            .iter()
            .any(|f| expr_contains_continue(&f.value, label, depth)),
        Expr::ArrayLit(v, _) => expr_contains_continue(v, label, depth),
        Expr::ArrayList(xs) => xs.iter().any(|a| expr_contains_continue(a, label, depth)),
        Expr::MethodCall { recv, args, .. } => {
            expr_contains_continue(recv, label, depth)
                || args.iter().any(|a| expr_contains_continue(a, label, depth))
        }
        Expr::EnumCtor { args, .. } => args.iter().any(|a| expr_contains_continue(a, label, depth)),
        Expr::VariantTest { scrutinee, .. } => expr_contains_continue(scrutinee, label, depth),
        Expr::EnumPayload { recv, .. } => expr_contains_continue(recv, label, depth),
        _ => false,
    }
}
/// True when a `break` in `b` targets the loop `b` is the body of. A nested
/// loop captures its own `break`s, so the walk stops at one; an `if`/block does
/// not, so it descends.
fn block_has_break(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_break) || b.tail.as_deref().map_or(false, expr_has_break)
}

fn stmt_has_break(s: &Stmt) -> bool {
    match s {
        Stmt::Break => true,
        Stmt::While(..) | Stmt::Loop(..) | Stmt::For { .. } => false,
        Stmt::Expr(e) => expr_has_break(e),
        Stmt::Let { init: Some(e), .. } => expr_has_break(e),
        Stmt::Assign { value, .. } => expr_has_break(value),
        _ => false,
    }
}

fn expr_has_break(e: &Expr) -> bool {
    match e {
        Expr::Block(b) => block_has_break(b),
        Expr::MatchBoundary(b, _) => block_has_break(b),
        Expr::If(_, t, els) => {
            block_has_break(t) || els.as_ref().map_or(false, |b| block_has_break(b))
        }
        _ => false,
    }
}
