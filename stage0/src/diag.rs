// Diagnostics. Every error carries a stable code (E-PARSE, E-LINEAR-LEAK, ...)
// so the test suite can assert *why* a program was rejected, not just that it was.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pos {
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

    pub fn render(&self, file: &str, src: &str) -> String {
        let mut s = format!("{}:{}:{}: error[{}]: {}\n", file, self.pos.line, self.pos.col, self.code, self.msg);
        if let Some(line) = src.lines().nth(self.pos.line.saturating_sub(1)) {
            s.push_str(&format!("  | {}\n", line));
            s.push_str(&format!("  | {}^\n", " ".repeat(self.pos.col.saturating_sub(1))));
        }
        for n in &self.notes {
            s.push_str(&format!("  = {}\n", n));
        }
        s
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: error[{}]: {}", self.pos.line, self.pos.col, self.code, self.msg)
    }
}
