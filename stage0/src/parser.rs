// Recursive-descent parser.
//
// Grammar:
//   program  := (fn | specfn | const)*
//   fn       := 'fn' IDENT '(' (param (',' param)*)? ')' ('->' type)?
//               ('requires' expr | 'ensures' expr)* block
//   specfn   := 'spec' 'fn' IDENT '(' params ')' '->' type '{' expr '}'
//   const    := 'const' IDENT ':' 'u64' '=' expr ';'   (constant-folded; define before use)
//   param    := 'mut'? IDENT ':' type
//   type     := 'u64' | 'bool' | 'Str' | linear | '&' linear | '&' 'mut' linear
//   linear   := 'Buf' | 'Serial' | 'Frame' | 'PageTable' | 'Pool' | 'Mmu' | 'Cpu'
//   primary  := ... | '&' 'mut' IDENT | 'include_bytes' '(' STRING ')' | 'old' '(' expr ')' 
//   block    := '{' stmt* '}'
//   stmt     := 'let' 'mut'? IDENT (':' type)? '=' expr ';'
//             | IDENT '=' expr ';'
//             | 'if' expr block ('else' (block | if-stmt))?
//             | 'while' expr ('invariant' expr)* block
//             | 'return' expr? ';'
//             | 'assert' expr ';'
//             | expr ';'
//   expr precedence, lowest first: '==>' (right) < '||' < '&&' < comparisons
//             < '+' '-' < '*' '/' '%' < '!' < call / primary

use crate::ast::*;
use crate::diag::{Diag, Pos};
use crate::lexer::{Tok, Token};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    /// Directory of the file being parsed, for `include_bytes`.
    dir: PathBuf,
    /// Constants defined so far (in this and earlier files).
    pub consts: HashMap<String, u128>,
}

type PResult<T> = Result<T, Diag>;

impl Parser {
    pub fn new(toks: Vec<Token>, dir: PathBuf, consts: HashMap<String, u128>) -> Parser {
        Parser { toks, i: 0, dir, consts }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn pos(&self) -> Pos {
        self.toks[self.i].pos
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Tok, what: &str) -> PResult<Token> {
        if *self.peek() == t {
            Ok(self.bump())
        } else {
            Err(Diag::new("E-PARSE", self.pos(), format!("expected {}, found {:?}", what, self.peek())))
        }
    }

    fn ident(&mut self) -> PResult<(String, Pos)> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok((s, pos))
            }
            t => Err(Diag::new("E-PARSE", pos, format!("expected identifier, found {:?}", t))),
        }
    }

    pub fn program(&mut self) -> PResult<Program> {
        let mut fns = Vec::new();
        let mut specs = Vec::new();
        while *self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Spec => specs.push(self.spec_def()?),
                Tok::Const => self.const_def()?,
                _ => fns.push(self.fn_def()?),
            }
        }
        Ok(Program { fns, specs })
    }

    fn const_def(&mut self) -> PResult<()> {
        self.expect(Tok::Const, "`const`")?;
        let (name, pos) = self.ident()?;
        self.expect(Tok::Colon, "`:`")?;
        if self.ty()? != Type::U64 {
            return Err(Diag::new("E-PARSE", pos, "constants must be u64"));
        }
        self.expect(Tok::Eq, "`=`")?;
        let e = self.expr()?;
        self.expect(Tok::Semi, "`;`")?;
        let v = const_eval(&e)?;
        if self.consts.contains_key(&name) {
            return Err(Diag::new("E-PARSE", pos, format!("constant `{}` is defined twice", name)));
        }
        self.consts.insert(name, v);
        Ok(())
    }

    fn spec_def(&mut self) -> PResult<SpecFn> {
        let pos = self.expect(Tok::Spec, "`spec`")?.pos;
        self.expect(Tok::Fn, "`fn` after `spec`")?;
        let (name, _) = self.ident()?;
        let params = self.params()?;
        self.expect(Tok::Arrow, "`->`")?;
        let ret = self.ty()?;
        self.expect(Tok::LBrace, "`{`")?;
        let body = self.expr()?;
        self.expect(Tok::RBrace, "`}`")?;
        Ok(SpecFn { name, params, ret, body, pos })
    }

    fn ty(&mut self) -> PResult<Type> {
        let pos = self.pos();
        if self.eat(&Tok::Amp) {
            let m = self.eat(&Tok::Mut);
            let inner = self.ty()?;
            if !inner.is_linear() {
                return Err(Diag::new("E-PARSE", pos, format!("only linear values can be borrowed, found `&{}`", inner.name())));
            }
            return Ok(if m { Type::MutRef(Box::new(inner)) } else { Type::Ref(Box::new(inner)) });
        }
        let (n, _) = self.ident()?;
        match n.as_str() {
            "u64" => Ok(Type::U64),
            "bool" => Ok(Type::Bool),
            "Buf" => Ok(Type::Buf),
            "Serial" => Ok(Type::Serial),
            "Str" => Ok(Type::Str),
            "Frame" => Ok(Type::Frame),
            "PageTable" => Ok(Type::PageTable),
            "Pool" => Ok(Type::Pool),
            "Mmu" => Ok(Type::Mmu),
            "Cpu" => Ok(Type::Cpu),
            _ => Err(Diag::new("E-PARSE", pos, format!("unknown type `{}`", n))),
        }
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                let ppos = self.pos();
                let mutable = self.eat(&Tok::Mut);
                let (pname, _) = self.ident()?;
                self.expect(Tok::Colon, "`:`")?;
                let ty = self.ty()?;
                params.push(Param { name: pname, mutable, ty, pos: ppos });
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(params)
    }

    fn fn_def(&mut self) -> PResult<FnDef> {
        let pos = self.expect(Tok::Fn, "`fn`")?.pos;
        let (name, _) = self.ident()?;
        let params = self.params()?;
        let ret = if self.eat(&Tok::Arrow) { self.ty()? } else { Type::Unit };
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        loop {
            if self.eat(&Tok::Requires) {
                requires.push(self.expr()?);
            } else if self.eat(&Tok::Ensures) {
                ensures.push(self.expr()?);
            } else {
                break;
            }
        }
        let body = self.block()?;
        Ok(FnDef { name, params, ret, requires, ensures, body, pos })
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(Tok::LBrace, "`{`")?;
        let mut out = Vec::new();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(Diag::new("E-PARSE", self.pos(), "unexpected end of file, missing `}`"));
            }
            out.push(self.stmt()?);
        }
        self.bump();
        Ok(out)
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Let => {
                self.bump();
                let mutable = self.eat(&Tok::Mut);
                let (name, _) = self.ident()?;
                let ty = if self.eat(&Tok::Colon) { Some(self.ty()?) } else { None };
                self.expect(Tok::Eq, "`=`")?;
                let init = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt { kind: StmtKind::Let { name, mutable, ty, init }, pos })
            }
            Tok::If => self.if_stmt(),
            Tok::While => {
                self.bump();
                let cond = self.expr()?;
                let mut invariants = Vec::new();
                while self.eat(&Tok::Invariant) {
                    invariants.push(self.expr()?);
                }
                let body = self.block()?;
                Ok(Stmt { kind: StmtKind::While { cond, invariants, body }, pos })
            }
            Tok::Return => {
                self.bump();
                let e = if *self.peek() == Tok::Semi { None } else { Some(self.expr()?) };
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt { kind: StmtKind::Return(e), pos })
            }
            Tok::Assert => {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt { kind: StmtKind::Assert(e), pos })
            }
            Tok::Ident(name) if self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::Eq) => {
                self.bump();
                self.bump();
                let value = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt { kind: StmtKind::Assign { name, value }, pos })
            }
            _ => {
                let e = self.expr()?;
                self.expect(Tok::Semi, "`;`")?;
                Ok(Stmt { kind: StmtKind::Expr(e), pos })
            }
        }
    }

    fn if_stmt(&mut self) -> PResult<Stmt> {
        let pos = self.expect(Tok::If, "`if`")?.pos;
        let cond = self.expr()?;
        let then_b = self.block()?;
        let else_b = if self.eat(&Tok::Else) {
            if *self.peek() == Tok::If {
                vec![self.if_stmt()?]
            } else {
                self.block()?
            }
        } else {
            Vec::new()
        };
        Ok(Stmt { kind: StmtKind::If { cond, then_b, else_b }, pos })
    }

    pub fn expr(&mut self) -> PResult<Expr> {
        self.implies()
    }

    fn implies(&mut self) -> PResult<Expr> {
        let lhs = self.or()?;
        if *self.peek() == Tok::Implies {
            let pos = self.bump().pos;
            let rhs = self.implies()?;
            return Ok(Expr { kind: ExprKind::Bin(BinOp::Implies, Box::new(lhs), Box::new(rhs)), pos });
        }
        Ok(lhs)
    }

    fn or(&mut self) -> PResult<Expr> {
        let mut lhs = self.and()?;
        while *self.peek() == Tok::OrOr {
            let pos = self.bump().pos;
            let rhs = self.and()?;
            lhs = Expr { kind: ExprKind::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs)), pos };
        }
        Ok(lhs)
    }

    fn and(&mut self) -> PResult<Expr> {
        let mut lhs = self.cmp()?;
        while *self.peek() == Tok::AndAnd {
            let pos = self.bump().pos;
            let rhs = self.cmp()?;
            lhs = Expr { kind: ExprKind::Bin(BinOp::And, Box::new(lhs), Box::new(rhs)), pos };
        }
        Ok(lhs)
    }

    fn cmp(&mut self) -> PResult<Expr> {
        let lhs = self.add()?;
        let op = match self.peek() {
            Tok::EqEq => BinOp::Eq,
            Tok::Ne => BinOp::Ne,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            _ => return Ok(lhs),
        };
        let pos = self.bump().pos;
        let rhs = self.add()?;
        Ok(Expr { kind: ExprKind::Bin(op, Box::new(lhs), Box::new(rhs)), pos })
    }

    fn add(&mut self) -> PResult<Expr> {
        let mut lhs = self.mul()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(lhs),
            };
            let pos = self.bump().pos;
            let rhs = self.mul()?;
            lhs = Expr { kind: ExprKind::Bin(op, Box::new(lhs), Box::new(rhs)), pos };
        }
    }

    fn mul(&mut self) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Rem,
                _ => return Ok(lhs),
            };
            let pos = self.bump().pos;
            let rhs = self.unary()?;
            lhs = Expr { kind: ExprKind::Bin(op, Box::new(lhs), Box::new(rhs)), pos };
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        if *self.peek() == Tok::Bang {
            let pos = self.bump().pos;
            let e = self.unary()?;
            return Ok(Expr { kind: ExprKind::Not(Box::new(e)), pos });
        }
        self.primary()
    }

    fn primary(&mut self) -> PResult<Expr> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Int(v), pos })
            }
            Tok::True => {
                self.bump();
                Ok(Expr { kind: ExprKind::Bool(true), pos })
            }
            Tok::Str(b) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Str(b), pos })
            }
            Tok::False => {
                self.bump();
                Ok(Expr { kind: ExprKind::Bool(false), pos })
            }
            Tok::LParen => {
                self.bump();
                let e = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            Tok::Forall => {
                self.bump();
                let (name, _) = self.ident()?;
                self.expect(Tok::Colon, "`:`")?;
                let ty = self.ty()?;
                if ty != Type::U64 {
                    return Err(Diag::new("E-PARSE", pos, "forall only ranges over u64 in v0"));
                }
                self.expect(Tok::ColonColon, "`::`")?;
                let body = self.expr()?;
                Ok(Expr { kind: ExprKind::Forall(name, Box::new(body)), pos })
            }
            Tok::Amp => {
                self.bump();
                self.expect(Tok::Mut, "`mut` (a shared borrow `&x` is written as just `x`)")?;
                let (n, _) = self.ident()?;
                Ok(Expr { kind: ExprKind::MutBorrow(n), pos })
            }
            Tok::Ident(name) if name == "include_bytes" => {
                self.bump();
                self.expect(Tok::LParen, "`(`")?;
                let path = match self.bump().tok {
                    Tok::Str(b) => String::from_utf8_lossy(&b).to_string(),
                    _ => return Err(Diag::new("E-PARSE", pos, "include_bytes takes a string literal path")),
                };
                self.expect(Tok::RParen, "`)`")?;
                let full = self.dir.join(&path);
                let bytes = std::fs::read(&full).map_err(|e| Diag::new("E-PARSE", pos, format!("include_bytes: cannot read {}: {}", full.display(), e)))?;
                Ok(Expr { kind: ExprKind::Blob(bytes), pos })
            }
            Tok::Ident(name) if self.consts.contains_key(&name) && self.toks.get(self.i + 1).map(|t| &t.tok) != Some(&Tok::LParen) => {
                self.bump();
                Ok(Expr { kind: ExprKind::Int(self.consts[&name]), pos })
            }
            Tok::Ident(name) => {
                self.bump();
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen, "`)`")?;
                    Ok(Expr { kind: ExprKind::Call(name, args), pos })
                } else {
                    Ok(Expr { kind: ExprKind::Var(name), pos })
                }
            }
            t => Err(Diag::new("E-PARSE", pos, format!("expected expression, found {:?}", t))),
        }
    }
}

/// Folds a constant initializer. Only literals, earlier constants and
/// arithmetic; the result must fit in u64.
fn const_eval(e: &Expr) -> PResult<u128> {
    let bad = |m: &str| Diag::new("E-PARSE", e.pos, format!("constant initializer: {}", m));
    match &e.kind {
        ExprKind::Int(v) => Ok(*v),
        ExprKind::Bin(op, a, b) => {
            let (x, y) = (const_eval(a)?, const_eval(b)?);
            let r = match op {
                BinOp::Add => x.checked_add(y),
                BinOp::Sub => x.checked_sub(y),
                BinOp::Mul => x.checked_mul(y),
                BinOp::Div => x.checked_div(y),
                BinOp::Rem => x.checked_rem(y),
                _ => return Err(bad("only + - * / % are allowed")),
            };
            r.filter(|v| *v <= u64::MAX as u128).ok_or_else(|| bad("the value is out of the u64 range"))
        }
        _ => Err(bad("only integer literals, earlier constants and arithmetic are allowed")),
    }
}
