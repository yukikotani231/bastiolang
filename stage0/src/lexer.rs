// Lexer. Tokens carry their source position so every later diagnostic can
// point at the exact place.

use crate::diag::{Diag, Pos};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(u128),
    Ident(String),
    // keywords
    Fn,
    Let,
    Mut,
    If,
    Else,
    While,
    Return,
    Requires,
    Ensures,
    Invariant,
    Assert,
    Forall,
    True,
    False,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
    ColonColon,
    Arrow,
    Amp,
    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Implies,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

pub fn lex(src: &str) -> Result<Vec<Token>, Diag> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut col = 1;
    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, col };
        // whitespace
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        // line comment
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                i += 1;
            }
            let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            col += i - start;
            let v: u128 = text
                .parse()
                .map_err(|_| Diag::new("E-PARSE", pos, format!("integer literal `{}` is too large", text)))?;
            if v > u64::MAX as u128 {
                return Err(Diag::new("E-PARSE", pos, format!("integer literal `{}` does not fit in u64", text)));
            }
            out.push(Token { tok: Tok::Int(v), pos });
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            col += i - start;
            let tok = match text.as_str() {
                "fn" => Tok::Fn,
                "let" => Tok::Let,
                "mut" => Tok::Mut,
                "if" => Tok::If,
                "else" => Tok::Else,
                "while" => Tok::While,
                "return" => Tok::Return,
                "requires" => Tok::Requires,
                "ensures" => Tok::Ensures,
                "invariant" => Tok::Invariant,
                "assert" => Tok::Assert,
                "forall" => Tok::Forall,
                "true" => Tok::True,
                "false" => Tok::False,
                _ => Tok::Ident(text),
            };
            out.push(Token { tok, pos });
            continue;
        }
        let two = if i + 1 < chars.len() { Some(chars[i + 1]) } else { None };
        let three = if i + 2 < chars.len() { Some(chars[i + 2]) } else { None };
        let (tok, len) = match (c, two, three) {
            ('=', Some('='), Some('>')) => (Tok::Implies, 3),
            ('=', Some('='), _) => (Tok::EqEq, 2),
            ('!', Some('='), _) => (Tok::Ne, 2),
            ('<', Some('='), _) => (Tok::Le, 2),
            ('>', Some('='), _) => (Tok::Ge, 2),
            ('&', Some('&'), _) => (Tok::AndAnd, 2),
            ('|', Some('|'), _) => (Tok::OrOr, 2),
            ('-', Some('>'), _) => (Tok::Arrow, 2),
            (':', Some(':'), _) => (Tok::ColonColon, 2),
            ('(', _, _) => (Tok::LParen, 1),
            (')', _, _) => (Tok::RParen, 1),
            ('{', _, _) => (Tok::LBrace, 1),
            ('}', _, _) => (Tok::RBrace, 1),
            (',', _, _) => (Tok::Comma, 1),
            (';', _, _) => (Tok::Semi, 1),
            (':', _, _) => (Tok::Colon, 1),
            ('&', _, _) => (Tok::Amp, 1),
            ('+', _, _) => (Tok::Plus, 1),
            ('-', _, _) => (Tok::Minus, 1),
            ('*', _, _) => (Tok::Star, 1),
            ('/', _, _) => (Tok::Slash, 1),
            ('%', _, _) => (Tok::Percent, 1),
            ('=', _, _) => (Tok::Eq, 1),
            ('<', _, _) => (Tok::Lt, 1),
            ('>', _, _) => (Tok::Gt, 1),
            ('!', _, _) => (Tok::Bang, 1),
            _ => return Err(Diag::new("E-PARSE", pos, format!("unexpected character `{}`", c))),
        };
        i += len;
        col += len;
        out.push(Token { tok, pos });
    }
    out.push(Token { tok: Tok::Eof, pos: Pos { line, col } });
    Ok(out)
}
