//! Hand-written lexer. Produces a flat token stream with byte offsets for errors.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    // literals / identifiers
    Ident(String),
    Int(i64),
    /// An integer literal with a width suffix (`200u8`, `1i64`, `0xffu32`).
    /// The value is the raw magnitude; the suffix names the width, which the
    /// parser types at that width and keeps in it for later arithmetic.
    /// A suffix that names the native width (`i64`/`u64`/`isize`/`usize`) keeps
    /// the plain `i64` value.
    IntSuffixed(i64, String),
    /// The magnitude `2^63`, which does not fit an `i64` but is the operand of
    /// the `i64::MIN` literal `-9223372036854775808`. The parser folds it into
    /// `Expr::Int(i64::MIN)` only when it directly follows a `-`; anywhere else
    /// it is out of range, matching rustc.
    IntMinMagnitude,
    Str(String),
    /// A character literal, `'A'`, holding its Unicode scalar value (a `char`
    /// is 4 bytes in Rust, but every value rmc accepts fits a `u32`).
    Char(u32),
    /// A loop label, `'name` (the quote is part of the token). Rust labels are
    /// written `'outer: loop { .. }` and targeted by `break 'outer` /
    /// `continue 'outer`.
    Label(String),
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
    Type,
    For,
    In,
    True,
    False,
    // types
    TyI64,
    TyBool,
    TyChar,
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
    DotDotEq,
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
    /// `@`, the binding operator in a pattern: `name @ subpat`.
    At,
}

impl Tok {
    /// Human readable name used in diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("identifier `{s}`"),
            Tok::Int(v) => format!("integer `{v}`"),
            Tok::IntSuffixed(v, s) => format!("integer `{v}{s}`"),
            Tok::IntMinMagnitude => "integer `9223372036854775808`".into(),
            Tok::Str(_) => "string literal".into(),
            Tok::Char(c) => format!("character `{}`", char::from_u32(*c).unwrap_or('\u{fffd}')),
            Tok::Label(n) => format!("label `'{n}`"),
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
            Tok::Type => "`type`".into(),
            Tok::For => "`for`".into(),
            Tok::In => "`in`".into(),
            Tok::True => "`true`".into(),
            Tok::False => "`false`".into(),
            Tok::TyI64 => "`i64`".into(),
            Tok::TyBool => "`bool`".into(),
            Tok::TyChar => "`char`".into(),
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
            Tok::DotDotEq => "..=",
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
            Tok::At => "@",
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
        // A byte literal `b'A'`: its value is the byte, an integer. rmc has no
        // separate `u8` type (`u8` is an alias for `i64`), so it lexes to an
        // `i64` holding the byte value. `b".."` (a byte string) is not handled;
        // it would need an array of bytes, which rmc does not have.
        if c == b'b' && bytes.get(i + 1) == Some(&b'\'') {
            let (val, next) = lex_byte_lit(bytes, i)?;
            out.push(Token { tok: Tok::Int(val), pos });
            i = next;
            continue;
        }
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
                "type" => Tok::Type,
                "for" => Tok::For,
                "in" => Tok::In,
                "true" => Tok::True,
                "false" => Tok::False,
                "i64" => Tok::TyI64,
                "bool" => Tok::TyBool,
                "char" => Tok::TyChar,
                "_" => Tok::Underscore,
                _ => Tok::Ident(s.to_string()),
            };
            out.push(Token { tok, pos });
            continue;
        }
        // A character literal `'A'`, or a loop label `'name`. A `'` starts a
        // character literal when the next character is a backslash escape or the
        // character after it is a closing `'`; otherwise it is a label. This is
        // exactly how rustc disambiguates the two (a label is never followed by
        // a lone quote).
        if c == b'\'' {
            let start = i;
            let is_char = if bytes.get(i + 1) == Some(&b'\\') {
                true
            } else if bytes.get(i + 1) == Some(&b'\'') {
                // `''`: an empty character literal, not a label (a label is
                // never just a quote).
                true
            } else if let Some(&b) = bytes.get(i + 1) {
                // A multi-byte character's bytes are all >= 0x80; step over the
                // UTF-8 sequence to find the closing quote.
                let len = utf8_len(b);
                bytes.get(i + 1 + len) == Some(&b'\'')
            } else {
                false
            };
            if is_char {
                let (val, next) = lex_char(src, bytes, start)?;
                out.push(Token { tok: Tok::Char(val), pos: start as u32 });
                i = next;
                continue;
            }
            i += 1;
            if i >= bytes.len() || !(bytes[i] == b'_' || bytes[i].is_ascii_alphabetic()) {
                return Err(format!("expected a label name after `'` at byte {start}"));
            }
            let name_start = i;
            while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let name = src[name_start..i].to_string();
            out.push(Token { tok: Tok::Label(name), pos: start as u32 });
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
            let (body, radix) = if let Some(h) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
                (h, 16)
            } else if let Some(b) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
                (b, 2)
            } else if let Some(o) = raw.strip_prefix("0o").or_else(|| raw.strip_prefix("0O")) {
                (o, 8)
            } else {
                (raw.as_str(), 10)
            };
            // A trailing integer suffix (`1i64`, `0u8`, `0xffu32`) names the
            // literal's width: split off the leading run of radix digits and
            // check the rest against the known suffixes. The suffix is kept on
            // the token so the parser can narrow the value the way rustc does
            // (`200u8` is `200`, `300u8` is an error); an unsuffixed literal is `i64`.
            let digits_len = body
                .find(|c: char| c.to_digit(radix).is_none())
                .unwrap_or(body.len());
            let (digits, suffix) = body.split_at(digits_len);
            if !suffix.is_empty() && !is_int_suffix(suffix) {
                return Err(format!("invalid integer literal `{raw}` at byte {start}"));
            }
            let v = i64::from_str_radix(digits, radix);
            let tok = match v {
                Ok(v) => {
                    if suffix.is_empty() {
                        Tok::Int(v)
                    } else {
                        Tok::IntSuffixed(v, suffix.to_string())
                    }
                }
                // The one magnitude above `i64::MAX` that a `-` can make valid:
                // the operand of `i64::MIN`. Defer the sign check to the parser.
                Err(_) if is_i64_min_magnitude(digits, radix) => Tok::IntMinMagnitude,
                Err(_) => {
                    return Err(format!("invalid integer literal `{raw}` at byte {start}"));
                }
            };
            out.push(Token { tok, pos });
            continue;
        }
        // three-char operators (must precede `<<`/`>>`)
        let three = if i + 2 < bytes.len() {
            match &bytes[i..i + 3] {
                b"<<=" => Some(Tok::ShlEq),
                b">>=" => Some(Tok::ShrEq),
                b"..=" => Some(Tok::DotDotEq),
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
            b'@' => Tok::At,
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

/// True when `s` is a Rust integer-literal suffix (`i8`..`i128`, `u8`..`u128`,
/// `isize`/`usize`). The suffix names the literal's width; the parser narrows
/// the value at it and keeps it in the width, so a Rust source that
/// names a width compiles and behaves as rustc does.
fn is_int_suffix(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
    )
}

/// The length in bytes of the UTF-8 sequence that starts with `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        // A continuation or invalid byte; treat it as one byte so the scan
        // terminates and the caller reports the malformed literal.
        1
    }
}

/// Lex the character literal that starts at the `'` at `start`. Returns the
/// scalar value and the index just past the closing `'`.
fn lex_char(src: &str, bytes: &[u8], start: usize) -> Result<(u32, usize), String> {
    let mut i = start + 1;
    let ch = if bytes[i] == b'\\' {
        let e = *bytes
            .get(i + 1)
            .ok_or_else(|| format!("unterminated character literal at byte {start}"))?;
        i += 2;
        match e {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'0' => '\0',
            b'\\' => '\\',
            b'\'' => '\'',
            b'"' => '"',
            b'x' => {
                // `'\xNN'`, exactly two hex digits, and rustc restricts a
                // character's `\x` escape to the ASCII range (`\xFF` is a valid
                // byte escape but not a `char`).
                let hi = hex_val(*bytes.get(i).unwrap_or(&0))
                    .ok_or_else(|| format!("invalid `\\x` escape at byte {start}"))?;
                let lo = hex_val(*bytes.get(i + 1).unwrap_or(&0))
                    .ok_or_else(|| format!("invalid `\\x` escape at byte {start}"))?;
                i += 2;
                let v = hi * 16 + lo;
                if v > 0x7F {
                    return Err(format!(
                        "out of range hex escape `\\x{v:02x}` at byte {start} (must be in [\\x00-\\x7f])"
                    ));
                }
                v as u8 as char
            }
            b'u' => {
                // `'\u{NNNN}'`: `{`, hex digits, `}`.
                if bytes.get(i) != Some(&b'{') {
                    return Err(format!("expected `{{` after `\\u` at byte {start}"));
                }
                i += 1;
                let ds = i;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                let hex = &src[ds..i];
                i += 1; // `}`
                let v = u32::from_str_radix(hex, 16)
                    .map_err(|_| format!("invalid `\\u{{..}}` escape at byte {start}"))?;
                char::from_u32(v)
                    .ok_or_else(|| format!("`\\u{{{hex}}}` is not a Unicode scalar at byte {start}"))?
            }
            other => {
                return Err(format!("unknown escape `\\{}` at byte {}", other as char, start))
            }
        }
    } else {
        // An empty literal `''` and a bare unescaped quote `'''` both land
        // here with the quote as the candidate character. rustc tells them
        // apart by what follows: a second quote means the quote itself was
        // written unescaped, otherwise the literal is empty.
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                return Err(format!("character constant must be escaped at byte {start}"));
            }
            return Err(format!("empty character literal at byte {start}"));
        }
        // A whole character: take its UTF-8 length, then decode.
        let len = utf8_len(bytes[i]);
        let s = src
            .get(i..i + len)
            .ok_or_else(|| format!("unterminated character literal at byte {start}"))?;
        let mut cs = s.chars();
        let ch = cs
            .next()
            .ok_or_else(|| format!("empty character literal at byte {start}"))?;
        i += len;
        ch
    };
    if bytes.get(i) != Some(&b'\'') {
        return Err(format!("unterminated character literal at byte {start}"));
    }
    Ok((ch as u32, i + 1))
}

fn hex_val(b: u8) -> Option<u32> {
    (b as char).to_digit(16)
}

/// A byte literal `b'A'` / `b'\n'` / `b'\xFF'`. Its value is the byte (0..=255),
/// returned as an `i64` (rmc has no `u8`). Only a single ASCII character or a
/// byte escape is allowed: rustc rejects a non-ASCII character (`b'é'`), since a
/// byte literal holds one byte. `\xFF` is accepted here (unlike in a `char`
/// literal, where `\x` is restricted to ASCII), and `\u{..}` is rejected, as in
/// rustc. `start` is the index of the leading `b`.
fn lex_byte_lit(bytes: &[u8], start: usize) -> Result<(i64, usize), String> {
    let mut i = start + 2; // past `b'`
    let first = *bytes
        .get(i)
        .ok_or_else(|| format!("unterminated byte literal at byte {start}"))?;
    let v: u32 = if first == b'\\' {
        let e = *bytes
            .get(i + 1)
            .ok_or_else(|| format!("unterminated byte literal at byte {start}"))?;
        i += 2;
        match e {
            b'n' => b'\n' as u32,
            b't' => b'\t' as u32,
            b'r' => b'\r' as u32,
            b'0' => 0,
            b'\\' => b'\\' as u32,
            b'\'' => b'\'' as u32,
            b'"' => b'"' as u32,
            b'x' => {
                // Exactly two hex digits, up to `\xFF`.
                let hi = hex_val(*bytes.get(i).unwrap_or(&0))
                    .ok_or_else(|| format!("invalid `\\x` escape at byte {start}"))?;
                let lo = hex_val(*bytes.get(i + 1).unwrap_or(&0))
                    .ok_or_else(|| format!("invalid `\\x` escape at byte {start}"))?;
                i += 2;
                hi * 16 + lo
            }
            b'u' => {
                return Err(format!(
                    "`\\u` is not allowed in a byte literal at byte {start}"
                ))
            }
            other => {
                return Err(format!("unknown escape `\\{}` at byte {}", other as char, start))
            }
        }
    } else {
        // A single character, which must be ASCII (one byte).
        let b = *bytes
            .get(i)
            .ok_or_else(|| format!("unterminated byte literal at byte {start}"))?;
        if b >= 0x80 {
            return Err(format!(
                "non-ASCII character in a byte literal at byte {start}"
            ));
        }
        i += 1;
        b as u32
    };
    if bytes.get(i) != Some(&b'\'') {
        return Err(format!("unterminated byte literal at byte {start}"));
    }
    Ok((v as i64, i + 1))
}
