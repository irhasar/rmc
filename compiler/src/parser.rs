//! Recursive-descent parser for the rmc Rust subset.

use crate::ast::*;
use crate::lexer::{Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    /// Suppresses struct-literal parsing in expression position, so that
    /// `while n < 10 {` reads `{` as the loop body rather than as the start of
    /// `10 { ... }`. Parentheses lift the restriction, exactly like rustc.
    no_struct_lit: bool,
    /// Counter for the temporaries introduced when desugaring a compound
    /// assignment, so each one gets a distinct name.
    tmp: u32,
    /// Current expression/type/block nesting depth, bounded by `MAX_DEPTH`.
    depth: u32,
    /// Name of the struct whose `impl` block is being parsed, if any. It lets
    /// `Self` in a method signature resolve to the right struct.
    current_impl: Option<String>,
}

/// The deepest expression, block or type nesting the parser accepts. Deeply
/// nested input (thousands of `(` or `{`) would otherwise recurse until the
/// native stack overflows and the process aborts, which is a crash on
/// untrusted input.
///
/// The value is bounded by the smallest stack the parser runs on: a spawned
/// Rust thread has 2 MiB, and the expression grammar costs about 8 KiB of stack
/// per level (each paren threads through all the precedence levels). 100 levels
/// is roughly 800 KiB, which leaves room, and is far above any hand-written
/// program.
const MAX_DEPTH: u32 = 100;

/// Expressions that are allowed as statements without a trailing `;`.
fn is_block_like(e: &Expr) -> bool {
    matches!(e, Expr::If(..) | Expr::Block(..) | Expr::MatchBoundary(..))
}

type PResult<T> = Result<T, String>;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Parser { toks, i: 0, no_struct_lit: false, tmp: 0, depth: 0, current_impl: None }
    }

    /// Count one level of nesting and fail past `MAX_DEPTH`.
    fn bump_depth(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!(
                "at byte {}: input nests too deeply (limit {})",
                self.pos(),
                MAX_DEPTH
            ));
        }
        Ok(())
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }

    fn pos(&self) -> u32 {
        self.toks.get(self.i).map(|t| t.pos).unwrap_or(u32::MAX)
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i].tok.clone();
        self.i += 1;
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Tok) -> PResult<()> {
        if self.peek() == Some(&t) {
            self.i += 1;
            Ok(())
        } else {
            let got = self
                .peek()
                .map(|x| x.describe())
                .unwrap_or_else(|| "end of input".into());
            Err(format!("at byte {}: expected {}, found {}", self.pos(), t.describe(), got))
        }
    }

    fn expect_ident(&mut self) -> PResult<String> {
        match self.peek() {
            Some(Tok::Ident(_)) => {
                if let Tok::Ident(s) = self.bump() {
                    Ok(s)
                } else {
                    unreachable!()
                }
            }
            other => Err(format!(
                "at byte {}: expected identifier, found {}",
                self.pos(),
                other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
            )),
        }
    }

    pub fn parse_program(&mut self) -> PResult<Program> {
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut fns = Vec::new();
        while self.peek().is_some() {
            match self.peek() {
                Some(Tok::Struct) => structs.push(self.parse_struct()?),
                Some(Tok::Enum) => enums.push(self.parse_enum()?),
                Some(Tok::Fn) => fns.push(self.parse_fn()?),
                Some(Tok::Impl) => self.parse_impl(&mut fns)?,
                other => {
                    return Err(format!(
                        "at byte {}: expected `fn`, `struct`, `enum` or `impl`, found {}",
                        self.pos(),
                        other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
                    ))
                }
            }
        }
        Ok(Program { structs, enums, fns })
    }

    fn parse_enum(&mut self) -> PResult<EnumDef> {
        self.expect(Tok::Enum)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LBrace)?;
        let mut variants = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err(format!("at byte {}: unexpected end of input in `enum`", self.pos()));
            }
            let vname = self.expect_ident()?;
            // `Variant` or `Variant(T, T, ..)`.
            let mut fields = Vec::new();
            if self.eat(&Tok::LParen) {
                while self.peek() != Some(&Tok::RParen) {
                    fields.push(self.parse_type()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen)?;
            }
            variants.push(EnumVariantDef { name: vname, fields });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        if variants.is_empty() {
            return Err(format!("enum `{name}` needs at least one variant"));
        }
        Ok(EnumDef { name, variants })
    }

    fn parse_struct(&mut self) -> PResult<StructDef> {
        self.expect(Tok::Struct)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            let fname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let fty = self.parse_type()?;
            fields.push((fname, fty));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        Ok(StructDef { name, fields })
    }

    fn parse_fn(&mut self) -> PResult<FnDef> {
        self.expect(Tok::Fn)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        let mut param_mut = Vec::new();
        while self.peek() != Some(&Tok::RParen) {
            let m = self.eat(&Tok::Mut);
            let pname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.parse_type()?;
            params.push((pname, ty));
            param_mut.push(m);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        let ret = if self.eat(&Tok::Arrow) {
            self.parse_type()?
        } else {
            Type::Unit
        };
        let body = self.parse_block()?;
        Ok(FnDef { name, self_ty: None, takes_self: false, ref_self: false, ref_self_mut: false, params, param_mut, ret, body, param_slots: Vec::new(), ret_slot: -1, nslots: 0 })
    }

    /// `impl Type { fn method(self, ..) -> .. { .. } }`. The methods are pushed
    /// onto the program's function list with a mangled name (`Type$method`) and
    /// a leading `self` parameter, so sema and codegen treat them as ordinary
    /// functions once a call site has been rewritten.
    fn parse_impl(&mut self, fns: &mut Vec<FnDef>) -> PResult<()> {
        self.expect(Tok::Impl)?;
        let ty = self.expect_ident()?;
        self.expect(Tok::LBrace)?;
        let saved = self.current_impl.take();
        self.current_impl = Some(ty.clone());
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                self.current_impl = saved;
                return Err(format!("at byte {}: unexpected end of input in `impl`", self.pos()));
            }
            match self.parse_method(&ty) {
                Ok(m) => fns.push(m),
                Err(e) => {
                    self.current_impl = saved;
                    return Err(e);
                }
            }
        }
        self.expect(Tok::RBrace)?;
        self.current_impl = saved;
        Ok(())
    }

    fn parse_method(&mut self, ty: &str) -> PResult<FnDef> {
        self.expect(Tok::Fn)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LParen)?;
        // First parameter must be `self`. `&self` / `&mut self` make the receiver
        // a reference, so the call passes the value's address rather than a copy;
        // `self` takes the receiver by value.
        let by_ref = self.eat(&Tok::Amp);
        let by_mut = by_ref && self.eat(&Tok::Mut);
        let first = self.expect_ident()?;
        if first != "self" {
            return Err(format!(
                "at byte {}: method `{name}` must take `self` as its first parameter",
                self.pos()
            ));
        }
        let mut params = Vec::new();
        let mut param_mut = Vec::new();
        if self.eat(&Tok::Comma) {
            while self.peek() != Some(&Tok::RParen) {
                let m = self.eat(&Tok::Mut);
                let pname = self.expect_ident()?;
                self.expect(Tok::Colon)?;
                let pty = self.parse_type()?;
                params.push((pname, pty));
                param_mut.push(m);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen)?;
        let ret = if self.eat(&Tok::Arrow) { self.parse_type()? } else { Type::Unit };
        let body = self.parse_block()?;
        Ok(FnDef {
            name: format!("{ty}${name}"),
            self_ty: Some(ty.to_string()),
            takes_self: true,
            ref_self: by_ref,
            ref_self_mut: by_mut,
            params,
            param_mut,
            ret,
            body,
            param_slots: Vec::new(),
            ret_slot: -1,
            nslots: 0,
        })
    }

    fn parse_type(&mut self) -> PResult<Type> {
        self.bump_depth()?;
        let r = self.parse_type_inner();
        self.depth -= 1;
        r
    }

    fn parse_type_inner(&mut self) -> PResult<Type> {
        if self.eat(&Tok::Amp) {
            let mutable = self.eat(&Tok::Mut);
            let inner = self.parse_type()?;
            return Ok(Type::Ref(Box::new(inner), mutable));
        }
        // `&&T` lexes as one token; treat it as two references.
        if self.eat(&Tok::AmpAmp) {
            let inner = self.parse_type()?;
            return Ok(Type::Ref(Box::new(Type::Ref(Box::new(inner), false)), false));
        }
        match self.peek() {
            Some(Tok::Ident(n)) if n == "Self" => {
                self.i += 1;
                match self.current_impl.clone() {
                    Some(name) => Ok(Type::Named(name)),
                    None => Err(format!("at byte {}: `Self` outside of an `impl`", self.pos())),
                }
            }
            Some(Tok::TyI64) => {
                self.i += 1;
                Ok(Type::I64)
            }
            Some(Tok::TyBool) => {
                self.i += 1;
                Ok(Type::Bool)
            }
            Some(Tok::LBracket) => {
                self.i += 1;
                let elem = self.parse_type()?;
                self.expect(Tok::Semi)?;
                let n = match self.bump() {
                    Tok::Int(v) if v >= 0 => v as usize,
                    other => return Err(format!("array length must be a non-negative integer, found {}", other.describe())),
                };
                self.expect(Tok::RBracket)?;
                Ok(Type::Array(Box::new(elem), n))
            }
            Some(Tok::LParen) => {
                // allow `()` and `(T)` for unit / grouping
                self.i += 1;
                if self.eat(&Tok::RParen) {
                    return Ok(Type::Unit);
                }
                let inner = self.parse_type()?;
                self.expect(Tok::RParen)?;
                Ok(inner)
            }
            Some(Tok::Ident(_)) => {
                let name = match self.bump() {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                Ok(Type::Named(name))
            }
            other => Err(format!(
                "at byte {}: expected a type, found {}",
                self.pos(),
                other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
            )),
        }
    }

    fn parse_block(&mut self) -> PResult<Block> {
        self.bump_depth()?;
        let r = self.parse_block_inner();
        self.depth -= 1;
        r
    }

    fn parse_block_inner(&mut self) -> PResult<Block> {
        self.expect(Tok::LBrace)?;
        let mut stmts = Vec::new();
        let mut tail: Option<Box<Expr>> = None;
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err(format!("at byte {}: unexpected end of input, expected `}}`", self.pos()));
            }
            // tail expression: an expression not followed by `;`
            if self.starts_expr() {
                let e = self.parse_expr()?;
                if self.eat(&Tok::Eq) {
                    let value = self.parse_expr()?;
                    self.expect(Tok::Semi)?;
                    stmts.push(Stmt::Assign { target: e, value });
                    continue;
                }
                if let Some(op) = self.peek_compound_op() {
                    self.i += 1;
                    let value = self.parse_expr()?;
                    self.expect(Tok::Semi)?;
                    stmts.push(self.desugar_compound(&e, op, value));
                    continue;
                }
                if self.eat(&Tok::Semi) {
                    stmts.push(Stmt::Expr(e));
                    continue;
                }
                if self.peek() == Some(&Tok::RBrace) {
                    tail = Some(Box::new(e));
                    break;
                }
                if is_block_like(&e) {
                    // `if`/block expressions are valid statements without a `;`
                    stmts.push(Stmt::Expr(e));
                    continue;
                }
                return Err(format!(
                    "at byte {}: expected `;` after expression",
                    self.pos()
                ));
            }
            stmts.push(self.parse_stmt()?);
        }
        self.expect(Tok::RBrace)?;
        Ok(Block { stmts, tail })
    }

    fn starts_expr(&self) -> bool {
        matches!(
            self.peek(),
            Some(Tok::Int(_))
                | Some(Tok::Str(_))
                | Some(Tok::Ident(_))
                | Some(Tok::True)
                | Some(Tok::False)
                | Some(Tok::LParen)
                | Some(Tok::Minus)
                | Some(Tok::Bang)
                | Some(Tok::If)
                | Some(Tok::Match)
                | Some(Tok::LBracket)
                | Some(Tok::Amp)
                | Some(Tok::Star)
                // A bare `{ ... }` is a block expression, valid as a statement
                // without a `;`. Without this it fell through to `parse_stmt`,
                // which has no arm for `{`, so a nested block failed to parse.
                | Some(Tok::LBrace)
        )
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        match self.peek() {
            Some(Tok::Let) => {
                self.i += 1;
                let mutable = self.eat(&Tok::Mut);
                let name = self.expect_ident()?;
                let ty = if self.eat(&Tok::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let init = if self.eat(&Tok::Eq) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect(Tok::Semi)?;
                Ok(Stmt::Let { name, ty, init, slot: -1, mutable })
            }
            Some(Tok::While) => {
                self.i += 1;
                let cond = self.parse_cond()?;
                let body = self.parse_block()?;
                Ok(Stmt::While(cond, body))
            }
            Some(Tok::Loop) => {
                self.i += 1;
                let body = self.parse_block()?;
                Ok(Stmt::Loop(body))
            }
            Some(Tok::For) => {
                self.i += 1;
                let var = self.expect_ident()?;
                self.expect(Tok::In)?;
                let start = self.parse_cond()?;
                self.expect(Tok::DotDot)?;
                let end = self.parse_cond()?;
                let body = self.parse_block()?;
                Ok(Stmt::For { var, var_slot: -1, start, end, end_slot: -1, body })
            }
            Some(Tok::Return) => {
                self.i += 1;
                let v = if self.eat(&Tok::Semi) {
                    None
                } else {
                    let e = self.parse_expr()?;
                    self.expect(Tok::Semi)?;
                    Some(e)
                };
                Ok(Stmt::Return(v))
            }
            Some(Tok::Break) => {
                self.i += 1;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Break)
            }
            Some(Tok::Continue) => {
                self.i += 1;
                self.expect(Tok::Semi)?;
                Ok(Stmt::Continue)
            }
            _ => {
                // expression statement, possibly an assignment
                let e = self.parse_expr()?;
                if self.eat(&Tok::Eq) {
                    let value = self.parse_expr()?;
                    self.expect(Tok::Semi)?;
                    return Ok(Stmt::Assign { target: e, value });
                }
                if let Some(op) = self.peek_compound_op() {
                    self.i += 1;
                    let value = self.parse_expr()?;
                    self.expect(Tok::Semi)?;
                    return Ok(self.desugar_compound(&e, op, value));
                }
                self.expect(Tok::Semi)?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    /// Desugar `target op= value` into a block that reads the target once,
    /// combines it with `value`, and writes it back. The read is bound to a
    /// temporary so that an index with side effects (`a[next()] += 1`) is
    /// evaluated exactly once, and the block yields the new value so the
    /// statement keeps its value in tail position.
    fn desugar_compound(&mut self, target: &Expr, op: BinOp, value: Expr) -> Stmt {
        // Re-evaluating a `Var`/`Field` target, or an index that is not a call,
        // has no observable effect, so the cheap form (target repeated in the
        // right-hand side) is used. Only a call inside a subscript is bound to a
        // temporary, since evaluating it twice would run the call twice.
        if !expr_has_call(target) {
            return Stmt::Assign {
                target: target.clone(),
                value: Expr::Bin(op, Box::new(target.clone()), Box::new(value)),
            };
        }

        let tv = |name: &str| Expr::Var { name: name.to_string(), slot: -1, ty: Type::I64 };
        let mut stmts = Vec::new();

        // Bind the read target and the write target to the *same* location. A
        // side-effecting component (a call in the base or the subscript) is
        // evaluated once into a temporary and reused, so `make().x += 1` calls
        // `make` once and `a[next()] += 1` advances the counter once. A plain
        // `Var` or call-free `Field`/`Index` has no side effect on
        // re-evaluation and is cloned directly.
        let mut bind = |e: &Expr| -> Expr {
            // NUL is not a valid identifier byte, so a generated name can
            // never collide with one the source spells.
            let n = format!("\u{0}tmp{}", self.tmp);
            self.tmp += 1;
            stmts.push(Stmt::Let {
                name: n.clone(),
                ty: None,
                init: Some(e.clone()),
                slot: -1,
                mutable: true,
            });
            tv(&n)
        };
        let (read, write) = match target {
            Expr::Index(base, idx) => {
                // Bind only the components that call: binding an array base
                // would copy the whole array and redirect the write to the
                // copy, so a call-free base is cloned as before.
                let b = if expr_has_call(base) { bind(base) } else { (**base).clone() };
                let i = if expr_has_call(idx) { bind(idx) } else { (**idx).clone() };
                (
                    Expr::Index(Box::new(b.clone()), Box::new(i.clone())),
                    Expr::Index(Box::new(b), Box::new(i)),
                )
            }
            Expr::Field { base, name, offset, ty } => {
                let b = if expr_has_call(base) { bind(base) } else { (**base).clone() };
                (
                    Expr::Field { base: Box::new(b.clone()), name: name.clone(), offset: *offset, ty: ty.clone() },
                    Expr::Field { base: Box::new(b), name: name.clone(), offset: *offset, ty: ty.clone() },
                )
            }
            other => (other.clone(), other.clone()),
        };

        let tmp = format!("\u{0}ca{}", self.tmp);
        self.tmp += 1;
        stmts.push(Stmt::Let {
            name: tmp.clone(),
            ty: Some(Type::I64),
            init: Some(read),
            slot: -1,
            mutable: true,
        });
        stmts.push(Stmt::Assign {
            target: write,
            value: Expr::Bin(op, Box::new(tv(&tmp)), Box::new(value)),
        });
        Stmt::Expr(Expr::Block(Block {
            stmts,
            tail: Some(Box::new(tv(&tmp))),
        }))
    }

    /// The `BinOp` for a compound-assignment operator, if the next token is one.
    fn peek_compound_op(&self) -> Option<BinOp> {
        match self.peek() {
            Some(Tok::PlusEq) => Some(BinOp::Add),
            Some(Tok::MinusEq) => Some(BinOp::Sub),
            Some(Tok::StarEq) => Some(BinOp::Mul),
            Some(Tok::SlashEq) => Some(BinOp::Div),
            Some(Tok::PercentEq) => Some(BinOp::Mod),
            Some(Tok::AmpEq) => Some(BinOp::BitAnd),
            Some(Tok::PipeEq) => Some(BinOp::BitOr),
            Some(Tok::CaretEq) => Some(BinOp::BitXor),
            Some(Tok::ShlEq) => Some(BinOp::Shl),
            Some(Tok::ShrEq) => Some(BinOp::Shr),
            _ => None,
        }
    }

    fn parse_expr(&mut self) -> PResult<Expr> {
        // `depth` is saved and restored rather than decremented, because the
        // precedence loops below each add to it once per operator. A left-deep
        // chain like `1+1+...+1` is only bounded if those additions accumulate
        // over the whole expression, then reset when it is complete.
        let saved = self.depth;
        self.bump_depth()?;
        let r = self.parse_or();
        self.depth = saved;
        r
    }

    /// Parse an expression in a context where a following `{` must start a
    /// block, not a struct literal (`if`/`while`/`for` conditions and bounds).
    fn parse_cond(&mut self) -> PResult<Expr> {
        let saved = self.no_struct_lit;
        self.no_struct_lit = true;
        let r = self.parse_expr();
        self.no_struct_lit = saved;
        r
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while self.eat(&Tok::PipePipe) {
            let rhs = self.parse_and()?;
            self.bump_depth()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_cmp()?;
        while self.eat(&Tok::AmpAmp) {
            let rhs = self.parse_cmp()?;
            self.bump_depth()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_bitor()?;
        loop {
            let op = match self.peek() {
                Some(Tok::EqEq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Ge) => BinOp::Ge,
                _ => break,
            };
            self.i += 1;
            let rhs = self.parse_bitor()?;
            self.bump_depth()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `|`, then `^`, then `&`, matching Rust's precedence. Each is left
    /// associative and sits between comparison and `+`/`-`.
    fn parse_bitor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_bitxor()?;
        while self.eat(&Tok::Pipe) {
            let rhs = self.parse_bitxor()?;
            self.bump_depth()?;
            lhs = Expr::Bin(BinOp::BitOr, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_bitxor(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_bitand()?;
        while self.eat(&Tok::Caret) {
            let rhs = self.parse_bitand()?;
            self.bump_depth()?;
            lhs = Expr::Bin(BinOp::BitXor, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_bitand(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_shift()?;
        while self.eat(&Tok::Amp) {
            let rhs = self.parse_shift()?;
            self.bump_depth()?;
            lhs = Expr::Bin(BinOp::BitAnd, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => break,
            };
            self.i += 1;
            let rhs = self.parse_mul()?;
            self.bump_depth()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `<<` and `>>`, which bind looser than `+`/`-` and tighter than `&`.
    fn parse_shift(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_add()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Shl) => BinOp::Shl,
                Some(Tok::Shr) => BinOp::Shr,
                _ => break,
            };
            self.i += 1;
            let rhs = self.parse_add()?;
            self.bump_depth()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                Some(Tok::Percent) => BinOp::Mod,
                _ => break,
            };
            self.i += 1;
            let rhs = self.parse_unary()?;
            self.bump_depth()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.i += 1;
                self.bump_depth()?;
                let e = self.parse_unary()?;
                Ok(Expr::Un(UnOp::Neg, Box::new(e)))
            }
            Some(Tok::Bang) => {
                self.i += 1;
                self.bump_depth()?;
                let e = self.parse_unary()?;
                Ok(Expr::Un(UnOp::Not, Box::new(e)))
            }
            Some(Tok::Amp) => {
                self.i += 1;
                let mutable = self.eat(&Tok::Mut);
                self.bump_depth()?;
                let e = self.parse_unary()?;
                Ok(Expr::AddrOf { expr: Box::new(e), ty: Type::Named(String::new()), mutable, slot: -1 })
            }
            // `&&e` lexes as one token; treat it as two address-ofs.
            Some(Tok::AmpAmp) => {
                self.i += 1;
                self.bump_depth()?;
                let e = self.parse_unary()?;
                let inner = Expr::AddrOf { expr: Box::new(e), ty: Type::Named(String::new()), mutable: false, slot: -1 };
                Ok(Expr::AddrOf { expr: Box::new(inner), ty: Type::Named(String::new()), mutable: false, slot: -1 })
            }
            Some(Tok::Star) => {
                self.i += 1;
                self.bump_depth()?;
                let e = self.parse_unary()?;
                Ok(Expr::Deref { expr: Box::new(e), ty: Type::Named(String::new()) })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            self.bump_depth()?;
            if self.eat(&Tok::LBracket) {
                let idx = self.parse_expr()?;
                self.expect(Tok::RBracket)?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.eat(&Tok::Dot) {
                let name = self.expect_ident()?;
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while self.peek() != Some(&Tok::RParen) {
                        args.push(self.parse_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    e = Expr::MethodCall { recv: Box::new(e), name, args };
                } else {
                    e = Expr::Field { base: Box::new(e), name, offset: 0, ty: Type::I64 };
                }
            } else if self.eat(&Tok::As) {
                let ty = self.parse_type()?;
                e = Expr::Cast(Box::new(e), ty);
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// `match scrutinee { pat => expr, ... }` on integers. It desugars to a
    /// chain of `if` comparisons, so sema and codegen need no new cases. A
    /// pattern is an integer literal (optionally negated) or `_`; the first arm
    /// whose pattern equals the scrutinee runs, and `_` matches anything. The
    /// scrutinee is bound to a hidden `let` when it is not already a variable,
    /// so it is evaluated exactly once even when it is a call.
    fn parse_match(&mut self) -> PResult<Expr> {
        self.expect(Tok::Match)?;
        let scrutinee = self.parse_cond()?;
        self.expect(Tok::LBrace)?;
        let mut arms: Vec<(Pat, Expr)> = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err(format!("at byte {}: unexpected end of input in `match`", self.pos()));
            }
            let pat = match self.peek().cloned() {
                Some(Tok::Underscore) => {
                    self.i += 1;
                    Pat::Wild
                }
                Some(Tok::Minus) => {
                    self.i += 1;
                    match self.bump() {
                        Tok::Int(v) => Pat::Int(v.checked_neg().ok_or_else(|| {
                            format!("at byte {}: pattern literal out of range", self.pos())
                        })?),
                        other => return Err(format!(
                            "at byte {}: expected an integer after `-` in a pattern, found {}",
                            self.pos(), other.describe()
                        )),
                    }
                }
                Some(Tok::Int(v)) => {
                    self.i += 1;
                    Pat::Int(v)
                }
                // `Enum::Variant`, `Enum::Variant(a, b)`, or a binding name.
                Some(Tok::Ident(n)) => {
                    self.i += 1;
                    if self.eat(&Tok::ColonColon) {
                        let variant = self.expect_ident()?;
                        let mut bindings = None;
                        if self.eat(&Tok::LParen) {
                            let mut names = Vec::new();
                            while self.peek() != Some(&Tok::RParen) {
                                names.push(self.expect_ident()?);
                                if !self.eat(&Tok::Comma) {
                                    break;
                                }
                            }
                            self.expect(Tok::RParen)?;
                            bindings = Some(names);
                        }
                        Pat::Variant(n, variant, bindings)
                    } else {
                        // A bare name binds the scrutinee; `_` already matched
                        // the wildcard case above.
                        Pat::Bind(n)
                    }
                }
                other => return Err(format!(
                    "at byte {}: expected an integer, `Enum::Variant` or `_` pattern, found {}",
                    self.pos(),
                    other.map(|t| t.describe()).unwrap_or_else(|| "end of input".into())
                )),
            };
            self.expect(Tok::FatArrow)?;
            let body = self.parse_expr()?;
            arms.push((pat, body));
            // A comma separates arms, but a block-bodied arm may omit it, so
            // only `}` ends the list early.
            if self.eat(&Tok::Comma) {
                continue;
            }
            if self.peek() == Some(&Tok::RBrace) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        if arms.is_empty() {
            return Err(format!("at byte {}: `match` needs at least one arm", self.pos()));
        }
        Ok(self.desugar_match(scrutinee, arms))
    }

    /// Fold the arms into nested `if`s. The scrutinee is named once; an arm
    /// with `_` becomes the `else` (so any arm after it is unreachable, as in
    /// Rust). With no `_`, the chain has no final `else`, so a non-matching
    /// value yields `()`.
    fn desugar_match(&mut self, scrutinee: Expr, arms: Vec<(Pat, Expr)>) -> Expr {
        let (subj, prelude) = match scrutinee {
            Expr::Var { .. } | Expr::Int(_) | Expr::EnumVariant { .. } => (scrutinee, None),
            other => {
                let name = format!("\u{0}mt{}", self.tmp);
                self.tmp += 1;
                (
                    Expr::Var { name: name.clone(), slot: -1, ty: Type::I64 },
                    // No annotation: the temp takes the scrutinee's type, which
                    // may be an enum rather than an integer.
                    Some(Stmt::Let { name, ty: None, init: Some(other), slot: -1, mutable: true }),
                )
            }
        };
        let mut acc: Option<Expr> = None;
        for (pat, body) in arms.into_iter().rev() {
            acc = Some(match pat {
                Pat::Wild => body,
                // `name => body` binds the whole scrutinee.
                Pat::Bind(name) => Expr::Block(Block {
                    stmts: vec![Stmt::Let {
                        name,
                        ty: None,
                        init: Some(subj.clone()),
                        slot: -1,
                        mutable: true,
                    }],
                    tail: Some(Box::new(body)),
                }),
                Pat::Int(k) => {
                    let cond = Expr::Bin(BinOp::Eq, Box::new(subj.clone()), Box::new(Expr::Int(k)));
                    Self::match_arm(cond, body, acc)
                }
                Pat::Variant(enum_name, variant, bindings) => {
                    let n_binds = match &bindings {
                        Some(names) => names.len() as i32,
                        None => -1,
                    };
                    let cond = Expr::VariantTest {
                        scrutinee: Box::new(subj.clone()),
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        tag: -1,
                        n_binds,
                    };
                    // `Enum::V(w, x)` binds each payload field in the arm body.
                    let body = match bindings {
                        Some(names) if !names.is_empty() => {
                            let stmts = names
                                .into_iter()
                                .enumerate()
                                .map(|(idx, name)| Stmt::Let {
                                    name,
                                    ty: None,
                                    init: Some(Expr::EnumPayload {
                                        recv: Box::new(subj.clone()),
                                        enum_name: enum_name.clone(),
                                        variant: variant.clone(),
                                        idx,
                                        offset: 0,
                                        ty: Type::Unit,
                                    }),
                                    slot: -1,
                                    mutable: true,
                                })
                                .collect();
                            Expr::Block(Block { stmts, tail: Some(Box::new(body)) })
                        }
                        _ => body,
                    };
                    Self::match_arm(cond, body, acc)
                }
            });
        }
        let acc = acc.expect("a `match` has at least one arm");
        let acc = Expr::MatchBoundary(Block { stmts: Vec::new(), tail: Some(Box::new(acc)) });
        match prelude {
            None => acc,
            Some(stmt) => Expr::Block(Block { stmts: vec![stmt], tail: Some(Box::new(acc)) }),
        }
    }

    /// One `if cond { body } else { rest }` step of a desugared `match`.
    fn match_arm(cond: Expr, body: Expr, rest: Option<Expr>) -> Expr {
        let els = rest.map(|e| Block { stmts: Vec::new(), tail: Some(Box::new(e)) });
        Expr::If(
            Box::new(cond),
            Block { stmts: Vec::new(), tail: Some(Box::new(body)) },
            els,
        )
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        self.bump_depth()?;
        let r = self.parse_primary_inner();
        self.depth -= 1;
        r
    }

    fn parse_primary_inner(&mut self) -> PResult<Expr> {
        match self.peek().cloned() {
            Some(Tok::Int(v)) => {
                self.i += 1;
                Ok(Expr::Int(v))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Expr::Str(s))
            }
            Some(Tok::True) => {
                self.i += 1;
                Ok(Expr::Bool(true))
            }
            Some(Tok::False) => {
                self.i += 1;
                Ok(Expr::Bool(false))
            }
            Some(Tok::Ident(name)) => {
                self.i += 1;
                // `Self { .. }` names the struct of the enclosing `impl`.
                let name = if name == "Self" {
                    match self.current_impl.clone() {
                        Some(t) => t,
                        None => {
                            return Err(format!(
                                "at byte {}: `Self` outside of an `impl`",
                                self.pos()
                            ))
                        }
                    }
                } else {
                    name
                };
                // `Enum::Variant` or `Enum::Variant(args)` — resolved and
                // range-checked by sema.
                if self.eat(&Tok::ColonColon) {
                    let variant = self.expect_ident()?;
                    if self.eat(&Tok::LParen) {
                        let mut args = Vec::new();
                        while self.peek() != Some(&Tok::RParen) {
                            args.push(self.parse_expr()?);
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                        self.expect(Tok::RParen)?;
                        return Ok(Expr::EnumCtor {
                            enum_name: name,
                            variant,
                            tag: -1,
                            args,
                            arg_offsets: Vec::new(),
                            slot: -1,
                            size: 0,
                        });
                    }
                    return Ok(Expr::EnumVariant { enum_name: name, variant, tag: -1 });
                }
                if self.peek() == Some(&Tok::LBrace) && !self.no_struct_lit {
                    // struct literal `Name { field: value, ... }`
                    self.i += 1;
                    let mut fields = Vec::new();
                    while self.peek() != Some(&Tok::RBrace) {
                        let fname = self.expect_ident()?;
                        self.expect(Tok::Colon)?;
                        let fval = self.parse_expr()?;
                        fields.push(FieldInit { name: fname, offset: 0, ty: Type::I64, value: fval });
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RBrace)?;
                    return Ok(Expr::StructLit { name, fields, slot: -1 });
                }
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while self.peek() != Some(&Tok::RParen) {
                        args.push(self.parse_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    Ok(Expr::Call { name, args, arg_slots: Vec::new(), ret_slot: -1, ret_ty: Type::I64 })
                } else {
                    Ok(Expr::Var { name, slot: -1, ty: Type::I64 })
                }
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let saved = self.no_struct_lit;
                self.no_struct_lit = false;
                let e = self.parse_expr();
                self.no_struct_lit = saved;
                let e = e?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::If) => {
                self.i += 1;
                let cond = self.parse_cond()?;
                let then = self.parse_block()?;
                let els = if self.eat(&Tok::Else) {
                    if self.peek() == Some(&Tok::If) {
                        // `else if` desugars to a block whose tail is the nested if
                        let nested = self.parse_primary()?;
                        Some(Block {
                            stmts: Vec::new(),
                            tail: Some(Box::new(nested)),
                        })
                    } else {
                        Some(self.parse_block()?)
                    }
                } else {
                    None
                };
                Ok(Expr::If(Box::new(cond), then, els))
            }
            Some(Tok::Match) => self.parse_match(),
            Some(Tok::LBrace) => Ok(Expr::Block(self.parse_block()?)),
            Some(Tok::LBracket) => {
                self.i += 1;
                let first = self.parse_expr()?;
                if self.eat(&Tok::Semi) {
                    let n = match self.bump() {
                        Tok::Int(v) if v >= 0 => v as usize,
                        other => return Err(format!("array length must be a non-negative integer, found {}", other.describe())),
                    };
                    self.expect(Tok::RBracket)?;
                    Ok(Expr::ArrayLit(Box::new(first), n))
                } else {
                    // list literal `[a, b, c]`, desugared to a block that builds
                    // the array one element at a time through a `let mut`.
                    let mut elems = vec![first];
                    while self.eat(&Tok::Comma) {
                        if self.peek() == Some(&Tok::RBracket) {
                            break;
                        }
                        elems.push(self.parse_expr()?);
                    }
                    self.expect(Tok::RBracket)?;
                    // The element type is not known here; sema infers it from
                    // the values and rewrites this into an array-building block.
                    Ok(Expr::ArrayList(elems))
                }
            }
            other => Err(format!(
                "at byte {}: expected an expression, found {}",
                self.pos(),
                other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
            )),
        }
    }
}

/// A `match` pattern: a wildcard, an integer literal, or an `Enum::Variant`.
/// The variant is resolved to its tag by sema, once the enum is known.
enum Pat {
    Wild,
    Int(i64),
    /// `Enum::Variant` or `Enum::Variant(..)`. The constructor is resolved by
    /// sema; the optional bindings name the payload fields, in field order.
    Variant(String, String, Option<Vec<String>>),
    /// A plain name: binds the whole scrutinee (an irrefutable catch-all). Kept
    /// distinct from `Wild` only so the binding is named.
    Bind(String),
}

/// Whether `e` syntactically contains a call, used to decide if a subscript in
/// a compound assignment must be evaluated into a temporary.
fn expr_has_call(e: &Expr) -> bool {
    match e {
        Expr::Call { .. } => true,
        Expr::Bin(_, a, b) => expr_has_call(a) || expr_has_call(b),
        Expr::Un(_, a) | Expr::Cast(a, _) => expr_has_call(a),
        Expr::Index(a, b) => expr_has_call(a) || expr_has_call(b),
        Expr::Field { base, .. } => expr_has_call(base),
        Expr::If(c, t, els) => {
            expr_has_call(c) || block_has_call_p(t) || els.as_ref().map_or(false, block_has_call_p)
        }
        Expr::Block(b) => block_has_call_p(b),
        Expr::MatchBoundary(b) => block_has_call_p(b),
        Expr::StructLit { fields, .. } => fields.iter().any(|f| expr_has_call(&f.value)),
        Expr::ArrayLit(v, _) => expr_has_call(v),
        _ => false,
    }
}

fn block_has_call_p(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_call_p) || b.tail.as_deref().map_or(false, expr_has_call)
}

fn stmt_has_call_p(s: &Stmt) -> bool {
    match s {
        Stmt::Let { init, .. } => init.as_ref().map_or(false, expr_has_call),
        Stmt::Assign { target, value } => expr_has_call(target) || expr_has_call(value),
        Stmt::Expr(e) => expr_has_call(e),
        Stmt::Return(v) => v.as_ref().map_or(false, expr_has_call),
        Stmt::While(c, b) => expr_has_call(c) || block_has_call_p(b),
        Stmt::Loop(b) => block_has_call_p(b),
        Stmt::For { start, end, body, .. } => {
            expr_has_call(start) || expr_has_call(end) || block_has_call_p(body)
        }
        Stmt::Break | Stmt::Continue => false,
    }
}
