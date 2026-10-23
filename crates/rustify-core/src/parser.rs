//! Recursive-descent parser for the supported Python subset.
//!
//! The grammar mirrors CPython's `Grammar/python.gram` but is restricted to
//! the constructs the transpiler can lower faithfully.

use crate::ast::*;
use crate::lexer::{Lexer, Tok};

#[derive(Debug, Clone)]
pub struct ParseError {
    pub line: usize,
    pub msg: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

pub struct Parser {
    toks: Vec<Tok>,
    lines: Vec<usize>,
    pos: usize,
}

/// Keywords that cannot start an expression.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

impl Parser {
    pub fn new(toks: Vec<Tok>, lines: Vec<usize>) -> Self {
        Parser {
            toks,
            lines,
            pos: 0,
        }
    }

    pub fn parse_module(source: &str) -> Result<Module, ParseError> {
        let (toks, lines) = Lexer::new(source).tokenize().map_err(|e| ParseError {
            line: e.line,
            msg: e.msg,
        })?;
        let mut p = Parser::new(toks, lines);
        let body = p.parse_statements_until_eof()?;
        Ok(Module { body })
    }

    // ------------------------------------------------------------------
    // Token helpers
    // ------------------------------------------------------------------

    fn line(&self) -> usize {
        self.lines.get(self.pos).copied().unwrap_or(0)
    }

    fn peek(&self) -> &Tok {
        self.toks.get(self.pos).unwrap_or(&Tok::Eof)
    }

    fn peek_at(&self, n: usize) -> &Tok {
        self.toks.get(self.pos + n).unwrap_or(&Tok::Eof)
    }

    fn advance(&mut self) -> Tok {
        let t = self.toks.get(self.pos).cloned().unwrap_or(Tok::Eof);
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            line: self.line(),
            msg: format!("{} (near {})", msg.into(), self.peek().describe()),
        })
    }

    fn at_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Name(n) if n == kw)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.at_op(op) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_op(&mut self, op: &str) -> Result<(), ParseError> {
        if self.eat_op(op) {
            Ok(())
        } else {
            self.err(format!("expected '{}'", op))
        }
    }

    fn expect_name(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Tok::Name(n) => Ok(n),
            _ => self.err("expected a name"),
        }
    }

    /// True when the current token starts a new logical line.
    fn at_logical_end(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::Eof | Tok::Dedent)
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn parse_statements_until_eof(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut out = Vec::new();
        loop {
            while matches!(self.peek(), Tok::Newline) {
                self.advance();
            }
            if matches!(self.peek(), Tok::Eof) {
                break;
            }
            if matches!(self.peek(), Tok::Indent) {
                return self.err("unexpected indent");
            }
            if matches!(self.peek(), Tok::Dedent) {
                self.advance();
                continue;
            }
            out.extend(self.parse_statement()?);
        }
        Ok(out)
    }

    /// Parse an indented block (after the `:` of a compound statement).
    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        // Either a simple statement on the same line, or an indented suite.
        if matches!(self.peek(), Tok::Newline) {
            self.advance();
            if !matches!(self.peek(), Tok::Indent) {
                return self.err("expected an indented block");
            }
            self.advance();
            let mut out = Vec::new();
            loop {
                while matches!(self.peek(), Tok::Newline) {
                    self.advance();
                }
                if matches!(self.peek(), Tok::Dedent) {
                    self.advance();
                    break;
                }
                if matches!(self.peek(), Tok::Eof) {
                    break;
                }
                out.extend(self.parse_statement()?);
            }
            Ok(out)
        } else {
            self.parse_simple_stmt_line()
        }
    }

    fn parse_statement(&mut self) -> Result<Vec<Stmt>, ParseError> {
        // Decorators
        if self.at_op("@") {
            let mut decorators = Vec::new();
            while self.eat_op("@") {
                decorators.push(self.parse_expr()?);
                // The lexer terminates a decorated line with a NEWLINE token.
                if matches!(self.peek(), Tok::Newline) {
                    self.advance();
                }
            }
            return Ok(vec![self.parse_decorated(decorators)?]);
        }
        match self.peek() {
            Tok::Name(n) => match n.as_str() {
                "if" => Ok(vec![self.parse_if()?]),
                "while" => Ok(vec![self.parse_while()?]),
                "for" => Ok(vec![self.parse_for()?]),
                "try" => Ok(vec![self.parse_try()?]),
                "with" => Ok(vec![self.parse_with()?]),
                "def" => Ok(vec![self.parse_funcdef(Vec::new())?]),
                "class" => Ok(vec![self.parse_classdef(Vec::new())?]),
                "async" => self.err("async def is not supported"),
                _ => self.parse_simple_stmt_line(),
            },
            _ => self.parse_simple_stmt_line(),
        }
    }

    fn parse_decorated(&mut self, decorators: Vec<Expr>) -> Result<Stmt, ParseError> {
        match self.peek() {
            Tok::Name(n) if n == "def" => self.parse_funcdef(decorators),
            Tok::Name(n) if n == "class" => self.parse_classdef(decorators),
            _ => self.err("expected a decorated function or class"),
        }
    }

    /// One logical line of simple statements (possibly `;`-separated).
    fn parse_simple_stmt_line(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut out = Vec::new();
        loop {
            out.push(self.parse_small_stmt()?);
            if self.eat_op(";") {
                if self.at_logical_end() {
                    break;
                }
                continue;
            }
            break;
        }
        // Consume the logical line terminator.
        if matches!(self.peek(), Tok::Newline) {
            self.advance();
        }
        Ok(out)
    }

    fn parse_small_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek() {
            Tok::Name(n) => {
                let n = n.clone();
                match n.as_str() {
                    "pass" => {
                        self.advance();
                        Ok(Stmt::Pass)
                    }
                    "break" => {
                        self.advance();
                        Ok(Stmt::Break)
                    }
                    "continue" => {
                        self.advance();
                        Ok(Stmt::Continue)
                    }
                    "return" => {
                        self.advance();
                        if self.at_logical_end() || self.at_op(";") {
                            return Ok(Stmt::Return(None));
                        }
                        let e = self.parse_expr_list_value()?;
                        Ok(Stmt::Return(Some(e)))
                    }
                    "raise" => {
                        self.advance();
                        if self.at_logical_end() || self.at_op(";") {
                            return Ok(Stmt::Raise {
                                exc: None,
                                cause: None,
                            });
                        }
                        let e = self.parse_expr()?;
                        // `raise X from Y`
                        let cause = if self.eat_kw("from") {
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        Ok(Stmt::Raise { exc: Some(e), cause })
                    }
                    "global" => {
                        self.advance();
                        let mut names = Vec::new();
                        loop {
                            names.push(self.expect_name()?);
                            if !self.eat_op(",") {
                                break;
                            }
                        }
                        Ok(Stmt::Global(names))
                    }
                    "nonlocal" => {
                        self.advance();
                        let mut names = Vec::new();
                        loop {
                            names.push(self.expect_name()?);
                            if !self.eat_op(",") {
                                break;
                            }
                        }
                        Ok(Stmt::Nonlocal(names))
                    }
                    "assert" => {
                        self.advance();
                        let test = self.parse_expr()?;
                        let msg = if self.eat_op(",") {
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        Ok(Stmt::Assert { test, msg })
                    }
                    "del" => {
                        self.advance();
                        let mut targets = Vec::new();
                        loop {
                            targets.push(self.parse_expr()?);
                            if !self.eat_op(",") {
                                break;
                            }
                        }
                        Ok(Stmt::Delete(targets))
                    }
                    "import" => self.parse_import(),
                    "from" => self.parse_from_import(),
                    _ => self.parse_expr_stmt(),
                }
            }
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_import(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // import
        let mut names = Vec::new();
        loop {
            let mut module = self.expect_name()?;
            while self.at_op(".") {
                self.advance();
                module.push('.');
                module.push_str(&self.expect_name()?);
            }
            let asname = if self.eat_kw("as") {
                Some(self.expect_name()?)
            } else {
                None
            };
            names.push((module, asname));
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(Stmt::Import { names })
    }

    fn parse_from_import(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // from
        let mut level = 0usize;
        while self.at_op(".") || self.at_op("...") {
            if self.eat_op("...") {
                level += 3;
            } else {
                self.advance();
                level += 1;
            }
        }
        let mut module = String::new();
        if !self.at_kw("import") {
            module = self.expect_name()?;
            while self.at_op(".") {
                self.advance();
                module.push('.');
                module.push_str(&self.expect_name()?);
            }
        }
        if !self.eat_kw("import") {
            return self.err("expected 'import'");
        }
        let mut names = Vec::new();
        let parenthesized = self.eat_op("(");
        loop {
            if self.at_op("*") {
                self.advance();
                names.push(("*".to_string(), None));
            } else {
                let name = self.expect_name()?;
                let asname = if self.eat_kw("as") {
                    Some(self.expect_name()?)
                } else {
                    None
                };
                names.push((name, asname));
            }
            if !self.eat_op(",") {
                break;
            }
            if parenthesized && self.at_op(")") {
                break;
            }
        }
        if parenthesized {
            self.expect_op(")")?;
        }
        Ok(Stmt::ImportFrom {
            module,
            names,
            level,
        })
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ParseError> {
        let first = self.parse_expr_list()?;
        if self.at_op("=") {
            let mut targets = vec![first];
            let mut value: Option<Expr> = None;
            while self.eat_op("=") {
                let e = self.parse_expr_list()?;
                if self.at_op("=") {
                    targets.push(e);
                } else {
                    value = Some(e);
                }
            }
            let value = match value {
                Some(v) => v,
                None => return self.err("assignment is missing a value"),
            };
            return Ok(Stmt::Assign { targets, value });
        }
        // Augmented assignment
        for (op, name) in [
            ("+=", BinOp::Add),
            ("-=", BinOp::Sub),
            ("*=", BinOp::Mul),
            ("/=", BinOp::Div),
            ("//=", BinOp::FloorDiv),
            ("%=", BinOp::Mod),
            ("**=", BinOp::Pow),
            ("<<=", BinOp::LShift),
            (">>=", BinOp::RShift),
            ("|=", BinOp::BitOr),
            ("&=", BinOp::BitAnd),
            ("^=", BinOp::BitXor),
        ] {
            if self.at_op(op) {
                self.advance();
                let value = self.parse_expr_list()?;
                return Ok(Stmt::AugAssign {
                    target: first,
                    op: name,
                    value,
                });
            }
        }
        // Annotated assignment
        if self.at_op(":") {
            self.advance();
            let annotation = self.parse_expr()?;
            let value = if self.eat_op("=") {
                Some(self.parse_expr_list()?)
            } else {
                None
            };
            return Ok(Stmt::AnnAssign {
                target: first,
                annotation,
                value,
            });
        }
        if first.is_name("__all__") {
            // still an expression statement
        }
        Ok(Stmt::ExprStmt(first))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.advance(); // if / elif
        let test = self.parse_expr()?;
        self.expect_op(":")?;
        let body = self.parse_block()?;
        let orelse = if self.at_kw("elif") {
            vec![self.parse_if()?]
        } else if self.eat_kw("else") {
            self.expect_op(":")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        Ok(Stmt::If { test, body, orelse })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let test = self.parse_expr()?;
        self.expect_op(":")?;
        let body = self.parse_block()?;
        let orelse = if self.eat_kw("else") {
            self.expect_op(":")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        Ok(Stmt::While { test, body, orelse })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let target = self.parse_target_list()?;
        if !self.eat_kw("in") {
            return self.err("expected 'in'");
        }
        let iter = self.parse_expr_list()?;
        self.expect_op(":")?;
        let body = self.parse_block()?;
        let orelse = if self.eat_kw("else") {
            self.expect_op(":")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        Ok(Stmt::For {
            target,
            iter,
            body,
            orelse,
        })
    }

    fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        self.expect_op(":")?;
        let body = self.parse_block()?;
        let mut handlers = Vec::new();
        while self.at_kw("except") {
            self.advance();
            let typ = if self.at_op(":") {
                None
            } else {
                Some(self.parse_expr()?)
            };
            let name = if self.eat_kw("as") {
                Some(self.expect_name()?)
            } else {
                None
            };
            self.expect_op(":")?;
            let hbody = self.parse_block()?;
            handlers.push(Handler { typ, name, body: hbody });
        }
        let orelse = if self.eat_kw("else") {
            self.expect_op(":")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        let finalbody = if self.eat_kw("finally") {
            self.expect_op(":")?;
            self.parse_block()?
        } else {
            Vec::new()
        };
        Ok(Stmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        })
    }

    fn parse_with(&mut self) -> Result<Stmt, ParseError> {
        self.advance();
        let mut items = Vec::new();
        loop {
            let ctx = self.parse_expr()?;
            let target = if self.eat_kw("as") {
                Some(self.parse_target()?)
            } else {
                None
            };
            items.push((ctx, target));
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(":")?;
        let body = self.parse_block()?;
        Ok(Stmt::With { items, body })
    }

    fn parse_funcdef(&mut self, decorators: Vec<Expr>) -> Result<Stmt, ParseError> {
        self.advance(); // def
        let name = self.expect_name()?;
        self.expect_op("(")?;
        let params = self.parse_params()?;
        self.expect_op(")")?;
        let returns = if self.eat_op("->") {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect_op(":")?;
        let body = self.parse_block()?;
        Ok(Stmt::FuncDef {
            name,
            params,
            returns,
            body,
            decorators,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        let mut seen_star = false;
        while !self.at_op(")") {
            if self.eat_op("*") {
                seen_star = true;
                if self.at_op(",") || self.at_op(")") {
                    // bare '*' separator
                    params.push(Param {
                        name: "*".into(),
                        annotation: None,
                        default: None,
                    });
                } else {
                    let mut p = self.parse_param()?;
                    p.name = format!("*{}", p.name);
                    params.push(p);
                }
            } else if self.eat_op("**") {
                let mut p = self.parse_param()?;
                p.name = format!("**{}", p.name);
                params.push(p);
            } else if self.eat_op("/") {
                params.push(Param {
                    name: "/".into(),
                    annotation: None,
                    default: None,
                });
            } else {
                let p = self.parse_param()?;
                if seen_star {
                    // keyword-only
                }
                params.push(p);
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(params)
    }

    fn parse_param(&mut self) -> Result<Param, ParseError> {
        let name = self.expect_name()?;
        let annotation = if self.eat_op(":") {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let default = if self.eat_op("=") {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Param {
            name,
            annotation,
            default,
        })
    }

    fn parse_classdef(&mut self, decorators: Vec<Expr>) -> Result<Stmt, ParseError> {
        self.advance(); // class
        let name = self.expect_name()?;
        let mut bases = Vec::new();
        if self.eat_op("(") {
            while !self.at_op(")") {
                bases.push(self.parse_expr()?);
                if !self.eat_op(",") {
                    break;
                }
            }
            self.expect_op(")")?;
        }
        self.expect_op(":")?;
        let body = self.parse_block()?;
        Ok(Stmt::ClassDef {
            name,
            bases,
            body,
            decorators,
        })
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    /// Parse an expression, collapsing a comma-separated list into a tuple.
    fn parse_expr_list(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_expr()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.at_logical_end() || self.at_op("=") || self.at_op(")") || self.at_op("]")
                || self.at_op("}") || self.at_op(":")
            {
                break;
            }
            items.push(self.parse_expr()?);
        }
        Ok(Expr::Tuple(items))
    }

    fn parse_expr_list_value(&mut self) -> Result<Expr, ParseError> {
        self.parse_expr_list()
    }

    fn parse_target_list(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_target()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",") {
            if self.at_kw("in") {
                break;
            }
            items.push(self.parse_target()?);
        }
        Ok(Expr::Tuple(items))
    }

    /// Parse an assignment target.
    ///
    /// Targets are restricted to names, attribute chains, subscripts and
    /// tuples thereof — deliberately *not* a full expression, so that the `in`
    /// keyword of a `for` statement is not swallowed by a comparison.
    fn parse_target(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_atom()?;
        loop {
            if self.at_op(".") {
                self.advance();
                let attr = self.expect_name()?;
                e = Expr::Attribute {
                    value: Box::new(e),
                    attr,
                };
            } else if self.at_op("[") {
                self.advance();
                let index = self.parse_subscript_items()?;
                self.expect_op("]")?;
                e = Expr::Subscript {
                    value: Box::new(e),
                    index,
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        if self.at_kw("lambda") {
            self.advance();
            let params = self.parse_lambda_params()?;
            self.expect_op(":")?;
            let body = self.parse_expr()?;
            return Ok(Expr::Lambda {
                params,
                body: Box::new(body),
            });
        }
        let body = self.parse_or()?;
        if self.at_kw("if") {
            // Conditional expression: `body if test else orelse`
            self.advance();
            let test = self.parse_or()?;
            if !self.eat_kw("else") {
                return self.err("expected 'else' in conditional expression");
            }
            let orelse = self.parse_expr()?;
            return Ok(Expr::IfExp {
                test: Box::new(test),
                body: Box::new(body),
                orelse: Box::new(orelse),
            });
        }
        Ok(body)
    }

    fn parse_lambda_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        while !self.at_op(":") {
            let mut star = String::new();
            if self.eat_op("**") {
                star = "**".to_string();
            } else if self.eat_op("*") {
                star = "*".to_string();
            }
            let name = self.expect_name()?;
            let default = if self.eat_op("=") {
                Some(self.parse_expr()?)
            } else {
                None
            };
            params.push(Param {
                name: format!("{}{}", star, name),
                annotation: None,
                default,
            });
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(params)
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.at_kw("or") {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BoolOp {
                op: BoolOp::Or,
                values: vec![left, right],
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_not()?;
        while self.at_kw("and") {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::BoolOp {
                op: BoolOp::And,
                values: vec![left, right],
            };
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if self.at_kw("not") {
            self.advance();
            let operand = self.parse_not()?;
            return Ok(Expr::UnaryOp {
                op: UnOp::Not,
                operand: Box::new(operand),
            });
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_bitor()?;
        let mut ops = Vec::new();
        let mut comparators = Vec::new();
        loop {
            let op = if self.at_op("==") {
                CmpOp::Eq
            } else if self.at_op("!=") {
                CmpOp::NotEq
            } else if self.at_op("<=") {
                CmpOp::LtE
            } else if self.at_op(">=") {
                CmpOp::GtE
            } else if self.at_op("<") {
                CmpOp::Lt
            } else if self.at_op(">") {
                CmpOp::Gt
            } else if self.at_kw("in") {
                CmpOp::In
            } else if self.at_kw("not") {
                // `not in`
                self.advance();
                if !self.eat_kw("in") {
                    return self.err("expected 'in' after 'not'");
                }
                ops.push(CmpOp::NotIn);
                comparators.push(self.parse_bitor()?);
                continue;
            } else if self.at_kw("is") {
                self.advance();
                let negated = self.eat_kw("not");
                ops.push(if negated { CmpOp::IsNot } else { CmpOp::Is });
                comparators.push(self.parse_bitor()?);
                continue;
            } else {
                break;
            };
            self.advance();
            ops.push(op);
            comparators.push(self.parse_bitor()?);
        }
        if ops.is_empty() {
            Ok(left)
        } else {
            Ok(Expr::Compare {
                left: Box::new(left),
                ops,
                comparators,
            })
        }
    }

    fn parse_bitor(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bitxor()?;
        while self.at_op("|") {
            self.advance();
            let right = self.parse_bitxor()?;
            left = Expr::BinOp {
                op: BinOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitxor(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_bitand()?;
        while self.at_op("^") {
            self.advance();
            let right = self.parse_bitand()?;
            left = Expr::BinOp {
                op: BinOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitand(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_shift()?;
        while self.at_op("&") {
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::BinOp {
                op: BinOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_arith()?;
        loop {
            let op = if self.at_op("<<") {
                BinOp::LShift
            } else if self.at_op(">>") {
                BinOp::RShift
            } else {
                break;
            };
            self.advance();
            let right = self.parse_arith()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_arith(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_term()?;
        loop {
            let op = if self.at_op("+") {
                BinOp::Add
            } else if self.at_op("-") {
                BinOp::Sub
            } else {
                break;
            };
            self.advance();
            let right = self.parse_term()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_factor()?;
        loop {
            let op = if self.at_op("*") {
                BinOp::Mul
            } else if self.at_op("/") {
                BinOp::Div
            } else if self.at_op("//") {
                BinOp::FloorDiv
            } else if self.at_op("%") {
                BinOp::Mod
            } else if self.at_op("@") {
                BinOp::Mul // matrix-multiply is treated as multiplication
            } else {
                break;
            };
            self.advance();
            let right = self.parse_factor()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr, ParseError> {
        if self.at_op("-") || self.at_op("+") || self.at_op("~") {
            let op = match self.peek() {
                Tok::Op("-") => UnOp::USub,
                Tok::Op("+") => UnOp::UAdd,
                _ => UnOp::Invert,
            };
            self.advance();
            let operand = self.parse_factor()?;
            return Ok(Expr::UnaryOp {
                op,
                operand: Box::new(operand),
            });
        }
        self.parse_power()
    }

    fn parse_power(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_postfix()?;
        if self.at_op("**") {
            self.advance();
            // Right-associative; the exponent may be unary.
            let exp = self.parse_factor()?;
            return Ok(Expr::BinOp {
                op: BinOp::Pow,
                left: Box::new(base),
                right: Box::new(exp),
            });
        }
        Ok(base)
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_atom()?;
        loop {
            if self.at_op(".") {
                self.advance();
                let attr = self.expect_name()?;
                e = Expr::Attribute {
                    value: Box::new(e),
                    attr,
                };
            } else if self.at_op("(") {
                self.advance();
                let args = self.parse_call_args()?;
                self.expect_op(")")?;
                e = Expr::Call {
                    func: Box::new(e),
                    args,
                };
            } else if self.at_op("[") {
                self.advance();
                let index = self.parse_subscript_items()?;
                self.expect_op("]")?;
                e = Expr::Subscript {
                    value: Box::new(e),
                    index,
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_subscript_items(&mut self) -> Result<Vec<IndexItem>, ParseError> {
        let mut items = Vec::new();
        if self.at_op("]") {
            return Ok(items);
        }
        loop {
            let lower = if self.at_op(":") {
                None
            } else {
                Some(self.parse_expr()?)
            };
            if self.at_op(":") {
                self.advance();
                let upper = if self.at_op(":") || self.at_op("]") {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                let step = if self.eat_op(":") {
                    if self.at_op("]") {
                        None
                    } else {
                        Some(self.parse_expr()?)
                    }
                } else {
                    None
                };
                items.push(IndexItem::Slice { lower, upper, step });
            } else {
                items.push(IndexItem::Single(lower.unwrap()));
            }
            if !self.eat_op(",") {
                break;
            }
            if self.at_op("]") {
                break;
            }
        }
        Ok(items)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Arg>, ParseError> {
        let mut args = Vec::new();
        while !self.at_op(")") {
            if self.eat_op("**") {
                args.push(Arg::DoubleStar(self.parse_expr()?));
            } else if self.eat_op("*") {
                args.push(Arg::Star(self.parse_expr()?));
            } else {
                // keyword argument?
                if let Tok::Name(n) = self.peek().clone() {
                    if !KEYWORDS.contains(&n.as_str()) && matches!(self.peek_at(1), Tok::Op("=")) {
                        self.advance();
                        self.advance();
                        let value = self.parse_expr()?;
                        args.push(Arg::Kw(n, value));
                        if !self.eat_op(",") {
                            break;
                        }
                        continue;
                    }
                }
                args.push(Arg::Pos(self.parse_expr()?));
                // A bare generator expression is only allowed as the sole
                // argument: `any(x for x in y)`.
                if self.at_kw("for") {
                    let generators = self.parse_comprehensions()?;
                    if let Some(Arg::Pos(last)) = args.last_mut() {
                        *last = Expr::GeneratorExp {
                            elt: Box::new(last.clone()),
                            generators,
                        };
                    }
                }
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(args)
    }

    /// Parse the elements of a list/tuple/set literal, allowing `*iterable`
    /// unpacking. The first element may already have been parsed.
    fn parse_literal_elements(&mut self, terminator: &str) -> Result<Vec<Expr>, ParseError> {
        let mut items = Vec::new();
        if self.at_op(",") {
            self.advance();
        }
        loop {
            if self.at_op(terminator) {
                break;
            }
            if self.eat_op("*") {
                let inner = self.parse_expr()?;
                items.push(Expr::Starred(Box::new(inner)));
            } else {
                items.push(self.parse_expr()?);
            }
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(items)
    }

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Tok::Int(v) => {
                self.advance();
                Ok(Expr::Num(NumVal::Int(v)))
            }
            Tok::Float(v) => {
                self.advance();
                Ok(Expr::Num(NumVal::Float(v)))
            }
            Tok::Str { value: _, kind } | Tok::FStr { segments: _, kind } => {
                // Adjacent string literals are implicitly concatenated, and
                // plain and formatted strings may be mixed.
                let mut segs: Vec<FSeg> = Vec::new();
                let mut kind = kind;
                let mut formatted = false;
                match self.advance() {
                    Tok::Str { value, .. } => segs.push(FSeg::Text(value)),
                    Tok::FStr { segments, .. } => {
                        formatted = true;
                        for s in segments {
                            match s {
                                crate::lexer::FSegRaw::Text(t) => segs.push(FSeg::Text(t)),
                                crate::lexer::FSegRaw::Expr {
                                    src,
                                    conversion,
                                    format_spec,
                                } => {
                                    segs.push(FSeg::Expr(build_fstring_expr(
                                        &src, conversion, format_spec,
                                    )?));
                                }
                            }
                        }
                    }
                    _ => unreachable!(),
                }
                loop {
                    match self.peek().clone() {
                        Tok::Str { value, kind: k2 } if k2 == kind => {
                            self.advance();
                            segs.push(FSeg::Text(value));
                        }
                        Tok::FStr {
                            segments: s2,
                            kind: k2,
                        } => {
                            self.advance();
                            formatted = true;
                            for s in s2 {
                                match s {
                                    crate::lexer::FSegRaw::Text(t) => segs.push(FSeg::Text(t)),
                                    crate::lexer::FSegRaw::Expr {
                                        src,
                                        conversion,
                                        format_spec,
                                    } => {
                                        segs.push(FSeg::Expr(build_fstring_expr(
                                            &src, conversion, format_spec,
                                        )?));
                                    }
                                }
                            }
                            kind = k2;
                        }
                        _ => break,
                    }
                }
                if formatted {
                    Ok(Expr::FStr { segments: segs, kind })
                } else {
                    let mut text = String::new();
                    for s in segs {
                        if let FSeg::Text(t) = s {
                            text.push_str(&t);
                        }
                    }
                    Ok(Expr::Str { value: text, kind })
                }
            }
            Tok::Name(n) => match n.as_str() {
                "True" => {
                    self.advance();
                    Ok(Expr::Name("True".into()))
                }
                "False" => {
                    self.advance();
                    Ok(Expr::Name("False".into()))
                }
                "None" => {
                    self.advance();
                    Ok(Expr::Name("None".into()))
                }
                "lambda" => self.parse_expr(),
                "not" => self.parse_not(),
                "await" => {
                    self.advance();
                    let v = self.parse_factor()?;
                    Ok(Expr::Call {
                        func: Box::new(Expr::Name("__await__".into())),
                        args: vec![Arg::Pos(v)],
                    })
                }
                _ => {
                    self.advance();
                    Ok(Expr::Name(n))
                }
            },
            Tok::Op("...") => {
                self.advance();
                Ok(Expr::Name("...".into()))
            }
            Tok::Op("(") => {
                self.advance();
                if self.at_op(")") {
                    self.advance();
                    return Ok(Expr::Tuple(Vec::new()));
                }
                // A parenthesised starred expression: `(*a, *b)`.
                if self.at_op("*") {
                    let mut items = Vec::new();
                    while !self.at_op(")") {
                        if self.eat_op("*") {
                            let inner = self.parse_expr()?;
                            items.push(Expr::Starred(Box::new(inner)));
                        } else {
                            items.push(self.parse_expr()?);
                        }
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.expect_op(")")?;
                    return Ok(Expr::Tuple(items));
                }
                let first = self.parse_expr()?;
                if self.at_kw("for") {
                    let generators = self.parse_comprehensions()?;
                    self.expect_op(")")?;
                    return Ok(Expr::GeneratorExp {
                        elt: Box::new(first),
                        generators,
                    });
                }
                if self.at_op(",") {
                    let mut items = vec![first];
                    items.extend(self.parse_literal_elements(")")?);
                    self.expect_op(")")?;
                    return Ok(Expr::Tuple(items));
                }
                self.expect_op(")")?;
                Ok(first)
            }
            Tok::Op("[") => {
                self.advance();
                if self.at_op("]") {
                    self.advance();
                    return Ok(Expr::List(Vec::new()));
                }
                let first = self.parse_expr()?;
                if self.at_kw("for") {
                    let generators = self.parse_comprehensions()?;
                    self.expect_op("]")?;
                    return Ok(Expr::ListComp {
                        elt: Box::new(first),
                        generators,
                    });
                }
                let mut items = vec![first];
                while self.eat_op(",") {
                    if self.at_op("]") {
                        break;
                    }
                    if self.eat_op("*") {
                        let inner = self.parse_expr()?;
                        items.push(Expr::Starred(Box::new(inner)));
                    } else {
                        items.push(self.parse_expr()?);
                    }
                }
                self.expect_op("]")?;
                Ok(Expr::List(items))
            }
            Tok::Op("{") => {
                self.advance();
                if self.at_op("}") {
                    self.advance();
                    return Ok(Expr::Dict(Vec::new()));
                }
                // Dict or set
                if self.at_op("**") {
                    return self.parse_dict_rest(Vec::new(), None);
                }
                let first = self.parse_expr()?;
                if self.at_op(":") {
                    self.advance();
                    let value = self.parse_expr()?;
                    if self.at_kw("for") {
                        let generators = self.parse_comprehensions()?;
                        self.expect_op("}")?;
                        return Ok(Expr::DictComp {
                            key: Box::new(first),
                            value: Box::new(value),
                            generators,
                        });
                    }
                    return self.parse_dict_rest(vec![(Some(first), value)], None);
                }
                if self.at_kw("for") {
                    let generators = self.parse_comprehensions()?;
                    self.expect_op("}")?;
                    return Ok(Expr::SetComp {
                        elt: Box::new(first),
                        generators,
                    });
                }
                let mut items = vec![first];
                while self.eat_op(",") {
                    if self.at_op("}") {
                        break;
                    }
                    if self.eat_op("*") {
                        let inner = self.parse_expr()?;
                        items.push(Expr::Starred(Box::new(inner)));
                    } else {
                        items.push(self.parse_expr()?);
                    }
                }
                self.expect_op("}")?;
                Ok(Expr::Set(items))
            }
            _ => self.err("unexpected token in expression"),
        }
    }

    fn parse_dict_rest(
        &mut self,
        mut entries: Vec<(Option<Expr>, Expr)>,
        _first: Option<Expr>,
    ) -> Result<Expr, ParseError> {
        loop {
            if self.at_op(",") {
                self.advance();
            }
            if self.at_op("}") {
                self.advance();
                return Ok(Expr::Dict(entries));
            }
            if self.eat_op("**") {
                let v = self.parse_expr()?;
                entries.push((None, v));
            } else {
                let k = self.parse_expr()?;
                self.expect_op(":")?;
                let v = self.parse_expr()?;
                entries.push((Some(k), v));
            }
        }
    }

    fn parse_comprehensions(&mut self) -> Result<Vec<Comprehension>, ParseError> {
        let mut generators = Vec::new();
        while self.eat_kw("for") {
            let target = self.parse_target_list()?;
            if !self.eat_kw("in") {
                return self.err("expected 'in' in comprehension");
            }
            let iter = self.parse_or()?;
            let mut ifs = Vec::new();
            while self.eat_kw("if") {
                ifs.push(self.parse_or()?);
            }
            generators.push(Comprehension {
                target,
                iter,
                ifs,
            });
        }
        Ok(generators)
    }
}

/// Build the expression for one f-string replacement field, applying the
/// optional `!r`/`!s` conversion and format spec.
fn build_fstring_expr(
    src: &str,
    conversion: Option<char>,
    format_spec: Option<String>,
) -> Result<Expr, ParseError> {
    let mut e = parse_expr_source(src)?;
    if let Some(c) = conversion {
        e = Expr::Call {
            func: Box::new(Expr::Name(format!("__conv_{}__", c))),
            args: vec![Arg::Pos(e)],
        };
    }
    if let Some(spec) = format_spec {
        e = Expr::Call {
            func: Box::new(Expr::Name("__format__".into())),
            args: vec![
                Arg::Pos(Expr::Str {
                    value: spec,
                    kind: StrKind::Str,
                }),
                Arg::Pos(e),
            ],
        };
    }
    Ok(e)
}

/// Parse a standalone expression from source text (used for f-strings).
pub fn parse_expr_source(src: &str) -> Result<Expr, ParseError> {
    let (toks, lines) = Lexer::new(src).tokenize().map_err(|e| ParseError {
        line: e.line,
        msg: e.msg,
    })?;
    let mut p = Parser::new(toks, lines);
    p.parse_expr()
}
