//! Recursive-descent parser for the nrc Rust subset.

use crate::ast::*;
use crate::lexer::{Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
}

/// Expressions that are allowed as statements without a trailing `;`.
fn is_block_like(e: &Expr) -> bool {
    matches!(e, Expr::If(..) | Expr::Block(..))
}

type PResult<T> = Result<T, String>;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Parser { toks, i: 0 }
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
        let mut fns = Vec::new();
        while self.peek().is_some() {
            fns.push(self.parse_fn()?);
        }
        Ok(Program { fns })
    }

    fn parse_fn(&mut self) -> PResult<FnDef> {
        self.expect(Tok::Fn)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        while self.peek() != Some(&Tok::RParen) {
            self.eat(&Tok::Mut); // `mut` on a parameter is accepted and ignored
            let pname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.parse_type()?;
            params.push((pname, ty));
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
        Ok(FnDef { name, params, ret, body, nslots: 0 })
    }

    fn parse_type(&mut self) -> PResult<Type> {
        match self.peek() {
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
            other => Err(format!(
                "at byte {}: expected a type, found {}",
                self.pos(),
                other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
            )),
        }
    }

    fn parse_block(&mut self) -> PResult<Block> {
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
                | Some(Tok::LBracket)
        )
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        match self.peek() {
            Some(Tok::Let) => {
                self.i += 1;
                self.eat(&Tok::Mut);
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
                Ok(Stmt::Let { name, ty, init, slot: -1 })
            }
            Some(Tok::While) => {
                self.i += 1;
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Stmt::While(cond, body))
            }
            Some(Tok::Loop) => {
                self.i += 1;
                let body = self.parse_block()?;
                Ok(Stmt::Loop(body))
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
                self.expect(Tok::Semi)?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while self.eat(&Tok::PipePipe) {
            let rhs = self.parse_and()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_cmp()?;
        while self.eat(&Tok::AmpAmp) {
            let rhs = self.parse_cmp()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_add()?;
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
            let rhs = self.parse_add()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
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
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.i += 1;
                let e = self.parse_unary()?;
                Ok(Expr::Un(UnOp::Neg, Box::new(e)))
            }
            Some(Tok::Bang) => {
                self.i += 1;
                let e = self.parse_unary()?;
                Ok(Expr::Un(UnOp::Not, Box::new(e)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            if self.eat(&Tok::LBracket) {
                let idx = self.parse_expr()?;
                self.expect(Tok::RBracket)?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.eat(&Tok::As) {
                let ty = self.parse_type()?;
                e = Expr::Cast(Box::new(e), ty);
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
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
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while self.peek() != Some(&Tok::RParen) {
                        args.push(self.parse_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Var { name, slot: -1, ty: Type::I64 })
                }
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.parse_expr()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::If) => {
                self.i += 1;
                let cond = self.parse_expr()?;
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
                    // list literal [a, b, c] -> treated as [a; n] is not general;
                    // we support only the repeat form for now.
                    Err(format!(
                        "at byte {}: only array repeat literals `[value; N]` are supported",
                        self.pos()
                    ))
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
