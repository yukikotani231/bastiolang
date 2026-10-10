// Talks to an external SMT solver (Z3) over SMT-LIB 2 text on a pipe.
//
// The solver is a trusted external tool: the compiler only writes questions
// and reads answers, so swapping Z3 for cvc5 means changing `SOLVER_CMD`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub const SOLVER_CMD: &str = "z3";
pub const SOLVER_ARGS: &[&str] = &["-in", "-smt2"];
/// The work Z3 may spend on one query before it answers `unknown`, in its
/// resource units (`:rlimit`, counted per `check-sat`). A resource limit and
/// not a timeout, so that whether a proof succeeds never depends on how
/// loaded the machine is: the same question gets the same answer under
/// `make -j` as alone. 40 million units take about 10 s; the hardest query
/// in the BastiOS kernel and the compiler takes under 3 million.
pub const RLIMIT: u64 = 40_000_000;

pub struct Solver {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Every command sent, kept so `--dump-smt` can show the whole session.
    pub log: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum SatResult {
    Sat,
    Unsat,
    Unknown,
}

impl Solver {
    pub fn start() -> Result<Solver, String> {
        let solver = std::env::var("BASTIO_SOLVER").unwrap_or_else(|_| SOLVER_CMD.to_string());
        let mut child = Command::new(&solver)
            .args(SOLVER_ARGS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("could not start the SMT solver `{}`: {} (install Z3 and put `z3` on PATH)", solver, e))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Solver { child, stdin, stdout, log: Vec::new() };
        s.send("(set-option :produce-models true)")?;
        s.send(&format!("(set-option :rlimit {})", RLIMIT))?;
        Ok(s)
    }

    pub fn send(&mut self, cmd: &str) -> Result<(), String> {
        self.log.push(cmd.to_string());
        writeln!(self.stdin, "{}", cmd).map_err(|e| format!("solver pipe closed: {}", e))?;
        self.stdin.flush().map_err(|e| format!("solver pipe closed: {}", e))
    }

    /// Reads one complete response: a bare word or a balanced s-expression.
    fn read_response(&mut self) -> Result<String, String> {
        let mut out = String::new();
        let mut depth: i64 = 0;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).map_err(|e| format!("solver read failed: {}", e))?;
            if n == 0 {
                return Err("the SMT solver exited unexpectedly".into());
            }
            for c in line.chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            out.push_str(&line);
            if depth <= 0 && !out.trim().is_empty() {
                break;
            }
        }
        let r = out.trim().to_string();
        if r.starts_with("(error") {
            return Err(format!("solver rejected a query (compiler bug): {}", r));
        }
        Ok(r)
    }

    pub fn check_sat(&mut self) -> Result<SatResult, String> {
        self.send("(check-sat)")?;
        match self.read_response()?.as_str() {
            "sat" => Ok(SatResult::Sat),
            "unsat" => Ok(SatResult::Unsat),
            _ => Ok(SatResult::Unknown),
        }
    }

    /// Value of one term in the current model, as the solver prints it.
    pub fn get_value(&mut self, term: &str) -> Result<String, String> {
        self.send(&format!("(get-value ({}))", term))?;
        let r = self.read_response()?;
        // r looks like `((term value))`: drop the two outer parens, then take
        // the last balanced s-expression or atom.
        let inner = r.trim();
        let inner = inner.strip_prefix("((").and_then(|x| x.strip_suffix("))")).unwrap_or(inner).trim_end();
        Ok(pretty_value(last_sexp(inner)))
    }
}

/// The last balanced s-expression (or atom) in `s`.
fn last_sexp(s: &str) -> &str {
    let b = s.as_bytes();
    if b.last() == Some(&b')') {
        let mut depth = 0i64;
        for i in (0..b.len()).rev() {
            match b[i] {
                b')' => depth += 1,
                b'(' => {
                    depth -= 1;
                    if depth == 0 {
                        return &s[i..];
                    }
                }
                _ => {}
            }
        }
        s
    } else {
        s.rsplit(|c: char| c.is_whitespace()).next().unwrap_or(s)
    }
}

/// `(- 3)` -> `-3`; anything else unchanged.
fn pretty_value(v: &str) -> String {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix("(- ") {
        return format!("-{}", rest.trim_end_matches(')'));
    }
    v.to_string()
}

impl Drop for Solver {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "(exit)");
        let _ = self.child.wait();
    }
}
