//! Semantic analysis: resolves variables to stack slots, infers and checks types.
//!
//! Every local (parameters included) gets a unique 8-byte slot in the function
//! frame. `Expr::Var` is rewritten in place with its slot and type, so codegen
//! never needs to do name lookups.

use crate::ast::*;
use std::collections::HashMap;

#[derive(Clone)]
pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
}

pub struct Sema {
    sigs: HashMap<String, Sig>,
    next_slot: i32,
    loop_depth: u32,
    ret_ty: Type,
}

type Scope = Vec<(String, i32, Type)>;

pub fn analyze(prog: &mut Program) -> Result<(), String> {
    let mut sigs: HashMap<String, Sig> = HashMap::new();
    // Runtime helpers provided by the freestanding assembly runtime. They are
    // linked in automatically, so they do not appear in the source program.
    let builtins: [(&str, &[Type], Type); 6] = [
        ("print_i64", &[Type::I64], Type::Unit),
        ("print_str", &[Type::I64], Type::Unit),
        ("print_bool", &[Type::Bool], Type::Unit),
        ("putchar", &[Type::I64], Type::Unit),
        ("alloc", &[Type::I64], Type::I64),
        ("arg_i64", &[Type::I64], Type::I64),
    ];
    for (name, params, ret) in builtins {
        sigs.insert(
            name.to_string(),
            Sig {
                params: params.to_vec(),
                ret,
            },
        );
    }
    for f in &prog.fns {
        if sigs.contains_key(&f.name) {
            return Err(format!("function `{}` is defined more than once", f.name));
        }
        for (_, t) in &f.params {
            if !t.is_scalar() {
                return Err(format!(
                    "function `{}`: parameter types must be `i64` or `bool`",
                    f.name
                ));
            }
        }
        sigs.insert(
            f.name.clone(),
            Sig {
                params: f.params.iter().map(|(_, t)| t.clone()).collect(),
                ret: f.ret.clone(),
            },
        );
    }

    let mut sema = Sema {
        sigs,
        next_slot: 0,
        loop_depth: 0,
        ret_ty: Type::Unit,
    };
    for f in &mut prog.fns {
        sema.check_fn(f)?;
    }
    Ok(())
}

impl Sema {
    fn check_fn(&mut self, f: &mut FnDef) -> Result<(), String> {
        self.next_slot = 0;
        self.loop_depth = 0;
        self.ret_ty = f.ret.clone();

        let mut scope: Scope = Vec::with_capacity(f.params.len() + 8);
        for (name, ty) in &f.params {
            let slot = self.alloc_slot();
            scope.push((name.clone(), slot, ty.clone()));
        }

        self.check_block(&mut f.body, &mut scope)?;
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
                let declared = ty.clone();
                let init_ty = match init {
                    Some(e) => Some(self.check_expr(e, scope)?),
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
                if !final_ty.is_scalar() {
                    // arrays are supported as locals
                    if !matches!(final_ty, Type::Array(_, _)) {
                        return Err(format!("variable `{name}`: unsupported type {final_ty:?}"));
                    }
                }
                let s = self.alloc_slots((final_ty.size() / 8) as i32);
                *slot = s;
                scope.push((name.clone(), s, final_ty));
                Ok(())
            }
            Stmt::Assign { target, value } => {
                let vty = self.check_expr(value, scope)?;
                let tty = self.check_lvalue(target, scope)?;
                if vty != tty {
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

    /// Type-check an assignment target (variable or array element).
    fn check_lvalue(&mut self, e: &mut Expr, scope: &mut Scope) -> Result<Type, String> {
        match e {
            Expr::Var { .. } => self.check_expr(e, scope),
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
                }
            }
            Expr::Call(name, args) => {
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
                for (a, p) in args.iter_mut().zip(sig.params.iter()) {
                    let t = self.check_expr(a, scope)?;
                    if &t != p {
                        return Err(format!("function `{name}`: argument type mismatch, expected {p:?}, found {t:?}"));
                    }
                }
                Ok(sig.ret)
            }
            Expr::If(cond, then, els) => {
                let ct = self.check_expr(cond, scope)?;
                if ct != Type::Bool {
                    return Err(format!("`if` condition must be `bool`, found {ct:?}"));
                }
                let tt = self.check_block(then, scope)?;
                match els {
                    Some(e) => {
                        let et = self.check_block(e, scope)?;
                        if tt != et {
                            return Err(format!("`if` branches have different types: {tt:?} and {et:?}"));
                        }
                        Ok(tt)
                    }
                    None => {
                        if tt != Type::Unit {
                            return Err(format!(
                                "`if` without `else` must evaluate to `()`, but the branch is {tt:?}"
                            ));
                        }
                        Ok(Type::Unit)
                    }
                }
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
            Expr::ArrayLit(val, n) => {
                let t = self.check_expr(val, scope)?;
                if !t.is_scalar() {
                    return Err(format!("array elements must be `i64` or `bool`, found {t:?}"));
                }
                Ok(Type::Array(Box::new(t), *n))
            }
            Expr::Cast(inner, to) => {
                self.check_expr(inner, scope)?;
                Ok(to.clone())
            }
        }
    }
}
