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
    ret_ty: Type,
    /// While checking the `else` chain of a desugared `match`: whether the
    /// whole chain names every variant of one enum. Decided once at the chain's
    /// first `if` and carried down, so the final `if` (which has no `else`) may
    /// stand for a value. Cleared while checking a condition or an arm body.
    match_total: bool,
    /// Whether the `if` being checked is a non-root step of such a chain, so it
    /// inherits `match_total` instead of recomputing it.
    match_chain: bool,
}

type Scope = Vec<(String, i32, Type)>;

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

    let mut sema = Sema {
        sigs,
        structs,
        enums,
        next_slot: 0,
        loop_depth: 0,
        ret_ty: Type::Unit,
        match_total: false,
        match_chain: false,
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
        for (name, ty) in &mut f.params {
            let rt = self.resolve_ty(ty)?;
            *ty = rt.clone();
            // An aggregate parameter owns a frame buffer big enough for a copy;
            // a scalar (including a reference) owns a single slot.
            let slot = self.alloc_slots((rt.size() / 8) as i32);
            f.param_slots.push(slot);
            scope.push((name.clone(), slot, rt));
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

    fn lookup(scope: &Scope, name: &str) -> Option<(i32, Type)> {
        scope
            .iter()
            .rev()
            .find(|(n, _, _)| n == name)
            .map(|(_, s, t)| (*s, t.clone()))
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

    /// Whether a desugared `match` `if`-chain is provably total, i.e. it tests
    /// every variant of one enum exactly once and no other condition.
    ///
    /// A `match` with no `_` arm folds to `if c1 { a1 } else { if c2 { a2 } ... }`
    /// with no final `else`, so a value that matches no arm falls through to
    /// unit. That is exactly right when the match is non-exhaustive, but when
    /// the arms name *every* variant the fall-through is unreachable and the
    /// chain may be used as a value. rustc accepts that; rmc rejected it with a
    /// spurious "`if` without `else`" error.
    ///
    /// `VariantTest` nodes are produced only by `match` desugaring, so a chain
    /// whose conditions are all `VariantTest`s over one enum and whose tested
    /// variants are pairwise distinct and cover every variant is exhaustive.
    /// Anything else (an integer arm, a duplicate, a missing variant, a
    /// hand-written `if` chain) is not, and keeps the old error.
    ///
    /// The comparison is by variant *name*, not tag: the nested `VariantTest`s
    /// have not been visited by `check_expr` yet, so their tags are still -1.
    fn if_chain_is_exhaustive(&self, cond: &Expr, els: &Option<Block>) -> bool {
        let mut enum_name: Option<&str> = None;
        let mut seen: Vec<&str> = Vec::new();
        let mut cur_cond = cond;
        let mut cur_els = els;
        loop {
            let (name, variant) = match cur_cond {
                Expr::VariantTest { enum_name, variant, .. } => {
                    (enum_name.as_str(), variant.as_str())
                }
                _ => return false,
            };
            match enum_name {
                Some(existing) if existing != name => return false,
                None => enum_name = Some(name),
                _ => {}
            }
            if seen.contains(&variant) {
                return false;
            }
            seen.push(variant);
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
        let info = match enum_name.and_then(|n| self.enums.get(n)) {
            Some(i) => i,
            None => return false,
        };
        info.variants.len() == seen.len()
    }

    fn check_block(&mut self, b: &mut Block, scope: &mut Scope) -> Result<Type, String> {
        let mark = scope.len();
        for st in &mut b.stmts {
            self.check_stmt(st, scope)?;
        }
        let ty = match &mut b.tail {
            Some(e) => self.check_expr(e, scope)?,
            None => Type::Unit,
        };
        scope.truncate(mark);
        Ok(ty)
    }

    fn check_stmt(&mut self, st: &mut Stmt, scope: &mut Scope) -> Result<(), String> {
        match st {
            Stmt::Let { name, ty, init, slot } => {
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
                scope.push((name.clone(), s, final_ty));
                Ok(())
            }
            Stmt::Assign { target, value } => {
                let vty = self.check_expr(value, scope)?;
                let tty = self.check_lvalue(target, scope)?;
                if vty != tty && !ref_coerces(&vty, &tty) {
                    return Err(format!("assignment type mismatch: target is {tty:?}, value is {vty:?}"));
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
                self.loop_depth += 1;
                self.check_block(body, scope)?;
                self.loop_depth -= 1;
                Ok(())
            }
            Stmt::Loop(body) => {
                self.loop_depth += 1;
                self.check_block(body, scope)?;
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
                scope.push((var.clone(), s, Type::I64));
                self.loop_depth += 1;
                self.check_block(body, scope)?;
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
                Ok(())
            }
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    return Err("`continue` outside of a loop".into());
                }
                Ok(())
            }
        }
    }

    /// Type-check an assignment target (variable, array element or field).
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
                Some((s, t)) => {
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
                let tt = self.check_block(then, scope)?;
                match els {
                    Some(e) => {
                        // Only a total chain's `else` continues the chain: an
                        // all-variant `match` has no wildcard body, so the else
                        // block is always the next arm's test. A non-total chain
                        // may end in a wildcard body that is itself an `if`, and
                        // that nested `if` must be checked as an ordinary root.
                        self.match_chain = total;
                        self.match_total = total;
                        let et = self.check_block(e, scope)?;
                        self.match_chain = false;
                        self.match_total = false;
                        if tt != et {
                            return Err(format!("`if` branches have different types: {tt:?} and {et:?}"));
                        }
                        Ok(tt)
                    }
                    None => {
                        if tt != Type::Unit && !total {
                            return Err(format!(
                                "`if` without `else` must evaluate to `()`, but the branch is {tt:?}"
                            ));
                        }
                        Ok(tt)
                    }
                }
            }
            Expr::Block(b) => self.check_block(b, scope),
            Expr::Index(base, idx) => {
                let bt = deref_ty(self.check_expr(base, scope)?);
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
                // The receiver was checked above; the call is checked once more
                // as a whole, which re-checks the receiver — harmless, since a
                // value expression has no side effects on the frame other than
                // bumping `next_slot` for a temporary aggregate.
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
        // Both arms must diverge for the `if` to diverge.
        Expr::If(_, t, els) => {
            block_diverges(t) && els.as_ref().map_or(false, |b| block_diverges(b))
        }
        _ => false,
    }
}
