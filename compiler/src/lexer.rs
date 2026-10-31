//! Hand-written lexer. Produces a flat token stream with byte offsets for errors.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    // literals / identifiers
    Ident(String),
    Int(i64),
    Str(String),
    // keywords
    Fn,
    Let,
    Mut,
    While,
    Loop,
    If,
    Else,
    Return,
    Break,
    Continue,
    As,
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
    Arrow,
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
    AmpAmp,
    PipePipe,
    Bang,
    Underscore,
}

impl Tok {
    /// Human readable name used in diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("identifier `{s}`"),
            Tok::Int(v) => format!("integer `{v}`"),
            Tok::Str(_) => "string literal".into(),
            Tok::Fn => "`fn`".into(),
            Tok::Let => "`let`".into(),
            Tok::Mut => "`mut`".into(),
            Tok::While => "`while`".into(),
            Tok::Loop => "`loop`".into(),
            Tok::If => "`if`".into(),
            Tok::Else => "`else`".into(),
            Tok::Return => "`return`".into(),
            Tok::Break => "`break`".into(),
            Tok::Continue => "`continue`".into(),
            Tok::As => "`as`".into(),
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
            Tok::Arrow => "->",
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
            Tok::AmpAmp => "&&",
            Tok::PipePipe => "||",
            Tok::Bang => "!",
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
                "return" => Tok::Return,
                "break" => Tok::Break,
                "continue" => Tok::Continue,
                "as" => Tok::As,
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
            let mut s = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                let b = bytes[i];
                if b == b'\\' && i + 1 < bytes.len() {
                    let e = bytes[i + 1];
                    i += 2;
                    match e {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'0' => s.push('\0'),
                        b'\\' => s.push('\\'),
                        b'"' => s.push('"'),
                        other => {
                            return Err(format!("unknown escape `\\{}` at byte {}", other as char, i - 2))
                        }
                    }
                } else {
                    s.push(b as char);
                    i += 1;
                }
            }
            if i >= bytes.len() {
                return Err(format!("unterminated string literal at byte {start}"));
            }
            i += 1; // closing quote
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
            let v = i64::from_str_radix(digits, radix)
                .map_err(|_| format!("invalid integer literal `{raw}` at byte {start}"))?;
            out.push(Token { tok: Tok::Int(v), pos });
            continue;
        }
        // multi-char operators
        let two = if i + 1 < bytes.len() {
            match &bytes[i..i + 2] {
                b"->" => Some(Tok::Arrow),
                b"==" => Some(Tok::EqEq),
                b"!=" => Some(Tok::Ne),
                b"<=" => Some(Tok::Le),
                b">=" => Some(Tok::Ge),
                b"&&" => Some(Tok::AmpAmp),
                b"||" => Some(Tok::PipePipe),
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
            b':' => Tok::Colon,
            b'=' => Tok::Eq,
            b'<' => Tok::Lt,
            b'>' => Tok::Gt,
            b'+' => Tok::Plus,
            b'-' => Tok::Minus,
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'%' => Tok::Percent,
            b'!' => Tok::Bang,
            _ => return Err(format!("unexpected character `{}` at byte {}", c as char, i)),
        };
        out.push(Token { tok: one, pos });
        i += 1;
    }
    Ok(out)
}
