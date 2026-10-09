// Recursive-descent parser.
//
// Grammar (v0):
//   program  := fn*
//   fn       := 'fn' IDENT '(' (param (',' param)*)? ')' ('->' type)?
//               ('requires' expr | 'ensures' expr)* block
//   param    := 'mut'? IDENT ':' type
//   type     := 'u64' | 'bool' | 'Str' | 'Buf' | 'Serial' | '&' ('Buf' | 'Serial')
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

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
}

type PResult<T> = Result<T, Diag>;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Parser {
        Parser { toks, i: 0 }
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
        while *self.peek() != Tok::Eof {
            fns.push(self.fn_def()?);
        }
        Ok(Program { fns })
    }

    fn ty(&mut self) -> PResult<Type> {
        let pos = self.pos();
        if self.eat(&Tok::Amp) {
            let inner = self.ty()?;
            if !inner.is_linear() {
                return Err(Diag::new("E-PARSE", pos, format!("only linear values can be borrowed, found `&{}`", inner.name())));
            }
            return Ok(Type::Ref(Box::new(inner)));
        }
        let (n, _) = self.ident()?;
        match n.as_str() {
            "u64" => Ok(Type::U64),
            "bool" => Ok(Type::Bool),
            "Buf" => Ok(Type::Buf),
            "Serial" => Ok(Type::Serial),
            "Str" => Ok(Type::Str),
            _ => Err(Diag::new("E-PARSE", pos, format!("unknown type `{}`", n))),
        }
    }

    fn fn_def(&mut self) -> PResult<FnDef> {
        let pos = self.expect(Tok::Fn, "`fn`")?.pos;
        let (name, _) = self.ident()?;
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
