//! Hand-written lexer. Produces a flat token stream with byte offsets for errors.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    // literals / identifiers
    Ident(String),
    Int(i64),
    /// The magnitude `2^63`, which does not fit an `i64` but is the operand of
    /// the `i64::MIN` literal `-9223372036854775808`. The parser folds it into
    /// `Expr::Int(i64::MIN)` only when it directly follows a `-`; anywhere else
    /// it is out of range, matching rustc.
    IntMinMagnitude,
    Str(String),
    // keywords
    Fn,
    Let,
    Mut,
    While,
    Loop,
    If,
    Else,
    Match,
    Return,
    Break,
    Continue,
    As,
    Struct,
    Impl,
    Enum,
    Const,
    For,
    In,
    True,
    False,
    // types
    TyI64,
    TyBool,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Colon,
    ColonColon,
    Arrow,
    FatArrow,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Caret,
    Shl,
    Shr,
    Bang,
    Dot,
    DotDot,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    Underscore,
}

impl Tok {
    /// Human readable name used in diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("identifier `{s}`"),
            Tok::Int(v) => format!("integer `{v}`"),
            Tok::IntMinMagnitude => "integer `9223372036854775808`".into(),
            Tok::Str(_) => "string literal".into(),
            Tok::Fn => "`fn`".into(),
            Tok::Let => "`let`".into(),
            Tok::Mut => "`mut`".into(),
            Tok::While => "`while`".into(),
            Tok::Loop => "`loop`".into(),
            Tok::If => "`if`".into(),
            Tok::Else => "`else`".into(),
            Tok::Match => "`match`".into(),
            Tok::Return => "`return`".into(),
            Tok::Break => "`break`".into(),
            Tok::Continue => "`continue`".into(),
            Tok::As => "`as`".into(),
            Tok::Struct => "`struct`".into(),
            Tok::Const => "`const`".into(),
            Tok::For => "`for`".into(),
            Tok::In => "`in`".into(),
            Tok::True => "`true`".into(),
            Tok::False => "`false`".into(),
            Tok::TyI64 => "`i64`".into(),
            Tok::TyBool => "`bool`".into(),
            other => format!("`{}`", other.punct_str()),
        }
    }

    fn punct_str(&self) -> &'static str {
        match self {
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Semi => ";",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::ColonColon => "::",
            Tok::Arrow => "->",
            Tok::FatArrow => "=>",
            Tok::Eq => "=",
            Tok::EqEq => "==",
            Tok::Ne => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Amp => "&",
            Tok::AmpAmp => "&&",
            Tok::Pipe => "|",
            Tok::PipePipe => "||",
            Tok::Caret => "^",
            Tok::Shl => "<<",
            Tok::Shr => ">>",
            Tok::Bang => "!",
            Tok::Dot => ".",
            Tok::DotDot => "..",
            Tok::PlusEq => "+=",
            Tok::MinusEq => "-=",
            Tok::StarEq => "*=",
            Tok::SlashEq => "/=",
            Tok::PercentEq => "%=",
            Tok::AmpEq => "&=",
            Tok::PipeEq => "|=",
            Tok::CaretEq => "^=",
            Tok::ShlEq => "<<=",
            Tok::ShrEq => ">>=",
            Tok::Underscore => "_",
            _ => "?",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: u32,
}

pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::with_capacity(bytes.len() / 3 + 8);

    while i < bytes.len() {
        let c = bytes[i];
        // whitespace
        if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
            i += 1;
            continue;
        }
        // line comment
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // block comment
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // attribute such as `#[derive(Copy, Clone)]` or `#![allow(...)]`: parsed
        // and discarded, so rmc can compile the same source as rustc. Brackets
        // are balanced to survive nested groups.
        if c == b'#' {
            let start = i;
            i += 1;
            if i < bytes.len() && bytes[i] == b'!' {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'[' {
                return Err(format!("expected `[` after `#` at byte {start}"));
            }
            let mut depth = 0i32;
            while i < bytes.len() {
                match bytes[i] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                return Err(format!("unterminated attribute at byte {start}"));
            }
            continue;
        }
        let pos = i as u32;
        // identifiers / keywords
        if c == b'_' || c.is_ascii_alphabetic() {
            let start = i;
            while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let s = &src[start..i];
            let tok = match s {
                "fn" => Tok::Fn,
                "let" => Tok::Let,
                "mut" => Tok::Mut,
                "while" => Tok::While,
                "loop" => Tok::Loop,
                "if" => Tok::If,
                "else" => Tok::Else,
                "match" => Tok::Match,
                "return" => Tok::Return,
                "break" => Tok::Break,
                "continue" => Tok::Continue,
                "as" => Tok::As,
                "struct" => Tok::Struct,
                "impl" => Tok::Impl,
                "enum" => Tok::Enum,
                "const" => Tok::Const,
                "for" => Tok::For,
                "in" => Tok::In,
                "true" => Tok::True,
                "false" => Tok::False,
                "i64" => Tok::TyI64,
                "bool" => Tok::TyBool,
                "_" => Tok::Underscore,
                _ => Tok::Ident(s.to_string()),
            };
            out.push(Token { tok, pos });
            continue;
        }
        // string literals
        if c == b'"' {
            let start = i;
            i += 1;
            // Collect bytes, not chars: the source is UTF-8, so a multi-byte
            // character must round-trip unchanged. Building the literal with
            // `byte as char` would re-encode each byte and print mojibake.
            let mut s: Vec<u8> = Vec::new();
            while i < bytes.len() && bytes[i] != b'"' {
                let b = bytes[i];
                if b == b'\\' && i + 1 < bytes.len() {
                    let e = bytes[i + 1];
                    i += 2;
                    match e {
                        b'n' => s.push(b'\n'),
                        b't' => s.push(b'\t'),
                        b'r' => s.push(b'\r'),
                        b'0' => s.push(0),
                        b'\\' => s.push(b'\\'),
                        b'"' => s.push(b'"'),
                        other => {
                            return Err(format!("unknown escape `\\{}` at byte {}", other as char, i - 2))
                        }
                    }
                } else {
                    s.push(b);
                    i += 1;
                }
            }
            if i >= bytes.len() {
                return Err(format!("unterminated string literal at byte {start}"));
            }
            i += 1; // closing quote
            let s = String::from_utf8(s)
                .map_err(|_| format!("string literal at byte {start} is not valid UTF-8"))?;
            out.push(Token { tok: Tok::Str(s), pos });
            continue;
        }
        // numbers
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let raw = src[start..i].replace('_', "");
            let (digits, radix) = if let Some(h) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
                (h, 16)
            } else if let Some(b) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
                (b, 2)
            } else if let Some(o) = raw.strip_prefix("0o").or_else(|| raw.strip_prefix("0O")) {
                (o, 8)
            } else {
                (raw.as_str(), 10)
            };
            let v = i64::from_str_radix(digits, radix);
            match v {
                Ok(v) => out.push(Token { tok: Tok::Int(v), pos }),
                // The one magnitude above `i64::MAX` that a `-` can make valid:
                // the operand of `i64::MIN`. Defer the sign check to the parser.
                Err(_) if is_i64_min_magnitude(digits, radix) => {
                    out.push(Token { tok: Tok::IntMinMagnitude, pos })
                }
                Err(_) => {
                    return Err(format!("invalid integer literal `{raw}` at byte {start}"));
                }
            }
            continue;
        }
        // three-char operators (must precede `<<`/`>>`)
        let three = if i + 2 < bytes.len() {
            match &bytes[i..i + 3] {
                b"<<=" => Some(Tok::ShlEq),
                b">>=" => Some(Tok::ShrEq),
                _ => None,
            }
        } else {
            None
        };
        if let Some(t) = three {
            out.push(Token { tok: t, pos });
            i += 3;
            continue;
        }
        // multi-char operators
        let two = if i + 1 < bytes.len() {
            match &bytes[i..i + 2] {
                b"->" => Some(Tok::Arrow),
                b"=>" => Some(Tok::FatArrow),
                b"==" => Some(Tok::EqEq),
                b"!=" => Some(Tok::Ne),
                b"<=" => Some(Tok::Le),
                b">=" => Some(Tok::Ge),
                b"&&" => Some(Tok::AmpAmp),
                b"||" => Some(Tok::PipePipe),
                b"<<" => Some(Tok::Shl),
                b">>" => Some(Tok::Shr),
                b"&=" => Some(Tok::AmpEq),
                b"|=" => Some(Tok::PipeEq),
                b"^=" => Some(Tok::CaretEq),
                b".." => Some(Tok::DotDot),
                b"+=" => Some(Tok::PlusEq),
                b"-=" => Some(Tok::MinusEq),
                b"*=" => Some(Tok::StarEq),
                b"/=" => Some(Tok::SlashEq),
                b"%=" => Some(Tok::PercentEq),
                _ => None,
            }
        } else {
            None
        };
        if let Some(t) = two {
            out.push(Token { tok: t, pos });
            i += 2;
            continue;
        }
        let one = match c {
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            b'{' => Tok::LBrace,
            b'}' => Tok::RBrace,
            b'[' => Tok::LBracket,
            b']' => Tok::RBracket,
            b';' => Tok::Semi,
            b',' => Tok::Comma,
            b':' if i + 1 < bytes.len() && bytes[i + 1] == b':' => {
                out.push(Token { tok: Tok::ColonColon, pos });
                i += 2;
                continue;
            }
            b':' => Tok::Colon,
            b'=' => Tok::Eq,
            b'<' => Tok::Lt,
            b'>' => Tok::Gt,
            b'+' => Tok::Plus,
            b'-' => Tok::Minus,
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'%' => Tok::Percent,
            b'&' => Tok::Amp,
            b'|' => Tok::Pipe,
            b'^' => Tok::Caret,
            b'!' => Tok::Bang,
            b'.' => Tok::Dot,
            _ => return Err(format!("unexpected character `{}` at byte {}", c as char, i)),
        };
        out.push(Token { tok: one, pos });
        i += 1;
    }
    Ok(out)
}

/// True when `digits` in `radix` is exactly `2^63`, the magnitude of
/// `i64::MIN`. `from_str_radix` rejects it because it exceeds `i64::MAX`, so the
/// lexer recognizes it structurally and lets the parser decide whether the
/// preceding `-` makes it valid.
fn is_i64_min_magnitude(digits: &str, radix: u32) -> bool {
    let trimmed = digits.trim_start_matches('0');
    match radix {
        10 => trimmed == "9223372036854775808",
        16 => trimmed.eq_ignore_ascii_case("8000000000000000"),
        2 => trimmed.len() == 64 && trimmed.starts_with('1') && trimmed[1..].bytes().all(|b| b == b'0'),
        8 => trimmed.len() == 22 && trimmed.starts_with('1') && trimmed[1..].bytes().all(|b| b == b'0'),
        _ => false,
    }
}
