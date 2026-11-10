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
use std::collections::HashMap;

#[derive(Clone)]
pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
    /// A method receiver: such a function may only be called as `recv.name(..)`.
    pub takes_self: bool,
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
    /// One entry per enclosing loop, holding the scope length at the loop's
    /// entry. A `while`/`for` body may run again, so a binding that already
    /// existed outside the loop must not be written by a non-`mut` binding
    /// inside it (rustc's `cannot assign twice`); a binding declared *inside*
    /// the body is fresh each iteration and is exempt.
    loop_marks: Vec<usize>,
    /// One entry per enclosing loop, parallel to `loop_marks`: a binding may be
    /// written inside the loop without counting as a second write only if the
    /// write cannot be reached again after the loop's back edge. At a write the
    /// top entry says whether the *current statement and everything after it*
    /// can loop back to the top; `check_block` rewrites it per statement as the
    /// block is walked. See `block_repeat_suffixes`.
    loop_repeats: Vec<bool>,
    /// Parallel to `loop_repeats`: at the current point, whether control can
    /// still reach the top of that loop. Inside a `loop` body this is the
    /// per-statement repeat flag. Inside a `break`-valued expression it becomes
    /// `false`, because a `break` jumps past the loop top. A `continue` in a
    /// branch restores it to `true` while that branch is checked.
    loop_reach: Vec<bool>,
    /// Parallel to `loop_reach`: the reach of the code *after* the statement
    /// currently being checked. A nested block (an `if` arm, a `{}` expression)
    /// falls through to this point, so its own reach analysis is seeded with it
    /// rather than with the statement's own reach.
    loop_cont: Vec<bool>,
    /// One entry per enclosing loop: the may-assigned state of the bindings in
    /// scope at the loop's entry, accumulated over every `break` that leaves the
    /// loop. A `break` ends the loop, so its branch's writes are not seen by the
    /// code after the loop unless they are merged here; without it a write on a
    /// `break` path would let a later statement write a non-`mut` binding twice.
    break_assigned: Vec<Vec<bool>>,
    ret_ty: Type,
}

/// One in-scope binding: its name, frame slot, type, and whether it may be
/// assigned. Parameters and `let mut` bindings are mutable; a plain `let` is
/// not, so an assignment to it is rejected the way rustc rejects it.
struct Binding {
    name: String,
    slot: i32,
    ty: Type,
    mutable: bool,
    /// True for a function/method parameter, which rustc reports as an
    /// "immutable argument" rather than a twice-assigned variable.
    param: bool,
    /// True once the binding has been assigned on some path (or was initialized
    /// at its declaration). A plain `let` without an initialiser may be written
    /// exactly once, so a second write to a non-`mut` binding is rejected the
    /// way rustc rejects `cannot assign twice to immutable variable`.
    assigned: bool,
}

type Scope = Vec<Binding>;

pub fn analyze(prog: &mut Program) -> Result<(), String> {
    let (structs, enums) = resolve_layouts(&prog.structs, &prog.enums)?;

    let mut sigs: HashMap<String, Sig> = HashMap::new();
    // Runtime helpers provided by the freestanding assembly runtime. They are
    // linked in automatically, so they do not appear in the source program.
    let builtins: [(&str, &[Type], Type); 7] = [
        ("print_i64", &[Type::I64], Type::Unit),
        ("print_str", &[Type::I64], Type::Unit),
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
            },
        );
    }
    // A method's receiver is its first parameter, with the impl's struct type.
    // Give it to every method up front so its signature (and later its frame)
    // includes `self` from the start. `param_mut` tracks the explicitly declared
    // parameters, so it must gain a matching entry for the synthesized receiver
    // (`self` is immutable); otherwise the flag for the first declared parameter
    // lands on `self` and the last declared parameter is treated as immutable.
    for f in &mut prog.fns {
        if let Some(ty) = f.self_ty.clone() {
            f.params.insert(0, ("self".to_string(), Type::Named(ty)));
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
        sigs.insert(f.name.clone(), Sig { params, ret, takes_self: f.takes_self });
    }

    let mut sema = Sema {
        sigs,
        structs,
        enums,
        next_slot: 0,
        loop_depth: 0,
        loop_marks: Vec::new(),
        loop_repeats: Vec::new(),
        loop_reach: Vec::new(),
        loop_cont: Vec::new(),
        break_assigned: Vec::new(),
        ret_ty: Type::Unit,
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
                return Err(format!("array elements must be `i64` or `bool`, not {e:?}"));
            }
            Ok(Type::Array(Box::new(e), *n))
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
                return Err(format!("array elements must be `i64` or `bool`, not {e:?}"));
            }
            Ok(Type::Array(Box::new(e), *n))
        }
        other => Ok(other.clone()),
    }
}

impl Sema {
    /// Every type site goes through here, so an enum name resolves wherever a
    /// type may appear.
    fn resolve_ty(&self, t: &Type) -> Result<Type, String> {
        resolve_in(t, &self.structs, &self.enums)
    }

    fn check_fn(&mut self, f: &mut FnDef) -> Result<(), String> {
        self.next_slot = 0;
        self.loop_depth = 0;
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
        let param_mut = f.param_mut.clone();
        for (i, (name, ty)) in f.params.iter_mut().enumerate() {
            let rt = self.resolve_ty(ty)?;
            *ty = rt.clone();
            // An aggregate parameter owns a frame buffer big enough for a copy;
            // a scalar owns a single slot.
            let slot = self.alloc_slots((rt.size() / 8) as i32);
            f.param_slots.push(slot);
            scope.push(Binding {
                name: name.clone(),
                slot,
                ty: rt,
                mutable: param_mut.get(i).copied().unwrap_or(false),
                param: true,
                assigned: true,
            });
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

    fn lookup(scope: &Scope, name: &str) -> Option<(i32, Type, bool)> {
        scope
            .iter()
            .rev()
            .find(|b| b.name == name)
            .map(|b| (b.slot, b.ty.clone(), b.mutable))
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
                return Err(format!("array elements must be `i64` or `bool`, found {t:?}"));
            }
            return Ok(Type::Array(Box::new(t), *n));
        }
        self.check_expr(e, scope)
    }

    fn check_block(&mut self, b: &mut Block, scope: &mut Scope) -> Result<Type, String> {
        self.check_block_inner(b, scope, false)
    }

    /// Check a block. When `loop_body` is set the block is a loop body, whose
    /// normal completion loops back to the top (seed `true`); any other block
    /// falls through to the code after its statement (`loop_cont`). Each
    /// statement's reach and continuation come from the block's suffix walk, so
    /// a nested `if`/block sees the continuation that really follows it rather
    /// than the enclosing statement's own reach. `loop_reach`/`loop_cont` are
    /// restored on exit.
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
        // A loop body's completion goes back to the loop top. A loop that runs
        // at most once (`loop_repeats` false) has no back edge, so every reach
        // is `false` and a single write to an outer binding stays legal.
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
                        if d != i {
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
                scope.push(Binding {
                    name: name.clone(),
                    slot: s,
                    ty: final_ty,
                    mutable: *mutable,
                    param: false,
                    assigned: init.is_some(),
                });
                Ok(())
            }
            Stmt::Assign { target, value } => {
                let vty = self.check_expr(value, scope)?;
                let tty = self.check_lvalue(target, scope)?;
                self.check_writable(target, scope)?;
                if vty != tty {
                    return Err(format!("assignment type mismatch: target is {tty:?}, value is {vty:?}"));
                }
                self.mark_assigned(target, scope);
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
                self.loop_depth += 1;
                // The body may run again, so a non-`mut` binding that already
                // existed outside the loop cannot be written inside it.
                self.loop_marks.push(scope.len());
                self.loop_repeats.push(true);
                self.loop_reach.push(false);
                self.break_assigned.push(Self::snapshot_assigned(scope));
                self.check_block_inner(body, scope, true)?;
                let breaks = self.break_assigned.pop().unwrap();
                self.loop_reach.pop();
                self.loop_repeats.pop();
                self.loop_marks.pop();
                if breaks.len() == scope.len() {
                    for (b, a) in scope.iter_mut().zip(breaks.iter()) {
                        b.assigned |= *a;
                    }
                }
                self.loop_depth -= 1;
                Ok(())
            }
            Stmt::Loop(body) => {
                self.loop_depth += 1;
                // A `loop` runs at least once, so a write to an outer binding is
                // a straight-line write unless the body can reach the top again.
                // Then it is a second write on the next pass and rustc rejects
                // it, exactly like a `while` body.
                self.loop_marks.push(scope.len());
                self.loop_repeats.push(block_reaches_top(body));
                self.loop_reach.push(false);
                self.break_assigned.push(Self::snapshot_assigned(scope));
                self.check_block_inner(body, scope, true)?;
                let breaks = self.break_assigned.pop().unwrap();
                self.loop_reach.pop();
                self.loop_repeats.pop();
                self.loop_marks.pop();
                if breaks.len() == scope.len() {
                    for (b, a) in scope.iter_mut().zip(breaks.iter()) {
                        b.assigned |= *a;
                    }
                }
                self.loop_depth -= 1;
                Ok(())
            }
            Stmt::For { var, var_slot, start, end, end_slot, body } => {
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
                scope.push(Binding {
                    name: var.clone(),
                    slot: s,
                    ty: Type::I64,
                    mutable: false,
                    param: false,
                    assigned: true,
                });
                self.loop_depth += 1;
                self.loop_marks.push(mark);
                self.loop_repeats.push(true);
                self.loop_reach.push(false);
                self.break_assigned.push(Self::snapshot_assigned(scope));
                self.check_block_inner(body, scope, true)?;
                let breaks = self.break_assigned.pop().unwrap();
                self.loop_reach.pop();
                self.loop_repeats.pop();
                self.loop_marks.pop();
                if breaks.len() == scope.len() {
                    for (b, a) in scope.iter_mut().zip(breaks.iter()) {
                        b.assigned |= *a;
                    }
                }
                self.loop_depth -= 1;
                scope.truncate(mark);
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
                // The branch's writes are seen by the code after the loop, so
                // merge this state into the loop's break-exit state before the
                // branch's local state is discarded.
                if let Some(acc) = self.break_assigned.last_mut() {
                    if acc.len() == scope.len() {
                        for (a, b) in acc.iter_mut().zip(scope.iter()) {
                            *a |= b.assigned;
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

    /// Reject an assignment whose root binding may not be written here,
    /// matching rustc's errors. A `mut` binding is always writable. A plain
    /// `let` is writable exactly once: the write to its uninitialised value,
    /// or the declaration's initialiser. A later write is `cannot assign twice
    /// to immutable variable`. A `while`/`for` body may run again, so writing a
    /// binding that was already in scope before the loop is a second write even
    /// on the first iteration, which rustc also rejects.
    fn check_writable(&self, e: &Expr, scope: &Scope) -> Result<(), String> {
        match e {
            Expr::Var { name, .. } => {
                let deny = |b: &Binding| {
                    if b.param {
                        Err(format!(
                            "cannot assign to immutable argument `{name}` (add `mut`)"
                        ))
                    } else {
                        Err(format!(
                            "cannot assign twice to immutable variable `{name}` (add `mut`)"
                        ))
                    }
                };
                match scope.iter().enumerate().rev().find(|(_, b)| &b.name == name) {
                    Some((_, b)) if b.mutable => Ok(()),
                    Some((_, b)) if b.assigned => deny(b),
                    // A write inside a repeating loop to a binding declared
                    // before that loop would run once per iteration, so it is a
                    // second write even on the first pass. A `loop` that runs at
                    // most once is exempt (its flag is `false`).
                    Some((idx, b))
                        if self
                            .loop_marks
                            .iter()
                            .zip(self.loop_reach.iter())
                            .any(|(&m, &reach)| reach && idx < m) =>
                    {
                        deny(b)
                    }
                    Some(_) => Ok(()),
                    None => Ok(()),
                }
            }
            Expr::Field { base, .. } | Expr::Index(base, _) => self.check_writable(base, scope),
            _ => Ok(()),
        }
    }

    /// The root variable name of an assignment target (`a` for `a.f[i]`).
    fn root_name(e: &Expr) -> Option<&str> {
        match e {
            Expr::Var { name, .. } => Some(name.as_str()),
            Expr::Field { base, .. } | Expr::Index(base, _) => Self::root_name(base),
            _ => None,
        }
    }

    /// Record that an assignment wrote its root binding, so a later write to a
    /// non-`mut` binding is the `cannot assign twice` error.
    fn mark_assigned(&self, target: &Expr, scope: &mut Scope) {
        if let Some(name) = Self::root_name(target) {
            if let Some(b) = scope.iter_mut().rev().find(|b| b.name == name) {
                b.assigned = true;
            }
        }
    }

    fn snapshot_assigned(scope: &Scope) -> Vec<bool> {
        scope.iter().map(|b| b.assigned).collect()
    }

    fn restore_assigned(scope: &mut Scope, state: &[bool]) {
        if scope.len() == state.len() {
            for (b, s) in scope.iter_mut().zip(state) {
                b.assigned = *s;
            }
        }
    }

    /// Mark a binding assigned when *either* merged path assigns it. The
    /// second-write check asks whether the binding may already have been
    /// written, so a write on one path is enough to make a later write illegal.
    fn merge_assigned(scope: &mut Scope, a: &[bool], b: &[bool]) {
        if scope.len() == a.len() && scope.len() == b.len() {
            for (i, binding) in scope.iter_mut().enumerate() {
                binding.assigned = a[i] || b[i];
            }
        }
    }

    /// Type-check an assignment target (variable, array element or field).
    fn check_lvalue(&mut self, e: &mut Expr, scope: &mut Scope) -> Result<Type, String> {
        match e {
            Expr::Var { .. } | Expr::Field { .. } => self.check_expr(e, scope),
            Expr::Index(base, idx) => {
                let bt = self.check_expr(base, scope)?;
                let it = self.check_expr(idx, scope)?;
                if it != Type::I64 {
                    return Err(format!("array index must be `i64`, found {it:?}"));
                }
                match bt {
                    Type::Array(elem, _) => Ok(*elem),
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
            Expr::Str(_) => Ok(Type::I64),
            Expr::Var { name, slot, ty } => match Self::lookup(scope, name) {
                Some((s, t, _)) => {
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
                        Ok(Type::I64)
                    }
                    UnOp::Not => {
                        if t != Type::Bool {
                            return Err(format!("unary `!` expects `bool`, found {t:?}"));
                        }
                        Ok(Type::Bool)
                    }
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
                    if &t != p {
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
                let ct = self.check_expr(cond, scope)?;
                if ct != Type::Bool {
                    return Err(format!("`if` condition must be `bool`, found {ct:?}"));
                }
                // Each arm is checked from the incoming state, then the results
                // are merged. A path that returns/breaks/continues cannot reach
                // the statement after the `if`, so it must not contribute: only
                // arms that can fall through decide the merged state. An `if`
                // without an `else` has an implicit fall-through arm keeping the
                // incoming state.
                let before = Self::snapshot_assigned(scope);
                let tt = self.check_block(then, scope)?;
                let then_state = Self::snapshot_assigned(scope);
                let then_div = block_diverges(then);
                Self::restore_assigned(scope, &before);
                let (et, else_state, else_div) = match els {
                    Some(e) => {
                        let et = self.check_block(e, scope)?;
                        let st = Self::snapshot_assigned(scope);
                        if tt != et {
                            return Err(format!("`if` branches have different types: {tt:?} and {et:?}"));
                        }
                        (tt, st, block_diverges(e))
                    }
                    None => {
                        if tt != Type::Unit {
                            return Err(format!(
                                "`if` without `else` must evaluate to `()`, but the branch is {tt:?}"
                            ));
                        }
                        // No else: the fall-through keeps the incoming state.
                        (tt, before.clone(), false)
                    }
                };
                // A may-analysis: a binding is "assigned" when any reaching path
                // assigned it. Only arms that fall through contribute.
                match (then_div, else_div) {
                    (false, false) => Self::merge_assigned(scope, &then_state, &else_state),
                    (true, false) => Self::restore_assigned(scope, &else_state),
                    (false, true) => Self::restore_assigned(scope, &then_state),
                    // Both arms diverge: nothing after the `if` is reachable.
                    (true, true) => {}
                }
                Ok(et)
            }
            Expr::Block(b) => self.check_block(b, scope),
            Expr::Index(base, idx) => {
                let bt = self.check_expr(base, scope)?;
                let it = self.check_expr(idx, scope)?;
                if it != Type::I64 {
                    return Err(format!("array index must be `i64`, found {it:?}"));
                }
                match bt {
                    Type::Array(elem, _) => Ok(*elem),
                    other => Err(format!("cannot index a value of type {other:?}")),
                }
            }
            Expr::Field { base, name, offset, ty } => {
                let bt = self.check_expr(base, scope)?;
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
                    return Err(format!("array elements must be `i64` or `bool`, found {t:?}"));
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
                        "array elements must be `i64` or `bool`, found {elem_ty:?}"
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
                let assigned_before = Self::snapshot_assigned(scope);
                let rt = self.check_expr(recv, scope)?;
                // A method lives on a struct or an enum; both give the mangled
                // prefix `Type$method`.
                let tyname = match &rt {
                    Type::Struct(n, _) | Type::Enum(n) | Type::EnumData(n, _) => n.clone(),
                    other => {
                        return Err(format!(
                            "method `{name}` called on a value of type {other:?}, which is not a struct or enum"
                        ))
                    }
                };
                let mangled = format!("{tyname}${name}");
                if !self.sigs.contains_key(&mangled) {
                    return Err(format!("no method `{name}` on `{tyname}`"));
                }
                Self::restore_assigned(scope, &assigned_before);
                // Rewrite into a call with the receiver as the first argument.
                let recv = std::mem::replace(recv.as_mut(), Expr::Int(0));
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
                *to = rt.clone();
                Ok(rt)
            }
        }
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
        // A `return` leaves the function, so control never reaches the following
        // statement. Its value is evaluated first (and may itself `continue`,
        // which is handled by `stmt_continues_to`), but either way the next
        // statement is unreachable.
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
    // Normal completion of a loop body loops back to the top.
    block_reaches_top_from(b, true)
}

/// `reach[i]` is true when some path from statement `i` onward (then the tail,
/// then the continuation `after`) can reach the loop's top. The walk is
/// right-to-left; a statement that cannot complete normally and does not itself
/// reach the top makes every earlier position's `reach` false.
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
    let n = b.stmts.len();
    let mut reach = match &b.tail {
        Some(t) => expr_reaches_top(t, after),
        None => after,
    };
    for i in (0..n).rev() {
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
/// does not always `break`/`return`/`continue` first. Short-circuit operators
/// evaluate the right operand only when the left does not decide, so the whole
/// expression can complete whenever the left can.
fn expr_can_fall(e: &Expr) -> bool {
    match e {
        Expr::Block(b) => block_can_fall(b),
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

/// True when every path through `b` reaches the end of the block: no statement
/// diverges and the tail (if any) can complete normally.
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
