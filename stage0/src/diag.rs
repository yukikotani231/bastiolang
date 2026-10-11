// Diagnostics. Every error carries a stable code (E-PARSE, E-LINEAR-LEAK, ...)
// so the test suite can assert *why* a program was rejected, not just that it was.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pos {
    /// Index into the list of input files.
    pub file: usize,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct Diag {
    pub code: &'static str,
    pub pos: Pos,
    pub msg: String,
    pub notes: Vec<String>,
}

impl Diag {
    pub fn new(code: &'static str, pos: Pos, msg: impl Into<String>) -> Diag {
        Diag { code, pos, msg: msg.into(), notes: Vec::new() }
    }

    pub fn note(mut self, n: impl Into<String>) -> Diag {
        self.notes.push(n.into());
        self
    }

    /// `files` holds (path, source) for every input file. The source line
    /// is shown with its number, the token at the error underlined, and the
    /// notes under it:
    ///
    ///     t.bastio:2:10: error[E-TYPE]: unknown function `foo`
    ///      2 |   return foo(1);
    ///        |          ^^^
    ///        = a note
    pub fn render(&self, files: &[(String, String)]) -> String {
        let (file, src) = files.get(self.pos.file).map(|(f, s)| (f.as_str(), s.as_str())).unwrap_or(("?", ""));
        let mut s = format!("{}:{}:{}: error[{}]: {}\n", file, self.pos.line, self.pos.col, self.code, self.msg);
        let num = self.pos.line.to_string();
        let pad = " ".repeat(num.len() + 1);
        if let Some(line) = src.lines().nth(self.pos.line.saturating_sub(1)) {
            s.push_str(&format!(" {} | {}\n", num, line));
            // Columns count characters, as the lexer counts them.
            let cs: Vec<char> = line.chars().collect();
            let at = self.pos.col.saturating_sub(1);
            // Tabs before the error stay tabs, so the caret lines up.
            let mut under = String::new();
            for i in 0..at {
                under.push(if cs.get(i) == Some(&'\t') { '\t' } else { ' ' });
            }
            under.push_str(&"^".repeat(token_len(&cs, at)));
            s.push_str(&format!("{} | {}\n", pad, under));
        }
        for n in &self.notes {
            s.push_str(&format!("{} = {}\n", pad, n));
        }
        s
    }
}

/// The length of the token starting at character `at` of `line`: a word
/// (a name or a number), a string literal, or else one character.
pub fn token_len(line: &[char], at: usize) -> usize {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    match line.get(at) {
        Some(&c) if word(c) => line[at..].iter().take_while(|&&c| word(c)).count(),
        Some(&'"') => {
            let mut k = at + 1;
            while k < line.len() && line[k] != '"' {
                k += if line[k] == '\\' { 2 } else { 1 };
            }
            k.min(line.len() - 1) + 1 - at
        }
        _ => 1,
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: error[{}]: {}", self.pos.line, self.pos.col, self.code, self.msg)
    }
}
