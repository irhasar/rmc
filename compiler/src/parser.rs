//! Recursive-descent parser for the rmc Rust subset.

use crate::ast::*;
use crate::consts::eval_i64;
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
    /// Constant `i64` items by name, collected as they are parsed. The parser
    /// needs their values to size an array type (`[i64; N]`) and to turn a
    /// constant used as a `match` pattern into a literal, both of which happen
    /// before sema. Only `i64` constants that fold to a literal are entered; a
    /// constant that is not yet known at a use site simply stays unresolved and
    /// is diagnosed there, matching rustc's forward-reference rules.
    consts: std::collections::HashMap<String, i64>,
    /// Constant `bool` items by name, so a `const F: bool` used as a `match`
    /// pattern becomes the `true`/`false` pattern.
    const_bools: std::collections::HashMap<String, bool>,
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
    matches!(e, Expr::If(..) | Expr::Block(..) | Expr::MatchBoundary(..) | Expr::Loop(..))
}

type PResult<T> = Result<T, String>;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Parser {
            toks,
            i: 0,
            no_struct_lit: false,
            tmp: 0,
            depth: 0,
            current_impl: None,
            consts: std::collections::HashMap::new(),
            const_bools: std::collections::HashMap::new(),
        }
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

    /// The token after the next one, for a two-token lookahead (`'a: loop`).
    fn peek2(&self) -> Option<&Tok> {
        self.toks.get(self.i + 1).map(|t| &t.tok)
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

    /// Consume a `'label` token if one is next, returning its name.
    fn eat_label(&mut self) -> PResult<Option<String>> {
        if let Some(Tok::Label(n)) = self.peek().cloned() {
            self.i += 1;
            Ok(Some(n))
        } else {
            Ok(None)
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
        // A `const` may be named before it is declared (an array length
        // `[i64; N]`, a pattern `N =>`), so collect and evaluate every `const`
        // item first, then parse the program with the values known.
        self.prescan_consts()?;
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut consts = Vec::new();
        let mut fns = Vec::new();
        while self.peek().is_some() {
            match self.peek() {
                Some(Tok::Struct) => structs.push(self.parse_struct()?),
                Some(Tok::Enum) => enums.push(self.parse_enum()?),
                Some(Tok::Const) => consts.push(self.parse_const()?),
                Some(Tok::Fn) => fns.push(self.parse_fn()?),
                Some(Tok::Impl) => self.parse_impl(&mut fns)?,
                other => {
                    return Err(format!(
                        "at byte {}: expected `fn`, `struct`, `enum`, `const` or `impl`, found {}",
                        self.pos(),
                        other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
                    ))
                }
            }
        }
        Ok(Program { structs, enums, consts, fns })
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
            // `Variant = N`: an explicit discriminant. Only a unit variant may
            // have one, as in rustc (a payload variant's tag is an
            // implementation detail there).
            let mut discr = None;
            if self.eat(&Tok::Eq) {
                if !fields.is_empty() {
                    return Err(format!(
                        "at byte {}: a variant with a payload cannot have an explicit discriminant",
                        self.pos()
                    ));
                }
                let neg = self.eat(&Tok::Minus);
                match self.peek().cloned() {
                    Some(Tok::Int(n)) => {
                        self.bump();
                        discr = Some(if neg {
                            n.checked_neg().ok_or_else(|| {
                                format!("at byte {}: discriminant out of range", self.pos())
                            })?
                        } else {
                            n
                        });
                    }
                    // `-9223372036854775808` is `i64::MIN`; the magnitude alone
                    // does not fit, so it is only valid under a `-`.
                    Some(Tok::IntMinMagnitude) if neg => {
                        self.bump();
                        discr = Some(i64::MIN);
                    }
                    _ => {
                        return Err(format!(
                            "at byte {}: expected an integer after `=` in a variant discriminant",
                            self.pos()
                        ))
                    }
                }
            }
            variants.push(EnumVariantDef { name: vname, fields, discr });
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

    /// Scan the token stream for top-level `const` items and evaluate them, so
    /// a use that precedes the declaration (an array length, a pattern) sees
    /// the value. Only the item's own tokens are consumed: a `{ .. }` or
    /// `( .. )` in its initializer is skipped by bracket matching, and every
    /// other item's body is skipped the same way, so no nested `const` inside a
    /// function body is collected here.
    fn prescan_consts(&mut self) -> PResult<()> {
        let saved = self.i;
        let mut items: Vec<ConstDef> = Vec::new();
        self.i = 0;
        loop {
            match self.peek() {
                None => break,
                Some(Tok::Const) => items.push(self.parse_const()?),
                Some(Tok::Struct) | Some(Tok::Enum) => {
                    // Skip `Name` then a balanced `{ .. }` body.
                    self.i += 1;
                    if matches!(self.peek(), Some(Tok::Ident(_))) {
                        self.i += 1;
                    }
                    self.skip_balanced(Tok::LBrace, Tok::RBrace)?;
                }
                Some(Tok::Impl) => {
                    self.i += 1;
                    if matches!(self.peek(), Some(Tok::Ident(_))) {
                        self.i += 1;
                    }
                    self.skip_balanced(Tok::LBrace, Tok::RBrace)?;
                }
                Some(Tok::Fn) => {
                    // Skip the name, the parameter list, the return type and the
                    // body, so a `const` inside a function body is not seen.
                    self.i += 1;
                    if matches!(self.peek(), Some(Tok::Ident(_))) {
                        self.i += 1;
                    }
                    self.skip_balanced(Tok::LParen, Tok::RParen)?;
                    // Skip the return type to the body. It is not parsed here,
                    // because a `[T; N]` return type would need the constant
                    // values that are still being collected.
                    if self.eat(&Tok::Arrow) {
                        while let Some(t) = self.peek() {
                            if *t == Tok::LBrace {
                                break;
                            }
                            self.i += 1;
                        }
                    }
                    self.skip_balanced(Tok::LBrace, Tok::RBrace)?;
                }
                Some(_) => self.i += 1,
            }
        }
        self.i = saved;
        self.resolve_consts(&items);
        Ok(())
    }

    /// Advance past a balanced `open .. close` group, starting at `open`.
    fn skip_balanced(&mut self, open: Tok, close: Tok) -> PResult<()> {
        if self.peek() != Some(&open) {
            return Ok(());
        }
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            if *t == open {
                depth += 1;
            } else if *t == close {
                depth -= 1;
                if depth == 0 {
                    self.i += 1;
                    return Ok(());
                }
            }
            self.i += 1;
        }
        Err(format!("at byte {}: unexpected end of input", self.pos()))
    }

    /// `const NAME: TYPE = EXPR;`. The declared type is resolved by sema; the
    /// value is evaluated here, as soon as it is parsed, so a later array type
    /// or pattern can use it. A value that does not fold yet (it names a
    /// constant declared later) is left for `resolve_consts`, which reruns once
    /// every item has been seen.
    fn parse_const(&mut self) -> PResult<ConstDef> {
        self.expect(Tok::Const)?;
        let name = self.expect_ident()?;
        self.expect(Tok::Colon)?;
        let ty = self.parse_type()?;
        self.expect(Tok::Eq)?;
        let value = self.parse_expr()?;
        self.expect(Tok::Semi)?;
        Ok(ConstDef { name, ty, value })
    }

    /// Evaluate every `const` item into `self.consts`/`self.const_bools`, to a
    /// fixpoint so a constant may name one declared later. Runs after the whole
    /// program is parsed, so a forward reference resolves here even though it
    /// did not while the use site was parsed.
    fn resolve_consts(&mut self, defs: &[ConstDef]) {
        let items: Vec<(String, Type, Expr)> = defs
            .iter()
            .map(|d| (d.name.clone(), d.ty.clone(), d.value.clone()))
            .collect();
        for (name, val) in crate::consts::resolve(&items) {
            match val {
                crate::consts::ConstVal::Int(v) => {
                    self.consts.insert(name, v);
                }
                crate::consts::ConstVal::Bool(v) => {
                    self.const_bools.insert(name, v);
                }
            }
        }
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
            let pm = self.eat(&Tok::Mut);
            let pname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.parse_type()?;
            params.push((pname, ty));
            param_mut.push(pm);
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
        // A method takes `self` as its first parameter; an associated function
        // (`P::new(..)`) does not. `&self` / `&mut self` make the receiver a
        // reference, so the call passes the value's address rather than a copy;
        // `self` takes the receiver by value. An associated function is parsed
        // exactly like a free function, minus the `self` receiver.
        let takes_self = match self.peek() {
            Some(Tok::Amp) => true,
            Some(Tok::Ident(n)) => n == "self",
            _ => false,
        };
        let mut params = Vec::new();
        let mut param_mut = Vec::new();
        let (by_ref, by_mut) = if takes_self {
            let by_ref = self.eat(&Tok::Amp);
            let by_mut = by_ref && self.eat(&Tok::Mut);
            let first = self.expect_ident()?;
            debug_assert_eq!(first, "self");
            if self.eat(&Tok::Comma) {
                self.parse_param_list(&mut params, &mut param_mut)?;
            }
            (by_ref, by_mut)
        } else {
            if self.peek() != Some(&Tok::RParen) {
                self.parse_param_list(&mut params, &mut param_mut)?;
            }
            (false, false)
        };
        self.expect(Tok::RParen)?;
        let ret = if self.eat(&Tok::Arrow) { self.parse_type()? } else { Type::Unit };
        let body = self.parse_block()?;
        Ok(FnDef {
            name: format!("{ty}${name}"),
            self_ty: Some(ty.to_string()),
            takes_self,
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

    /// Parse `p: T, q: T, ..` up to (not including) the closing `)`. The caller
    /// has already consumed the comma that follows the previous parameter.
    fn parse_param_list(
        &mut self,
        params: &mut Vec<(String, Type)>,
        param_mut: &mut Vec<bool>,
    ) -> PResult<()> {
        while self.peek() != Some(&Tok::RParen) {
            let pm = self.eat(&Tok::Mut);
            let pname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let pty = self.parse_type()?;
            params.push((pname, pty));
            param_mut.push(pm);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(())
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
                // A constant length: a literal, or an expression over constants
                // (`[i64; N]`, `[i64; N + 1]`). It is evaluated here because the
                // array's size is fixed at parse time.
                let len = self.parse_expr()?;
                let n = match eval_i64(&len, &self.consts) {
                    Some(v) if v >= 0 => v as usize,
                    _ => {
                        return Err(format!(
                            "at byte {}: array length must be a non-negative constant integer",
                            self.pos()
                        ))
                    }
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
            // `'a: while ..` / `'a: for ..` are statements; `'a: loop ..` is an
            // expression and flows through the tail-expression path below.
            if let Some(Tok::Label(_)) = self.peek() {
                if self.peek2() == Some(&Tok::Colon)
                    && self.toks.get(self.i + 2).map(|t| &t.tok) != Some(&Tok::Loop)
                {
                    stmts.push(self.parse_stmt()?);
                    continue;
                }
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
                    // `if`/block/`loop` expressions are valid statements without
                    // a `;`. A `loop` here is a statement, so a value it yields
                    // (via `break value`) is discarded: wrap it so it is typed
                    // as `()`, not as its break type.
                    let e = match e {
                        Expr::Loop(b, _, ty, label) => Expr::Loop(b, -2, ty, label),
                        other => other,
                    };
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

    fn parse_for(&mut self) -> PResult<Stmt> {
        self.parse_for_labeled(None)
    }

    /// A labelled loop statement, `'name: while ..` or `'name: for ..`. A
    /// labelled `loop` is an expression, so it does not reach here.
    fn parse_labeled_loop_stmt(&mut self, label: String) -> PResult<Stmt> {
        match self.peek() {
            Some(Tok::While) => {
                self.i += 1;
                // `'a: while let PAT = EXPR { body }`: the label would be lost
                // in the desugar to `loop { if let .. }`, so reject it rather
                // than silently drop the label.
                if self.peek() == Some(&Tok::Let) {
                    return Err(format!(
                        "at byte {}: a label on `while let` is not supported",
                        self.pos()
                    ));
                }
                let cond = self.parse_cond()?;
                let body = self.parse_block()?;
                Ok(Stmt::While(cond, body, Some(label)))
            }
            Some(Tok::For) => {
                self.i += 1;
                self.parse_for_labeled(Some(label))
            }
            other => Err(format!(
                "at byte {}: a label must prefix `loop`, `while` or `for`, found {}",
                self.pos(),
                other.map(|t| t.describe()).unwrap_or_else(|| "end of input".into())
            )),
        }
    }

    /// `for PAT in ITER { body }`. Two iterables are supported: an integer
    /// range (`a..b` / `a..=b`) and an array.
    ///
    /// The range form keeps its own node, because `continue` must still run the
    /// increment, so it cannot be desugared to a `while`.
    ///
    /// The array form desugars to that same range loop, indexing the array:
    ///
    /// ```text
    /// { let arr = EXPR; for i in 0..arr.len() { let PAT = arr[i]; body } }
    /// ```
    ///
    /// `EXPR` is bound to a hidden temporary so an array expression with a side
    /// effect (a call, a literal) runs exactly once, and `arr.len()` folds to
    /// the array's compile-time length, so the loop bound is a literal and the
    /// loop carries no hidden slot.
    fn parse_for_labeled(&mut self, label: Option<String>) -> PResult<Stmt> {
        // A loop variable that names a constant shadows it (rustc E0530).
        // `parse_pattern` would otherwise read the name as that constant's
        // value, so the check happens here, before the pattern is parsed.
        if let Some(Tok::Ident(n)) = self.peek() {
            if self.consts.contains_key(n) || self.const_bools.contains_key(n) {
                return Err(format!(
                    "identifier `{n}` shadows a constant of the same name"
                ));
            }
        }
        let pat = self.parse_pattern()?;
        self.expect(Tok::In)?;
        let head = self.parse_cond()?;
        // `for x in &arr` iterates by *reference* in Rust: `x` is `&i64`, not
        // `i64`. rmc iterates by value, so accepting the `&` form would silently
        // give `x` the wrong type (and diverge from rustc's typing). Reject it
        // rather than mis-compile; the by-value form is `for x in arr`.
        if matches!(head, Expr::AddrOf { .. }) {
            return Err(format!(
                "at byte {}: `for` over a reference is not supported; \
                 iterate by value with `for x in arr`",
                self.pos()
            ));
        }
        if self.peek() == Some(&Tok::DotDot) || self.peek() == Some(&Tok::DotDotEq) {
            let inclusive = self.eat(&Tok::DotDotEq);
            if !inclusive {
                self.expect(Tok::DotDot)?;
            }
            let end = self.parse_cond()?;
            let body = self.parse_block()?;
            let var = match pat {
                Pat::Bind(n) => n,
                Pat::Wild => {
                    let n = format!("\u{0}fi{}", self.tmp);
                    self.tmp += 1;
                    n
                }
                _ => {
                    return Err(format!(
                        "at byte {}: `for` over a range needs a binding or `_`",
                        self.pos()
                    ))
                }
            };
            return Ok(Stmt::For {
                var,
                var_slot: -1,
                start: head,
                end,
                end_slot: -1,
                inclusive,
                body,
                label,
            });
        }
        let body = self.parse_block()?;
        Ok(self.desugar_for_array(pat, head, body, label))
    }

    /// `for PAT in ARRAY { body }` -> an index loop over the array (see
    /// `parse_for`). `PAT` must be a binding or `_`; a compound pattern is
    /// rejected rather than mis-bound, since rmc has no destructuring `let`.
    /// A label on the `for` is carried to the desugared loop, so `break 'a`
    /// still targets it.
    fn desugar_for_array(&mut self, pat: Pat, arr: Expr, body: Block, label: Option<String>) -> Stmt {
        let n = self.tmp;
        self.tmp += 1;
        let arr_name = format!("\u{0}fa{n}");
        let idx_name = format!("\u{0}fi{n}");
        let bind_name = match pat {
            Pat::Bind(name) => name,
            Pat::Wild => format!("\u{0}fd{n}"),
            _ => {
                return Stmt::Expr(Expr::Block(Block {
                    stmts: vec![Stmt::Expr(Expr::Int(0))],
                    tail: None,
                }))
            }
        };
        let arr_var =
            Expr::Var { name: arr_name.clone(), slot: -1, ty: Type::I64 };
        let idx_var =
            Expr::Var { name: idx_name.clone(), slot: -1, ty: Type::I64 };
        // `arr.len()` — a `MethodCall` sema folds to the array's constant length.
        let len = Expr::MethodCall {
            recv: Box::new(arr_var.clone()),
            name: "len".to_string(),
            args: Vec::new(),
        };
        let elem = Expr::Index(Box::new(arr_var), Box::new(idx_var));
        // `let PAT = arr[i];` runs before the user body each iteration.
        let mut stmts = vec![Stmt::Let {
            name: bind_name,
            ty: None,
            init: Some(elem),
            slot: -1,
            mutable: false,
        }];
        stmts.extend(body.stmts);
        let loop_body = Block { stmts, tail: body.tail };
        let for_stmt = Stmt::For {
            var: idx_name,
            var_slot: -1,
            start: Expr::Int(0),
            end: len,
            end_slot: -1,
            inclusive: false,
            body: loop_body,
            label,
        };
        // Bind the array once, outside the loop.
        let arr_let = Stmt::Let {
            name: arr_name,
            ty: None,
            init: Some(arr),
            slot: -1,
            mutable: false,
        };
        Stmt::Expr(Expr::Block(Block { stmts: vec![arr_let, for_stmt], tail: None }))
    }

    /// Consume the `;` that ends a statement, but allow it to be omitted when
    /// the statement is the last thing in a block (`{ return 2 }`, `{ break }`).
    /// Rust treats `return`/`break`/`continue` as diverging, so a trailing
    /// expression is impossible after them and the `;` is optional.
    fn eat_semi_at_block_end(&mut self) -> PResult<()> {
        if self.eat(&Tok::Semi) || self.peek() == Some(&Tok::RBrace) {
            Ok(())
        } else {
            Err(format!("at byte {}: expected `;`", self.pos()))
        }
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
                | Some(Tok::Loop)
                // `'a: loop { .. }` is a labelled loop expression.
                | Some(Tok::Label(_))
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
        // A loop label may prefix `while` or `for`; a labelled `loop` is an
        // expression and is handled by `parse_primary` (routed here or there by
        // `parse_block_inner` depending on whether it is a tail expression).
        if let Some(Tok::Label(n)) = self.peek().cloned() {
            self.i += 1;
            self.expect(Tok::Colon)?;
            return self.parse_labeled_loop_stmt(n);
        }
        match self.peek() {
            Some(Tok::Let) => {
                self.i += 1;
                let mutable = self.eat(&Tok::Mut);
                // `let _ = e;` (and `let _: T = e;`, and bare `let _;`) binds
                // nothing. Desugar to a block holding a hidden binding so the
                // initializer is still evaluated once and any annotation is
                // still type-checked, then dropped; with no initializer there is
                // nothing to check.
                if self.peek() == Some(&Tok::Underscore) {
                    self.i += 1;
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
                    let stmts = match init {
                        Some(e) => {
                            // Bind to `_` so a type-mismatch diagnostic names the
                            // discard, not an internal temporary. `_` cannot be
                            // written as an ordinary binding, so it never clashes.
                            vec![Stmt::Let {
                                name: "_".to_string(),
                                ty,
                                init: Some(e),
                                slot: -1,
                                mutable: false,
                            }]
                        }
                        // `let _;` with no annotation has no type to infer, which
                        // rustc rejects (E0282); `let _: T;` is a no-op.
                        None if ty.is_none() => {
                            return Err(format!(
                                "at byte {}: `let _;` needs a type annotation",
                                self.pos()
                            ))
                        }
                        None => Vec::new(),
                    };
                    return Ok(Stmt::Expr(Expr::Block(Block { stmts, tail: None })));
                }
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
                // `while let PAT = EXPR { body }` re-tests `EXPR` each iteration
                // and stops at the first value `PAT` does not match.
                if self.peek() == Some(&Tok::Let) {
                    self.i += 1;
                    let pat = self.parse_pattern()?;
                    self.expect(Tok::Eq)?;
                    let scrutinee = self.parse_cond()?;
                    let body = self.parse_block()?;
                    return Ok(self.desugar_while_let(pat, scrutinee, body));
                }
                let cond = self.parse_cond()?;
                let body = self.parse_block()?;
                Ok(Stmt::While(cond, body, None))
            }
            Some(Tok::For) => {
                self.i += 1;
                self.parse_for()
            }
            Some(Tok::Return) => {
                self.i += 1;
                // `return`, `return expr` and `return;` are all valid, and the
                // `;` may be omitted when the `return` is the last thing in the
                // block (`{ return 2 }`), as in Rust.
                if self.eat(&Tok::Semi) || self.peek() == Some(&Tok::RBrace) {
                    return Ok(Stmt::Return(None));
                }
                let e = self.parse_expr()?;
                self.eat_semi_at_block_end()?;
                Ok(Stmt::Return(Some(e)))
            }
            Some(Tok::Break) => {
                self.i += 1;
                // `break`, `break 'label`, `break expr` or `break 'label expr`;
                // the `;` may be omitted at the end of a block. A value is only
                // legal in a `loop`, checked by sema.
                let label = self.eat_label()?;
                if self.eat(&Tok::Semi) || self.peek() == Some(&Tok::RBrace) {
                    return Ok(Stmt::Break(None, label));
                }
                let e = self.parse_expr()?;
                self.eat_semi_at_block_end()?;
                Ok(Stmt::Break(Some(e), label))
            }
            Some(Tok::Continue) => {
                self.i += 1;
                let label = self.eat_label()?;
                self.eat_semi_at_block_end()?;
                Ok(Stmt::Continue(label))
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
        let needs_temp = match target {
            Expr::Index(_, idx) => expr_has_call(idx),
            _ => false,
        };
        if !needs_temp {
            return Stmt::Assign {
                target: target.clone(),
                value: Expr::Bin(op, Box::new(target.clone()), Box::new(value)),
            };
        }

        let tv = |name: &str| Expr::Var { name: name.to_string(), slot: -1, ty: Type::I64 };
        let mut stmts = Vec::new();

        // The modifying operand is evaluated before the assignee place (the one
        // right-to-left expression), so its side effects run first: bind it now,
        // ahead of the index temporary, or a call in the index would overtake it.
        let rv = format!("\u{0}rv{}", self.tmp);
        self.tmp += 1;
        stmts.push(Stmt::Let {
            name: rv.clone(),
            ty: Some(Type::I64),
            init: Some(value),
            slot: -1,
            mutable: true,
        });

        // Bind the read target and the write target to the *same* location.
        // For an index the subscript is evaluated once into a temporary and
        // reused, so `a[next()] += 1` advances the counter once, not twice.
        // A `Var` or `Field` target has no side effect on re-evaluation.
        let (read, write) = match target {
            Expr::Index(base, idx) => {
                // NUL is not a valid identifier byte, so a generated name can
                // never collide with one the source spells.
                let it = format!("\u{0}ix{}", self.tmp);
                self.tmp += 1;
                stmts.push(Stmt::Let {
                    name: it.clone(),
                    ty: Some(Type::I64),
                    init: Some((**idx).clone()),
                    slot: -1,
                    mutable: true,
                });
                let idxv = Box::new(tv(&it));
                (
                    Expr::Index(base.clone(), idxv.clone()),
                    Expr::Index(base.clone(), idxv),
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
            value: Expr::Bin(op, Box::new(tv(&tmp)), Box::new(tv(&rv))),
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
        let mut lhs = self.parse_cast()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                Some(Tok::Percent) => BinOp::Mod,
                _ => break,
            };
            self.i += 1;
            let rhs = self.parse_cast()?;
            self.bump_depth()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `e as T`. `as` binds looser than every unary operator, so `*r as i64`
    /// is `(*r) as i64` and `-x as i64` is `(-x) as i64`, as in Rust; it binds
    /// tighter than `* / %`. Rust places `as` in the binary precedence chain
    /// just above `*`, which is where this level sits. The right-hand side is a
    /// type, not an expression, so there is no operand recursion.
    fn parse_cast(&mut self) -> PResult<Expr> {
        let mut e = self.parse_unary()?;
        while self.eat(&Tok::As) {
            self.bump_depth()?;
            let ty = self.parse_type()?;
            e = Expr::Cast(Box::new(e), ty);
        }
        Ok(e)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.i += 1;
                self.bump_depth()?;
                // `-9223372036854775808` is the one literal whose magnitude
                // exceeds `i64::MAX`; the lexer hands it over as a distinct
                // token so only this `-` position can make it valid.
                if self.peek() == Some(&Tok::IntMinMagnitude) {
                    self.i += 1;
                    return Ok(Expr::Int(i64::MIN));
                }
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
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// One pattern: an integer, `true`/`false`, `Enum::Variant`, `Enum::Variant(a, b)`,
    /// a bare binding name, or `_`. Shared by `match` arms and `if let`.
    fn parse_pattern(&mut self) -> PResult<Pat> {
        let first = self.parse_pattern_primary()?;
        if self.peek() != Some(&Tok::Pipe) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat(&Tok::Pipe) {
            alts.push(self.parse_pattern_primary()?);
        }
        // A binding must bind the same names in every alternative (rustc's
        // E0408); rmc has no name-consistency check, so it rejects a binding in
        // an or-pattern outright rather than mis-bind it.
        if let Some(Pat::Bind(n)) = alts.iter().find(|a| matches!(a, Pat::Bind(_))) {
            return Err(format!(
                "at byte {}: variable `{n}` is not bound in all patterns of the or-pattern",
                self.pos()
            ));
        }
        Ok(Pat::Or(alts))
    }

    /// An integer pattern literal: `123` or `-123`. The magnitude `2^63` is in
    /// range only directly under a `-`, where it is `i64::MIN`; standing alone
    /// it is out of range, as in rustc. Shared by a plain integer pattern and
    /// both bounds of a range pattern.
    fn parse_signed_pat_int(&mut self) -> PResult<i64> {
        let neg = self.eat(&Tok::Minus);
        match self.bump() {
            Tok::Int(v) if neg => Ok(v.checked_neg().ok_or_else(|| {
                format!("at byte {}: pattern literal out of range", self.pos())
            })?),
            Tok::Int(v) => Ok(v),
            Tok::IntMinMagnitude if neg => Ok(i64::MIN),
            Tok::IntMinMagnitude => Err(format!(
                "at byte {}: integer literal `9223372036854775808` is out of range for `i64`",
                self.pos()
            )),
            other => Err(format!(
                "at byte {}: expected an integer in a pattern, found {}",
                self.pos(), other.describe()
            )),
        }
    }

    fn parse_pattern_primary(&mut self) -> PResult<Pat> {
        match self.peek().cloned() {
            Some(Tok::Underscore) => {
                self.i += 1;
                Ok(Pat::Wild)
            }
            Some(Tok::Minus) | Some(Tok::Int(_)) => {
                let lo = self.parse_signed_pat_int()?;
                self.maybe_range(lo)
            }
            Some(Tok::True) => {
                self.i += 1;
                Ok(Pat::Bool(true))
            }
            Some(Tok::False) => {
                self.i += 1;
                Ok(Pat::Bool(false))
            }
            // A string literal pattern, matched by content.
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Pat::Str(s))
            }
            // `Enum::Variant`, `Enum::Variant(a, b)`, a constant, or a binding.
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
                    Ok(Pat::Variant(n, variant, bindings))
                } else if let Some(v) = self.consts.get(&n) {
                    // A bare name that names a constant is that constant's
                    // value, not a fresh binding, exactly as rustc reads it.
                    Ok(Pat::Int(*v))
                } else if let Some(b) = self.const_bools.get(&n) {
                    Ok(Pat::Bool(*b))
                } else {
                    // A bare name binds the scrutinee; `_` already matched the
                    // wildcard case above.
                    Ok(Pat::Bind(n))
                }
            }
            other => Err(format!(
                "at byte {}: expected an integer, `true`/`false`, a string literal, `Enum::Variant` or `_` pattern, found {}",
                self.pos(),
                other.map(|t| t.describe()).unwrap_or_else(|| "end of input".into())
            )),
        }
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
        let mut arms: Vec<(Pat, Option<Expr>, Expr)> = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err(format!("at byte {}: unexpected end of input in `match`", self.pos()));
            }
            let pat = self.parse_pattern()?;
            // An optional guard: `pat if cond => body`. The condition is parsed
            // like an `if` condition, so a struct literal needs parentheses.
            let guard = if self.eat(&Tok::If) {
                Some(self.parse_cond()?)
            } else {
                None
            };
            self.expect(Tok::FatArrow)?;
            let body = self.parse_arm_body()?;
            arms.push((pat, guard, body));
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

    /// Parse a `match` arm body. As in Rust, an arm body is an expression, but a
    /// statement is also accepted: a bare assignment (`1 => x = 1`), or a
    /// `return`/`break`/`continue`. rmc parses those only as statements, so
    /// `parse_expr` would stop at the `=` or reject the keyword outright.
    /// Accepting them here keeps an arm body from having to be wrapped in a
    /// block. `if let`/`while let` bodies are blocks, so they do not reach this.
    fn parse_arm_body(&mut self) -> PResult<Expr> {
        // A terminator is a statement, not an expression: build the block and
        // leave the arm's `,`/`}` for the caller. The value, if any, is an
        // expression that stops before that terminator.
        let stmt = match self.peek() {
            Some(Tok::Return) => {
                self.i += 1;
                Stmt::Return(self.arm_terminator_value()?)
            }
            Some(Tok::Break) => {
                self.i += 1;
                let label = self.eat_label()?;
                Stmt::Break(self.arm_terminator_value()?, label)
            }
            Some(Tok::Continue) => {
                self.i += 1;
                let label = self.eat_label()?;
                Stmt::Continue(label)
            }
            _ => {
                let target = self.parse_expr()?;
                if self.eat(&Tok::Eq) {
                    Stmt::Assign { target, value: self.parse_expr()? }
                } else if let Some(op) = self.peek_compound_op() {
                    self.i += 1;
                    let value = self.parse_expr()?;
                    self.desugar_compound(&target, op, value)
                } else {
                    return Ok(target);
                }
            }
        };
        Ok(Expr::Block(Block { stmts: vec![stmt], tail: None }))
    }

    /// The optional value of a `return`/`break` arm body: absent when the arm
    /// ends right there (`,` or `}`), otherwise the following expression.
    fn arm_terminator_value(&mut self) -> PResult<Option<Expr>> {
        if self.peek() == Some(&Tok::Comma) || self.peek() == Some(&Tok::RBrace) {
            Ok(None)
        } else {
            Ok(Some(self.parse_expr()?))
        }
    }

    /// If a `..`/`..=` follows the integer just parsed, consume it and the
    /// upper bound to build a range pattern; otherwise the integer stands
    /// alone. Both bounds are `i64`, so `1..=5` and `-3..0` both work.
    fn maybe_range(&mut self, lo: i64) -> PResult<Pat> {
        let inclusive = match self.peek() {
            Some(Tok::DotDot) => false,
            Some(Tok::DotDotEq) => true,
            _ => return Ok(Pat::Int(lo)),
        };
        self.i += 1;
        let hi = self.parse_signed_pat_int()?;
        if !inclusive && lo >= hi {
            return Err(format!(
                "at byte {}: empty range pattern `{}..{}`",
                self.pos(), lo, hi
            ));
        }
        if inclusive && lo > hi {
            return Err(format!(
                "at byte {}: empty range pattern `{}..={}`",
                self.pos(), lo, hi
            ));
        }
        Ok(Pat::Range(lo, hi, inclusive))
    }

    /// Whether a pattern matches every value: `_`, a binding, or an
    /// or-pattern with such an alternative.
    fn pat_irrefutable(p: &Pat) -> bool {
        match p {
            Pat::Wild | Pat::Bind(_) => true,
            Pat::Or(alts) => alts.iter().any(Self::pat_irrefutable),
            _ => false,
        }
    }

    /// Fold the arms into nested `if`s. The scrutinee is named once; an arm
    /// with `_` becomes the `else` (so any arm after it is unreachable, as in
    /// Rust). With no `_`, the chain has no final `else`, so a non-matching
    /// value yields `()`.
    fn desugar_match(&mut self, scrutinee: Expr, arms: Vec<(Pat, Option<Expr>, Expr)>) -> Expr {
        self.desugar_match_arms(scrutinee, arms, false)
    }

    /// `desugar_match`, with `check_dead_arms` deciding how an irrefutable arm
    /// (`_` or a binding) treats the arms after it. `match` leaves them out of
    /// the tree, as before; `if let` keeps them behind an `if true` so their
    /// type is still checked (see `irrefutable_arm`).
    fn desugar_match_arms(
        &mut self,
        scrutinee: Expr,
        arms: Vec<(Pat, Option<Expr>, Expr)>,
        check_dead_arms: bool,
    ) -> Expr {
        // A `_` or binding arm makes the match exhaustive in every position;
        // an or-pattern is irrefutable when any alternative is (`1 | _`). A
        // *guarded* irrefutable arm is not a catch-all: `_ if c` can fail, so
        // the arms after it are still reachable.
        let has_catch_all = arms.iter().any(|(p, g, _)| g.is_none() && Self::pat_irrefutable(p));
        let (subj, prelude) = match scrutinee {
            Expr::Var { .. } | Expr::Int(_) | Expr::EnumVariant { .. } => (scrutinee, None),
            other => {
                let name = format!("\u{0}mt{}", self.tmp);
                self.tmp += 1;
                (
                    Expr::Var { name: name.clone(), slot: -1, ty: Type::I64 },
                    // No annotation: the temp takes the scrutinee's type, which
                    // may be an enum rather than an integer.
                    Some(Stmt::Let { name, ty: None, init: Some(other), slot: -1, mutable: false }),
                )
            }
        };
        let mut acc: Option<Expr> = None;
        for (pat, guard, body) in arms.into_iter().rev() {
            acc = Some(Self::desugar_arm(pat, guard, &subj, body, acc, check_dead_arms));
        }
        let acc = acc.expect("a `match` has at least one arm");
        let acc = Expr::MatchBoundary(
            Block { stmts: Vec::new(), tail: Some(Box::new(acc)) },
            has_catch_all,
        );
        match prelude {
            None => acc,
            Some(stmt) => Expr::Block(Block { stmts: vec![stmt], tail: Some(Box::new(acc)) }),
        }
    }

    /// `if let PAT = EXPR { then } else { els }` is Rust's sugar for
    /// `match EXPR { PAT => then, _ => els_or_unit }`, so it desugars to a
    /// two-arm `match` and reuses that path. Unlike `match`, an `if let` need
    /// not be exhaustive: a non-matching value falls through to the `_` arm,
    /// which is the `else` block or `()` — so a non-unit `then` with no `else`
    /// is a branch-type error, as in rustc.
    fn desugar_if_let(&mut self, pat: Pat, scrutinee: Expr, then: Block, els: Option<Block>) -> Expr {
        let fallback = match els {
            Some(b) => Expr::Block(b),
            None => Expr::Block(Block { stmts: Vec::new(), tail: None }),
        };
        let arms = vec![(pat, None, Expr::Block(then)), (Pat::Wild, None, fallback)];
        self.desugar_match_arms(scrutinee, arms, true)
    }

    /// `while let PAT = EXPR { body }` re-tests `EXPR` each iteration and stops
    /// at the first value `PAT` does not match. It is exactly an `if let` whose
    /// `else` breaks out of an enclosing `loop`:
    ///
    /// ```text
    /// loop { if let PAT = EXPR { body } else { break; } }
    /// ```
    ///
    /// so it desugars to those two existing statements and sema and codegen
    /// need no new case. `EXPR` sits inside the loop, so it is re-evaluated
    /// every iteration (as in Rust); the `if let` binds its non-`Var` scrutinee
    /// to a fresh temporary each time.
    fn desugar_while_let(&mut self, pat: Pat, scrutinee: Expr, body: Block) -> Stmt {
        let break_else = Block {
            stmts: vec![Stmt::Break(None, None)],
            tail: None,
        };
        // The `if let` is a *statement* in the loop body, so its value is
        // discarded and the loop body is unit even when `body` yields a value.
        let test = self.desugar_if_let(pat, scrutinee, body, Some(break_else));
        let loop_body = Block { stmts: vec![Stmt::Expr(test)], tail: None };
        // The loop is unit-valued: the `else` arm always contains a plain
        // `break;`, so a `break value` in the body is rejected as a type
        // mismatch (rustc E0571 rejects it too).
        Stmt::Expr(Expr::Loop(loop_body, -1, Type::Unit, None))
    }

    /// One arm of a desugared `match` or `if let`: the pattern test, the
    /// bindings it introduces, and the fallback `rest` for a non-match.
    /// The boolean test for one alternative of an or-pattern. Only a test, not
    /// an arm: the shared body and fallback stay outside, so an or-pattern does
    /// not duplicate the body. A binding or `_` alternative is always true, but
    /// rmc does not support bindings in an or-pattern, so `_` is the only
    /// irrefutable alternative here.
    fn or_cond(pat: &Pat, subj: &Expr) -> Expr {
        match pat {
            Pat::Int(k) => Expr::Bin(BinOp::Eq, Box::new(subj.clone()), Box::new(Expr::Int(*k))),
            Pat::Bool(b) => Expr::BoolTest { scrutinee: Box::new(subj.clone()), value: *b },
            // A string pattern tests by content: `Bin(Eq, ..)` over a `str`,
            // which sema lowers to the byte-wise helper. A pointer compare
            // would never match two distinct-but-equal literals.
            Pat::Str(s) => Expr::Bin(
                BinOp::Eq,
                Box::new(subj.clone()),
                Box::new(Expr::Str(s.clone())),
            ),
            Pat::Range(lo, hi, inclusive) => {
                let lower = Expr::Bin(BinOp::Le, Box::new(Expr::Int(*lo)), Box::new(subj.clone()));
                let upper = Expr::Bin(
                    if *inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(subj.clone()),
                    Box::new(Expr::Int(*hi)),
                );
                Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper))
            }
            Pat::Variant(enum_name, variant, _) => Expr::VariantTest {
                scrutinee: Box::new(subj.clone()),
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                tag: -1,
                n_binds: -1,
            },
            // `_` in an or-pattern: always true, so the whole condition is true.
            Pat::Wild => Expr::Bool(true),
            // A binding inside an or-pattern is rejected before this point.
            Pat::Bind(_) | Pat::Or(_) => unreachable!("or-pattern alternative"),
        }
    }

    fn desugar_arm(
        pat: Pat,
        guard: Option<Expr>,
        subj: &Expr,
        body: Expr,
        rest: Option<Expr>,
        check_dead: bool,
    ) -> Expr {
        // Each pattern yields its test (`None` when it is irrefutable) and any
        // bindings it introduces. The test is an ordinary expression, so sema
        // and codegen need no new case for a guard: it is folded in with `&&`,
        // and an irrefutable arm's guard becomes the whole `if` condition.
        let (bindings, test): (Vec<Stmt>, Option<Expr>) = match pat {
            Pat::Wild => (Vec::new(), None),
            // `name => body` binds the whole scrutinee.
            Pat::Bind(name) => (
                vec![Stmt::Let {
                    name,
                    ty: None,
                    init: Some(subj.clone()),
                    slot: -1,
                    mutable: false,
                }],
                None,
            ),
            Pat::Int(k) => (
                Vec::new(),
                Some(Expr::Bin(BinOp::Eq, Box::new(subj.clone()), Box::new(Expr::Int(k)))),
            ),
            // A string literal pattern: a content compare, which sema lowers to
            // the byte-wise helper (a pointer compare would never match).
            Pat::Str(s) => (
                Vec::new(),
                Some(Expr::Bin(BinOp::Eq, Box::new(subj.clone()), Box::new(Expr::Str(s)))),
            ),
            // `lo..hi` is `lo <= s && s < hi`; `lo..=hi` uses `<=` on both
            // sides. A plain `Bin(And, ..)` of comparisons, so sema and codegen
            // need no new case.
            Pat::Range(lo, hi, inclusive) => {
                let lower =
                    Expr::Bin(BinOp::Le, Box::new(Expr::Int(lo)), Box::new(subj.clone()));
                let upper = Expr::Bin(
                    if inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(subj.clone()),
                    Box::new(Expr::Int(hi)),
                );
                (Vec::new(), Some(Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper))))
            }
            // `a | b | c => body` runs `body` when any alternative matches, so
            // fold the alternatives into `cond_a || cond_b || cond_c`. `or_cond`
            // builds just the test; the body is not duplicated.
            Pat::Or(alts) => {
                // An or-pattern with a `_` alternative matches every value, so
                // the arm is irrefutable.
                if alts.iter().any(|a| matches!(a, Pat::Wild)) {
                    (Vec::new(), None)
                } else {
                    let mut cond = Self::or_cond(&alts[0], subj);
                    for alt in &alts[1..] {
                        cond = Expr::Bin(BinOp::Or, Box::new(cond), Box::new(Self::or_cond(alt, subj)));
                    }
                    (Vec::new(), Some(cond))
                }
            }
            // `true`/`false` become a `BoolTest`, not a `Bin(Eq, ..)`, so a
            // chain of them can be recognised as an exhaustive bool match.
            Pat::Bool(b) => (
                Vec::new(),
                Some(Expr::BoolTest { scrutinee: Box::new(subj.clone()), value: b }),
            ),
            Pat::Variant(enum_name, variant, payload) => {
                let n_binds = match &payload {
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
                let binds = match payload {
                    Some(names) => names
                        .into_iter()
                        .enumerate()
                        .map(|(idx, name)| Stmt::Let {
                            name,
                            ty: None,
                            mutable: false,
                            init: Some(Expr::EnumPayload {
                                recv: Box::new(subj.clone()),
                                enum_name: enum_name.clone(),
                                variant: variant.clone(),
                                idx,
                                offset: 0,
                                ty: Type::Unit,
                            }),
                            slot: -1,
                        })
                        .collect(),
                    None => Vec::new(),
                };
                (binds, Some(cond))
            }
        };
        let arm = match test {
            Some(cond) => {
                // `pat if g => body` is `pat => body` only when `g` holds, so
                // the guard joins the pattern test with `&&` and a failed guard
                // falls through to `rest`.
                let cond = match guard {
                    Some(g) => Expr::Bin(BinOp::And, Box::new(cond), Box::new(g)),
                    None => cond,
                };
                Self::match_arm(cond, body, rest)
            }
            // An irrefutable arm: without a guard it always matches (and `rest`
            // is dead); with one it is `if g { body } else { rest }`. The guard
            // is paired with a literal `true` pattern test so the
            // exhaustiveness walk can tell a guarded binding arm (covers
            // nothing) from an unguarded one (covers everything).
            None => match guard {
                None => Self::irrefutable_arm(body, rest, check_dead),
                Some(g) => Self::match_arm(
                    Expr::Bin(BinOp::And, Box::new(Expr::Bool(true)), Box::new(g)),
                    body,
                    rest,
                ),
            },
        };
        if bindings.is_empty() {
            arm
        } else {
            Expr::Block(Block { stmts: bindings, tail: Some(Box::new(arm)) })
        }
    }

    /// An irrefutable arm (`_` or a binding) always matches, so `rest` (the
    /// arms after it, or an `if let`'s `else`) is unreachable and dropped by
    /// Rust. For `if let`, `check_dead` is set and `rest` is kept behind an
    /// `if true` so its type is still checked: rustc rejects a mismatch (E0308)
    /// between a non-unit `then` and the `else` even when the pattern is
    /// irrefutable, and rmc must too. Desugaring to `if true { body } else {
    /// rest }` reuses the ordinary branch-type check for exactly that — the
    /// `else` is never run, but its type must equal `body`'s. For `match` the
    /// dead arms are dropped, as before. With no `rest` the arm is just `body`.
    fn irrefutable_arm(body: Expr, rest: Option<Expr>, check_dead: bool) -> Expr {
        match (check_dead, rest) {
            (true, Some(r)) => Expr::If(
                Box::new(Expr::Bool(true)),
                Block { stmts: Vec::new(), tail: Some(Box::new(body)) },
                Some(Block { stmts: Vec::new(), tail: Some(Box::new(r)) }),
            ),
            _ => body,
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
            // The magnitude `2^63` is only valid directly under a `-`, which
            // `parse_unary` folds into `Expr::Int(i64::MIN)`. Reaching it here
            // means it stood alone, so it is out of range, as in rustc.
            Some(Tok::IntMinMagnitude) => Err(format!(
                "at byte {}: integer literal `9223372036854775808` is out of range for `i64`",
                self.pos()
            )),
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
                        // Field init shorthand: `P { x, y }` means `P { x: x, y: y }`,
                        // so a bare name (no `:`) reuses the local of that name.
                        let fval = if self.peek() == Some(&Tok::Colon) {
                            self.i += 1;
                            self.parse_expr()?
                        } else {
                            Expr::Var { name: fname.clone(), slot: -1, ty: Type::I64 }
                        };
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
                } else if let Some(v) = self.consts.get(&name) {
                    // A reference to a constant is its value, substituted here
                    // so sema and codegen never see a constant as a variable.
                    Ok(Expr::Int(*v))
                } else if let Some(b) = self.const_bools.get(&name) {
                    Ok(Expr::Bool(*b))
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
                // `if let PAT = EXPR { .. } else { .. }` is sugar for a one-arm
                // `match`, so it desugars through the same path and sema and
                // codegen need no new case.
                if self.peek() == Some(&Tok::Let) {
                    self.i += 1;
                    let pat = self.parse_pattern()?;
                    self.expect(Tok::Eq)?;
                    let scrutinee = self.parse_cond()?;
                    let then = self.parse_block()?;
                    let els = if self.eat(&Tok::Else) {
                        if self.peek() == Some(&Tok::If) {
                            let nested = self.parse_primary()?;
                            Some(Block { stmts: Vec::new(), tail: Some(Box::new(nested)) })
                        } else {
                            Some(self.parse_block()?)
                        }
                    } else {
                        None
                    };
                    return Ok(self.desugar_if_let(pat, scrutinee, then, els));
                }
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
            Some(Tok::Loop) => {
                self.i += 1;
                // `loop` is always parsed as `Expr::Loop`; a `loop` in statement
                // position is wrapped in `Stmt::Expr` by `parse_block`, which
                // rewrites its result slot to `-2` so a `break value` is
                // discarded and the statement is typed as `()`.
                let body = self.parse_block()?;
                Ok(Expr::Loop(body, -1, Type::Unit, None))
            }
            // `'a: loop { .. }`. A label only labels a loop; `'a: while`/`'a: for`
            // are statements and never reach here.
            Some(Tok::Label(name)) => {
                self.i += 1;
                self.expect(Tok::Colon)?;
                match self.peek() {
                    Some(Tok::Loop) => {
                        self.i += 1;
                        let body = self.parse_block()?;
                        Ok(Expr::Loop(body, -1, Type::Unit, Some(name)))
                    }
                    other => Err(format!(
                        "at byte {}: expected `loop` after the label `'{name}`, found {}",
                        self.pos(),
                        other.map(|t| t.describe()).unwrap_or_else(|| "end of input".into())
                    )),
                }
            }
            Some(Tok::LBrace) => Ok(Expr::Block(self.parse_block()?)),
            Some(Tok::LBracket) => {
                self.i += 1;
                let first = self.parse_expr()?;
                if self.eat(&Tok::Semi) {
                    // `[v; N]`: the repeat count is a constant expression.
                    let len = self.parse_expr()?;
                    let n = match eval_i64(&len, &self.consts) {
                        Some(v) if v >= 0 => v as usize,
                        _ => {
                            return Err(format!(
                                "at byte {}: array length must be a non-negative constant integer",
                                self.pos()
                            ))
                        }
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

/// A `match` pattern: a wildcard, an integer or `bool` literal, or an
/// `Enum::Variant`. The variant is resolved to its tag by sema, once the enum
/// is known.
enum Pat {
    Wild,
    Int(i64),
    /// `lo..hi` / `lo..=hi`, inclusive flag set for `..=`.
    Range(i64, i64, bool),
    /// `a | b | c`, alternatives tried left to right.
    Or(Vec<Pat>),
    /// `true` or `false`.
    Bool(bool),
    /// A string literal, matched by content. A `str` is a pointer to its
    /// record, so the test is a content comparison (sema lowers it to the
    /// byte-wise helper), never a pointer compare.
    Str(String),
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
        Expr::MatchBoundary(b, _) => block_has_call_p(b),
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
        Stmt::While(c, b, _) => expr_has_call(c) || block_has_call_p(b),
        Stmt::For { start, end, body, .. } => {
            expr_has_call(start) || expr_has_call(end) || block_has_call_p(body)
        }
        Stmt::Break(v, _) => v.as_ref().map_or(false, expr_has_call),
        Stmt::Continue(_) => false,
    }
}
