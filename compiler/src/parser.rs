//! Recursive-descent parser for the rmc Rust subset.

use crate::ast::*;
use crate::consts::eval_i64;
use crate::lexer::{Tok, Token};

/// One saved entry of a parser name table, so a block's bindings are forgotten
/// when the block ends. Each `let`/`for` that records a `Vec` or raw-pointer
/// binding first pushes the name's previous state; leaving the block undoes the
/// entries in reverse, which both removes the inner binding and restores an
/// outer binding the block shadowed. Without this, a nested `let v: Vec<i64>`
/// would leave the outer `v: Vec<u8>` with an eight-byte stride once its block
/// ended.
enum NameScope {
    /// `vec_names`: whether the name was already present.
    VecName(String, bool),
    VecCap(String, Option<i64>),
    VecLen(String, Option<i64>),
    VecElem(String, Option<i64>),
    Ptr(String, Option<i64>),
    /// `args_vecs`: a `std::env::args()...collect()` binding.
    ArgsVec(String, Option<i64>),
    /// A `const` declared inside the block, so the name is removed (or its
    /// outer value restored) when the block ends. The `Option` is the value the
    /// name had before, if any.
    ConstInt(String, Option<i64>),
    ConstBool(String, Option<bool>),
    ConstChar(String, Option<u32>),
}

/// A `const` (or immutable `static`) declared inside a block. It is folded by
/// the block scan and in scope for the rest of the block, exactly like a
/// top-level `const` is in scope for the rest of the file; it is never added to
/// the program's item list, so codegen never sees it. `start`/`end` are token
/// indices: the `const`/`static` keyword and the token just past the `;`, so the
/// main parse can skip the item the scan already folded.
#[derive(Debug, Clone)]
struct LocalConst {
    start: usize,
    end: usize,
}

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
    /// Constant `char` items by name, so a `const C: char` used as a `match`
    /// pattern becomes the character pattern.
    const_chars: std::collections::HashMap<String, u32>,
    /// Struct field names by struct name, collected before parsing so a struct
    /// pattern (`S { f, .. }`) can be checked at parse time. A struct pattern
    /// has no runtime test, so the parser rewrites it into field reads; that
    /// rewrite needs the field list, which sema does not reach until later. A
    /// struct declared later in the file is covered because the whole list is
    /// scanned first.
    struct_fields: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// Anonymous structs introduced by tuple *types*, keyed by the canonical
    /// name `ast::tuple_struct_name` produces. A tuple is lowered to a struct
    /// (element `k` is field `"k"`), so it reuses the whole struct path through
    /// sema and codegen; this set only deduplicates the definitions so the same
    /// tuple type is not declared twice. The definitions are collected in
    /// `tuple_defs` and appended to the program's structs once parsing is done.
    tuple_structs: std::collections::HashSet<String>,
    tuple_defs: Vec<StructDef>,
    /// Module names brought into scope by a `use` of a `std` subtree, so that
    /// `use std::process;` makes a later `process::exit(0)` resolve. Only the
    /// `std` tree is tracked: any other import is skipped, because a same-file
    /// item is already visible by its own name. `std` is present from the start.
    std_mods: std::collections::HashSet<String>,
    /// Functions declared inside a block (`fn f() { .. }` as a statement),
    /// collected here and appended to the program's flat function list. A nested
    /// `fn` is an item, not a closure: it captures no locals, so it is hoisted
    /// to the top level and the flat list carries it like any other. It sees
    /// only the program's items (a top-level `fn`, `struct`, `enum` or `const`),
    /// never an enclosing local, so nothing needs to be closed over.
    nested_fns: Vec<FnDef>,
    /// One frame per block being parsed, innermost last, of the `const`s (and
    /// immutable `static`s) declared directly in that block. A block-scoped
    /// `const` is folded when it is parsed and shadows an outer one until the
    /// block ends, matching rustc's scoping; the name table `consts`/`const_bools`/
    /// `const_chars` is updated in place and restored by `pop_name_scope`.
    local_consts: Vec<Vec<LocalConst>>,
    /// True while parsing a pattern that may be refutable (a `match` arm, an
    /// `if let` / `while let` scrutinee). A tuple pattern is irrefutable only
    /// when every element is, but in a refutable position its elements may be
    /// literal or range tests, exactly as an array pattern's may not (rmc has
    /// no array-element test, but it does have the scalar tests a tuple element
    /// reuses). A `let`, a `for` head and a parameter are irrefutable, so there
    /// the flag is false and a literal element is rejected.
    pat_refutable_ok: bool,
    /// One frame per block being parsed, innermost last, mapping the original
    /// name of each nested `fn` declared directly in that block to the unique
    /// name it is hoisted under. A nested `fn` shadows a top-level one of the
    /// same name inside its block, as rustc's scoping does, so a call is
    /// rewritten through the innermost frame that names it; a block with no
    /// nested `fn` pushes an empty frame. A frame is filled by `scan_block_fns`
    /// before the block's statements are parsed, so a `fn` is in scope for the
    /// whole block and siblings may call each other in either order, exactly as
    /// Rust hoists items.
    fn_scopes: Vec<Vec<(String, String)>>,
    /// Unique name of each nested `fn`, keyed by the token index of its `fn`
    /// keyword. Filled in by `scan_block_fns` and read when the `fn` is parsed,
    /// so a name discovered by the pre-scan is the one the definition is given.
    fn_at: std::collections::HashMap<usize, String>,
    /// Monotonic counter for the unique names of hoisted nested functions, so
    /// two `fn`s of the same name in different blocks get distinct symbols.
    nf_counter: u32,
    /// `type` aliases, name -> target, collected by `prescan` so an alias may be
    /// used before it is declared. A use is rewritten to its target at parse
    /// time, so no later pass sees the alias and layout is unaffected.
    aliases: std::collections::HashMap<String, Type>,
    /// The `skip(n)` index recorded by the most recent
    /// `std::env::args().skip(n).map(..).collect()` in a `let` initializer.
    /// `parse_let` reads it to record the binding, and the `for v in &args`
    /// desugar reads the binding's skip to start its range.
    args_skip: i64,
    /// Names bound to a `std::env::args()...collect()` vector, mapped to their
    /// `skip` index. rmc has no `Vec`; a `for v in &args` over such a binding
    /// desugars to `for i in skip..__rmc_nargs() { let v = __rmc_arg_or(i, 0); .. }`,
    /// so no array is ever materialised.
    args_vecs: std::collections::HashMap<String, i64>,
    /// The loop variable of a `for v in &args` desugar, while its body is being
    /// parsed. `v` is an `i64`, but Rust types it `&i64`, so `*v` in the body is
    /// a copy of `v`; the parser drops the `*` for a name in this set. Scoped to
    /// the loop body, so a shadowing `let` outside it is unaffected.
    args_elems: std::collections::HashSet<String>,
    /// Names bound to a `Vec` by a supported shape (`Vec::with_capacity(k)` or
    /// `vec![e; n]`). rmc has no `Vec`; the binding holds the backing allocation
    /// as an `i64` address, and the few methods the corpus uses are folded here
    /// (`.as_mut_ptr()`, `.capacity()`, `.len()`).
    vec_names: std::collections::HashSet<String>,
    /// The constant capacity of a `Vec::with_capacity(k)` binding, for
    /// `v.capacity()`.
    vec_cap: std::collections::HashMap<String, i64>,
    /// The constant length of a `vec![e; n]` binding with a constant `n`, for
    /// `v.len()`.
    vec_len: std::collections::HashMap<String, i64>,
    /// Names holding a raw pointer (an `i64` address) taken by `.as_mut_ptr()`
    /// or `.as_ptr()`, mapped to the byte stride of one element. A `.add(off)`
    /// on one is `ptr + off * stride`, matching the raw-pointer method: `add`
    /// advances by whole elements, so a `Vec<i64>` pointer steps eight bytes
    /// while a `Vec<u8>` one steps one.
    ptr_vars: std::collections::HashMap<String, i64>,
    /// The element byte width of each `Vec` binding, taken from its `Vec<T>`
    /// annotation. A pointer derived from the binding inherits it as its
    /// stride. rmc stores every element in a word, so an absent or unrecognised
    /// element type defaults to eight.
    vec_elem: std::collections::HashMap<String, i64>,
    /// The element width of the most recently parsed `Vec<T>` type, so the
    /// `let` that consumes the type can record it against the binding name.
    /// Only `Vec` bindings read it, and they always follow their annotation.
    vec_elem_pending: Option<i64>,
    /// The bare-identifier target of each `type` alias (`type Byte = u8` ->
    /// `Byte` -> `u8`), so a `Vec<Byte>` can resolve the element to its
    /// underlying byte width. Only a target that is a single identifier is
    /// recorded; anything else keeps the default width.
    alias_target: std::collections::HashMap<String, String>,
    /// Undo log for the `Vec`/pointer name tables above, one frame per block
    /// being parsed (innermost last). A binding a block introduces is undone
    /// when the block ends, and an outer binding it shadowed is restored, so a
    /// name's recorded stride and shape are lexically scoped.
    name_scopes: Vec<Vec<NameScope>>,
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
            const_chars: std::collections::HashMap::new(),
            struct_fields: std::collections::HashMap::new(),
            tuple_structs: std::collections::HashSet::new(),
            tuple_defs: Vec::new(),
            std_mods: {
                let mut m = std::collections::HashSet::new();
                m.insert("std".to_string());
                m
            },
            nested_fns: Vec::new(),
            local_consts: Vec::new(),
            pat_refutable_ok: false,
            fn_scopes: vec![Vec::new()],
            fn_at: std::collections::HashMap::new(),
            nf_counter: 0,
            aliases: builtin_aliases(),
            args_skip: 1,
            args_vecs: std::collections::HashMap::new(),
            args_elems: std::collections::HashSet::new(),
            vec_names: std::collections::HashSet::new(),
            vec_cap: std::collections::HashMap::new(),
            vec_len: std::collections::HashMap::new(),
            ptr_vars: std::collections::HashMap::new(),
            vec_elem: std::collections::HashMap::new(),
            vec_elem_pending: None,
            alias_target: std::collections::HashMap::new(),
            name_scopes: Vec::new(),
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
        // `[i64; N]`, a pattern `N =>`), and a struct pattern `S { f }` names a
        // struct that may be declared later, so collect both first, then parse
        // the program with the values and field lists known.
        self.prescan()?;
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut consts = Vec::new();
        let mut fns = Vec::new();
        while self.peek().is_some() {
            // `pub` (and `pub(crate)` / `pub(super)` / `pub(in ..)`) may prefix
            // any item. rmc has one flat namespace, so the visibility is
            // dropped; skipping it here lets an ordinary `.rs` file parse.
            self.skip_pub()?;
            match self.peek() {
                Some(Tok::Struct) => structs.push(self.parse_struct()?),
                Some(Tok::Enum) => enums.push(self.parse_enum()?),
                Some(Tok::Const) => consts.push(self.parse_const()?),
                // `static [mut] NAME: T = EXPR;`. An immutable `static` is a
                // `const`; `static mut` is rejected (rmc has no mutable global).
                Some(Tok::Ident(id)) if id == "static" => consts.push(self.parse_static()?),
                Some(Tok::Type) => self.parse_type_alias()?,
                Some(Tok::Fn) => fns.push(self.parse_fn()?),
                Some(Tok::Impl) => self.parse_impl(&mut fns)?,
                // `unsafe fn` is a plain `fn`: rmc performs no safety checking,
                // so the qualifier carries no meaning and is dropped here.
                Some(Tok::Ident(id)) if id == "unsafe" => {
                    self.i += 1;
                    if self.peek() != Some(&Tok::Fn) {
                        return Err(format!(
                            "at byte {}: expected `fn` after `unsafe`",
                            self.pos()
                        ));
                    }
                    fns.push(self.parse_fn()?);
                }
                // `use` imports a path into scope. rmc has no module system and
                // resolves names globally, so the import is skipped; the items
                // it would bring in (`std::process::exit`, a same-file `fn`,
                // ...) are already visible. A `use` that brings in an item rmc
                // does not provide fails later at its use site, with a clearer
                // message than a parse error here.
                Some(Tok::Ident(id)) if id == "use" => self.skip_use()?,
                // `mod NAME;` / `mod NAME { .. }`: rmc has no module system, so
                // the module is skipped. The items it would contain are not
                // visible; a use of one fails later at its use site.
                Some(Tok::Ident(id)) if id == "mod" => self.skip_mod()?,
                other => {
                    return Err(format!(
                        "at byte {}: expected `fn`, `struct`, `enum`, `const`, `type` or `impl`, found {}",
                        self.pos(),
                        other.map(|x| x.describe()).unwrap_or_else(|| "end of input".into())
                    ))
                }
            }
        }
        // Append the anonymous structs that tuple types introduced, so
        // `resolve_layouts` sees them alongside the user's structs. They are
        // added after the scan so a tuple element naming a user struct declared
        // later still resolves.
        structs.append(&mut self.tuple_defs);
        // Nested functions are hoisted to the top level, so the flat list sema
        // and codegen walk holds them like any other function.
        fns.append(&mut self.nested_fns);
        Ok(Program { structs, enums, consts, fns })
    }

    fn parse_enum(&mut self) -> PResult<EnumDef> {
        self.skip_pub()?;
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

    /// Scan the token stream once, before the real parse, for two things that a
    /// use may need before their declaration: the value of every top-level
    /// `const` (an array length `[i64; N]`, a pattern `N =>`), and the field
    /// names of every struct (a struct pattern `S { f }`). Only an item's own
    /// tokens are consumed: a `{ .. }` or `( .. )` in an initializer is skipped
    /// by bracket matching, and every other item's body is skipped the same way,
    /// so no nested `const` inside a function body is collected here.
    fn prescan(&mut self) -> PResult<()> {
        let saved = self.i;
        let mut items: Vec<ConstDef> = Vec::new();
        // `type` aliases are deferred: (name, token index of the target). Their
        // targets are parsed after `resolve_consts`, so a `[T; N]` target sees
        // the constants, and after the first pass so `type A = B;` may name a
        // later `type B`.
        let mut alias_spans: Vec<(String, usize)> = Vec::new();
        // Struct and enum names, for the type-namespace collision check below.
        let mut struct_names: Vec<String> = Vec::new();
        let mut enum_names: Vec<String> = Vec::new();
        self.i = 0;
        loop {
            // A `pub` on an item is dropped (rmc has one flat namespace), so
            // skip it before matching the item keyword, exactly as the real
            // parse loop does.
            self.skip_pub()?;
            match self.peek() {
                None => break,
                Some(Tok::Const) => items.push(self.parse_const()?),
                // `static [mut] NAME: T = EXPR;` is collected like a `const`:
                // an immutable `static` is lowered to one, and `static mut` is
                // rejected here too.
                Some(Tok::Ident(id)) if id == "static" => items.push(self.parse_static()?),
                Some(Tok::Type) => {
                    // `type NAME = TYPE;`. The target is not parsed yet: a
                    // `[T; N]` target needs the constant values, which are still
                    // being collected. Record the name and where its target
                    // starts, skip to the `;`, and parse the target after
                    // `resolve_consts` (below) so it sees them.
                    self.i += 1;
                    let name = self.expect_ident()?;
                    self.expect(Tok::Eq)?;
                    let start = self.i;
                    // If the target is a single identifier, remember it so a
                    // `Vec<NAME>` can chase the alias to its underlying byte
                    // width. A compound target (`[u8; 4]`, `(u8, u8)`) is left
                    // out; its width is the default.
                    if let Some(Tok::Ident(target)) = self.toks.get(self.i).map(|t| t.tok.clone()) {
                        if matches!(self.toks.get(self.i + 1).map(|t| &t.tok), Some(Tok::Semi)) {
                            self.alias_target.insert(name.clone(), target);
                        }
                    }
                    self.skip_to_semi()?;
                    alias_spans.push((name, start));
                }
                Some(Tok::Struct) => {
                    // Record the field names, then skip the body. Only a braced
                    // struct has named fields to collect; a later `struct S { .. }`
                    // is what a pattern `S { f }` needs.
                    self.i += 1;
                    let name = if let Some(Tok::Ident(n)) = self.peek() {
                        let n = n.clone();
                        self.i += 1;
                        Some(n)
                    } else {
                        None
                    };
                    if let Some(n) = &name {
                        struct_names.push(n.clone());
                    }
                    if self.peek() == Some(&Tok::LBrace) {
                        self.i += 1;
                        let mut fields = std::collections::HashSet::new();
                        while self.peek() != Some(&Tok::RBrace) && self.peek().is_some() {
                            // A field may carry its own `pub`; skip it so the
                            // field name is recorded, not `pub`.
                            self.skip_pub()?;
                            if let Some(Tok::Ident(f)) = self.peek() {
                                fields.insert(f.clone());
                                self.i += 1;
                            } else {
                                self.i += 1;
                                continue;
                            }
                            // Skip the `: Type`, which may contain braces in an
                            // array type; stop at the next `,` or `}`.
                            while let Some(t) = self.peek() {
                                if *t == Tok::Comma || *t == Tok::RBrace {
                                    break;
                                }
                                if *t == Tok::LBracket {
                                    self.skip_balanced(Tok::LBracket, Tok::RBracket)?;
                                } else {
                                    self.i += 1;
                                }
                            }
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                        self.expect(Tok::RBrace)?;
                        if let Some(n) = name {
                            self.struct_fields.insert(n, fields);
                        }
                    } else {
                        self.skip_balanced(Tok::LBrace, Tok::RBrace)?;
                    }
                }
                Some(Tok::Enum) => {
                    // Record the name, then skip a balanced `{ .. }` body.
                    self.i += 1;
                    if let Some(Tok::Ident(n)) = self.peek() {
                        enum_names.push(n.clone());
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
                // A `mod NAME;` / `mod NAME { .. }` is skipped, so a `const`
                // inside a module body is not collected (its items are not
                // visible to the flat namespace anyway).
                Some(Tok::Ident(id)) if id == "mod" => self.skip_mod()?,
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
        // Parse each alias target now that the constants are known. Two passes
        // let `type A = B; type B = i64;` resolve: the first lays down every
        // direct target, the second fills any left as a bare `Named` (a target
        // that named a not-yet-parsed alias).
        for (name, start) in &alias_spans {
            self.i = *start;
            let ty = self.parse_type()?;
            self.aliases.insert(name.clone(), ty);
        }
        for (name, _) in &alias_spans {
            if let Some(t) = self.aliases.get(name).cloned() {
                let expanded = self.expand_alias_ty(&t, 0)?;
                self.aliases.insert(name.clone(), expanded);
            }
        }
        // Parse every target once more, now that the whole alias map is known.
        // The first pass may have lowered a tuple (or array) whose element named
        // an alias not yet parsed, baking a `Named` placeholder into the tuple
        // struct's fields; a later pass would then see a stale layout. Rebuilding
        // the tuple structs here, with every alias expanded, fixes
        // `type Pair = (T, i64); type T = i64;` to match the backward order.
        self.tuple_structs.clear();
        self.tuple_defs.clear();
        for (name, start) in &alias_spans {
            self.i = *start;
            let ty = self.parse_type()?;
            self.aliases.insert(name.clone(), ty);
        }
        for (name, _) in &alias_spans {
            if let Some(t) = self.aliases.get(name).cloned() {
                let expanded = self.expand_alias_ty(&t, 0)?;
                self.aliases.insert(name.clone(), expanded);
            }
        }
        // A `const`'s declared type may name an alias (`const C: Letter = 'a';
        // type Letter = char;`). The first `resolve_consts` ran before the
        // aliases were parsed, so it saw the placeholder `Named("Letter")` and
        // skipped the item. Expand each type through the now-complete alias map
        // and resolve again, so the constant lands in the table a later use
        // reads (`print_char(C)`).
        let expanded_items: Vec<ConstDef> = items
            .iter()
            .map(|d| ConstDef {
                name: d.name.clone(),
                ty: self.expand_alias_ty(&d.ty, 0).unwrap_or_else(|_| d.ty.clone()),
                value: d.value.clone(),
            })
            .collect();
        self.resolve_consts(&expanded_items);
        // A `type` shares the type namespace with a struct and an enum, so a
        // collision is an error (rustc's E0428). It does *not* collide with a
        // `const` or a `fn`, which live in the value namespace. rmc keeps a
        // single map for the target, so a duplicate `type` is detected by the
        // count rather than by the map's length.
        let mut alias_names: Vec<&str> = alias_spans.iter().map(|(n, _)| n.as_str()).collect();
        alias_names.sort_unstable();
        if let Some(dup) = alias_names.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("the name `{}` is defined multiple times", dup[0]));
        }
        for n in &alias_names {
            if struct_names.iter().any(|s| s == n) || enum_names.iter().any(|e| e == n) {
                return Err(format!("the name `{}` is defined multiple times", n));
            }
        }
        self.i = saved;
        Ok(())
    }

    /// Skip a whole `use ...;` item, whose path may span braces
    /// (`use std::{env, process};`). The import is discarded — rmc resolves
    /// every name globally — but a `std` subtree is recorded so that a later
    /// `process::exit(0)` after `use std::process;` resolves to the `std` path.
    fn skip_use(&mut self) -> PResult<()> {
        self.i += 1; // the `use` identifier
        self.collect_std_use();
        self.skip_to_semi()
    }

    /// Skip a `pub` visibility qualifier, with its optional restriction
    /// (`pub(crate)`, `pub(super)`, `pub(in path)`). rmc resolves every name in
    /// one flat namespace, so visibility carries no meaning and is dropped here.
    /// Accepting it lets an ordinary Rust source file parse unchanged.
    fn skip_pub(&mut self) -> PResult<()> {
        if self.peek() == Some(&Tok::Ident("pub".into())) {
            self.i += 1;
            if self.peek() == Some(&Tok::LParen) {
                self.skip_balanced(Tok::LParen, Tok::RParen)?;
            }
        }
        Ok(())
    }

    /// Skip an inline `mod NAME { .. }`. rmc has no module system — every name
    /// is resolved in one flat namespace — so the module body is dropped, which
    /// lets an ordinary `.rs` file that declares inline modules parse. A
    /// `mod NAME;` that would load another file is rejected: rmc compiles one
    /// file and cannot resolve it, and accepting it would diverge from rustc
    /// when the file is absent.
    fn skip_mod(&mut self) -> PResult<()> {
        let pos = self.pos();
        self.i += 1; // the `mod` keyword
        let name = self.expect_ident()?;
        if self.peek() == Some(&Tok::Semi) {
            return Err(format!(
                "at byte {pos}: `mod {name};` loads another file, which rmc does not support"
            ));
        }
        self.skip_balanced(Tok::LBrace, Tok::RBrace)
    }

    /// Record the module names a `use` of the `std` tree brings into scope.
    /// Walks the token run up to the `;`, so `use std::process;`,
    /// `use std::process::exit;` and `use std::{process, hint};` all register
    /// `process` (and `hint`). A `use` that does not start at `std` is ignored.
    fn collect_std_use(&mut self) {
        let mut j = self.i;
        if !matches!(self.toks.get(j).map(|t| &t.tok), Some(Tok::Ident(n)) if n == "std") {
            return;
        }
        let mut want = true; // the next ident is a module name
        j += 1;
        while j < self.toks.len() {
            match &self.toks[j].tok {
                Tok::Semi => break,
                Tok::ColonColon => want = true,
                Tok::LBrace => {} // the group of a `std::{..}`; names inside count
                Tok::Comma | Tok::RBrace => want = true,
                Tok::Ident(n) if want => {
                    self.std_mods.insert(n.clone());
                    want = false;
                }
                _ => {}
            }
            j += 1;
        }
    }

    /// Advance past the next `;` at bracket depth 0, used to skip an alias
    /// target in the pre-scan before it is parsed.
    fn skip_to_semi(&mut self) -> PResult<()> {
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            match t {
                Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBracket => depth -= 1,
                Tok::Semi if depth <= 0 => {
                    self.i += 1;
                    return Ok(());
                }
                _ => {}
            }
            self.i += 1;
        }
        Err(format!("at byte {}: unexpected end of input", self.pos()))
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
        self.skip_pub()?;
        self.expect(Tok::Const)?;
        self.parse_const_tail()
    }

    /// The `NAME : TYPE = EXPR ;` tail shared by `const` and an immutable
    /// `static`. rmc has no mutable global storage, so a `static` is treated
    /// exactly like a `const`: its initialiser must be a constant expression
    /// (as rustc requires too), and every read yields that value. `static mut`
    /// is rejected before this point, since mutation has no meaning here.
    fn parse_const_tail(&mut self) -> PResult<ConstDef> {
        let name = self.expect_ident()?;
        self.expect(Tok::Colon)?;
        let ty = self.parse_type()?;
        self.expect(Tok::Eq)?;
        let value = self.parse_expr()?;
        self.expect(Tok::Semi)?;
        Ok(ConstDef { name, ty, value })
    }

    /// `static [mut] NAME: TYPE = EXPR;`. An immutable `static` is lowered to a
    /// `const`. `static mut` is rejected: rmc has no mutable global, and a
    /// program that mutates one would diverge, so refusing is safer than
    /// silently dropping the mutation.
    fn parse_static(&mut self) -> PResult<ConstDef> {
        self.skip_pub()?;
        self.i += 1; // `static`
        if let Some(Tok::Ident(id)) = self.peek() {
            if id == "mut" {
                return Err(format!(
                    "at byte {}: `static mut` is not supported; rmc has no mutable global",
                    self.pos()
                ));
            }
        }
        self.parse_const_tail()
    }

    /// Parse the `const`/immutable-`static` item whose keyword is at token
    /// `start` (past any `pub`), returning the definition and the token index
    /// just past its `;`. `self.i` is left at `start` so a caller that wants to
    /// parse the item normally (the top-level loop) can. `static mut` is
    /// refused, as at the top level.
    fn parse_const_item_at(&mut self, start: usize) -> PResult<(ConstDef, usize)> {
        let saved = self.i;
        self.i = start;
        self.skip_pub()?;
        let def = if self.peek() == Some(&Tok::Const) {
            self.i += 1;
            self.parse_const_tail()?
        } else {
            // `static [mut] ..`; `parse_static` consumes the keyword itself.
            self.parse_static()?
        };
        let end = self.i;
        self.i = saved;
        Ok((def, end))
    }

    /// The declared type of a `const`/`static`, resolved the way a top-level
    /// `const` is (`str`, and an alias to a scalar), then required to be a kind
    /// rmc can fold. Without this a block-scoped `const S: str = "hi";` would be
    /// accepted while the same item at the top level is rejected.
    fn resolve_const_ty(&self, ty: &Type, name: &str, pos: u32) -> PResult<Type> {
        let ty = self.expand_alias_ty(ty, 0)?;
        let ty = match ty {
            Type::Named(n) if n == "str" => Type::Str,
            other => other,
        };
        match ty {
            Type::I64 | Type::Bool | Type::Char => Ok(ty),
            other => Err(format!(
                "at byte {pos}: constant `{name}` has type {other:?}; rmc only folds `i64`, `bool` and `char` constants"
            )),
        }
    }

    /// Rewrite every alias name in `t` to its target, so no later pass sees an
    /// alias. `depth` bounds a cycle (`type A = B; type B = A;`), which would
    /// otherwise expand forever; a cycle is reported as an error, matching
    /// rustc's E0391 rather than hanging or emitting a wrong layout. The limit
    /// is generous because each step is one named type.
    fn expand_alias_ty(&self, t: &Type, depth: u32) -> PResult<Type> {
        if depth > 64 {
            return Err("type alias cycle detected".into());
        }
        match t {
            Type::Named(n) => match self.aliases.get(n) {
                Some(target) => self.expand_alias_ty(target, depth + 1),
                None => Ok(t.clone()),
            },
            Type::Ref(inner, m) => Ok(Type::Ref(Box::new(self.expand_alias_ty(inner, depth + 1)?), *m)),
            Type::Array(elem, n) => Ok(Type::Array(Box::new(self.expand_alias_ty(elem, depth + 1)?), *n)),
            _ => Ok(t.clone()),
        }
    }

    /// The byte width of a `Vec<T>` element named by the identifier `t`,
    /// following `type` aliases to their target so `type Byte = u8` and
    /// `Vec<Byte>` still get a one-byte stride. The chain is bounded so a
    /// cyclic alias (which `expand_alias_ty` rejects elsewhere) cannot loop.
    fn elem_width_of(&self, t: &str) -> i64 {
        let mut cur = t.to_string();
        for _ in 0..64 {
            match self.alias_target.get(&cur) {
                Some(next) if *next != cur => cur = next.clone(),
                _ => break,
            }
        }
        elem_byte_width(&cur)
    }

    /// `type NAME = TYPE;`. The target is parsed here (aliases in it expanded by
    /// `parse_type_inner`) and stored; a use is rewritten to it by
    /// `expand_alias_ty`, so the alias never reaches sema. The name was
    /// registered by `prescan`, so it may be used before this point, which means
    /// `type A = B;` and `type B = i64;` resolve in either order.
    fn parse_type_alias(&mut self) -> PResult<()> {
        self.skip_pub()?;
        self.expect(Tok::Type)?;
        let name = self.expect_ident()?;
        self.expect(Tok::Eq)?;
        let ty = self.parse_type()?;
        self.expect(Tok::Semi)?;
        self.aliases.insert(name, ty);
        Ok(())
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
                crate::consts::ConstVal::Char(v) => {
                    self.const_chars.insert(name, v);
                }
            }
        }
    }

    fn parse_struct(&mut self) -> PResult<StructDef> {
        self.skip_pub()?;
        self.expect(Tok::Struct)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            self.skip_pub()?;
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

    /// Lower a parsed tuple *type* `(A, B)` into its anonymous struct. The
    /// element types may themselves be tuples, which are lowered first. The
    /// struct definition is recorded in `tuple_defs` (once per distinct tuple
    /// type) and appended to the program once parsing is done, so
    /// `resolve_layouts` lays it out exactly like a user struct; the returned
    /// `Type::Named` is what `resolve_in` then resolves to `Type::Struct`.
    fn lower_tuple_type(&mut self, elems: Vec<Type>) -> Type {
        let mut fields = Vec::with_capacity(elems.len());
        let mut keys = Vec::with_capacity(elems.len());
        for (k, e) in elems.into_iter().enumerate() {
            keys.push(e.key());
            fields.push((k.to_string(), e));
        }
        let name = crate::ast::tuple_struct_name(&keys);
        if self.tuple_structs.insert(name.clone()) {
            self.tuple_defs.push(StructDef { name: name.clone(), fields });
        }
        Type::Named(name)
    }

    fn parse_fn(&mut self) -> PResult<FnDef> {
        self.skip_pub()?;
        self.expect(Tok::Fn)?;
        let name = self.expect_ident()?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        let mut param_mut = Vec::new();
        let mut param_pats = Vec::new();
        while self.peek() != Some(&Tok::RParen) {
            self.parse_param(&mut params, &mut param_mut, &mut param_pats)?;
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        Self::check_dup_param_names(&params, &param_pats)?;
        let ret = if self.eat(&Tok::Arrow) {
            self.parse_type()?
        } else {
            Type::Unit
        };
        let body = self.parse_block()?;
        Ok(FnDef { name, self_ty: None, takes_self: false, ref_self: false, ref_self_mut: false, params, param_mut, param_pats, ret, body, param_slots: Vec::new(), ret_slot: -1, nslots: 0 })
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
        self.skip_pub()?;
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
        let mut param_pats = Vec::new();
        let (by_ref, by_mut) = if takes_self {
            let by_ref = self.eat(&Tok::Amp);
            let by_mut = by_ref && self.eat(&Tok::Mut);
            let first = self.expect_ident()?;
            debug_assert_eq!(first, "self");
            if self.eat(&Tok::Comma) {
                self.parse_param_list(&mut params, &mut param_mut, &mut param_pats)?;
            }
            (by_ref, by_mut)
        } else {
            if self.peek() != Some(&Tok::RParen) {
                self.parse_param_list(&mut params, &mut param_mut, &mut param_pats)?;
            }
            (false, false)
        };
        self.expect(Tok::RParen)?;
        Self::check_dup_param_names(&params, &param_pats)?;
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
            param_pats,
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
        param_pats: &mut Vec<Option<Vec<Stmt>>>,
    ) -> PResult<()> {
        while self.peek() != Some(&Tok::RParen) {
            self.parse_param(params, param_mut, param_pats)?;
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(())
    }

    /// One parameter, `p: T`, `mut p: T`, or an aggregate pattern, `S { x, y }:
    /// S` / `[a, b]: [i64; 2]`. A pattern parameter is stored as an ordinary
    /// parameter (a hidden name of the annotation's type) plus the statements
    /// that bind the pattern's names from it, spliced at the top of the body.
    fn parse_param(
        &mut self,
        params: &mut Vec<(String, Type)>,
        param_mut: &mut Vec<bool>,
        param_pats: &mut Vec<Option<Vec<Stmt>>>,
    ) -> PResult<()> {
        let pm = self.eat(&Tok::Mut);
        // `S {`, `[` or `(` at the start of a parameter is a destructuring
        // pattern, not a name: none can begin a plain `p: T`. A leading `mut`
        // on a pattern is rejected, as rustc rejects `mut S { x }: S` (the
        // `mut` belongs on each binding). `n @ subpat` is likewise a pattern,
        // but there `mut` is legal and attaches to `n`.
        let at_pat = matches!(self.peek(), Some(Tok::Ident(_)))
            && self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::At);
        let pat = if matches!(self.peek(), Some(Tok::LBracket))
            || matches!(self.peek(), Some(Tok::LParen))
            || at_pat
            || matches!(self.peek(), Some(Tok::Ident(_))
                if self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::LBrace))
        {
            if pm && !at_pat {
                return Err(format!(
                    "at byte {}: `mut` must be attached to each individual binding",
                    self.pos()
                ));
            }
            let mut p = self.parse_pattern()?;
            if at_pat {
                if let Pat::At(_, ref mut m, _) = p {
                    *m = pm;
                }
            }
            Some(p)
        } else {
            None
        };
        if let Some(p) = pat {
            self.expect(Tok::Colon)?;
            let ty = self.parse_type()?;
            let pname = format!("\u{0}p{}", self.tmp);
            self.tmp += 1;
            let subj = Expr::Var { name: pname.clone(), slot: -1, ty: ty.clone() };
            let binds = self.param_pat_binds(p, &subj)?;
            params.push((pname, ty));
            param_mut.push(false);
            param_pats.push(Some(binds));
        } else {
            let pname = self.expect_ident()?;
            self.expect(Tok::Colon)?;
            let ty = self.parse_type()?;
            params.push((pname, ty));
            param_mut.push(pm);
            param_pats.push(None);
        }
        Ok(())
    }

    /// The statements that bind a parameter pattern's names from the parameter
    /// variable `subj`. A struct or array pattern is irrefutable, so this reuses
    /// the `let` destructuring expansion; a refutable pattern (a literal, a
    /// variant, an or-pattern) is rejected, as rustc rejects it for a parameter.
    fn param_pat_binds(&mut self, p: Pat, subj: &Expr) -> PResult<Vec<Stmt>> {
        match p {
            Pat::Struct(name, fields) => Ok(Self::struct_pat_binds(&fields, subj, &name)),
            Pat::Tuple(elems) => Ok(Self::tuple_pat_binds(&elems, subj)),
            Pat::Array { elems, rest, front } => {
                Ok(Self::array_pat_binds(&elems, rest, front, subj))
            }
            Pat::Bind(name) => Ok(vec![Stmt::Let {
                name,
                ty: None,
                mutable: false,
                init: Some(subj.clone()),
                slot: -1,
            }]),
            // `n @ subpat: T` binds the whole argument to `n` and also
            // destructures it. `subpat` must be irrefutable, as for any
            // parameter pattern.
            Pat::At(name, am, sub) => {
                if !Self::pat_irrefutable(&sub) {
                    return Err(format!(
                        "at byte {}: a parameter pattern must be irrefutable (a struct, tuple or array pattern)",
                        self.pos()
                    ));
                }
                let mut binds = vec![Stmt::Let {
                    name,
                    ty: None,
                    mutable: am,
                    init: Some(subj.clone()),
                    slot: -1,
                }];
                Self::pat_binds(&sub, false, subj, &mut binds);
                Ok(binds)
            }
            _ => Err(format!(
                "at byte {}: a parameter pattern must be irrefutable (a struct, tuple or array pattern)",
                self.pos()
            )),
        }
    }

    /// Reject a parameter list where a name is bound twice (rustc E0415),
    /// counting both plain parameters and the names a pattern parameter binds.
    /// A pattern parameter's own hidden variable is skipped: it is not written
    /// in source and cannot collide.
    fn check_dup_param_names(
        params: &[(String, Type)],
        pats: &[Option<Vec<Stmt>>],
    ) -> PResult<()> {
        let mut seen: Vec<&str> = Vec::new();
        for (i, (name, _)) in params.iter().enumerate() {
            let names: Vec<&str> = match &pats[i] {
                Some(binds) => binds
                    .iter()
                    .filter_map(|s| match s {
                        Stmt::Let { name, .. } => Some(name.as_str()),
                        _ => None,
                    })
                    .collect(),
                None => vec![name.as_str()],
            };
            for n in names {
                if seen.contains(&n) {
                    return Err(format!(
                        "identifier `{n}` is bound more than once in this parameter list"
                    ));
                }
                seen.push(n);
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
        // `*const T` / `*mut T`: a raw pointer. rmc has no borrow checking and a
        // raw pointer is one address, exactly like a reference, so it maps to
        // the same `Ref` type (the `mut` flag is kept for symmetry).
        if self.eat(&Tok::Star) {
            let mutable = match self.peek() {
                Some(Tok::Const) => {
                    self.i += 1;
                    false
                }
                Some(Tok::Mut) => {
                    self.i += 1;
                    true
                }
                _ => {
                    return Err(format!(
                        "at byte {}: expected `const` or `mut` after `*`",
                        self.pos()
                    ))
                }
            };
            let inner = self.parse_type()?;
            return Ok(Type::Ref(Box::new(inner), mutable));
        }
        if self.eat(&Tok::Amp) {
            // A reference may carry a lifetime (`&'static str`, `&'a mut T`).
            // rmc does not model lifetimes, so it is skipped here.
            if let Some(Tok::Label(_)) = self.peek() {
                self.i += 1;
            }
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
            Some(Tok::TyChar) => {
                self.i += 1;
                Ok(Type::Char)
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
                self.i += 1;
                // `()` is unit; `(T)` is grouping; `(A, B)` and `(A,)` are
                // tuples, lowered to an anonymous struct so the whole struct
                // path (layout, by-pointer passing, field reads) is reused.
                if self.eat(&Tok::RParen) {
                    return Ok(Type::Unit);
                }
                let first = self.parse_type()?;
                if !self.eat(&Tok::Comma) {
                    self.expect(Tok::RParen)?;
                    return Ok(first);
                }
                let mut elems = vec![first];
                while self.peek() != Some(&Tok::RParen) {
                    elems.push(self.parse_type()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen)?;
                Ok(self.lower_tuple_type(elems))
            }
            Some(Tok::Ident(_)) => {
                let name = match self.bump() {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                // `Vec<T>` has no real type in rmc; the `let` that declares an
                // `args()`-vector consumes the `<..>` and its initializer is
                // desugared. Here only the `<..>` is skipped and a placeholder
                // `Vec` name returned, so the annotation parses. A `Vec` used
                // anywhere but that one `let` fails when it reaches sema (the
                // name resolves to no struct).
                if self.peek() == Some(&Tok::Lt) {
                    if name != "Vec" {
                        return Err(format!(
                            "at byte {}: unsupported generic type `{name}` (only `Vec<T>` is accepted)",
                            self.pos()
                        ));
                    }
                    self.i += 1; // `<`
                    // The element type fixes the pointer stride for a later
                    // `.add(off)`, so read its name before the alias map
                    // collapses `u8` to `i64`; `size_of::<T>()` is what rustc
                    // scales `add` by. A `type Byte = u8` alias is chased to its
                    // target so `Vec<Byte>` still gets a one-byte stride.
                    let width = match self.peek() {
                        Some(Tok::Ident(t)) => self.elem_width_of(t),
                        _ => 8,
                    };
                    let mut depth = 1i32;
                    while depth > 0 {
                        match self.peek() {
                            None => {
                                return Err(format!(
                                    "at byte {}: unexpected end of input in `Vec<..>`",
                                    self.pos()
                                ))
                            }
                            Some(Tok::Lt) => depth += 1,
                            Some(Tok::Gt) => depth -= 1,
                            _ => {}
                        }
                        self.i += 1;
                    }
                    self.vec_elem_pending = Some(width);
                    return Ok(Type::Named("Vec".into()));
                }
                // An alias is transparent: a use is rewritten to its target here,
                // at parse time, so no later pass sees it. A struct name is not an
                // alias and stays `Type::Named` for sema to lay out.
                self.expand_alias_ty(&Type::Named(name), 0)
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
        // A block is a `fn` scope: a nested `fn` declared directly in it is
        // visible throughout, shadows a same-named one outside, and is gone when
        // the block ends. The frame is filled before the statements are parsed,
        // so a `fn` is in scope for the whole block and siblings may call each
        // other in either order, as Rust hoists items. It is popped on every
        // exit, so an error part way through does not leave a stale binding.
        let frame = self.scan_block_fns()?;
        self.fn_scopes.push(frame);
        // The `Vec`/pointer name tables are scoped the same way: a binding this
        // block introduces (or shadows) is undone when the block ends, so an
        // outer `v: Vec<u8>` regains its byte stride after a nested `v`. Pushed
        // before the const scan so the restore notes it records land in this
        // block's frame.
        self.name_scopes.push(Vec::new());
        // A block-scoped `const` is an item too: rustc hoists it for the whole
        // block, so `println!("{}", N); const N: i64 = 5;` resolves. Fold every
        // one now, in declaration order (a later one may name an earlier one),
        // and note the names so the main parse skips the items it already folded.
        let cframe = self.scan_block_consts()?;
        self.local_consts.push(cframe);
        let r = self.parse_block_body();
        self.pop_name_scope();
        self.local_consts.pop();
        self.fn_scopes.pop();
        r
    }

    /// Scan the block starting at the current `{` for the `const`/immutable
    /// `static` items declared directly in it, resolve their values together,
    /// and enter them in the live constant tables so a later statement in the
    /// block (and a nested block) resolves the name. Returns the frame of their
    /// token spans, which the main parse skips. A `const` nested deeper is left
    /// to the scan of that block.
    ///
    /// The whole block's items are collected before any is folded, then resolved
    /// to a fixpoint seeded with the enclosing scope, so a forward reference
    /// (`const A = B + 1; const B = 2;`) resolves and a self-reference
    /// (`const K = K + 1;`, where an outer `K` exists) is a cycle rather than a
    /// read of the outer value. Every item is validated, including an unused
    /// one: a non-foldable initializer (`const N: i64 = true;`) or an
    /// unsupported type is an error, as at the top level. A `const` colliding
    /// with another `const` or with a `fn` in the same block is E0428.
    fn scan_block_consts(&mut self) -> PResult<Vec<LocalConst>> {
        let mut raw: Vec<(usize, usize, usize, ConstDef)> = Vec::new();
        let mut depth = 0u32;
        let mut i = self.i;
        // Parse the items with the live constant tables cleared, so a name in an
        // initializer stays symbolic: otherwise `const K: i64 = K + 1;` would
        // have the enclosing `K` substituted in while parsing, hiding the cycle,
        // and a forward reference to a later item that shadows an outer constant
        // would bake in the outer value. The tables are restored for the resolve
        // below, which needs them as the enclosing scope.
        let saved_consts = std::mem::take(&mut self.consts);
        let saved_bools = std::mem::take(&mut self.const_bools);
        let saved_chars = std::mem::take(&mut self.const_chars);
        while i < self.toks.len() {
            // Decide by a short-lived borrow, so the parsing below does not
            // conflict with it.
            let mut is_item = false;
            match &self.toks[i].tok {
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Tok::RParen | Tok::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                // A `const`/`static` item at the top level of this block. A
                // `pub` before it is skipped, as in a top-level item. A `const`
                // inside a `fn` body or a nested `{ .. }` has `depth > 0`, so it
                // is left to that block's own scan.
                Tok::Const if depth == 0 => {
                    // A raw pointer type `*const T` also lexes the `const`
                    // keyword; an item's `const` is never preceded by `*`.
                    if self.toks.get(i.wrapping_sub(1)).map(|t| &t.tok) != Some(&Tok::Star) {
                        is_item = true;
                    }
                }
                Tok::Ident(id) if id == "static" && depth == 0 => is_item = true,
                _ => {}
            }
            if is_item {
                let raw_start = i;
                // `static mut` has no meaning in rmc; reject it here, where the
                // block scan sees it, rather than let the parse fail with a less
                // specific message.
                if let Some(Tok::Ident(id)) = self.toks.get(i + 1).map(|t| &t.tok) {
                    if id == "mut" {
                        self.consts = saved_consts;
                        self.const_bools = saved_bools;
                        self.const_chars = saved_chars;
                        return Err(format!(
                            "at byte {}: `static mut` is not supported; rmc has no mutable global",
                            self.toks[i].pos
                        ));
                    }
                }
                let start = self.skip_pub_at(i);
                let (def, end) = self.parse_const_item_at(start)?;
                raw.push((raw_start, start, end, def));
                i = end;
                continue;
            }
            i += 1;
        }
        self.consts = saved_consts;
        self.const_bools = saved_bools;
        self.const_chars = saved_chars;
        // E0428: two items of the same name, a `const` and a `fn`, or two
        // `fn`s (`scan_block_fns`). The block's nested `fn` names are already in
        // the frame `parse_block_inner` pushed, so include them here, along with
        // any `fn` declared later in the block.
        let mut fn_names: Vec<String> = self
            .fn_scopes
            .last()
            .map(|f| f.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        fn_names.extend(self.scan_block_fn_names());
        for (k, (_, _, _, def)) in raw.iter().enumerate() {
            if raw[..k].iter().any(|(_, _, _, d)| d.name == def.name)
                || fn_names.contains(&def.name)
            {
                return Err(format!(
                    "at byte {}: the name `{}` is defined multiple times in this block",
                    self.toks[raw[k].0].pos, def.name
                ));
            }
        }
        // Resolve the whole block at once against the enclosing scope.
        let items: Vec<(String, Type, Expr)> = raw
            .iter()
            .map(|(raw_start, _, _, d)| {
                let ty = self.resolve_const_ty(&d.ty, &d.name, self.toks[*raw_start].pos)?;
                Ok((d.name.clone(), ty, d.value.clone()))
            })
            .collect::<PResult<Vec<_>>>()?;
        let resolved =
            crate::consts::resolve_with(&items, &self.consts, &self.const_bools, &self.const_chars);
        // Every item must fold, including an unused one. `resolve_with` leaves
        // out an item whose initializer is not a constant expression (a call, a
        // cycle, a reference to an unresolved name), which at the top level
        // `check_consts` rejects; do the same here.
        for ((raw_start, _, _, def), (_, ty, _)) in raw.iter().zip(items.iter()) {
            if !resolved.contains_key(&def.name) {
                let kind = match ty {
                    Type::Bool => "bool",
                    Type::Char => "char",
                    _ => "i64",
                };
                return Err(format!(
                    "at byte {}: constant `{}` is not a constant `{kind}` expression",
                    self.toks[*raw_start].pos, def.name
                ));
            }
        }
        let mut frame: Vec<LocalConst> = Vec::new();
        for (_, start, end, def) in &raw {
            self.enter_local_const(&def.name, resolved.get(&def.name).copied());
            // `start` is the `const`/`static` keyword (past any `pub`), so the
            // main parse, which drops a `pub` before it, matches on the same
            // index.
            frame.push(LocalConst { start: *start, end: *end });
        }
        Ok(frame)
    }

    /// The names of the `fn`s declared directly in the block at the cursor,
    /// without registering them (used only for the `const`/`fn` collision check;
    /// `scan_block_fns` does the real hoisting).
    fn scan_block_fn_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut depth = 0u32;
        let mut i = self.i;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Tok::RParen | Tok::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                Tok::Fn if depth == 0 => {
                    // The name always follows the `fn` keyword.
                    if let Some(Token { tok: Tok::Ident(name), .. }) = self.toks.get(i + 1) {
                        names.push(name.clone());
                    }
                }
                _ => {}
            }
            i += 1;
        }
        names
    }

    /// True when the token at the cursor begins a `const`/immutable-`static`
    /// item that `scan_block_consts` already folded for this block, so the main
    /// parse should skip it. A `const` a block does not own (a nested block's,
    /// seen from outside) is not in any active frame and returns false.
    fn at_local_const(&self) -> bool {
        self.local_consts.iter().any(|f| f.iter().any(|c| c.start == self.i))
    }

    /// Reject a local binding that shadows a constant (rustc E0530). sema
    /// enforces this for a top-level constant, but a block-scoped one is folded
    /// at parse time and never reaches sema, so the parser checks every binding
    /// it lowers against the live constant tables. `names` are the names a `let`
    /// pattern binds, in source order.
    fn reject_const_shadow(&self, names: &[String]) -> PResult<()> {
        for name in names {
            if self.consts.contains_key(name)
                || self.const_bools.contains_key(name)
                || self.const_chars.contains_key(name)
            {
                return Err(format!(
                    "at byte {}: identifier `{name}` shadows a constant of the same name",
                    self.pos()
                ));
            }
        }
        Ok(())
    }

    /// The token index of the item at `i`, skipping a `pub` / `pub(..)` before
    /// it. Used by `scan_block_consts`, which needs the keyword's own index.
    fn skip_pub_at(&self, mut i: usize) -> usize {
        if matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Ident(p)) if p == "pub") {
            i += 1;
            if self.toks.get(i).map(|t| &t.tok) == Some(&Tok::LParen) {
                let mut d = 0u32;
                while i < self.toks.len() {
                    match &self.toks[i].tok {
                        Tok::LParen => d += 1,
                        Tok::RParen => {
                            d -= 1;
                            if d == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
        i
    }

    /// Enter a folded block-scoped constant in the live tables, noting the
    /// previous value so `pop_name_scope` can restore it when the block ends.
    fn enter_local_const(&mut self, name: &str, val: Option<crate::consts::ConstVal>) {
        use crate::consts::ConstVal;
        // The name is in the value namespace, but `consts`/`const_bools`/
        // `const_chars` are three tables; a name could in principle be in two
        // (never in one well-typed program). Note all three so the restore is
        // exact.
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::ConstInt(name.to_string(), self.consts.get(name).copied()));
            frame.push(NameScope::ConstBool(name.to_string(), self.const_bools.get(name).copied()));
            frame.push(NameScope::ConstChar(name.to_string(), self.const_chars.get(name).copied()));
        }
        // A `const` that does not fold (a call, a cycle) is left out, so a use
        // is diagnosed the way an unresolved constant is; drop any outer value
        // of the same name so the shadowing is honest.
        self.consts.remove(name);
        self.const_bools.remove(name);
        self.const_chars.remove(name);
        match val {
            Some(ConstVal::Int(v)) => {
                self.consts.insert(name.to_string(), v);
            }
            Some(ConstVal::Bool(v)) => {
                self.const_bools.insert(name.to_string(), v);
            }
            Some(ConstVal::Char(v)) => {
                self.const_chars.insert(name.to_string(), v);
            }
            None => {}
        }
    }

    /// Record the current state of one `Vec`/pointer name table entry before a
    /// binding overwrites it, so `pop_name_scope` can restore it. Called for
    /// every binding a block introduces, including one that shadows an outer
    /// binding of the same name.
    fn note_vec_name(&mut self, name: &str) {
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::VecName(name.to_string(), self.vec_names.contains(name)));
        }
    }
    fn note_vec_cap(&mut self, name: &str) {
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::VecCap(name.to_string(), self.vec_cap.get(name).copied()));
        }
    }
    fn note_vec_len(&mut self, name: &str) {
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::VecLen(name.to_string(), self.vec_len.get(name).copied()));
        }
    }
    fn note_vec_elem(&mut self, name: &str) {
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::VecElem(name.to_string(), self.vec_elem.get(name).copied()));
        }
    }
    fn note_ptr(&mut self, name: &str) {
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::Ptr(name.to_string(), self.ptr_vars.get(name).copied()));
        }
    }
    fn note_args_vec(&mut self, name: &str) {
        if let Some(frame) = self.name_scopes.last_mut() {
            frame.push(NameScope::ArgsVec(name.to_string(), self.args_vecs.get(name).copied()));
        }
    }

    /// Undo the bindings the innermost block introduced, in reverse, so each
    /// name returns to the state it had before the block (present-and-old, or
    /// absent). Called on every exit from a block, including the error path.
    fn pop_name_scope(&mut self) {
        if let Some(frame) = self.name_scopes.pop() {
            for entry in frame.into_iter().rev() {
                match entry {
                    NameScope::VecName(n, had) => {
                        if had {
                            self.vec_names.insert(n);
                        } else {
                            self.vec_names.remove(&n);
                        }
                    }
                    NameScope::VecCap(n, prev) => match prev {
                        Some(v) => {
                            self.vec_cap.insert(n, v);
                        }
                        None => {
                            self.vec_cap.remove(&n);
                        }
                    },
                    NameScope::VecLen(n, prev) => match prev {
                        Some(v) => {
                            self.vec_len.insert(n, v);
                        }
                        None => {
                            self.vec_len.remove(&n);
                        }
                    },
                    NameScope::VecElem(n, prev) => match prev {
                        Some(v) => {
                            self.vec_elem.insert(n, v);
                        }
                        None => {
                            self.vec_elem.remove(&n);
                        }
                    },
                    NameScope::Ptr(n, prev) => match prev {
                        Some(v) => {
                            self.ptr_vars.insert(n, v);
                        }
                        None => {
                            self.ptr_vars.remove(&n);
                        }
                    },
                    NameScope::ArgsVec(n, prev) => match prev {
                        Some(v) => {
                            self.args_vecs.insert(n, v);
                        }
                        None => {
                            self.args_vecs.remove(&n);
                        }
                    },
                    NameScope::ConstInt(n, prev) => match prev {
                        Some(v) => {
                            self.consts.insert(n, v);
                        }
                        None => {
                            self.consts.remove(&n);
                        }
                    },
                    NameScope::ConstBool(n, prev) => match prev {
                        Some(v) => {
                            self.const_bools.insert(n, v);
                        }
                        None => {
                            self.const_bools.remove(&n);
                        }
                    },
                    NameScope::ConstChar(n, prev) => match prev {
                        Some(v) => {
                            self.const_chars.insert(n, v);
                        }
                        None => {
                            self.const_chars.remove(&n);
                        }
                    },
                }
            }
        }
    }

    /// Collect the nested `fn`s declared directly in the block whose `{` has
    /// just been consumed, without parsing the block. Each gets a unique hoisted
    /// name, recorded both in the frame returned (name -> unique) and in `fn_at`
    /// keyed by its `fn` keyword, so the definition is later parsed under the
    /// same name. A `fn` nested in an inner block or a nested item's body is
    /// skipped, since it belongs to that block's frame, not this one.
    fn scan_block_fns(&mut self) -> PResult<Vec<(String, String)>> {
        let mut frame: Vec<(String, String)> = Vec::new();
        let mut depth = 0u32;
        let mut i = self.i;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::LBrace | Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RBrace => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Tok::RParen | Tok::RBracket => {
                    depth = depth.saturating_sub(1);
                }
                Tok::Fn if depth == 0 => {
                    // `fn name ( .. ) -> .. { body }`. The body brace is skipped
                    // as a unit by the `depth` counter, so a `fn` inside it is
                    // not seen here. The name always follows the `fn` keyword;
                    // a preceding `pub` is irrelevant, since `i` still points at
                    // the `fn` itself (the loop only advances one token at a
                    // time, so it never lands on the `pub`). `fn_at` is keyed to
                    // that same `i`, which is what `parse_stmt_into` looks up.
                    if let Some(Token { tok: Tok::Ident(name), .. }) = self.toks.get(i + 1) {
                        let name = name.clone();
                        // E0428. A later one shadowing an outer one is fine and
                        // keeps its own unique symbol.
                        if frame.iter().any(|(n, _)| *n == name) {
                            return Err(format!(
                                "at byte {}: the name `{name}` is defined multiple times in this block",
                                self.toks[i].pos
                            ));
                        }
                        // The name carries two `$`, which no source identifier
                        // can contain (a method's mangled name has one), so it
                        // cannot collide with a user function. A NUL would break
                        // the `--binutils` path, whose label goes through GNU
                        // `as`; `$` is a valid symbol character there.
                        let unique = format!("rmc$nf{}${}", self.nf_counter, name);
                        self.nf_counter += 1;
                        self.fn_at.insert(i, unique.clone());
                        frame.push((name, unique));
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Ok(frame)
    }

    /// The hoisted name of the nested `fn` called `name` in the innermost block
    /// that declares one, or `name` itself when no enclosing block does (so a
    /// top-level `fn` and a builtin are left alone).
    fn resolve_fn_name(&self, name: String) -> String {
        for frame in self.fn_scopes.iter().rev() {
            if let Some((_, unique)) = frame.iter().rev().find(|(n, _)| *n == name) {
                return unique.clone();
            }
        }
        name
    }

    fn parse_block_body(&mut self) -> PResult<Block> {
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
                    self.parse_stmt_into(&mut stmts)?;
                    continue;
                }
            }
            // `use` is allowed inside a block as well as at the top level;
            // skip it either way. Guard on what follows so a variable named
            // `use` (which rmc, unlike rustc, would otherwise allow) is not
            // swallowed by mistake.
            if self.peek() == Some(&Tok::Ident("use".into()))
                && matches!(self.peek2(), Some(Tok::Ident(_)) | Some(Tok::ColonColon))
            {
                self.skip_use()?;
                continue;
            }
            // `pub fn` (or `pub` before any item) in a block: drop the
            // visibility and parse the item. Guarded on what follows so a
            // variable named `pub` (which rmc, unlike rustc, would otherwise
            // allow) is not swallowed.
            if self.peek() == Some(&Tok::Ident("pub".into())) {
                let p2 = self.peek2().cloned();
                if p2 == Some(Tok::Fn) {
                    self.skip_pub()?;
                    self.parse_stmt_into(&mut stmts)?;
                    continue;
                }
                // `pub const` / `pub static`: drop the `pub` and let the local
                // constant skip below consume the item (already folded).
                if p2 == Some(Tok::Const)
                    || matches!(p2, Some(Tok::Ident(ref id)) if id == "static")
                {
                    self.skip_pub()?;
                    continue;
                }
            }
            // A `const`/immutable-`static` item directly in the block. Its value
            // was folded by `scan_block_consts` when the block began (rustc
            // hoists block items), so skip the item here; the constant is
            // substituted at every use, so it has no statement.
            if self.at_local_const() {
                let end = self
                    .local_consts
                    .iter()
                    .rev()
                    .flat_map(|f| f.iter().rev())
                    .find(|c| c.start == self.i)
                    .map(|c| c.end);
                if let Some(end) = end {
                    self.i = end;
                    continue;
                }
            }
            // A stray `;` is an empty statement; Rust accepts any number of
            // them between statements and at the end of a block.
            if self.eat(&Tok::Semi) {
                continue;
            }
            // tail expression: an expression not followed by `;`
            if self.starts_expr() {
                let e = self.parse_expr()?;
                if self.eat(&Tok::Eq) {
                    let value = self.parse_expr()?;
                    // An assignment is a `()`-valued expression, so its trailing
                    // `;` may be omitted when it is the last thing in a block,
                    // just as after `return`/`break`/`continue`.
                    self.eat_semi_at_block_end()?;
                    stmts.push(Stmt::Assign { target: e, value });
                    continue;
                }
                if let Some(op) = self.peek_compound_op() {
                    self.i += 1;
                    let value = self.parse_expr()?;
                    self.eat_semi_at_block_end()?;
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
            self.parse_stmt_into(&mut stmts)?;
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
                // `'a: while let PAT = EXPR { body }` desugars to a labelled
                // `loop { if let PAT = EXPR { body } else { break; } }`: the
                // label attaches to the `loop`, so `break 'a` / `continue 'a`
                // in the body target it just as they would a labelled `while`.
                if self.peek() == Some(&Tok::Let) {
                    self.i += 1;
                    let pat = self.parse_refutable_pattern()?;
                    self.expect(Tok::Eq)?;
                    let scrutinee = self.parse_cond()?;
                    let body = self.parse_block()?;
                    let s = self.desugar_while_let(pat, scrutinee, body, Some(label))?;
                    return Ok(s);
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
            if self.consts.contains_key(n)
                || self.const_bools.contains_key(n)
                || self.const_chars.contains_key(n)
            {
                return Err(format!(
                    "identifier `{n}` shadows a constant of the same name"
                ));
            }
        }
        let pat = self.parse_pattern()?;
        self.expect(Tok::In)?;
        let head = self.parse_cond()?;
        // `for v in &args` (and `for v in args`) over an `args()`-vector binding
        // desugars to a range over the argument count; the element is read from
        // the argument array, so the vector is never materialised. `head` is
        // `&args` (an `AddrOf`) or the binding itself.
        let args_name = match &head {
            Expr::AddrOf { expr, .. } => match expr.as_ref() {
                Expr::Var { name, .. } if self.args_vecs.contains_key(name) => Some(name.clone()),
                _ => None,
            },
            Expr::Var { name, .. } if self.args_vecs.contains_key(name) => Some(name.clone()),
            _ => None,
        };
        if let Some(name) = args_name {
            let skip = self.args_vecs[&name];
            let elem = match &pat {
                Pat::Bind(n) => Some(n.clone()),
                _ => None,
            };
            // Mark the loop variable before the body is parsed, so a `*v` in it
            // (Rust's `&i64` element) lowers to the plain value.
            if let Some(ref n) = elem {
                self.args_elems.insert(n.clone());
            }
            let body = self.parse_block();
            if let Some(ref n) = elem {
                self.args_elems.remove(n);
            }
            return self.desugar_for_argsvec(pat, skip, body?, label);
        }
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
            let mut body = self.parse_block()?;
            // A range's loop variable is bound directly, so only an irrefutable
            // pattern is allowed: `for n @ _ in 0..5` binds `n`, and
            // `for n @ m in 0..5` also binds `m` (an alias) in the body.
            let (var, extra) = match pat {
                Pat::Bind(n) => (n, Vec::new()),
                Pat::Wild => {
                    let n = format!("\u{0}fi{}", self.tmp);
                    self.tmp += 1;
                    (n, Vec::new())
                }
                Pat::At(n, _, sub) => {
                    if !Self::pat_irrefutable(&sub) {
                        return Err(format!(
                            "at byte {}: refutable pattern in `for` binding",
                            self.pos()
                        ));
                    }
                    let subj = Expr::Var { name: n.clone(), slot: -1, ty: Type::I64 };
                    let mut binds = Vec::new();
                    Self::pat_binds(&sub, false, &subj, &mut binds);
                    (n, binds)
                }
                _ => {
                    return Err(format!(
                        "at byte {}: `for` over a range needs a binding or `_`",
                        self.pos()
                    ))
                }
            };
            if !extra.is_empty() {
                let mut stmts = extra;
                stmts.extend(body.stmts);
                body = Block { stmts, tail: body.tail };
            }
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
        self.desugar_for_array(pat, head, body, label)
    }

    /// `for PAT in ARRAY { body }` -> an index loop over the array (see
    /// `parse_for`). `PAT` may be a binding, `_`, or a struct pattern; any other
    /// pattern is refutable (rustc requires an irrefutable `for` pattern) and is
    /// rejected rather than mis-bound. A label on the `for` is carried to the
    /// desugared loop, so `break 'a` still targets it.
    fn desugar_for_array(
        &mut self,
        pat: Pat,
        arr: Expr,
        body: Block,
        label: Option<String>,
    ) -> PResult<Stmt> {
        let n = self.tmp;
        self.tmp += 1;
        let arr_name = format!("\u{0}fa{n}");
        let idx_name = format!("\u{0}fi{n}");
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
        // The per-iteration bindings, run before the user body: `let PAT =
        // arr[i];` for a binding or `_`, or a hidden temporary plus the field
        // reads for a struct pattern.
        let mut stmts = match pat {
            Pat::Bind(name) => vec![Stmt::Let {
                name,
                ty: None,
                init: Some(elem),
                slot: -1,
                mutable: false,
            }],
            Pat::Wild => vec![Stmt::Let {
                name: format!("\u{0}fd{n}"),
                ty: None,
                init: Some(elem),
                slot: -1,
                mutable: false,
            }],
            Pat::Struct(name, fields) => self.let_destructure_stmts(name, fields, None, elem),
            Pat::Tuple(elems) => self.let_destructure_tuple_stmts(elems, None, elem),
            Pat::Array { elems, rest, front } => {
                self.let_destructure_array_stmts(elems, rest, front, None, elem)
            }
            // `for n @ subpat in arr` binds the element to `n` and also matches
            // it against `subpat`. The sub-pattern must be irrefutable (a `for`
            // pattern is), which rustc enforces too.
            Pat::At(name, am, sub) => {
                if !Self::pat_irrefutable(&sub) {
                    return Err(format!(
                        "at byte {}: refutable pattern in `for` binding",
                        self.pos()
                    ));
                }
                let mut stmts = vec![Stmt::Let {
                    name,
                    ty: None,
                    init: Some(elem.clone()),
                    slot: -1,
                    mutable: am,
                }];
                Self::pat_binds(&sub, false, &elem, &mut stmts);
                stmts
            }
            _ => {
                return Err(format!(
                    "at byte {}: `for` over an array needs a binding, `_`, a struct, tuple or array pattern",
                    self.pos()
                ))
            }
        };
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
        Ok(Stmt::Expr(Expr::Block(Block { stmts: vec![arr_let, for_stmt], tail: None })))
    }

    /// `for PAT in &args { body }` over an `args()`-vector binding, desugared to
    /// `for i in skip..__rmc_nargs() { let PAT = __rmc_arg_or(i, 0); body }`. The
    /// element read is the runtime's argv parse, so no `Vec` is materialised.
    /// `elem` names the loop variable when `PAT` is a binding, so the body's
    /// `*v` (Rust's `&i64` element) can be lowered to `v`.
    fn desugar_for_argsvec(
        &mut self,
        pat: Pat,
        skip: i64,
        mut body: Block,
        label: Option<String>,
    ) -> PResult<Stmt> {
        let n = self.tmp;
        self.tmp += 1;
        let idx_name = format!("\u{0}ai{n}");
        let idx_var = Expr::Var { name: idx_name.clone(), slot: -1, ty: Type::I64 };
        let elem_expr = self.args_or_at(&idx_var, 0);
        let stmts = match pat {
            Pat::Bind(name) => vec![Stmt::Let {
                name,
                ty: None,
                init: Some(elem_expr),
                slot: -1,
                mutable: false,
            }],
            Pat::Wild => vec![Stmt::Let {
                name: format!("\u{0}ad{n}"),
                ty: None,
                init: Some(elem_expr),
                slot: -1,
                mutable: false,
            }],
            other => {
                let _ = other;
                return Err(format!(
                    "at byte {}: `for` over an `args()` vector needs a binding or `_`",
                    self.pos()
                ));
            }
        };
        let mut stmts = stmts;
        stmts.extend(body.stmts.drain(..));
        let for_stmt = Stmt::For {
            var: idx_name,
            var_slot: -1,
            start: Expr::Int(skip),
            end: Expr::Call {
                name: "__rmc_nargs".into(),
                args: Vec::new(),
                arg_slots: Vec::new(),
                ret_slot: -1,
                ret_ty: Type::I64,
            },
            end_slot: -1,
            inclusive: false,
            body: Block { stmts, tail: body.tail },
            label,
        };
        Ok(for_stmt)
    }

    /// `__rmc_arg_or(i, default)` for a runtime index `i` (the constant form is
    /// `args_or`).
    fn args_or_at(&self, idx: &Expr, default: i64) -> Expr {
        Expr::Call {
            name: "__rmc_arg_or".into(),
            args: vec![idx.clone(), Expr::Int(default)],
            arg_slots: Vec::new(),
            ret_slot: -1,
            ret_ty: Type::I64,
        }
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
                // without a `;`. Without this it fell through to the statement
                // parser, which has no arm for `{`, so a nested block failed to
                // parse.
                | Some(Tok::LBrace)
        )
    }

    /// Parse one statement and append it to `out`. A statement is usually one
    /// `Stmt`, but `let` destructuring expands to several (a hidden scrutinee
    /// plus one binding per field), so the caller passes the enclosing block's
    /// statement list: the expansion is spliced into it rather than wrapped in
    /// a nested block, which would hide the bindings from the rest of the block.
    fn parse_stmt_into(&mut self, out: &mut Vec<Stmt>) -> PResult<()> {
        // A loop label may prefix `while` or `for`; a labelled `loop` is an
        // expression and is handled by `parse_primary` (routed here or there by
        // `parse_block_inner` depending on whether it is a tail expression).
        if let Some(Tok::Label(n)) = self.peek().cloned() {
            self.i += 1;
            self.expect(Tok::Colon)?;
            let s = self.parse_labeled_loop_stmt(n)?;
            out.push(s);
            return Ok(());
        }
        match self.peek() {
            // A nested `fn` is an item, not an expression: it binds a name for
            // the whole enclosing block (and, since it captures nothing, is
            // hoisted to the top level by `parse_program`). It is parsed here so
            // it may appear among statements; the trailing `;` is not required.
            // `scan_block_fns` already gave it a unique name and put it in the
            // block's frame, so a call in this block resolves to it.
            Some(Tok::Fn) => {
                let at = self.i;
                let mut f = self.parse_fn()?;
                // The pre-scan already named this `fn` and put it in the block's
                // frame; the definition takes that same name. A `fn` the scan
                // did not see (it never should) falls back to its own name.
                if let Some(unique) = self.fn_at.get(&at).cloned() {
                    f.name = unique;
                }
                self.nested_fns.push(f);
                return Ok(());
            }
            Some(Tok::Let) => {
                self.i += 1;
                // `mut` before the name marks that binding mutable. A struct
                // pattern attaches `mut` to each field instead, so `let mut
                // S { .. }` is rejected, as in rustc.
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
                    out.push(Stmt::Expr(Expr::Block(Block { stmts, tail: None })));
                    return Ok(());
                }
                // `let S { f, g: h } [: T] = e;` destructures a struct, and
                // `let [a, b] [: T] = e;` destructures an array. The pattern is
                // read by the same parser as a `match` pattern, but here it is
                // irrefutable by construction, so the annotation goes on the
                // scrutinee temporary below and the fields/elements are bound
                // from it. `..` and `f: _` are the only way to drop fields.
                let struct_pat = matches!(self.peek(), Some(Tok::Ident(_)))
                    && self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::LBrace);
                // `let n @ subpat = e;` binds the whole value to `n` and also
                // destructures it. Detected by the `@` after the name.
                let at_pat = matches!(self.peek(), Some(Tok::Ident(_)))
                    && self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::At);
                if struct_pat
                    || at_pat
                    || self.peek() == Some(&Tok::LBracket)
                    || self.peek() == Some(&Tok::LParen)
                {
                    if mutable && !at_pat {
                        return Err(format!(
                            "at byte {}: `mut` must be attached to each individual binding, as `S {{ mut f }}`",
                            self.pos()
                        ));
                    }
                    let pat = self.parse_pattern_primary()?;
                    // Every name the pattern binds is a local binding, so none
                    // may shadow a constant (rustc E0530). sema covers a
                    // top-level constant; a block-scoped one is folded here in
                    // the parser, so check it here.
                    let mut bound = Vec::new();
                    Self::pat_bound_names(&pat, &mut bound);
                    self.reject_const_shadow(&bound)?;
                    let ty = if self.eat(&Tok::Colon) {
                        Some(self.parse_type()?)
                    } else {
                        None
                    };
                    self.expect(Tok::Eq)?;
                    let init = self.parse_expr()?;
                    self.expect(Tok::Semi)?;
                    match pat {
                        Pat::Struct(name, fields) => {
                            out.extend(self.let_destructure_stmts(name, fields, ty, init))
                        }
                        Pat::Tuple(elems) => {
                            out.extend(self.let_destructure_tuple_stmts(elems, ty, init))
                        }
                        Pat::Array { elems, rest, front } => {
                            out.extend(self.let_destructure_array_stmts(elems, rest, front, ty, init))
                        }
                        // A parenthesised pattern that is not a tuple: `(x)` is
                        // an ordinary binding and `(_)` ignores the value, as
                        // rustc reads them.
                        Pat::Bind(name) => out.push(Stmt::Let {
                            name,
                            ty,
                            init: Some(init),
                            slot: -1,
                            mutable: false,
                        }),
                        // `let n @ subpat = e;` binds the whole value to `n`
                        // and also matches it against `subpat`. It is a local
                        // binding, so `subpat` must be irrefutable: `let n @
                        // 1..=5 = e;` is a refutable local binding, which rustc
                        // rejects (E0005).
                        Pat::At(name, am, sub) => {
                            if !Self::pat_irrefutable(&sub) {
                                return Err(format!(
                                    "at byte {}: refutable pattern in local binding",
                                    self.pos()
                                ));
                            }
                            // `n` itself and the sub-pattern's names both bind;
                            // if the initializer reads `n` (or a name the
                            // sub-pattern binds), copy it first.
                            let mut names = vec![name.clone()];
                            Self::pat_bound_names(&sub, &mut names);
                            let force = Self::init_shadowed_by(&init, &names);
                            let (mut stmts, subj) = self.let_destructure_prelude(ty, init, force);
                            stmts.push(Stmt::Let {
                                name,
                                ty: None,
                                init: Some(subj.clone()),
                                slot: -1,
                                mutable: mutable || am,
                            });
                            Self::pat_binds(&sub, false, &subj, &mut stmts);
                            out.extend(stmts)
                        }
                        Pat::Wild => out.push(Stmt::Let {
                            name: "\u{0}wild".to_string(),
                            ty,
                            init: Some(init),
                            slot: -1,
                            mutable: false,
                        }),
                        _ => unreachable!("a `(` starts a tuple, binding or wildcard pattern"),
                    }
                    return Ok(());
                }
                let name = self.expect_ident()?;
                // A `let` may not shadow a constant (rustc E0530). sema enforces
                // this for a top-level constant, but a block-scoped one is folded
                // at parse time and never reaches sema, so enforce it here too.
                self.reject_const_shadow(&[name.clone()])?;
                // A `Vec<T>` annotation leaves its element width in
                // `vec_elem_pending`; consume it here so it is bound to this
                // name, and clear it for every other annotation.
                let ty = if self.eat(&Tok::Colon) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let elem_width = if matches!(ty, Some(Type::Named(ref n)) if n == "Vec") {
                    self.vec_elem_pending.take().unwrap_or(8)
                } else {
                    self.vec_elem_pending = None;
                    8
                };
                let init = if self.eat(&Tok::Eq) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect(Tok::Semi)?;
                // A `Vec` binding is lowered to an `i64` address of its backing
                // allocation. Three initializers are supported, each replaced by
                // the allocation (plus, for `vec![..]`, a fill). The binding and
                // its shape are recorded so the methods the corpus uses
                // (`.capacity()`, `.len()`, `.as_mut_ptr()`) fold at parse time.
                if let Some(Expr::Call { name: m, args, .. }) = init.clone() {
                    if m == "\u{0}argsvec" {
                        if !matches!(ty, Some(Type::Named(ref n)) if n == "Vec") {
                            return Err(format!(
                                "at byte {}: `std::env::args()...collect()` needs a `Vec<i64>` binding",
                                self.pos()
                            ));
                        }
                        self.note_args_vec(&name);
                        self.args_vecs.insert(name.clone(), self.args_skip);
                        return Ok(());
                    }
                    if m == "\u{0}vecnew" || m == "\u{0}vecrepeat" {
                        let n_expr = if m == "\u{0}vecnew" {
                            args[0].clone()
                        } else {
                            args[1].clone()
                        };
                        // The count may be a constant or a runtime value; only a
                        // constant is recorded for a later `.len()`/`.capacity()`
                        // fold. Either way the size is `count * 8` bytes at run
                        // time and `__rmc_fill` takes the count as a value.
                        let count = eval_i64(&n_expr, &self.consts);
                        self.note_vec_name(&name);
                        self.note_vec_elem(&name);
                        self.note_vec_len(&name);
                        self.note_vec_cap(&name);
                        self.vec_names.insert(name.clone());
                        self.vec_elem.insert(name.clone(), elem_width);
                        if let Some(c) = count {
                            self.vec_len.insert(name.clone(), c);
                            if m == "\u{0}vecnew" {
                                self.vec_cap.insert(name.clone(), c);
                            }
                        }
                        // `alloc(count * elem_width)`; an `i64`/pointer element
                        // is one machine word, a `u8`/`i8` element is one byte.
                        // Sizing by the element width (not always 8) is what
                        // keeps `vec![0u8; n]` from allocating eight times the
                        // memory the source asks for.
                        let bytes = Expr::Bin(
                            BinOp::Mul,
                            Box::new(n_expr.clone()),
                            Box::new(Expr::Int(elem_width)),
                        );
                        let addr = Expr::Call {
                            name: "alloc".into(),
                            args: vec![bytes],
                            arg_slots: Vec::new(),
                            ret_slot: -1,
                            ret_ty: Type::I64,
                        };
                        let init_expr = if m == "\u{0}vecnew" {
                            addr
                        } else {
                            // Write the repeated element. A one-byte element uses
                            // the byte fill (writing `count` words would overrun a
                            // `count`-byte block); every wider element uses the
                            // word fill, `count` words being `count * 8` bytes.
                            let fill = if elem_width == 1 {
                                "__rmc_fill_bytes"
                            } else {
                                "__rmc_fill"
                            };
                            Expr::Call {
                                name: fill.into(),
                                args: vec![addr, n_expr, args[0].clone()],
                                arg_slots: Vec::new(),
                                ret_slot: -1,
                                ret_ty: Type::I64,
                            }
                        };
                        out.push(Stmt::Let {
                            name,
                            ty: Some(Type::I64),
                            init: Some(init_expr),
                            slot: -1,
                            mutable,
                        });
                        return Ok(());
                    }
                    if m == "\u{0}veclist" {
                        return Err(format!(
                            "at byte {}: `vec![a, b, c]` is not supported (use `vec![e; n]`)",
                            self.pos()
                        ));
                    }
                    // `let p = base.as_mut_ptr();`: `base` is a `Vec` address, so
                    // `p` holds that same address. Record it a raw pointer, with
                    // the element stride of `base`, so a later `p.add(off)`
                    // scales `off` by the element width (`add` steps whole
                    // elements, not bytes).
                    if m == "\u{0}ptr" {
                        let stride = match &args[0] {
                            Expr::Var { name: base, .. } => {
                                self.vec_elem.get(base).copied().unwrap_or(8)
                            }
                            _ => 8,
                        };
                        self.note_ptr(&name);
                        self.ptr_vars.insert(name.clone(), stride);
                        out.push(Stmt::Let {
                            name,
                            ty: Some(Type::I64),
                            init: Some(args[0].clone()),
                            slot: -1,
                            mutable,
                        });
                        return Ok(());
                    }
                }
                if matches!(ty, Some(Type::Named(ref n)) if n == "Vec") {
                    return Err(format!(
                        "at byte {}: a `Vec` binding must be initialized by \
                         `Vec::with_capacity(k)`, `vec![e; n]` or an `args()` collect",
                        self.pos()
                    ));
                }
                out.push(Stmt::Let { name, ty, init, slot: -1, mutable });
                Ok(())
            }
            Some(Tok::While) => {
                self.i += 1;
                // `while let PAT = EXPR { body }` re-tests `EXPR` each iteration
                // and stops at the first value `PAT` does not match.
                if self.peek() == Some(&Tok::Let) {
                    self.i += 1;
                    let pat = self.parse_refutable_pattern()?;
                    self.expect(Tok::Eq)?;
                    let scrutinee = self.parse_cond()?;
                    let body = self.parse_block()?;
                    let s = self.desugar_while_let(pat, scrutinee, body, None)?;
                    out.push(s);
                    return Ok(());
                }
                let cond = self.parse_cond()?;
                let body = self.parse_block()?;
                out.push(Stmt::While(cond, body, None));
                Ok(())
            }
            Some(Tok::For) => {
                self.i += 1;
                let s = self.parse_for()?;
                out.push(s);
                Ok(())
            }
            Some(Tok::Return) => {
                self.i += 1;
                // `return`, `return expr` and `return;` are all valid, and the
                // `;` may be omitted when the `return` is the last thing in the
                // block (`{ return 2 }`), as in Rust.
                if self.eat(&Tok::Semi) || self.peek() == Some(&Tok::RBrace) {
                    out.push(Stmt::Return(None));
                    return Ok(());
                }
                let e = self.parse_expr()?;
                self.eat_semi_at_block_end()?;
                out.push(Stmt::Return(Some(e)));
                Ok(())
            }
            Some(Tok::Break) => {
                self.i += 1;
                // `break`, `break 'label`, `break expr` or `break 'label expr`;
                // the `;` may be omitted at the end of a block. A value is only
                // legal in a `loop`, checked by sema.
                let label = self.eat_label()?;
                if self.eat(&Tok::Semi) || self.peek() == Some(&Tok::RBrace) {
                    out.push(Stmt::Break(None, label));
                    return Ok(());
                }
                let e = self.parse_expr()?;
                self.eat_semi_at_block_end()?;
                out.push(Stmt::Break(Some(e), label));
                Ok(())
            }
            Some(Tok::Continue) => {
                self.i += 1;
                let label = self.eat_label()?;
                self.eat_semi_at_block_end()?;
                out.push(Stmt::Continue(label));
                Ok(())
            }
            _ => {
                // expression statement, possibly an assignment
                let e = self.parse_expr()?;
                if self.eat(&Tok::Eq) {
                    let value = self.parse_expr()?;
                    self.eat_semi_at_block_end()?;
                    out.push(Stmt::Assign { target: e, value });
                    return Ok(());
                }
                if let Some(op) = self.peek_compound_op() {
                    self.i += 1;
                    let value = self.parse_expr()?;
                    self.eat_semi_at_block_end()?;
                    let s = self.desugar_compound(&e, op, value);
                    out.push(s);
                    return Ok(());
                }
                self.expect(Tok::Semi)?;
                out.push(Stmt::Expr(e));
                Ok(())
            }
        }
    }

    /// `Name { f: v, ..base }`: fill the fields not written from `base`. The
    /// desugaring matches rustc's evaluation order — the written field values
    /// first, left to right, then the base — and produces a fresh value that
    /// does not alias `base`:
    ///
    /// ```text
    /// { let v = <field value>; ...; let mut t = base; t.f = v; ...; t }
    /// ```
    ///
    /// Every field value is bound to a temporary *before* the base runs, so its
    /// side effects happen first. `t` and the temporaries get fresh NUL-prefixed
    /// names, so they can never collide with a source identifier. An `i64` field
    /// is written straight into `t` (which stays in a register when it is hot);
    /// an aggregate field is copied into `t`'s storage from its temporary,
    /// because `t.f = v` would otherwise store the temporary's address rather
    /// than its words.
    fn desugar_struct_update(
        &mut self,
        name: String,
        fields: Vec<FieldInit>,
        base: Box<Expr>,
    ) -> Expr {
        let tname = format!("\u{0}su{}", self.tmp);
        self.tmp += 1;
        let tvar = || Expr::Var { name: tname.clone(), slot: -1, ty: Type::I64 };

        let mut stmts = Vec::new();
        // Evaluate the written fields first, in source order, each into its own
        // temporary. The type is left unresolved so sema checks the value
        // against the field (an `i64` literal is not forced to be the struct).
        let mut writes: Vec<(String, Type, String)> = Vec::new();
        for fi in fields {
            let vname = format!("\u{0}sv{}", self.tmp);
            self.tmp += 1;
            stmts.push(Stmt::Let {
                name: vname.clone(),
                ty: None,
                init: Some(fi.value),
                slot: -1,
                mutable: false,
            });
            writes.push((vname, fi.ty, fi.name));
        }

        // Then the base, into the value the block yields.
        stmts.push(Stmt::Let {
            name: tname.clone(),
            ty: Some(Type::Named(name)),
            init: Some(*base),
            slot: -1,
            mutable: true,
        });

        for (vname, ty, fname) in writes {
            let target = Expr::Field { base: Box::new(tvar()), name: fname, offset: 0, ty: Type::I64 };
            let value = Expr::Var { name: vname, slot: -1, ty };
            stmts.push(Stmt::Assign { target, value });
        }

        Expr::Block(Block { stmts, tail: Some(Box::new(tvar())) })
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

        // The modifying operand is evaluated before the assignee place (the one
        // right-to-left expression), so its side effects run first: bind it now,
        // ahead of any place temporary, or a call in the place would overtake it.
        let rv = format!("\u{0}rv{}", self.tmp);
        self.tmp += 1;
        stmts.push(Stmt::Let {
            name: rv.clone(),
            ty: Some(Type::I64),
            init: Some(value),
            slot: -1,
            mutable: true,
        });

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

    /// Parse an expression inside a bracketing delimiter (`(`, `[`), where a `{`
    /// is always a struct literal — even inside a `for`/`while`/`if` head, whose
    /// `no_struct_lit` flag exists only to stop the head from swallowing the
    /// body's `{`. rustc scopes that restriction to the head itself, not to a
    /// nested delimiter, so `for p in [P { x: 1 }] { .. }` is legal.
    fn parse_bracketed_expr(&mut self) -> PResult<Expr> {
        let saved = self.no_struct_lit;
        self.no_struct_lit = false;
        let r = self.parse_expr();
        self.no_struct_lit = saved;
        r
    }

    /// `assert!(cond)`, `assert!(cond, msg)`, `assert_eq!(a, b)` and
    /// `assert_ne!(a, b)`. Each becomes `if !(cond) { panic }` where the panic
    /// body exits with status 101, so a failing assertion matches rustc's exit
    /// code. The optional message is ignored (rmc prints no panic message, only
    /// the status). `assert_eq!`/`assert_ne!` compare with `==`/`!=`; when the
    /// operands are `str` that is the content comparison sema already lowers.
    fn parse_assert(&mut self, name: &str) -> PResult<Expr> {
        self.i += 1; // the `!`
        self.expect(Tok::LParen)?;
        let a = self.parse_bracketed_expr()?;
        let cond = match name {
            "assert" => a,
            "assert_eq" | "assert_ne" => {
                self.expect(Tok::Comma)?;
                let b = self.parse_bracketed_expr()?;
                let op = if name == "assert_eq" { BinOp::Eq } else { BinOp::Ne };
                Expr::Bin(op, Box::new(a), Box::new(b))
            }
            _ => unreachable!(),
        };
        // Any remaining arguments are a custom panic message; discard them.
        while self.eat(&Tok::Comma) {
            let _ = self.parse_bracketed_expr()?;
        }
        self.expect(Tok::RParen)?;
        Ok(Expr::If(
            Box::new(Expr::Un(UnOp::Not, Box::new(cond))),
            Block { stmts: vec![Stmt::Expr(Self::panic_call())], tail: None },
            None,
        ))
    }

    /// `panic!()`, `unreachable!()`, `todo!()` and `unimplemented!()`: exit with
    /// status 101, rustc's panic code. Any arguments (a message) are discarded.
    fn parse_panic(&mut self, _name: &str) -> PResult<Expr> {
        self.i += 1; // the `!`
        self.expect(Tok::LParen)?;
        while self.peek() != Some(&Tok::RParen) {
            let _ = self.parse_bracketed_expr()?;
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        Ok(Self::panic_call())
    }

    /// The `exit(101)` call a failed assertion or a `panic!` lowers to.
    fn panic_call() -> Expr {
        Expr::Call {
            name: "exit".into(),
            args: vec![Expr::Int(101)],
            arg_slots: vec![(-1, 0)],
            ret_slot: -1,
            ret_ty: Type::Unit,
        }
    }


    /// `std::<mod>::<item>(args)` (or `<mod>::<item>(args)` after a
    /// `use std::<mod>;`) for the few std items rmc provides without a standard
    /// library. Only `process::exit` and `hint::black_box` are known; both lower
    /// to something rmc already has, so the call type-checks and runs the same
    /// as under rustc for the programs rmc accepts. Returns `Ok(None)` without
    /// consuming anything when the path is not one rmc knows, so the caller can
    /// fall back to the ordinary `::` (enum variant) handling.
    fn try_parse_std_path(&mut self, name: &str) -> PResult<Option<Expr>> {
        // `self.i` is at the first `::`. Work out `(module, item)` by looking
        // ahead before consuming, so an unknown path can be declined.
        let seg = |p: &Self, k: usize| match p.toks.get(k).map(|t| &t.tok) {
            Some(Tok::Ident(n)) => Some(n.clone()),
            _ => None,
        };
        let (module, item) = if name == "std" {
            if self.toks.get(self.i + 2).map(|t| &t.tok) != Some(&Tok::ColonColon) {
                return Ok(None);
            }
            match (seg(self, self.i + 1), seg(self, self.i + 3)) {
                (Some(m), Some(it)) => (m, it),
                _ => return Ok(None),
            }
        } else {
            match seg(self, self.i + 1) {
                Some(it) => (name.to_string(), it),
                None => return Ok(None),
            }
        };
        let known = matches!(
            (module.as_str(), item.as_str()),
            ("process", "exit") | ("hint", "black_box") | ("ptr", "null")
        );
        // `std::env::args()` (and `env::args()` after `use std::env;`) has no
        // argument list; it lowers to a hidden marker the postfix parser folds
        // together with the iterator-method chain that follows it.
        if (module.as_str(), item.as_str()) == ("env", "args") {
            self.expect(Tok::ColonColon)?; // the first `::`
            if name == "std" {
                self.expect_ident()?; // the module
                self.expect(Tok::ColonColon)?;
            }
            self.expect_ident()?; // `args`
            self.expect(Tok::LParen)?;
            self.expect(Tok::RParen)?;
            return Ok(Some(Expr::Call {
                name: "\u{0}args".into(),
                args: Vec::new(),
                arg_slots: Vec::new(),
                ret_slot: -1,
                ret_ty: Type::I64,
            }));
        }
        if !known {
            // A full `std::` path is unambiguously a std import, so it gets a
            // clear message; an aliased module may just be a same-file item.
            if name == "std" {
                return Err(format!(
                    "at byte {}: unsupported path `std::{module}::{item}` \
                     (only `std::process::exit` and `std::hint::black_box` are provided)",
                    self.pos()
                ));
            }
            return Ok(None);
        }
        self.expect(Tok::ColonColon)?; // the first `::`
        if name == "std" {
            self.expect_ident()?; // the module
            self.expect(Tok::ColonColon)?;
        }
        self.expect_ident()?; // the item
        self.expect(Tok::LParen)?;
        // `std::ptr::null()` / `std::ptr::null_mut()` take no argument; every
        // other known path takes exactly one.
        if (module.as_str(), item.as_str()) == ("ptr", "null") {
            self.expect(Tok::RParen)?;
            return Ok(Some(Expr::Int(0)));
        }
        let arg = self.parse_bracketed_expr()?;
        self.expect(Tok::RParen)?;
        let e = match (module.as_str(), item.as_str()) {
            // `std::process::exit(code)`: terminate with `code`. `exit` is the
            // runtime's raw `exit` syscall wrapper; it diverges, so the parser
            // types the call `()` like any other.
            ("process", "exit") => Expr::Call {
                name: "exit".into(),
                args: vec![arg],
                arg_slots: Vec::new(),
                ret_slot: -1,
                ret_ty: Type::Unit,
            },
            // `std::hint::black_box(x)`: the identity. rmc has no optimizer that
            // would otherwise fold the value away, so the barrier is a no-op.
            ("hint", "black_box") => arg,
            _ => unreachable!(),
        };
        Ok(Some(e))
    }

    /// Lower the iterator-method chain that follows `std::env::args()`. rmc has
    /// no `Iterator`/`Option`/`Result` types, so it recognises the shapes the
    /// suite and benchmarks use and folds each to an `i64`:
    ///
    /// * `.nth(i).unwrap()` / `.nth(i).unwrap().parse().unwrap()`
    ///   / `.nth(i).unwrap_or(d)` / `.nth(i).and_then(|s| s.parse().ok()).unwrap_or(d)`
    ///     -> `__rmc_arg_or(i, d)` (`d` is `0` without an `unwrap_or`).
    /// * `.skip(k).map(|s| s.parse::<i64>().unwrap()).collect()` -> an array of
    ///   the arguments from index `k` on, used by the `for v in &args` form.
    ///
    /// A `|s| ...` closure body is not parsed for its own sake: only the
    /// `s.parse::<i64>().unwrap()` / `.parse().ok()` shape is accepted, and its
    /// binding name is ignored. Any other shape is rejected with a clear
    /// message rather than mis-compiled.
    fn finish_args_chain(&mut self, chain: Expr) -> PResult<Expr> {
        // `chain` is the `\0args` marker. It must be followed by `.`.
        debug_assert!(matches!(&chain, Expr::Call { name, .. } if name == "\u{0}args"));
        let mut skip = 0i64;
        // `skip(n)`: advance the starting index (only the last one counts, as
        // for any iterator).
        loop {
            if !self.eat(&Tok::Dot) {
                return Err(format!(
                    "at byte {}: `std::env::args()` must be followed by an iterator method",
                    self.pos()
                ));
            }
            let m = self.expect_ident()?;
            match m.as_str() {
                "skip" => {
                    self.expect(Tok::LParen)?;
                    let n = self.parse_bracketed_expr()?;
                    self.expect(Tok::RParen)?;
                    skip = eval_i64(&n, &self.consts).ok_or_else(|| {
                        format!("at byte {}: `skip` needs a constant count", self.pos())
                    })?;
                }
                "nth" => {
                    self.expect(Tok::LParen)?;
                    let n = self.parse_bracketed_expr()?;
                    self.expect(Tok::RParen)?;
                    let idx = eval_i64(&n, &self.consts).ok_or_else(|| {
                        format!("at byte {}: `nth` needs a constant index", self.pos())
                    })?;
                    return self.finish_args_nth(idx + skip);
                }
                "map" => {
                    self.expect(Tok::LParen)?;
                    self.skip_closure()?;
                    self.expect(Tok::RParen)?;
                    // Only the `.collect()` array form follows a `map` in the
                    // supported shapes.
                    self.expect(Tok::Dot)?;
                    let c = self.expect_ident()?;
                    if c != "collect" {
                        return Err(format!(
                            "at byte {}: `std::env::args().map(..)` must be followed by `collect()`",
                            self.pos()
                        ));
                    }
                    self.expect(Tok::LParen)?;
                    self.expect(Tok::RParen)?;
                    return self.finish_args_collect(skip);
                }
                other => {
                    return Err(format!(
                        "at byte {}: unsupported `std::env::args()` method `{other}`",
                        self.pos()
                    ))
                }
            }
        }
    }

    /// `.nth(idx)` already consumed: accept `unwrap()` / `parse().unwrap()` /
    /// `unwrap_or(d)` / `and_then(|s| s.parse().ok()).unwrap_or(d)` and fold to
    /// `__rmc_arg_or(idx, default)`.
    fn finish_args_nth(&mut self, idx: i64) -> PResult<Expr> {
        let mut default: Option<Expr> = None;
        let mut done = false;
        while self.peek() == Some(&Tok::Dot) {
            self.i += 1;
            let m = self.expect_ident()?;
            match m.as_str() {
                "parse" => {
                    // `parse::<i64>()` carries a turbofish; skip it.
                    if self.eat(&Tok::ColonColon) {
                        self.expect(Tok::Lt)?;
                        self.expect_ident()?; // the type name
                        self.expect(Tok::Gt)?;
                    }
                    self.expect(Tok::LParen)?;
                    self.expect(Tok::RParen)?;
                }
                "unwrap" => {
                    self.expect(Tok::LParen)?;
                    self.expect(Tok::RParen)?;
                    done = true;
                }
                "unwrap_or" => {
                    self.expect(Tok::LParen)?;
                    let d = self.parse_bracketed_expr()?;
                    self.expect(Tok::RParen)?;
                    default = Some(d);
                    done = true;
                }
                "and_then" => {
                    self.expect(Tok::LParen)?;
                    self.skip_closure()?;
                    self.expect(Tok::RParen)?;
                }
                other => {
                    return Err(format!(
                        "at byte {}: unsupported `std::env::args().nth(..)` method `{other}`",
                        self.pos()
                    ))
                }
            }
        }
        if !done {
            return Err(format!(
                "at byte {}: `std::env::args().nth(..)` must end with `unwrap()` or `unwrap_or(..)`",
                self.pos()
            ));
        }
        Ok(self.args_or(idx, default.unwrap_or(Expr::Int(0))))
    }

    /// `.skip(k).map(|s| s.parse::<i64>().unwrap()).collect()`: the arguments
    /// from index `k` on, as an array. Only the `for v in &args` loop reads it,
    /// so the array's length is `argc - k` clamped to the `[i64; N]` the source
    /// declares. Since `N` is a compile-time constant and `argc` is not, rmc
    /// emits a fixed-size array of `N` elements, each `__rmc_arg_or(k + j, 0)`.
    /// The caller supplies `N` by re-reading it from the `let` type; here the
    /// chain has no length, so this returns a marker the `let`/`for` fold
    /// completes. For the suite's shapes the length is the literal in the
    /// declared type, handled in `parse_let`.
    fn finish_args_collect(&mut self, skip: i64) -> PResult<Expr> {
        // The `let` that this initializes owns the array; record the skip so it
        // can size the desugared loop. `parse_let` rejects the marker anywhere
        // other than an `args`-vector binding.
        self.args_skip = skip;
        Ok(Expr::Call {
            name: "\u{0}argsvec".into(),
            args: Vec::new(),
            arg_slots: Vec::new(),
            ret_slot: -1,
            ret_ty: Type::I64,
        })
    }

    /// Skip a closure argument `|a, b| EXPR`, consuming its tokens without
    /// type-checking the body (rmc only accepts the `s.parse()...` shape).
    fn skip_closure(&mut self) -> PResult<()> {
        if self.peek() != Some(&Tok::Pipe) {
            return Err(format!("at byte {}: expected a closure `|..| ..`", self.pos()));
        }
        self.i += 1; // the opening `|`
        // Parameters up to the closing `|`.
        while self.peek() != Some(&Tok::Pipe) {
            match self.peek() {
                Some(Tok::Ident(_)) | Some(Tok::Comma) | Some(Tok::Underscore) => self.i += 1,
                _ => {
                    return Err(format!(
                        "at byte {}: unsupported closure parameter in `std::env::args()` chain",
                        self.pos()
                    ))
                }
            }
        }
        self.i += 1; // the closing `|`
        // The body: `s.parse::<i64>().unwrap()` or `s.parse().ok()`. Walk to the
        // end of the closure's expression (the enclosing `)` or `,`).
        let mut depth = 0i32;
        loop {
            match self.peek() {
                None => break,
                Some(Tok::LParen) => {
                    depth += 1;
                    self.i += 1;
                }
                Some(Tok::RParen) => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    self.i += 1;
                }
                Some(Tok::Comma) if depth == 0 => break,
                _ => self.i += 1,
            }
        }
        Ok(())
    }

    /// `__rmc_arg_or(idx, default)`.
    fn args_or(&self, idx: i64, default: Expr) -> Expr {
        Expr::Call {
            name: "__rmc_arg_or".into(),
            args: vec![Expr::Int(idx), default],
            arg_slots: Vec::new(),
            ret_slot: -1,
            ret_ty: Type::I64,
        }
    }
    /// `println!("...")` / `print!("...")`: format the literal and its `{}`
    /// arguments, then print. `println!` appends a newline. The format string
    /// must be a literal (a variable one is not supported), and a `{}` is filled
    /// by the next argument in order; `{{`/`}}` are literal braces, and `\n` and
    /// the other escapes are already resolved by the lexer. Each printed piece
    /// is a `__rmc_print` call, which sema rewrites to the runtime helper that
    /// matches the value's type (`__rmc_write_i64`, `__rmc_write_bool` or
    /// `__rmc_write_str`); none of those appends a newline, so `println!` adds
    /// the one trailing newline itself and `print!` adds none.
    fn parse_macro(&mut self, name: &str) -> PResult<Expr> {
        self.i += 1; // the `!`
        let newline = name == "println";
        if !self.eat(&Tok::LParen) {
            return Err(format!("at byte {}: `{name}!` expects `(`", self.pos()));
        }
        // The format string must be a literal. `println!()` with no format at
        // all is allowed and prints just a newline, as in rustc.
        let fmt = if self.peek() == Some(&Tok::RParen) {
            String::new()
        } else {
            match self.peek().cloned() {
                Some(Tok::Str(s)) => {
                    self.i += 1;
                    s
                }
                other => {
                    return Err(format!(
                        "at byte {}: `{name}!` expects a string literal, found {}",
                        self.pos(),
                        other.map(|t| t.describe()).unwrap_or_else(|| "end of input".into())
                    ))
                }
            }
        };
        let mut args: Vec<Expr> = Vec::new();
        // The format string is the first element of a comma-separated list, so
        // arguments (if any) follow a leading comma: `println!("x{}", a)`.
        if self.eat(&Tok::Comma) {
            while self.peek() != Some(&Tok::RParen) {
                args.push(self.parse_bracketed_expr()?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen)?;
        self.desugar_macro(name, newline, &fmt, args)
    }

    /// `vec![e; n]` (repeat form) and `vec![a, b, c]` (list form). rmc has no
    /// `Vec`; each lowers to a fresh heap allocation holding the elements, with
    /// the value being the block address (an `i64`). The repeat form's `n` and
    /// the list form's length are recorded so a later `.len()` folds. The
    /// element count must be a constant for the size to be known at compile
    /// time; a non-constant `n` is rejected.
    fn parse_vec_macro(&mut self) -> PResult<Expr> {
        self.i += 1; // the `!`
        self.expect(Tok::LBracket)?;
        // Empty `vec![]` is a zero-length allocation.
        if self.eat(&Tok::RBracket) {
            return Ok(Expr::Call {
                name: "\u{0}vecrepeat".into(),
                args: vec![Expr::Int(0), Expr::Int(0)],
                arg_slots: Vec::new(),
                ret_slot: -1,
                ret_ty: Type::I64,
            });
        }
        let first = self.parse_bracketed_expr()?;
        // `vec![e; n]`: the `;` separates the element from the count.
        if self.eat(&Tok::Semi) {
            let n = self.parse_bracketed_expr()?;
            self.expect(Tok::RBracket)?;
            return Ok(Expr::Call {
                name: "\u{0}vecrepeat".into(),
                args: vec![first, n],
                arg_slots: Vec::new(),
                ret_slot: -1,
                ret_ty: Type::I64,
            });
        }
        // `vec![a, b, c]`: the elements are stored in order.
        let mut elems = vec![first];
        while self.eat(&Tok::Comma) {
            if self.peek() == Some(&Tok::RBracket) {
                break;
            }
            elems.push(self.parse_bracketed_expr()?);
        }
        self.expect(Tok::RBracket)?;
        Ok(Expr::Call {
            name: "\u{0}veclist".into(),
            args: elems,
            arg_slots: Vec::new(),
            ret_slot: -1,
            ret_ty: Type::I64,
        })
    }

    /// Build the block for `println!`/`print!` from the parsed format and
    /// arguments. See `parse_macro` for the contract.
    fn desugar_macro(
        &mut self,
        name: &str,
        newline: bool,
        fmt: &str,
        args: Vec<Expr>,
    ) -> PResult<Expr> {
        let bytes = fmt.as_bytes();
        let mut stmts: Vec<Stmt> = Vec::new();
        let mut arg_i = 0usize;
        let mut lit: Vec<u8> = Vec::new();
        // Rust evaluates every format argument before the first byte of output.
        // The body below interleaves literal chunks with `{}` writes, so the
        // raw arguments are bound to fresh locals first and the writes read
        // those; otherwise `println!("A{}B{}", f(), g())` would print `A`, then
        // run `f`, then print `f`'s value. Each local is `_`-prefixed and
        // untyped so sema infers the value's type and `__rmc_print` still picks
        // the matching writer.
        let mut bound: Vec<Expr> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let name = format!("\u{0}fmt{i}");
            stmts.push(Stmt::Let {
                name: name.clone(),
                ty: None,
                init: Some(arg.clone()),
                slot: -1,
                mutable: false,
            });
            bound.push(Expr::Var { name, slot: -1, ty: Type::I64 });
        }
        let args = bound;
        let flush = |lit: &mut Vec<u8>, stmts: &mut Vec<Stmt>| {
            if !lit.is_empty() {
                let s = String::from_utf8(lit.clone()).unwrap_or_default();
                lit.clear();
                stmts.push(Stmt::Expr(Expr::Call {
                    name: "__rmc_print".into(),
                    args: vec![Expr::Str(s)],
                    arg_slots: vec![(-1, 0)],
                    ret_slot: -1,
                    ret_ty: Type::Unit,
                }));
            }
        };
        let mut k = 0usize;
        while k < bytes.len() {
            match bytes[k] {
                b'{' if bytes.get(k + 1) == Some(&b'{') => {
                    lit.push(b'{');
                    k += 2;
                }
                b'}' if bytes.get(k + 1) == Some(&b'}') => {
                    lit.push(b'}');
                    k += 2;
                }
                b'{' if bytes.get(k + 1) == Some(&b'}') => {
                    flush(&mut lit, &mut stmts);
                    let arg = args.get(arg_i).cloned().ok_or_else(|| {
                        format!("at byte {}: `{name}!` has more `{{}}` than arguments", self.pos())
                    })?;
                    arg_i += 1;
                    stmts.push(Stmt::Expr(Expr::Call {
                        name: "__rmc_print".into(),
                        args: vec![arg],
                        arg_slots: vec![(-1, 0)],
                        ret_slot: -1,
                        ret_ty: Type::Unit,
                    }));
                    k += 2;
                }
                b'{' | b'}' => {
                    return Err(format!(
                        "at byte {}: `{name}!` supports only `{{}}` in the format string",
                        self.pos()
                    ));
                }
                other => {
                    lit.push(other);
                    k += 1;
                }
            }
        }
        flush(&mut lit, &mut stmts);
        if arg_i != args.len() {
            return Err(format!(
                "at byte {}: `{name}!` has {} argument(s) but {} `{{}}`",
                self.pos(),
                args.len(),
                arg_i
            ));
        }
        if newline {
            stmts.push(Stmt::Expr(Expr::Call {
                name: "putchar".into(),
                args: vec![Expr::Int(10)],
                arg_slots: vec![(-1, 0)],
                ret_slot: -1,
                ret_ty: Type::Unit,
            }));
        }
        Ok(Expr::Block(Block { stmts, tail: None }))
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
        let lhs = self.parse_bitor()?;
        let op = match self.peek() {
            Some(Tok::EqEq) => BinOp::Eq,
            Some(Tok::Ne) => BinOp::Ne,
            Some(Tok::Lt) => BinOp::Lt,
            Some(Tok::Le) => BinOp::Le,
            Some(Tok::Gt) => BinOp::Gt,
            Some(Tok::Ge) => BinOp::Ge,
            _ => return Ok(lhs),
        };
        self.i += 1;
        let rhs = self.parse_bitor()?;
        self.bump_depth()?;
        // Comparison is non-associative in Rust: `a < b < c` is a hard error
        // ("comparison operators cannot be chained"), not `(a < b) < c`. Reject
        // a second comparison operator here so rmc does not accept what rustc
        // rejects.
        if matches!(
            self.peek(),
            Some(Tok::EqEq | Tok::Ne | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge)
        ) {
            return Err(format!(
                "at byte {}: comparison operators cannot be chained",
                self.pos()
            ));
        }
        Ok(Expr::Bin(op, Box::new(lhs), Box::new(rhs)))
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
                // `*v` where `v` is a `for v in &args` loop variable: Rust types
                // `v` as `&i64`, but rmc's desugar binds it as the `i64` value,
                // so the deref is a copy of `v` and is dropped.
                if let Expr::Var { name, .. } = &e {
                    if self.args_elems.contains(name) {
                        return Ok(e);
                    }
                }
                Ok(Expr::Deref { expr: Box::new(e), ty: Type::Named(String::new()) })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            self.bump_depth()?;
            // `args[i]` on an `args()`-vector binding lowers to
            // `__rmc_arg_or(i + skip, 0)`: no vector is materialised, and the
            // index must be a constant, exactly as the direct `args().nth(i)`
            // form requires.
            if let Expr::Var { name, .. } = &e {
                if let Some(&skip) = self.args_vecs.get(name) {
                    if self.peek() == Some(&Tok::LBracket) {
                        self.i += 1;
                        let idx = self.parse_bracketed_expr()?;
                        self.expect(Tok::RBracket)?;
                        let k = eval_i64(&idx, &self.consts).ok_or_else(|| {
                            format!(
                                "at byte {}: indexing an `args()` vector needs a constant index",
                                self.pos()
                            )
                        })?;
                        e = self.args_or(k + skip, Expr::Int(0));
                        continue;
                    }
                }
            }
            // `std::env::args()` lowers to a hidden marker; the iterator-method
            // chain that follows folds to an `i64` (`__rmc_arg_or`). A chain rmc
            // cannot lower is rejected there rather than mis-compiled.
            if let Expr::Call { name, .. } = &e {
                if name == "\u{0}args" {
                    let chain = std::mem::replace(&mut e, Expr::Int(0));
                    e = self.finish_args_chain(chain)?;
                    continue;
                }
            }
            if self.eat(&Tok::LBracket) {
                let idx = self.parse_bracketed_expr()?;
                self.expect(Tok::RBracket)?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.eat(&Tok::Dot) {
                // A tuple field is written `.0`, `.1`, ...: the "name" is a
                // decimal literal, which becomes the field name `"k"` of the
                // anonymous struct a tuple lowers to. A named field keeps its
                // identifier.
                let name = match self.peek().cloned() {
                    Some(Tok::Int(n)) if n >= 0 => {
                        self.i += 1;
                        n.to_string()
                    }
                    _ => self.expect_ident()?,
                };
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while self.peek() != Some(&Tok::RParen) {
                        args.push(self.parse_bracketed_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    // Methods on a `Vec`/raw-pointer binding that rmc lowers to
                    // plain arithmetic. The receiver is an `i64` (a `Vec` is its
                    // backing address), so `capacity`/`len` fold to the constant
                    // recorded at the binding, `as_mut_ptr`/`as_ptr` keep the
                    // address (marked so a later `.add` folds), and `.add(off)`
                    // is `ptr + off`.
                    if let Expr::Var { name: recv, .. } = &e {
                        if args.is_empty() && name == "capacity" && self.vec_cap.contains_key(recv)
                        {
                            e = Expr::Int(self.vec_cap[recv]);
                            continue;
                        }
                        if args.is_empty() && name == "len" && self.vec_len.contains_key(recv) {
                            e = Expr::Int(self.vec_len[recv]);
                            continue;
                        }
                        if args.is_empty()
                            && (name == "as_mut_ptr" || name == "as_ptr")
                            && self.vec_names.contains(recv)
                        {
                            let recv_expr = e.clone();
                            e = Expr::Call {
                                name: "\u{0}ptr".into(),
                                args: vec![recv_expr],
                                arg_slots: Vec::new(),
                                ret_slot: -1,
                                ret_ty: Type::I64,
                            };
                            continue;
                        }
                        if name == "add" && args.len() == 1 && self.ptr_vars.contains_key(recv) {
                            let stride = self.ptr_vars[recv];
                            let recv_expr = e.clone();
                            let mut off = args.pop().unwrap();
                            // `add(off)` advances by `off` whole elements, so
                            // scale by the element width. A byte-strided
                            // pointer (`Vec<u8>`) keeps the offset untouched.
                            if stride != 1 {
                                off = Expr::Bin(
                                    BinOp::Mul,
                                    Box::new(off),
                                    Box::new(Expr::Int(stride)),
                                );
                            }
                            e = Expr::Bin(BinOp::Add, Box::new(recv_expr), Box::new(off));
                            continue;
                        }
                    }
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

    /// One pattern in a *refutable* position (a `match` arm or an `if let` /
    /// `while let` scrutinee): the same as `parse_pattern`, but a tuple element
    /// may be a scalar test.
    fn parse_refutable_pattern(&mut self) -> PResult<Pat> {
        let saved = self.pat_refutable_ok;
        self.pat_refutable_ok = true;
        let r = self.parse_pattern();
        self.pat_refutable_ok = saved;
        r
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
        // an or-pattern outright rather than mis-bind it. A struct pattern may
        // appear as an irrefutable alternative only when it binds nothing.
        if let Some(n) = alts.iter().find_map(Self::first_bound_name) {
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
            // `mut name @ subpat` / `mut name`: a mutable binding. Only a
            // refutable position (a `match` arm or an `if let` / `while let`
            // scrutinee) writes `mut` here; a `let` or `for` writes it before
            // the whole pattern. Represented as `name @ subpat` with the
            // binding flag set, `_` standing in for the absent sub-pattern.
            Some(Tok::Mut) if self.pat_refutable_ok => {
                self.i += 1;
                let n = self.expect_ident()?;
                let sub = if self.eat(&Tok::At) {
                    self.parse_pattern_primary()?
                } else {
                    Pat::Wild
                };
                Ok(Pat::At(n, true, Box::new(sub)))
            }
            // `(a, b)`: a tuple pattern. `(` cannot start a binding, a constant
            // or a literal pattern, so it is unambiguous here.
            Some(Tok::LParen) => self.parse_tuple_pat(),
            // `[a, b, .., c]`: an array pattern. `[` cannot start a binding or a
            // constant, so it is unambiguous here.
            Some(Tok::LBracket) => self.parse_array_pat(),
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
            Some(Tok::Char(c)) => {
                self.i += 1;
                self.maybe_char_range(c)
            }
            // `name @ subpat`: bind the whole value and also match it against
            // the sub-pattern. `@` cannot begin any other pattern, so it is
            // unambiguous here; a `mut` may precede the name.
            Some(Tok::Ident(n)) if self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::At) => {
                // `n @ subpat` is a binding, so it may not shadow a constant
                // (rustc E0530); sema covers a top-level one, the parser a
                // block-scoped one.
                self.reject_const_shadow(std::slice::from_ref(&n))?;
                self.i += 2;
                let sub = self.parse_pattern_primary()?;
                Ok(Pat::At(n, false, Box::new(sub)))
            }
            // A bare name that is a constant is that constant's value, a name
            // that is a struct starts a struct pattern, and anything else binds
            // the scrutinee; `_` already matched the wildcard case above.
            Some(Tok::Ident(n)) => {
                self.i += 1;
                if self.eat(&Tok::ColonColon) {
                    let variant = self.expect_ident()?;
                    let mut args = None;
                    if self.eat(&Tok::LParen) {
                        args = Some(self.parse_variant_pat_args()?);
                    }
                    Ok(Pat::Variant(n, variant, args))
                } else if self.peek() == Some(&Tok::LBrace) {
                    // `S { f, g: h, i: _, .. }`: a struct pattern. Fields may be
                    // written in any order, and each is either a shorthand
                    // binding (`f`), an explicit binding (`g: h`) or a wildcard
                    // (`i: _`); a trailing `..` ignores the fields not named.
                    // The field list comes from the prescan, so a struct
                    // declared later in the file still matches, as rustc allows.
                    self.i += 1;
                    // A name that is not a struct cannot be destructured; the
                    // prescan saw every struct, so an unknown one is an error
                    // here rather than a field read on the wrong type later.
                    let known = match self.struct_fields.get(&n) {
                        Some(f) => f.clone(),
                        None => {
                            return Err(format!(
                                "at byte {}: unknown struct `{n}` in a pattern",
                                self.pos()
                            ))
                        }
                    };
                    let fields = self.parse_struct_pat_fields(&n, &known)?;
                    Ok(Pat::Struct(n, fields))
                } else if let Some(v) = self.consts.get(&n) {
                    // A bare name that names a constant is that constant's
                    // value, not a fresh binding, exactly as rustc reads it.
                    Ok(Pat::Int(*v))
                } else if let Some(b) = self.const_bools.get(&n) {
                    Ok(Pat::Bool(*b))
                } else if let Some(c) = self.const_chars.get(&n) {
                    Ok(Pat::Char(*c))
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

    /// The argument list of a payload variant pattern, `(v, _, mut w, [a, b],
    /// S { x }, ..)`, with the opening `(` already consumed and the closing `)`
    /// consumed here. Each argument is `(mut, pat)`; `mut` is only legal on a
    /// binding. A trailing `..` ignores the remaining fields (the returned
    /// flag). A bare identifier is a fresh binding, exactly as in a struct field
    /// or an array element. A nested *variant* pattern (`E::A(Inner::X(v))`) is
    /// refutable and so rejected by the irrefutability check below: rmc
    /// generates no per-field test.
    fn parse_variant_pat_args(&mut self) -> PResult<(Vec<(bool, Pat)>, bool)> {
        let mut args: Vec<(bool, Pat)> = Vec::new();
        let mut rest = false;
        while self.peek() != Some(&Tok::RParen) {
            if self.peek().is_none() {
                return Err(format!(
                    "at byte {}: unexpected end of input in variant pattern",
                    self.pos()
                ));
            }
            if self.eat(&Tok::DotDot) {
                rest = true;
                // A trailing comma after `..` is allowed, as in rustc; anything
                // else after it is left for the `)` check below to reject.
                self.eat(&Tok::Comma);
                break;
            }
            let m = self.eat(&Tok::Mut);
            // `_` ignores the field: it binds nothing and tests nothing.
            let p = if self.peek() == Some(&Tok::Underscore) {
                self.i += 1;
                Pat::Wild
            } else {
                self.parse_pattern_primary()?
            };
            if m && !matches!(p, Pat::Bind(_) | Pat::At(..)) {
                return Err(format!(
                    "at byte {}: `mut` is not allowed here; it attaches to a binding",
                    self.pos()
                ));
            }
            args.push((m, p));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        // Every argument must be irrefutable: a refutable one (`E::V(1)`) would
        // need a per-field test, which rmc does not generate. Reject it here
        // rather than mis-bind it as an always-true read.
        if args.iter().any(|(_, p)| !Self::pat_irrefutable(p)) {
            return Err(format!(
                "at byte {}: variant pattern arguments must be bindings, `_`, or nested patterns",
                self.pos()
            ));
        }
        Ok((args, rest))
    }

    /// The field list of a struct pattern, `{ f, g: h, i: _, .. }`, with the
    /// opening `{` already consumed and the closing `}` consumed here. `n` is
    /// the struct name (for error messages) and `known` its field set, from the
    /// prescan. A field's value may be a binding (`f`, `f: b`, `f: mut b`) or a
    /// nested struct pattern (`f: I { a }`); the latter is stored as a
    /// `Pat::Struct` with `mut` cleared, since `mut` is only legal on a binding.
    fn parse_struct_pat_fields(
        &mut self,
        n: &str,
        known: &std::collections::HashSet<String>,
    ) -> PResult<Vec<FieldPat>> {
        let mut fields: Vec<FieldPat> = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err(format!(
                    "at byte {}: unexpected end of input in struct pattern",
                    self.pos()
                ));
            }
            // `..` ignores the remaining fields; nothing may follow it.
            if self.eat(&Tok::DotDot) {
                break;
            }
            // `mut` may precede a shorthand field (`S { mut f }`), but not the
            // field name of a renamed one.
            let leading_mut = self.eat(&Tok::Mut);
            let fname = self.expect_ident()?;
            // A field the struct does not have is an error here, not a silent
            // no-op: a `f: _` field emits no read for sema to catch, and a wrong
            // name is a typo either way.
            if !known.contains(&fname) {
                return Err(format!(
                    "at byte {}: struct `{n}` has no field `{fname}`",
                    self.pos()
                ));
            }
            if fields.iter().any(|(f, _)| *f == fname) {
                return Err(format!(
                    "at byte {}: field `{fname}` bound twice in struct pattern",
                    self.pos()
                ));
            }
            let bind = if self.eat(&Tok::Colon) {
                // `f: mut b` marks just this binding mutable, as in rustc. A
                // leading `mut` before the field name is only legal on a
                // shorthand field.
                if leading_mut {
                    return Err(format!(
                        "at byte {}: `mut` must be attached to the binding, as `{fname}: mut b`",
                        self.pos()
                    ));
                }
                let m = self.eat(&Tok::Mut);
                if self.peek() == Some(&Tok::Underscore) {
                    self.i += 1;
                    if m {
                        return Err(format!(
                            "at byte {}: `mut` is not allowed on `_`",
                            self.pos()
                        ));
                    }
                    None
                } else if let Some(inner) = self.parse_nested_struct_pat()? {
                    // `f: I { a }` — a nested struct pattern, so `mut` (which
                    // only applies to a binding) is rejected, as rustc rejects
                    // `f: mut I { a }`.
                    if m {
                        return Err(format!(
                            "at byte {}: `mut` is not allowed on a nested pattern",
                            self.pos()
                        ));
                    }
                    Some((false, inner))
                } else if self.peek() == Some(&Tok::LBracket) {
                    // `f: [x, y]` — a nested array pattern for an array-typed
                    // field; `mut` is likewise rejected.
                    if m {
                        return Err(format!(
                            "at byte {}: `mut` is not allowed on a nested pattern",
                            self.pos()
                        ));
                    }
                    Some((false, self.parse_array_pat()?))
                } else if matches!(self.peek(), Some(Tok::Ident(_)))
                    && self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::At)
                {
                    // `f: n @ subpat` binds the field to `n` and also
                    // destructures it. The sub-pattern must be irrefutable,
                    // since rmc generates no per-field test.
                    let name = self.expect_ident()?;
                    self.reject_const_shadow(std::slice::from_ref(&name))?;
                    self.i += 1; // the `@`
                    let sub = self.parse_pattern_primary()?;
                    if !Self::pat_irrefutable(&sub) {
                        return Err(format!(
                            "at byte {}: a struct field sub-pattern must be irrefutable",
                            self.pos()
                        ));
                    }
                    Some((m, Pat::At(name, m, Box::new(sub))))
                } else {
                    let name = self.expect_ident()?;
                    self.reject_const_shadow(std::slice::from_ref(&name))?;
                    Some((m, Pat::Bind(name)))
                }
            } else {
                // Shorthand: `S { f }` binds `f`, or `S { mut f }` binds a
                // mutable `f`. The binding may not shadow a constant (E0530).
                self.reject_const_shadow(std::slice::from_ref(&fname))?;
                Some((leading_mut, Pat::Bind(fname.clone())))
            };
            fields.push((fname, bind));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        Ok(fields)
    }

    /// A nested struct pattern after a field name and `:`, i.e. `I { .. }`. The
    /// struct name must be in the prescan, as at the top level. Returns `None`
    /// when the next token is not an identifier followed by `{`, so the caller
    /// falls through to a plain binding (or a rename).
    fn parse_nested_struct_pat(&mut self) -> PResult<Option<Pat>> {
        let name = match self.peek().cloned() {
            Some(Tok::Ident(name)) => name,
            _ => return Ok(None),
        };
        // `Ident {` starts a nested pattern; anything else (`Ident`, `Ident,`,
        // `Ident }`) is a binding.
        if self.toks.get(self.i + 1).map(|t| &t.tok) != Some(&Tok::LBrace) {
            return Ok(None);
        }
        let known = match self.struct_fields.get(&name) {
            Some(f) => f.clone(),
            None => {
                return Err(format!(
                    "at byte {}: unknown struct `{name}` in a pattern",
                    self.pos()
                ))
            }
        };
        self.i += 1; // the struct name
        self.i += 1; // the `{`
        let fields = self.parse_struct_pat_fields(&name, &known)?;
        Ok(Some(Pat::Struct(name, fields)))
    }

    /// `[a, b, .., c]`: an array pattern, with the opening `[` already peeked.
    /// Each element is a sub-pattern; a single `..` matches any number of middle
    /// elements and records how many elements precede it in `front`. rustc
    /// allows `..` at most once per pattern, so a second one is rejected.
    fn parse_array_pat(&mut self) -> PResult<Pat> {
        self.i += 1; // the `[`
        let mut elems: Vec<Option<(bool, Pat)>> = Vec::new();
        let mut rest = false;
        let mut front = 0usize;
        while self.peek() != Some(&Tok::RBracket) {
            if self.peek().is_none() {
                return Err(format!(
                    "at byte {}: unexpected end of input in array pattern",
                    self.pos()
                ));
            }
            if self.eat(&Tok::DotDot) {
                if rest {
                    return Err(format!(
                        "at byte {}: `..` can only be used once per array pattern",
                        self.pos()
                    ));
                }
                rest = true;
                front = elems.len();
                if !self.eat(&Tok::Comma) {
                    break;
                }
                continue;
            }
            // `mut` may precede an element binding (`[mut a, b]`), as in rustc.
            let m = self.eat(&Tok::Mut);
            let p = self.parse_pattern_primary()?;
            if m && !matches!(p, Pat::Bind(_) | Pat::At(..)) {
                return Err(format!(
                    "at byte {}: `mut` is not allowed here; it attaches to a binding",
                    self.pos()
                ));
            }
            elems.push(Some((m, p)));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RBracket)?;
        // Every element must be irrefutable: a refutable one (`[1, x]`) would
        // need a per-element test, which rmc does not generate. Reject it here
        // rather than mis-bind it as an always-true read.
        if elems.iter().flatten().any(|(_, p)| !Self::pat_irrefutable(p)) {
            return Err(format!(
                "at byte {}: array pattern elements must be bindings, `_`, or nested patterns",
                self.pos()
            ));
        }
        // `elems` stays in source order; `front` separates the elements before
        // `..` from those after it, and `array_pat_binds` reads the tail
        // relative to the array's length.
        Ok(Pat::Array { elems, rest, front })
    }

    /// `(a, b)`: a tuple pattern, with the opening `(` already peeked. `()` is
    /// the unit pattern (binds nothing); `(a)` is a parenthesised sub-pattern,
    /// not a one-tuple, matching rustc's rule that a one-tuple needs a trailing
    /// comma `(a,)`. Every element must be irrefutable, since a tuple pattern has
    /// no test.
    fn parse_tuple_pat(&mut self) -> PResult<Pat> {
        self.i += 1; // the `(`
        if self.eat(&Tok::RParen) {
            return Ok(Pat::Tuple(Vec::new()));
        }
        let m = self.eat(&Tok::Mut);
        // An element may be an or-pattern (`(0 | 1, x)`), which `parse_pattern`
        // collects; a lone primary is returned unchanged.
        let first = self.parse_pattern()?;
        if m && !matches!(first, Pat::Bind(_) | Pat::At(..)) {
            return Err(format!(
                "at byte {}: `mut` is not allowed here; it attaches to a binding",
                self.pos()
            ));
        }
        // `(a)`: a parenthesised pattern, not a tuple.
        if !self.eat(&Tok::Comma) {
            self.expect(Tok::RParen)?;
            return Ok(first);
        }
        let mut elems = vec![(m, first)];
        while self.peek() != Some(&Tok::RParen) {
            if self.peek().is_none() {
                return Err(format!(
                    "at byte {}: unexpected end of input in tuple pattern",
                    self.pos()
                ));
            }
            let m = self.eat(&Tok::Mut);
            let p = self.parse_pattern()?;
            if m && !matches!(p, Pat::Bind(_) | Pat::At(..)) {
                return Err(format!(
                    "at byte {}: `mut` is not allowed here; it attaches to a binding",
                    self.pos()
                ));
            }
            elems.push((m, p));
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        // A tuple element must be irrefutable in a `let`, `for` head or
        // parameter, where the pattern has no test; in a `match` arm or an
        // `if let` / `while let` scrutinee it may instead be a scalar test
        // (an integer, bool, string or range), which rmc generates. Anything
        // else (a variant pattern, which would bind its payload through the
        // test) is rejected in both.
        for (_, p) in &elems {
            if !Self::pat_irrefutable(p) && !self.pat_refutable_ok {
                return Err(format!(
                    "at byte {}: tuple pattern elements must be bindings, `_`, or nested patterns",
                    self.pos()
                ));
            }
            if !Self::tuple_element_ok(p) {
                return Err(format!(
                    "at byte {}: a tuple pattern element must be irrefutable or a scalar test (an integer, `true`/`false`, a string or a range)",
                    self.pos()
                ));
            }
        }
        Ok(Pat::Tuple(elems))
    }

    /// The `let` statements a tuple pattern introduces: one per bound element,
    /// each a read of the element's field (`"k"` on the anonymous struct a tuple
    /// lowers to). Shared by the `let`, `for`, `match` and parameter paths.
    fn tuple_pat_binds(elems: &[(bool, Pat)], subj: &Expr) -> Vec<Stmt> {
        // A tuple pattern must name exactly the scrutinee's fields, so emit a
        // marker for sema to check before the element reads. The field reads
        // below would otherwise let `let (x,) = (1, 2);` silently ignore `1`.
        let mut out = vec![Stmt::Expr(Expr::PatArity {
            base: Box::new(subj.clone()),
            want: elems.len(),
            is_tuple: true,
        })];
        for (k, (m, pat)) in elems.iter().enumerate() {
            let field = Expr::Field {
                base: Box::new(subj.clone()),
                name: k.to_string(),
                offset: -1,
                ty: Type::Unit,
            };
            match pat {
                Pat::Bind(b) => out.push(Stmt::Let {
                    name: b.clone(),
                    ty: None,
                    mutable: *m,
                    init: Some(field),
                    slot: -1,
                }),
                Pat::Wild => {}
                // `name @ subpat`: bind the element to `name`, then bind the
                // sub-pattern's names from the same element read.
                Pat::At(name, am, sub) => {
                    out.push(Stmt::Let {
                        name: name.clone(),
                        ty: None,
                        mutable: *m || *am,
                        init: Some(field.clone()),
                        slot: -1,
                    });
                    Self::pat_binds(sub, false, &field, &mut out);
                }
                Pat::Struct(name, inner) => out.extend(Self::struct_pat_binds(inner, &field, &name)),
                Pat::Tuple(inner) => out.extend(Self::tuple_pat_binds(inner, &field)),
                Pat::Array { elems, rest, front } => {
                    out.extend(Self::array_pat_binds(elems, *rest, *front, &field))
                }
                // A scalar element (`(1, x)`) binds nothing here: its test is
                // generated separately by `tuple_pat_test`. An or-pattern element
                // is allowed in an irrefutable context only when a `_`
                // alternative makes it irrefutable, so it too binds nothing.
                Pat::Int(_) | Pat::Char(_) | Pat::CharRange(..) | Pat::Bool(_) | Pat::Str(_) | Pat::Range(..) | Pat::Or(_) => {}
                _ => unreachable!("a tuple element is validated when the tuple is parsed"),
            }
        }
        out
    }

    /// The `let` statements an array pattern introduces: one per bound element,
    /// each an index read of `subj` at that element's position. Shared by the
    /// `let`, `for` and `match` paths. A `..` rest shifts the elements after it
    /// to the back of the array: with `front` elements ahead and `k` behind, an
    /// element at pattern index `j` reads array index `j` (j < front) or
    /// `len - (k - (j - front))` (j >= front), where `len` is the array's length.
    fn array_pat_binds(
        elems: &[Option<(bool, Pat)>],
        rest: bool,
        front: usize,
        subj: &Expr,
    ) -> Vec<Stmt> {
        // An array pattern must name exactly the array's length (with no `..`
        // rest), so emit a marker for sema to check before the index reads.
        // Without it `let [x] = [1, 2];` would read `[0]` and ignore the rest.
        // A `..` rest matches any count, so it has nothing to check.
        let mut out = if rest {
            Vec::new()
        } else {
            vec![Stmt::Expr(Expr::PatArity {
                base: Box::new(subj.clone()),
                want: elems.len(),
                is_tuple: false,
            })]
        };
        for (j, elem) in elems.iter().enumerate() {
            let (m, pat) = match elem {
                Some(x) => x,
                None => continue,
            };
            let idx = if !rest || j < front {
                Expr::Int(j as i64)
            } else {
                let behind = elems.len() - front;
                let from_end = behind - (j - front);
                let len = Expr::MethodCall {
                    recv: Box::new(subj.clone()),
                    name: "len".to_string(),
                    args: Vec::new(),
                };
                Expr::Bin(
                    BinOp::Sub,
                    Box::new(len),
                    Box::new(Expr::Int(from_end as i64)),
                )
            };
            let index = Expr::Index(Box::new(subj.clone()), Box::new(idx));
            match pat {
                // `_` matches an element without binding it: no read.
                Pat::Wild => {}
                Pat::Bind(name) => out.push(Stmt::Let {
                    name: name.clone(),
                    ty: None,
                    mutable: *m,
                    init: Some(index),
                    slot: -1,
                }),
                // `name @ subpat`: bind the element to `name`, then bind the
                // sub-pattern's names from the same index read.
                Pat::At(name, am, sub) => {
                    out.push(Stmt::Let {
                        name: name.clone(),
                        ty: None,
                        mutable: *m || *am,
                        init: Some(index.clone()),
                        slot: -1,
                    });
                    Self::pat_binds(sub, false, &index, &mut out);
                }
                Pat::Array { elems, rest, front } => {
                    out.extend(Self::array_pat_binds(elems, *rest, *front, &index))
                }
                // A struct or tuple sub-pattern inside an array element.
                Pat::Struct(name, fields) => out.extend(Self::struct_pat_binds(fields, &index, &name)),
                Pat::Tuple(elems) => out.extend(Self::tuple_pat_binds(elems, &index)),
                _ => unreachable!("unexpected array sub-pattern"),
            }
        }
        out
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
            let pat = self.parse_refutable_pattern()?;
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
        self.desugar_match(scrutinee, arms)
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

    /// If a `..`/`..=` follows a `char` literal just parsed, consume it and the
    /// upper bound to build a `char` range pattern; otherwise the literal stands
    /// alone. Both bounds are `char`, and the upper bound must be a `char`
    /// literal, as in rustc (which also rejects an empty range).
    fn maybe_char_range(&mut self, lo: u32) -> PResult<Pat> {
        let inclusive = match self.peek() {
            Some(Tok::DotDot) => false,
            Some(Tok::DotDotEq) => true,
            _ => return Ok(Pat::Char(lo)),
        };
        self.i += 1;
        let hi = match self.peek() {
            Some(Tok::Char(c)) => {
                let c = *c;
                self.i += 1;
                c
            }
            _ => {
                return Err(format!(
                    "at byte {}: expected a `char` literal as the range pattern's upper bound",
                    self.pos()
                ))
            }
        };
        if !inclusive && lo >= hi {
            return Err(format!(
                "at byte {}: empty range pattern `'{}'..'{}'`",
                self.pos(),
                char::from_u32(lo).unwrap_or('\u{FFFD}'),
                char::from_u32(hi).unwrap_or('\u{FFFD}')
            ));
        }
        if inclusive && lo > hi {
            return Err(format!(
                "at byte {}: empty range pattern `'{}'..='{}'`",
                self.pos(),
                char::from_u32(lo).unwrap_or('\u{FFFD}'),
                char::from_u32(hi).unwrap_or('\u{FFFD}')
            ));
        }
        Ok(Pat::CharRange(lo, hi, inclusive))
    }

    /// The boolean test for one tuple *element* pattern, if it has one. A
    /// binding, `_`, a struct pattern and a nested irrefutable tuple are
    /// test-free (`None`); a scalar pattern reuses the same test the top-level
    /// `match` builds. A variant pattern is rejected when the tuple is parsed,
    /// so it is not reached here.
    fn pat_test(p: &Pat, subj: &Expr) -> Option<Expr> {
        match p {
            Pat::Wild | Pat::Bind(_) | Pat::Struct(..) => None,
            // `name @ subpat`: the binding itself adds no test; the sub-pattern's
            // test is the whole test.
            Pat::At(_, _, sub) => Self::pat_test(sub, subj),
            Pat::Tuple(elems) => Self::tuple_pat_test(elems, subj),
            // `a | b | c` as an element: any alternative matching passes. A `_`
            // alternative makes the whole element always true, so there is no
            // test; otherwise fold the alternatives' tests with `||`.
            Pat::Or(alts) => {
                if alts.iter().any(|a| matches!(a, Pat::Wild)) {
                    return None;
                }
                let mut cond = Self::or_cond(&alts[0], subj);
                for alt in &alts[1..] {
                    cond = Expr::Bin(BinOp::Or, Box::new(cond), Box::new(Self::or_cond(alt, subj)));
                }
                Some(cond)
            }
            Pat::Int(k) => Some(Expr::Bin(
                BinOp::Eq,
                Box::new(Self::scalar_scrutinee(subj)),
                Box::new(Expr::Int(*k)),
            )),
            Pat::Char(c) => Some(Expr::Bin(
                BinOp::Eq,
                Box::new(Self::scalar_scrutinee(subj)),
                Box::new(Expr::Char(*c)),
            )),
            Pat::Bool(b) => Some(Expr::BoolTest {
                scrutinee: Box::new(Self::scalar_scrutinee(subj)),
                value: *b,
            }),
            Pat::Str(s) => Some(Expr::Bin(
                BinOp::Eq,
                Box::new(subj.clone()),
                Box::new(Expr::Str(s.clone())),
            )),
            Pat::Range(lo, hi, inclusive) => {
                let lower = Expr::Bin(
                    BinOp::Le,
                    Box::new(Expr::Int(*lo)),
                    Box::new(Self::scalar_scrutinee(subj)),
                );
                let upper = Expr::Bin(
                    if *inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Int(*hi)),
                );
                Some(Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper)))
            }
            Pat::CharRange(lo, hi, inclusive) => {
                let lower = Expr::Bin(
                    BinOp::Le,
                    Box::new(Expr::Char(*lo)),
                    Box::new(Self::scalar_scrutinee(subj)),
                );
                let upper = Expr::Bin(
                    if *inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Char(*hi)),
                );
                Some(Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper)))
            }
            _ => unreachable!("a tuple element is validated when the tuple is parsed"),
        }
    }

    /// The boolean test for a tuple pattern: the `&&` of its elements' tests,
    /// each read through the element's field (`"k"` on the anonymous struct a
    /// tuple lowers to). `None` when every element is irrefutable, so the arm is
    /// a catch-all.
    fn tuple_pat_test(elems: &[(bool, Pat)], subj: &Expr) -> Option<Expr> {
        let mut cond: Option<Expr> = None;
        for (k, (_, p)) in elems.iter().enumerate() {
            let field = Expr::Field {
                base: Box::new(subj.clone()),
                name: k.to_string(),
                offset: -1,
                ty: Type::Unit,
            };
            if let Some(t) = Self::pat_test(p, &field) {
                cond = Some(match cond {
                    None => t,
                    Some(c) => Expr::Bin(BinOp::And, Box::new(c), Box::new(t)),
                });
            }
        }
        cond
    }

    /// True for a pattern a tuple element may hold: an irrefutable one, or a
    /// scalar test rmc can generate (an integer, `true`/`false`, a string or a
    /// range). A nested tuple is validated when it is parsed. A variant or an
    /// or-pattern is rejected: rmc would have to bind a payload or merge the
    /// alternatives through the element test.
    fn tuple_element_ok(p: &Pat) -> bool {
        match p {
            Pat::Wild | Pat::Bind(_) | Pat::Struct(..) | Pat::Tuple(_) => true,
            // `name @ subpat` is allowed when its sub-pattern is.
            Pat::At(_, _, sub) => Self::tuple_element_ok(sub),
            Pat::Array { .. } => Self::pat_irrefutable(p),
            Pat::Int(_) | Pat::Char(_) | Pat::CharRange(..) | Pat::Bool(_) | Pat::Str(_) | Pat::Range(..) => true,
            // An or-pattern is allowed when every alternative is a scalar test
            // or a `_` (no alternative binds or is a variant).
            Pat::Or(alts) => alts.iter().all(|a| {
                matches!(a, Pat::Wild | Pat::Int(_) | Pat::Char(_) | Pat::CharRange(..) | Pat::Bool(_) | Pat::Str(_) | Pat::Range(..))
            }),
            Pat::Variant(..) => false,
        }
    }

    /// Whether a pattern matches every value: `_`, a binding, or an
    /// or-pattern with such an alternative.
    fn pat_irrefutable(p: &Pat) -> bool {
        match p {
            // A struct pattern always matches: it binds the named fields and
            // ignores the rest, with no test, so an arm using one is a catch-all.
            // A tuple pattern matches when every element pattern does.
            Pat::Wild | Pat::Bind(_) | Pat::Struct(..) => true,
            // `name @ subpat` is irrefutable exactly when its sub-pattern is.
            Pat::At(_, _, sub) => Self::pat_irrefutable(sub),
            Pat::Tuple(elems) => elems.iter().all(|(_, p)| Self::pat_irrefutable(p)),
            // An array pattern matches when every element pattern does.
            Pat::Array { elems, .. } => elems
                .iter()
                .all(|e| e.as_ref().map_or(true, |(_, p)| Self::pat_irrefutable(p))),
            Pat::Or(alts) => alts.iter().any(Self::pat_irrefutable),
            _ => false,
        }
    }

    /// Fold the arms into nested `if`s. The scrutinee is named once; an arm
    /// with `_` becomes the `else` (so any arm after it is unreachable, as in
    /// Rust). With no `_`, the chain has no final `else`, so a non-matching
    /// value yields `()`.
    fn desugar_match(
        &mut self,
        scrutinee: Expr,
        arms: Vec<(Pat, Option<Expr>, Expr)>,
    ) -> PResult<Expr> {
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
    ) -> PResult<Expr> {
        // A `_` or binding arm makes the match exhaustive in every position;
        // an or-pattern is irrefutable when any alternative is (`1 | _`). A
        // *guarded* irrefutable arm is not a catch-all: `_ if c` can fail, so
        // the arms after it are still reachable.
        let has_catch_all = arms.iter().any(|(p, g, _)| g.is_none() && Self::pat_irrefutable(p));
        // Snapshot the scrutinee into a temporary before the arm chain whenever
        // it names a value that can change. A guard may assign the matched
        // variable (`match x { n if { x = 2; true } => n }`) or one of its
        // fields (`match p { P { x } if { p.x = 9; true } => x }`); the pattern
        // bindings and the body must keep the value captured when the arm
        // matched, so the guard and the body both read this snapshot rather than
        // a binding re-created after the guard ran. Without a guard nothing runs
        // between an arm's test and its body, so a plain variable stays inline
        // (that keeps the pattern lowering identical to the equivalent reads).
        // A non-variable scrutinee is always evaluated once, matching Rust.
        let has_guard = arms.iter().any(|(_, g, _)| g.is_some());
        let (subj, prelude) = match scrutinee {
            Expr::Int(_) | Expr::EnumVariant { .. } => (scrutinee, None),
            Expr::Var { .. } if !has_guard => (scrutinee, None),
            other => {
                let name = format!("\u{0}mt{}", self.tmp);
                self.tmp += 1;
                (
                    Expr::Var { name: name.clone(), slot: -1, ty: Type::I64 },
                    // No annotation: the temp takes the scrutinee's type, which
                    // may be an enum, a struct or a bool rather than an integer.
                    Some(Stmt::Let { name, ty: None, init: Some(other), slot: -1, mutable: false }),
                )
            }
        };
        let mut acc: Option<Expr> = None;
        for (pat, guard, body) in arms.into_iter().rev() {
            acc = Some(self.desugar_arm(pat, guard, &subj, body, acc, check_dead_arms)?);
        }
        let acc = acc.expect("a `match` has at least one arm");
        let acc = Expr::MatchBoundary(
            Block { stmts: Vec::new(), tail: Some(Box::new(acc)) },
            has_catch_all,
        );
        Ok(match prelude {
            None => acc,
            Some(stmt) => Expr::Block(Block { stmts: vec![stmt], tail: Some(Box::new(acc)) }),
        })
    }

    /// `if let PAT = EXPR { then } else { els }` is Rust's sugar for
    /// `match EXPR { PAT => then, _ => els_or_unit }`, so it desugars to a
    /// two-arm `match` and reuses that path. Unlike `match`, an `if let` need
    /// not be exhaustive: a non-matching value falls through to the `_` arm,
    /// which is the `else` block or `()` — so a non-unit `then` with no `else`
    /// is a branch-type error, as in rustc.
    fn desugar_if_let(
        &mut self,
        pat: Pat,
        scrutinee: Expr,
        then: Block,
        els: Option<Block>,
    ) -> PResult<Expr> {
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
    fn desugar_while_let(
        &mut self,
        pat: Pat,
        scrutinee: Expr,
        body: Block,
        label: Option<String>,
    ) -> PResult<Stmt> {
        let break_else = Block {
            stmts: vec![Stmt::Break(None, None)],
            tail: None,
        };
        // The `if let` is a *statement* in the loop body, so its value is
        // discarded and the loop body is unit even when `body` yields a value.
        let test = self.desugar_if_let(pat, scrutinee, body, Some(break_else))?;
        let loop_body = Block { stmts: vec![Stmt::Expr(test)], tail: None };
        // The loop is unit-valued: the `else` arm always contains a plain
        // `break;`, so a `break value` in the body is rejected as a type
        // mismatch (rustc E0571 rejects it too). A label on the `while let`
        // attaches to the desugared `loop`, so `break 'a` / `continue 'a` in
        // the body still target it.
        Ok(Stmt::Expr(Expr::Loop(loop_body, -1, Type::Unit, label)))
    }

    /// The scrutinee of a *scalar* pattern test, wrapped so sema can apply
    /// match ergonomics. rustc derefs a `&T` scrutinee for a literal, range or
    /// bool pattern (`match &x { 5 => .. }`), and the desugared test is a plain
    /// comparison that sema would otherwise reject on a `&i64`. The parser does
    /// not know the scrutinee's type, so it marks the node and `sema` rewrites
    /// it — to a `Deref` for a reference, to the bare expression otherwise.
    fn scalar_scrutinee(subj: &Expr) -> Expr {
        Expr::PatScrutinee(Box::new(subj.clone()))
    }

    /// The boolean test for one alternative of an or-pattern. Only a test, not
    /// an arm: the shared body and fallback stay outside, so an or-pattern does
    /// not duplicate the body. A binding or `_` alternative is always true, but
    /// rmc does not support bindings in an or-pattern, so `_` is the only
    /// irrefutable alternative here.
    fn or_cond(pat: &Pat, subj: &Expr) -> Expr {
        match pat {
            Pat::Int(k) => Expr::Bin(
                BinOp::Eq,
                Box::new(Self::scalar_scrutinee(subj)),
                Box::new(Expr::Int(*k)),
            ),
            Pat::Char(c) => Expr::Bin(
                BinOp::Eq,
                Box::new(Self::scalar_scrutinee(subj)),
                Box::new(Expr::Char(*c)),
            ),
            Pat::Bool(b) => {
                Expr::BoolTest { scrutinee: Box::new(Self::scalar_scrutinee(subj)), value: *b }
            }
            // A string pattern tests by content: `Bin(Eq, ..)` over a `str`,
            // which sema lowers to the byte-wise helper. A pointer compare
            // would never match two distinct-but-equal literals. A `str`
            // scrutinee is the string type itself (rmc has no unsized `str`), so
            // it is not auto-deref'd: `match &s { "hi" => .. }` on a `&str`
            // is rejected, as rustc rejects it for `&&str`.
            Pat::Str(s) => Expr::Bin(
                BinOp::Eq,
                Box::new(subj.clone()),
                Box::new(Expr::Str(s.clone())),
            ),
            Pat::Range(lo, hi, inclusive) => {
                let lower = Expr::Bin(
                    BinOp::Le,
                    Box::new(Expr::Int(*lo)),
                    Box::new(Self::scalar_scrutinee(subj)),
                );
                let upper = Expr::Bin(
                    if *inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Int(*hi)),
                );
                Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper))
            }
            Pat::CharRange(lo, hi, inclusive) => {
                let lower = Expr::Bin(
                    BinOp::Le,
                    Box::new(Expr::Char(*lo)),
                    Box::new(Self::scalar_scrutinee(subj)),
                );
                let upper = Expr::Bin(
                    if *inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Char(*hi)),
                );
                Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper))
            }
            Pat::Variant(enum_name, variant, args) => Expr::VariantTest {
                scrutinee: Box::new(subj.clone()),
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                tag: -1,
                n_binds: args.as_ref().map_or(-1, |(a, _)| a.len() as i32),
                has_rest: args.as_ref().map_or(false, |(_, rest)| *rest),
            },
            // `_` in an or-pattern: always true, so the whole condition is true.
            // A struct pattern is likewise always true; a binding one is
            // rejected before this point. A tuple alternative folds its
            // elements' tests with `&&`, or is `true` when it has none.
            Pat::Wild | Pat::Struct(..) => Expr::Bool(true),
            // `n @ subpat` as an alternative: its test is the sub-pattern's
            // (the binding is emitted separately, and an or-alternative binds
            // nothing, so `n` is rejected here like any other binding).
            Pat::At(_, _, sub) => Self::or_cond(sub, subj),
            Pat::Tuple(elems) => Self::tuple_pat_test(elems, subj).unwrap_or(Expr::Bool(true)),
            // A binding inside an or-pattern is rejected before this point; an
            // array pattern is not supported as an or-alternative.
            Pat::Bind(_) | Pat::Or(_) | Pat::Array { .. } => {
                unreachable!("or-pattern alternative")
            }
        }
    }

    /// The statements a `let` struct destructuring expands to: a hidden
    /// scrutinee binding (unless the initialiser is already a plain place, as in
    /// `desugar_match_arms`, so its re-read has no side effects and no copy is
    /// made) followed by one field read per binding. The statements are spliced
    /// into the enclosing block by the caller, so the bindings stay visible for
    /// the rest of it. `ty` is the optional annotation, kept on the temporary so
    /// it is still checked.
    fn let_destructure_stmts(
        &mut self,
        name: String,
        fields: Vec<FieldPat>,
        ty: Option<Type>,
        init: Expr,
    ) -> Vec<Stmt> {
        let mut names = Vec::new();
        for (_, b) in &fields {
            if let Some((_, sub)) = b {
                Self::pat_bound_names(sub, &mut names);
            }
        }
        let force = Self::init_shadowed_by(&init, &names);
        let (mut out, subj) = self.let_destructure_prelude(ty, init, force);
        out.extend(Self::struct_pat_binds(&fields, &subj, &name));
        out
    }

    /// The same expansion for an array pattern: bind the scrutinee to a hidden
    /// temporary (unless it is already a plain place) and read each named
    /// element out of it. The temporary carries the annotation, so `let [a, b]:
    /// [i64; 2] = e;` is still checked.
    fn let_destructure_array_stmts(
        &mut self,
        elems: Vec<Option<(bool, Pat)>>,
        rest: bool,
        front: usize,
        ty: Option<Type>,
        init: Expr,
    ) -> Vec<Stmt> {
        let mut names = Vec::new();
        for e in &elems {
            if let Some((_, sub)) = e {
                Self::pat_bound_names(sub, &mut names);
            }
        }
        let force = Self::init_shadowed_by(&init, &names);
        let (mut out, subj) = self.let_destructure_prelude(ty, init, force);
        out.extend(Self::array_pat_binds(&elems, rest, front, &subj));
        out
    }

    /// The same expansion for a tuple pattern: bind the scrutinee to a hidden
    /// temporary (unless it is already a plain place) and read each named
    /// element out of it. The temporary carries the annotation, so `let (a, b):
    /// (i64, i64) = e;` is still checked.
    fn let_destructure_tuple_stmts(
        &mut self,
        elems: Vec<(bool, Pat)>,
        ty: Option<Type>,
        init: Expr,
    ) -> Vec<Stmt> {
        let mut names = Vec::new();
        for (_, sub) in &elems {
            Self::pat_bound_names(sub, &mut names);
        }
        let force = Self::init_shadowed_by(&init, &names);
        let (mut out, subj) = self.let_destructure_prelude(ty, init, force);
        out.extend(Self::tuple_pat_binds(&elems, &subj));
        out
    }

    /// The hidden scrutinee a `let` destructuring binds from: the initializer
    /// itself when it is a plain place with no annotation to carry, otherwise a
    /// fresh temporary that holds it (so the initializer's side effects run
    /// once and the annotation is checked against the value). Shared by the
    /// struct, array and tuple destructuring paths. `force` makes the temporary
    /// unconditional: a caller sets it when a pattern binds the very name the
    /// initializer reads, so every element read sees the original value rather
    /// than a binding that shadowed it (`let (x, y) = x;`).
    fn let_destructure_prelude(
        &mut self,
        ty: Option<Type>,
        init: Expr,
        force: bool,
    ) -> (Vec<Stmt>, Expr) {
        if !force
            && ty.is_none()
            && matches!(init, Expr::Var { .. } | Expr::Int(_) | Expr::EnumVariant { .. })
        {
            (Vec::new(), init)
        } else {
            let tmp = format!("\u{0}lp{}", self.tmp);
            self.tmp += 1;
            let stmt = Stmt::Let {
                name: tmp.clone(),
                ty,
                init: Some(init),
                slot: -1,
                mutable: false,
            };
            (vec![stmt], Expr::Var { name: tmp, slot: -1, ty: Type::I64 })
        }
    }

    /// The `let` statements a struct pattern introduces: one per bound field,
    /// each a read of that field on `subj`. Shared by the `match` arm path,
    /// `let` destructuring and `for` destructuring, which differ only in what
    /// they do with the result. The reads are left for sema to type and locate,
    /// so a non-struct scrutinee is a sema error naming the field and a field
    /// may auto-deref a `&S` scrutinee. `f: _` contributes no binding. A nested
    /// sub-pattern (`f: I { a }`) recurses with the field read as its
    /// sub-scrutinee, so the nested names become ordinary chained reads.
    fn struct_pat_binds(fields: &[FieldPat], subj: &Expr, pat_struct: &str) -> Vec<Stmt> {
        // A check that `subj` is really a `pat_struct`, emitted even when the
        // pattern binds no fields (`A { .. }`), so a pattern naming the wrong
        // struct is rejected rather than silently matching.
        let mut out = vec![Stmt::Expr(Expr::PatCheck {
            base: Box::new(subj.clone()),
            pat_struct: pat_struct.to_string(),
        })];
        for (fname, bind) in fields {
            let (m, pat) = match bind {
                Some(x) => x,
                None => continue,
            };
            let field = Expr::Field {
                base: Box::new(subj.clone()),
                name: fname.clone(),
                offset: -1,
                ty: Type::Unit,
            };
            match pat {
                Pat::Bind(b) => out.push(Stmt::Let {
                    name: b.clone(),
                    ty: None,
                    mutable: *m,
                    init: Some(field),
                    slot: -1,
                }),
                // `f: name @ subpat`: bind the field to `name`, then bind the
                // sub-pattern's names from the same field read.
                Pat::At(name, am, sub) => {
                    out.push(Stmt::Let {
                        name: name.clone(),
                        ty: None,
                        mutable: *m || *am,
                        init: Some(field.clone()),
                        slot: -1,
                    });
                    Self::pat_binds(sub, false, &field, &mut out);
                }
                Pat::Struct(inner_name, inner) => {
                    out.extend(Self::struct_pat_binds(inner, &field, inner_name))
                }
                Pat::Tuple(inner) => out.extend(Self::tuple_pat_binds(inner, &field)),
                // An array-typed field may be destructured by an array pattern.
                Pat::Array { elems, rest, front } => {
                    out.extend(Self::array_pat_binds(elems, *rest, *front, &field))
                }
                // Only a binding, a nested struct/tuple pattern or a nested
                // array pattern is parsed here.
                _ => unreachable!("unexpected struct sub-pattern"),
            }
        }
        out
    }

    /// The `let` statements one sub-pattern introduces, reading from `subj`.
    /// `m` is a `mut` written before the pattern; only a binding may carry it.
    /// A binding becomes one `let`; a wildcard binds nothing; a struct, tuple
    /// or array pattern recurses, reading through `subj` (its own fields or
    /// elements). Shared by the `let`/`for`/`match` paths and a payload
    /// variant's arguments, so `E::A(S { x })` expands to the same read the
    /// equivalent `let` would.
    fn pat_binds(pat: &Pat, m: bool, subj: &Expr, out: &mut Vec<Stmt>) {
        match pat {
            Pat::Bind(name) => out.push(Stmt::Let {
                name: name.clone(),
                ty: None,
                mutable: m,
                init: Some(subj.clone()),
                slot: -1,
            }),
            Pat::Wild => {}
            // `name @ subpat`: bind the whole value to `name`, then bind the
            // sub-pattern's names from the same value.
            Pat::At(name, am, sub) => {
                out.push(Stmt::Let {
                    name: name.clone(),
                    ty: None,
                    mutable: m || *am,
                    init: Some(subj.clone()),
                    slot: -1,
                });
                Self::pat_binds(sub, false, subj, out);
            }
            Pat::Struct(name, fields) => out.extend(Self::struct_pat_binds(fields, subj, &name)),
            Pat::Tuple(elems) => out.extend(Self::tuple_pat_binds(elems, subj)),
            Pat::Array { elems, rest, front } => {
                out.extend(Self::array_pat_binds(elems, *rest, *front, subj))
            }
            // A scalar element binds nothing (its test is generated
            // separately); an or-pattern binds nothing (rmc rejects a binding
            // in an or-pattern when it is parsed).
            Pat::Int(_) | Pat::Char(_) | Pat::CharRange(..) | Pat::Bool(_) | Pat::Str(_) | Pat::Range(..) | Pat::Or(_) => {}
            _ => unreachable!("unexpected sub-pattern"),
        }
    }

    /// Every name a pattern binds, in binding order: a struct pattern's fields,
    /// a tuple/array's elements, an `@` binding's name followed by its
    /// sub-pattern's. Used to decide whether a `let` destructuring must copy its
    /// scrutinee before it binds, so an element that shadows the scrutinee name
    /// does not make a later read resolve against the new binding.
    fn pat_bound_names(p: &Pat, out: &mut Vec<String>) {
        match p {
            Pat::Bind(n) => out.push(n.clone()),
            Pat::At(n, _, sub) => {
                out.push(n.clone());
                Self::pat_bound_names(sub, out);
            }
            Pat::Struct(_, fields) => {
                for (_, b) in fields {
                    if let Some((_, sub)) = b {
                        Self::pat_bound_names(sub, out);
                    }
                }
            }
            Pat::Tuple(elems) => {
                for (_, sub) in elems {
                    Self::pat_bound_names(sub, out);
                }
            }
            Pat::Array { elems, .. } => {
                for e in elems {
                    if let Some((_, sub)) = e {
                        Self::pat_bound_names(sub, out);
                    }
                }
            }
            Pat::Variant(_, _, Some((args, _))) => {
                for (_, sub) in args {
                    Self::pat_bound_names(sub, out);
                }
            }
            _ => {}
        }
    }

    /// True when `init` reads a plain variable the pattern also binds. A `let`
    /// destructuring expands to one binding per element, each reading the
    /// scrutinee; if an element shadows the scrutinee's own name, a later read
    /// would resolve to the new binding (`let (x, y) = x;` read field `1` of the
    /// integer `x` just bound). Such a destructuring must first copy the
    /// scrutinee to a hidden temporary, so every read sees the original value.
    fn init_shadowed_by(init: &Expr, names: &[String]) -> bool {
        match init {
            Expr::Var { name, .. } => names.iter().any(|n| n == name),
            _ => false,
        }
    }

    /// The first variable a pattern binds, if any. A struct pattern binds the
    /// names of its sub-patterns; an or-pattern binds through its alternatives.
    /// Used to reject a binding in an or-pattern (rustc's E0408), which rmc
    /// cannot bind consistently across alternatives.
    fn first_bound_name(p: &Pat) -> Option<String> {
        match p {
            Pat::Bind(n) => Some(n.clone()),
            // `name @ subpat` binds `name` first, then whatever the sub-pattern
            // binds.
            Pat::At(n, _, _) => Some(n.clone()),
            // A payload variant pattern binds the names of its argument
            // sub-patterns; a nested variant pattern would too, but it is
            // rejected before this point.
            Pat::Variant(_, _, Some((args, _))) => {
                args.iter().find_map(|(_, sub)| Self::first_bound_name(sub))
            }
            Pat::Struct(_, fields) => fields.iter().find_map(|(_, b)| {
                b.as_ref().and_then(|(_, sub)| Self::first_bound_name(sub))
            }),
            Pat::Tuple(elems) => elems.iter().find_map(|(_, sub)| Self::first_bound_name(sub)),
            Pat::Array { elems, .. } => elems
                .iter()
                .find_map(|e| e.as_ref().and_then(|(_, p)| Self::first_bound_name(p))),
            Pat::Or(alts) => alts.iter().find_map(Self::first_bound_name),
            _ => None,
        }
    }

    /// The bindings and the optional test a pattern contributes to an arm.
    /// A test is `None` when the pattern is irrefutable, so the arm is a
    /// catch-all. `Pat::At` binds the whole value and recurses into its
    /// sub-pattern, so `n @ 1..=5` binds `n` and adds the range test.
    fn arm_pattern_parts(pat: Pat, subj: &Expr) -> (Vec<Stmt>, Option<Expr>) {
        match pat {
            Pat::Wild => (Vec::new(), None),
            // `name @ subpat` binds the whole value to `name`, then matches the
            // same value against `subpat`, which contributes its own bindings
            // and test: `n @ 1..=5` binds `n` and adds the range test, and
            // `n @ (a, b)` binds all three.
            Pat::At(name, m, sub) => {
                let mut binds = vec![Stmt::Let {
                    name,
                    ty: None,
                    mutable: m,
                    init: Some(subj.clone()),
                    slot: -1,
                }];
                let (sub_binds, test) = Self::arm_pattern_parts(*sub, subj);
                binds.extend(sub_binds);
                (binds, test)
            }
            // `S { f, g: h } => body` is irrefutable, so it has no test: it binds
            // each named field to a fresh local via a field read. The reads are
            // left for sema to type and locate, so a non-struct scrutinee is a
            // sema error naming the field, and a field may be matched on a `&S`
            // scrutinee by auto-deref. The struct name and each field name were
            // already checked against the prescan, so `f: _` (which emits no
            // read) is validated too.
            Pat::Struct(name, fields) => (Self::struct_pat_binds(&fields, &subj, &name), None),
            // `(a, b) => body` binds each named element via a field read. When
            // every element is irrefutable there is no test (a catch-all); a
            // scalar element (`(0, x)`) adds its test, folded into the arm's
            // condition like any other.
            Pat::Tuple(elems) => {
                (Self::tuple_pat_binds(&elems, &subj), Self::tuple_pat_test(&elems, &subj))
            }
            // `[a, b] => body` is irrefutable in the same way: it binds each
            // named element via an index read, with no test. A `..` rest shifts
            // the trailing bindings to the back of the array.
            Pat::Array { elems, rest, front } => {
                (Self::array_pat_binds(&elems, rest, front, &subj), None)
            }
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
                Some(Expr::Bin(
                    BinOp::Eq,
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Int(k)),
                )),
            ),
            Pat::Char(c) => (
                Vec::new(),
                Some(Expr::Bin(
                    BinOp::Eq,
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Char(c)),
                )),
            ),
            // A string literal pattern: a content compare, which sema lowers to
            // the byte-wise helper (a pointer compare would never match). Not
            // auto-deref'd: see `or_cond`.
            Pat::Str(s) => (
                Vec::new(),
                Some(Expr::Bin(BinOp::Eq, Box::new(subj.clone()), Box::new(Expr::Str(s)))),
            ),
            // `lo..hi` is `lo <= s && s < hi`; `lo..=hi` uses `<=` on both
            // sides. A plain `Bin(And, ..)` of comparisons, so sema and codegen
            // need no new case.
            Pat::Range(lo, hi, inclusive) => {
                let lower = Expr::Bin(
                    BinOp::Le,
                    Box::new(Expr::Int(lo)),
                    Box::new(Self::scalar_scrutinee(subj)),
                );
                let upper = Expr::Bin(
                    if inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Int(hi)),
                );
                (Vec::new(), Some(Expr::Bin(BinOp::And, Box::new(lower), Box::new(upper))))
            }
            // `'a'..'z'` / `'a'..='z'`: the char analogue of `Range`, the same
            // pair of comparisons with `char` bounds.
            Pat::CharRange(lo, hi, inclusive) => {
                let lower = Expr::Bin(
                    BinOp::Le,
                    Box::new(Expr::Char(lo)),
                    Box::new(Self::scalar_scrutinee(subj)),
                );
                let upper = Expr::Bin(
                    if inclusive { BinOp::Le } else { BinOp::Lt },
                    Box::new(Self::scalar_scrutinee(subj)),
                    Box::new(Expr::Char(hi)),
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
                Some(Expr::BoolTest {
                    scrutinee: Box::new(Self::scalar_scrutinee(subj)),
                    value: b,
                }),
            ),
            Pat::Variant(enum_name, variant, args) => {
                let n_binds = args.as_ref().map_or(-1, |(a, _)| a.len() as i32);
                let has_rest = args.as_ref().map_or(false, |(_, rest)| *rest);
                let cond = Expr::VariantTest {
                    scrutinee: Box::new(subj.clone()),
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    tag: -1,
                    n_binds,
                    has_rest,
                };
                // `Enum::V(w, x)` binds each payload field in the arm body. Each
                // argument is the payload read at its field index, fed to the
                // same per-pattern expansion the `let` and `match` paths use, so
                // `_` reads nothing and a nested struct/array sub-pattern reads
                // through the payload.
                let binds = match args {
                    Some((args, _)) => {
                        let mut out = Vec::new();
                        for (idx, (m, pat)) in args.into_iter().enumerate() {
                            let payload = Expr::EnumPayload {
                                recv: Box::new(subj.clone()),
                                enum_name: enum_name.clone(),
                                variant: variant.clone(),
                                idx,
                                offset: 0,
                                ty: Type::Unit,
                            };
                            Self::pat_binds(&pat, m, &payload, &mut out);
                        }
                        out
                    }
                    None => Vec::new(),
                };
                (binds, Some(cond))
            }
        }
    }

    /// One arm of a desugared `match` or `if let`: the pattern test, the
    /// bindings it introduces, and the fallback `rest` for a non-match.
    fn desugar_arm(
        &mut self,
        pat: Pat,
        guard: Option<Expr>,
        subj: &Expr,
        body: Expr,
        rest: Option<Expr>,
        check_dead: bool,
    ) -> PResult<Expr> {
        // Each pattern yields its test (`None` when it is irrefutable) and any
        // bindings it introduces. The test is an ordinary expression, so sema
        // and codegen need no new case for a guard: it is folded in with `&&`,
        // and an irrefutable arm's guard becomes the whole `if` condition.
        let (bindings, test) = Self::arm_pattern_parts(pat, &subj);
        // The pattern's bindings belong to the guard and to the arm body, but
        // NOT to the fallback `rest`: a later arm must not see this arm's names
        // (`match x { n if false => n, _ => n }` must let the `_` arm read the
        // outer `n`). So the bindings wrap the guard expression and the `then`
        // block, never the whole `if`.
        let with_bindings = |e: Expr| -> Expr {
            if bindings.is_empty() {
                e
            } else {
                Expr::Block(Block { stmts: bindings.clone(), tail: Some(Box::new(e)) })
            }
        };
        let then = with_bindings(body);
        Ok(match test {
            Some(cond) => {
                // `pat if g => body` is `pat => body` only when `g` holds, so
                // the guard joins the pattern test with `&&` and a failed guard
                // falls through to `rest`.
                let cond = match guard {
                    Some(g) => Expr::Bin(BinOp::And, Box::new(cond), Box::new(with_bindings(g))),
                    None => cond,
                };
                Self::match_arm(cond, then, rest)
            }
            // An irrefutable arm: without a guard it always matches (and `rest`
            // is dead); with one it is `if g { body } else { rest }`. The guard
            // is paired with a literal `true` pattern test so the
            // exhaustiveness walk can tell a guarded binding arm (covers
            // nothing) from an unguarded one (covers everything).
            None => match guard {
                None => Self::irrefutable_arm(then, rest, check_dead),
                Some(g) => Self::match_arm(
                    Expr::Bin(BinOp::And, Box::new(Expr::Bool(true)), Box::new(with_bindings(g))),
                    then,
                    rest,
                ),
            },
        })
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
            Some(Tok::Char(c)) => {
                self.i += 1;
                Ok(Expr::Char(c))
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
            // `i64::MIN` / `i64::MAX` / `i64::BITS`. `i64` is a keyword token,
            // so it never reaches the `Ident` arm below.
            Some(Tok::TyI64) => {
                self.i += 1;
                if self.eat(&Tok::ColonColon) {
                    let konst = self.expect_ident()?;
                    if let Some(v) = int_assoc_const("i64", &konst) {
                        return Ok(Expr::Int(v));
                    }
                    return Err(format!(
                        "at byte {}: `i64` has no associated constant `{konst}`",
                        self.pos()
                    ));
                }
                Err(format!("at byte {}: expected an expression, found `i64`", self.pos()))
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
                // `unsafe { .. }`: rmc does no borrow or safety checking, so the
                // block is transparent. `unsafe fn` is likewise a plain `fn`.
                if name == "unsafe" && self.peek() == Some(&Tok::LBrace) {
                    return self.parse_block().map(Expr::Block);
                }
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
                // A macro invocation `name!(...)`. `print!`/`println!` are
                // lowered to runtime writes; `assert!`, `assert_eq!`,
                // `assert_ne!`, `panic!` and `unreachable!` become an exit with
                // status 101 (rustc's panic code). Any other name is rejected.
                if self.peek() == Some(&Tok::Bang) {
                    match name.as_str() {
                        "println" | "print" => return self.parse_macro(&name),
                        "vec" => return self.parse_vec_macro(),
                        "assert" | "assert_eq" | "assert_ne" => {
                            return self.parse_assert(&name)
                        }
                        "panic" | "unreachable" | "todo" | "unimplemented" => {
                            return self.parse_panic(&name)
                        }
                        _ => {
                            return Err(format!(
                                "at byte {}: unknown macro `{name}!`",
                                self.pos()
                            ))
                        }
                    }
                }
                // `std::...` paths, or a module brought in by `use std::m;`, for
                // the handful of std items rmc provides. Flattening the path
                // lets `std::process::exit(0)` and `std::hint::black_box(x)`
                // compile; an unrecognised path is left to the normal `::`
                // handling below (so a struct named like a module still works).
                if self.peek() == Some(&Tok::ColonColon) && self.std_mods.contains(&name) {
                    if let Some(e) = self.try_parse_std_path(&name)? {
                        return Ok(e);
                    }
                }
                // `Enum::Variant` or `Enum::Variant(args)` — resolved and
                // range-checked by sema.
                if self.eat(&Tok::ColonColon) {
                    // `Vec::with_capacity(k)` and `Vec::new()` allocate the
                    // backing store; the returned `Vec` value is the block
                    // address (an `i64`). Only these two constructors are
                    // supported, and only as a `let` initializer.
                    if name == "Vec" {
                        let assoc = self.expect_ident()?;
                        if assoc != "with_capacity" && assoc != "new" {
                            return Err(format!(
                                "at byte {}: unsupported `Vec::{assoc}` (only `Vec::with_capacity` and `Vec::new`)",
                                self.pos()
                            ));
                        }
                        self.expect(Tok::LParen)?;
                        let cap = if assoc == "new" {
                            Expr::Int(0)
                        } else {
                            let e = self.parse_bracketed_expr()?;
                            self.expect(Tok::RParen)?;
                            e
                        };
                        return Ok(Expr::Call {
                            name: "\u{0}vecnew".into(),
                            args: vec![cap],
                            arg_slots: Vec::new(),
                            ret_slot: -1,
                            ret_ty: Type::I64,
                        });
                    }
                    // `i64::MIN`, `usize::MAX`, `u32::BITS`: an integer-width
                    // associated constant folds straight to its value.
                    if is_int_width(&name) {
                        if let Some(Tok::Ident(konst)) = self.peek().cloned() {
                            if let Some(v) = int_assoc_const(&name, &konst) {
                                self.i += 1;
                                return Ok(Expr::Int(v));
                            }
                        }
                    }
                    let variant = self.expect_ident()?;
                    if self.eat(&Tok::LParen) {
                        let mut args = Vec::new();
                        while self.peek() != Some(&Tok::RParen) {
                            args.push(self.parse_bracketed_expr()?);
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
                    // struct literal `Name { field: value, ... }`, optionally
                    // ending in a struct update base `..expr`.
                    self.i += 1;
                    let mut fields = Vec::new();
                    let mut base: Option<Box<Expr>> = None;
                    while self.peek() != Some(&Tok::RBrace) {
                        if self.eat(&Tok::DotDot) {
                            // `..expr` fills the fields not written above. It is
                            // the last thing before `}`, so the base parses
                            // greedily; rustc allows any expression here,
                            // including another struct literal.
                            let b = self.parse_expr()?;
                            base = Some(Box::new(b));
                            // No comma may follow the base: it must be the last
                            // thing before `}` (rustc E0747-style error).
                            break;
                        }
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
                    if let Some(base) = base {
                        // A field written twice is an error (rustc: "field `x`
                        // specified more than once"). The plain literal path
                        // catches this in sema; the update path desugars to
                        // assignments, so it must check here.
                        for (i, f) in fields.iter().enumerate() {
                            if fields[..i].iter().any(|g| g.name == f.name) {
                                return Err(format!(
                                    "at byte {}: field `{}` specified more than once",
                                    self.pos(),
                                    f.name
                                ));
                            }
                        }
                        return Ok(self.desugar_struct_update(name, fields, base));
                    }
                    return Ok(Expr::StructLit { name, fields, slot: -1 });
                }
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    while self.peek() != Some(&Tok::RParen) {
                        args.push(self.parse_bracketed_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    // A call names a nested `fn` if one is in scope, so a nested
                    // `fn` shadows a top-level one of the same name. The rewrite
                    // is to the hoisted function's unique name.
                    let name = self.resolve_fn_name(name);
                    Ok(Expr::Call { name, args, arg_slots: Vec::new(), ret_slot: -1, ret_ty: Type::I64 })
                } else if let Some(v) = self.consts.get(&name) {
                    // A reference to a constant is its value, substituted here
                    // so sema and codegen never see a constant as a variable.
                    Ok(Expr::Int(*v))
                } else if let Some(b) = self.const_bools.get(&name) {
                    Ok(Expr::Bool(*b))
                } else if let Some(c) = self.const_chars.get(&name) {
                    Ok(Expr::Char(*c))
                } else {
                    Ok(Expr::Var { name, slot: -1, ty: Type::I64 })
                }
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let saved = self.no_struct_lit;
                self.no_struct_lit = false;
                // `()` is unit; `(e)` is grouping; `(a, b)` and `(a,)` are
                // tuple literals. Sema resolves the literal's element types and
                // rewrites it to the anonymous struct that lays them out.
                if self.eat(&Tok::RParen) {
                    self.no_struct_lit = saved;
                    return Ok(Expr::Block(Block { stmts: Vec::new(), tail: None }));
                }
                let first = self.parse_expr();
                let first = match first {
                    Ok(e) => e,
                    Err(e) => {
                        self.no_struct_lit = saved;
                        return Err(e);
                    }
                };
                let r = if self.eat(&Tok::Comma) {
                    let mut elems = vec![first];
                    while self.peek() != Some(&Tok::RParen) {
                        elems.push(self.parse_expr()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                    self.expect(Tok::RParen)?;
                    Ok(Expr::TupleLit(elems))
                } else {
                    self.expect(Tok::RParen)?;
                    Ok(first)
                };
                self.no_struct_lit = saved;
                r
            }
            Some(Tok::If) => {
                self.i += 1;
                // `if let PAT = EXPR { .. } else { .. }` is sugar for a one-arm
                // `match`, so it desugars through the same path and sema and
                // codegen need no new case.
                if self.peek() == Some(&Tok::Let) {
                    self.i += 1;
                    let pat = self.parse_refutable_pattern()?;
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
                    return self.desugar_if_let(pat, scrutinee, then, els);
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
                let first = self.parse_bracketed_expr()?;
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
                        elems.push(self.parse_bracketed_expr()?);
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
    /// A character literal, matched by value.
    Char(u32),
    /// `'a'..'z'` / `'a'..='z'`, inclusive flag set for `..=`. The character
    /// analogue of `Range`.
    CharRange(u32, u32, bool),
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
    /// sema; the optional argument patterns name the payload fields, in field
    /// order. Each argument is `(mut, pat)`, where `pat` is a binding (`v`), a
    /// wildcard (`_`), or a nested struct/array/variant sub-pattern; the flag is
    /// whether a trailing `..` ignored the remaining fields.
    Variant(String, String, Option<(Vec<(bool, Pat)>, bool)>),
    /// A plain name: binds the whole scrutinee (an irrefutable catch-all). Kept
    /// distinct from `Wild` only so the binding is named.
    Bind(String),
    /// `S { f, g: h }`: a struct pattern, matched by name against the
    /// scrutinee's type. Each field is `(name, Some((mut, binding)))` when
    /// written `f: b` (or the shorthand `f`, which binds `f`), and
    /// `(name, None)` for `f: _`. `mut` is a per-binding flag, as in rustc: it
    /// is written on the binding (`f: mut b`, `mut f`) and there is no
    /// `mut` on the pattern as a whole. A struct pattern is irrefutable, so it
    /// has no test: it desugars to the field reads its bindings name (see
    /// `desugar_arm` and `let` destructuring). The struct name is resolved by
    /// sema, so it may name a struct declared later in the file.
    Struct(String, Vec<FieldPat>),
    /// `[a, b, .., c]`: an array pattern. Each element is a sub-pattern with its
    /// own `mut` flag (`[mut a, b]`); `None` marks the single `..` rest, which
    /// matches any number of middle elements. `front` counts the elements
    /// before the rest (and, with no rest, is the element count). Irrefutable
    /// when every element pattern is, so a `for` or `let` may use one; a `match`
    /// arm using one has no test. Desugars to the indexed reads its bindings
    /// name.
    Array { elems: Vec<Option<(bool, Pat)>>, rest: bool, front: usize },
    /// `(a, b, c)`: a tuple pattern. Each element is a sub-pattern with its own
    /// `mut` flag (`(mut a, b)`). It is irrefutable when every element is, so a
    /// `let`/`for`/parameter may use one; a `match` arm using one has no test.
    /// Desugars to the field reads its bindings name (a tuple is an anonymous
    /// struct, so element `k` is the field `"k"`). The element count must match
    /// the scrutinee's arity, which sema enforces through those reads.
    Tuple(Vec<(bool, Pat)>),
    /// `name @ subpat`: bind the whole value to `name` and also match it
    /// against `subpat`, so `n @ 1..=5` binds `n` and tests the range. The
    /// flag is a `mut` written before the name (`mut n @ pat`). The binding
    /// is emitted like any other, and the sub-pattern contributes its own test
    /// and bindings, so `n @ (a, b)` binds all three.
    At(String, bool, Box<Pat>),
}

/// One field of a struct pattern: its name and what it binds. `None` is
/// `f: _` (ignored); `Some((mut, pat))` binds the field, where `pat` is a
/// binding (`f`/`f: b`) or a nested struct pattern (`f: I { a }`). `mut` is only
/// meaningful on a binding, so a nested sub-pattern is stored with `false`.
type FieldPat = (String, Option<(bool, Pat)>);

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

/// True when `s` is one of the built-in integer-width type names.
fn is_int_width(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
    )
}

/// The byte width of one `Vec<T>` element named `t`, the stride a `.add(off)`
/// scales by. rmc stores every element in one 8-byte slot (see `alloc`/
/// `__rmc_fill`), so an element steps a whole word — except the byte types
/// `u8`/`i8`, which the corpus uses as a raw byte arena and which therefore
/// stay byte-addressed.
fn elem_byte_width(t: &str) -> i64 {
    match t {
        "u8" | "i8" => 1,
        _ => 8,
    }
}

/// The value of an integer-width associated constant (`i32::MAX`, `u8::BITS`),
/// or `None` when `ty`/`name` is not one. rmc has one integer type, so a width's
/// `MIN`/`MAX`/`BITS` is folded to the `i64` value of that width; an unsigned
/// `MAX` above `i64::MAX` becomes its two's-complement bit pattern, exactly what
/// `u64::MAX as i64` is, so `usize::MAX` reads as `-1`.
fn int_assoc_const(ty: &str, name: &str) -> Option<i64> {
    let (signed, bits): (bool, u32) = match ty {
        "i8" => (true, 8),
        "i16" => (true, 16),
        "i32" => (true, 32),
        "i64" => (true, 64),
        "i128" => (true, 128),
        "isize" => (true, 64),
        "u8" => (false, 8),
        "u16" => (false, 16),
        "u32" => (false, 32),
        "u64" => (false, 64),
        "u128" => (false, 128),
        "usize" => (false, 64),
        _ => return None,
    };
    let v: i128 = match name {
        "BITS" => bits as i128,
        "MIN" if signed => -(1i128 << (bits - 1)),
        "MIN" => 0,
        "MAX" if signed => (1i128 << (bits - 1)) - 1,
        // An unsigned `MAX` of a 128-bit width is `u128::MAX`, whose bit pattern
        // is all ones; the narrower ones fit an `i128` before truncating.
        "MAX" => ((1u128 << bits) - 1) as i128,
        _ => return None,
    };
    Some(v as i64)
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

/// The built-in integer-width type names. rmc has one integer type, `i64`, but
/// a Rust source spells widths (`usize`, `u32`, `i8`, ...). Each is pre-seeded
/// as a transparent alias for `i64` so `let i: usize = 0;` and
/// `x as usize` parse; the value is `i64` throughout, so the alias is exactly
/// what a `type usize = i64;` would give. A program may shadow any of them with
/// its own `type`, which is why this is the alias map and not a reserved word.
fn builtin_aliases() -> std::collections::HashMap<String, Type> {
    let mut m = std::collections::HashMap::new();
    for name in [
        "usize", "isize", "u8", "u16", "u32", "u64", "u128",
        "i8", "i16", "i32", "i128",
    ] {
        m.insert(name.to_string(), Type::I64);
    }
    m
}
