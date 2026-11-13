use crate::ast::*;
use crate::error::SyntaxError;
use crate::lexer::{self, Keyword, Punct, Token, TokenKind};

const MAX_PARSE_DEPTH: usize = 256;

pub fn parse(source: &str) -> Result<Program, SyntaxError> {
    let tokens = lexer::lex(source)?;
    let mut parser = Parser::new(tokens);
    parser.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    function_depth: usize,
    loop_depth: usize,
    depth: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            function_depth: 0,
            loop_depth: 0,
            depth: 0,
        }
    }

    fn parse_program(&mut self) -> Result<Program, SyntaxError> {
        let mut statements = Vec::new();
        while !self.check_eof() {
            statements.push(self.parse_statement()?);
        }
        Ok(Program { statements })
    }

    fn current(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().expect("parser always has Eof"))
    }

    fn previous(&self) -> &Token {
        if self.pos == 0 {
            self.current()
        } else {
            &self.tokens[self.pos - 1]
        }
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.current().kind
    }

    fn check_eof(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn check_punct(&self, punct: Punct) -> bool {
        matches!(self.peek_kind(), TokenKind::Punct(p) if *p == punct)
    }

    fn check_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek_kind(), TokenKind::Keyword(k) if *k == keyword)
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        token
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, SyntaxError> {
        let token = self.current();
        Err(SyntaxError::new(message, token.line, token.column))
    }

    fn expect_punct(&mut self, punct: Punct) -> Result<Token, SyntaxError> {
        if self.check_punct(punct) {
            Ok(self.advance())
        } else {
            self.error(format!(
                "expected '{}' but found {}",
                punct.as_str(),
                self.peek_kind().describe()
            ))
        }
    }

    fn expect_identifier(&mut self) -> Result<String, SyntaxError> {
        match self.peek_kind().clone() {
            TokenKind::Identifier(name) => {
                self.advance();
                Ok(name)
            }
            other => self.error(format!("expected identifier but found {}", other.describe())),
        }
    }

    fn consume_semicolon(&mut self) -> Result<(), SyntaxError> {
        if self.check_punct(Punct::Semicolon) {
            self.advance();
            return Ok(());
        }
        if self.check_punct(Punct::RBrace) || self.check_eof() {
            return Ok(());
        }
        let previous_line = self.previous().line;
        if self.current().line > previous_line {
            return Ok(());
        }
        self.error(format!(
            "expected ';' but found {}",
            self.peek_kind().describe()
        ))
    }

    fn with_depth<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, SyntaxError>,
    ) -> Result<T, SyntaxError> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            self.depth -= 1;
            return self.error("expression or statement nesting is too deep");
        }
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn parse_statement(&mut self) -> Result<Stmt, SyntaxError> {
        self.with_depth(Self::parse_statement_inner)
    }

    fn parse_statement_inner(&mut self) -> Result<Stmt, SyntaxError> {
        if self.check_punct(Punct::Semicolon) {
            self.advance();
            return Ok(Stmt::Empty);
        }
        if self.check_punct(Punct::LBrace) {
            return Ok(Stmt::Block(self.parse_block()?));
        }

        match self.peek_kind().clone() {
            TokenKind::Keyword(Keyword::Var)
            | TokenKind::Keyword(Keyword::Let)
            | TokenKind::Keyword(Keyword::Const) => {
                let decl = self.parse_var_decl()?;
                self.consume_semicolon()?;
                Ok(Stmt::Var(decl))
            }
            TokenKind::Keyword(Keyword::Function) => self.parse_function_declaration(),
            TokenKind::Keyword(Keyword::If) => self.parse_if(),
            TokenKind::Keyword(Keyword::While) => self.parse_while(),
            TokenKind::Keyword(Keyword::For) => self.parse_for(),
            TokenKind::Keyword(Keyword::Break) => {
                let token = self.advance();
                if self.loop_depth == 0 {
                    return Err(SyntaxError::new(
                        "'break' is only valid inside a loop",
                        token.line,
                        token.column,
                    ));
                }
                self.consume_semicolon()?;
                Ok(Stmt::Break)
            }
            TokenKind::Keyword(Keyword::Continue) => {
                let token = self.advance();
                if self.loop_depth == 0 {
                    return Err(SyntaxError::new(
                        "'continue' is only valid inside a loop",
                        token.line,
                        token.column,
                    ));
                }
                self.consume_semicolon()?;
                Ok(Stmt::Continue)
            }
            TokenKind::Keyword(Keyword::Return) => self.parse_return(),
            TokenKind::Keyword(Keyword::Else) => {
                self.error("unexpected 'else' without matching 'if'")
            }
            TokenKind::Keyword(Keyword::Unsupported(word)) => {
                self.error(format!("unsupported keyword '{word}'"))
            }
            TokenKind::Eof => self.error("unexpected end of input"),
            _ => {
                let expr = self.parse_expression()?;
                self.consume_semicolon()?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        self.expect_punct(Punct::LBrace)?;
        let mut statements = Vec::new();
        while !self.check_punct(Punct::RBrace) {
            if self.check_eof() {
                return self.error("unterminated block; expected '}'");
            }
            statements.push(self.parse_statement()?);
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(statements)
    }

    fn parse_var_decl(&mut self) -> Result<VarDecl, SyntaxError> {
        let kind = match self.peek_kind() {
            TokenKind::Keyword(Keyword::Var) => VarKind::Var,
            TokenKind::Keyword(Keyword::Let) => VarKind::Let,
            TokenKind::Keyword(Keyword::Const) => VarKind::Const,
            _ => return self.error("expected variable declaration"),
        };
        self.advance();
        let mut declarations = Vec::new();
        loop {
            let name_token = self.current().clone();
            let name = self.expect_identifier()?;
            let init = if self.check_punct(Punct::Assign) {
                self.advance();
                Some(self.parse_expression()?)
            } else {
                None
            };
            if kind == VarKind::Const && init.is_none() {
                return Err(SyntaxError::new(
                    "const declaration requires an initializer",
                    name_token.line,
                    name_token.column,
                ));
            }
            declarations.push(VarDeclarator { name, init });
            if !self.check_punct(Punct::Comma) {
                break;
            }
            self.advance();
        }
        Ok(VarDecl {
            kind,
            declarations,
        })
    }

    fn parse_function_declaration(&mut self) -> Result<Stmt, SyntaxError> {
        self.advance();
        let name = self.expect_identifier()?;
        let (params, body) = self.parse_function_tail()?;
        Ok(Stmt::Function(FunctionNode {
            name: Some(name),
            params,
            body,
        }))
    }

    fn parse_function_expression(&mut self) -> Result<Expr, SyntaxError> {
        self.advance();
        let name = if matches!(self.peek_kind(), TokenKind::Identifier(_)) {
            Some(self.expect_identifier()?)
        } else {
            None
        };
        let (params, body) = self.parse_function_tail()?;
        Ok(Expr::Function(FunctionNode { name, params, body }))
    }

    fn parse_function_tail(&mut self) -> Result<(Vec<String>, Vec<Stmt>), SyntaxError> {
        self.expect_punct(Punct::LParen)?;
        let mut params = Vec::new();
        if !self.check_punct(Punct::RParen) {
            loop {
                params.push(self.expect_identifier()?);
                if !self.check_punct(Punct::Comma) {
                    break;
                }
                self.advance();
                if self.check_punct(Punct::RParen) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        self.expect_punct(Punct::LBrace)?;

        let saved_loop_depth = self.loop_depth;
        self.loop_depth = 0;
        self.function_depth += 1;
        let mut body = Vec::new();
        while !self.check_punct(Punct::RBrace) {
            if self.check_eof() {
                self.function_depth -= 1;
                self.loop_depth = saved_loop_depth;
                return self.error("unterminated function body; expected '}'");
            }
            body.push(self.parse_statement()?);
        }
        self.expect_punct(Punct::RBrace)?;
        self.function_depth -= 1;
        self.loop_depth = saved_loop_depth;
        Ok((params, body))
    }

    fn parse_if(&mut self) -> Result<Stmt, SyntaxError> {
        self.advance();
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression()?;
        self.expect_punct(Punct::RParen)?;
        let consequent = Box::new(self.parse_statement()?);
        let alternate = if self.check_keyword(Keyword::Else) {
            self.advance();
            Some(Box::new(self.parse_statement()?))
        } else {
            None
        };
        Ok(Stmt::If {
            test,
            consequent,
            alternate,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, SyntaxError> {
        self.advance();
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression()?;
        self.expect_punct(Punct::RParen)?;
        self.loop_depth += 1;
        let body = self.parse_statement();
        self.loop_depth -= 1;
        Ok(Stmt::While {
            test,
            body: Box::new(body?),
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, SyntaxError> {
        self.advance();
        self.expect_punct(Punct::LParen)?;
        let init = if self.check_punct(Punct::Semicolon) {
            None
        } else if matches!(
            self.peek_kind(),
            TokenKind::Keyword(Keyword::Var)
                | TokenKind::Keyword(Keyword::Let)
                | TokenKind::Keyword(Keyword::Const)
        ) {
            Some(ForInit::Var(self.parse_var_decl()?))
        } else {
            Some(ForInit::Expr(self.parse_expression()?))
        };
        self.expect_punct(Punct::Semicolon)?;
        let test = if self.check_punct(Punct::Semicolon) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect_punct(Punct::Semicolon)?;
        let update = if self.check_punct(Punct::RParen) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect_punct(Punct::RParen)?;

        self.loop_depth += 1;
        let body = self.parse_statement();
        self.loop_depth -= 1;
        Ok(Stmt::For {
            init,
            test,
            update,
            body: Box::new(body?),
        })
    }

    fn parse_return(&mut self) -> Result<Stmt, SyntaxError> {
        let token = self.advance();
        if self.function_depth == 0 {
            return Err(SyntaxError::new(
                "'return' is only valid inside a function",
                token.line,
                token.column,
            ));
        }
        let has_value = !self.check_punct(Punct::Semicolon)
            && !self.check_punct(Punct::RBrace)
            && !self.check_eof()
            && self.current().line == token.line;
        let value = if has_value {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.consume_semicolon()?;
        Ok(Stmt::Return(value))
    }

    fn parse_expression(&mut self) -> Result<Expr, SyntaxError> {
        self.with_depth(Self::parse_assignment)
    }

    fn parse_assignment(&mut self) -> Result<Expr, SyntaxError> {
        let left = self.parse_conditional()?;
        let op = match self.peek_kind() {
            TokenKind::Punct(Punct::Assign) => Some(AssignOp::Assign),
            TokenKind::Punct(Punct::PlusAssign) => Some(AssignOp::Add),
            TokenKind::Punct(Punct::MinusAssign) => Some(AssignOp::Sub),
            TokenKind::Punct(Punct::StarAssign) => Some(AssignOp::Mul),
            TokenKind::Punct(Punct::SlashAssign) => Some(AssignOp::Div),
            TokenKind::Punct(Punct::PercentAssign) => Some(AssignOp::Rem),
            TokenKind::Punct(Punct::StarStarAssign) => Some(AssignOp::Pow),
            TokenKind::Punct(Punct::ShlAssign) => Some(AssignOp::Shl),
            TokenKind::Punct(Punct::ShrAssign) => Some(AssignOp::Shr),
            TokenKind::Punct(Punct::UShrAssign) => Some(AssignOp::UShr),
            TokenKind::Punct(Punct::AmpAssign) => Some(AssignOp::BitAnd),
            TokenKind::Punct(Punct::PipeAssign) => Some(AssignOp::BitOr),
            TokenKind::Punct(Punct::CaretAssign) => Some(AssignOp::BitXor),
            _ => None,
        };
        let Some(op) = op else {
            return Ok(left);
        };
        let operator_token = self.advance();
        if !is_assignment_target(&left) {
            return Err(SyntaxError::new(
                "invalid assignment target",
                operator_token.line,
                operator_token.column,
            ));
        }
        let value = self.parse_assignment()?;
        Ok(Expr::Assign {
            op,
            target: Box::new(left),
            value: Box::new(value),
        })
    }

    fn parse_conditional(&mut self) -> Result<Expr, SyntaxError> {
        let test = self.parse_logical_or()?;
        if !self.check_punct(Punct::Question) {
            return Ok(test);
        }
        self.advance();
        let consequent = self.parse_assignment()?;
        self.expect_punct(Punct::Colon)?;
        let alternate = self.parse_assignment()?;
        Ok(Expr::Conditional {
            test: Box::new(test),
            consequent: Box::new(consequent),
            alternate: Box::new(alternate),
        })
    }

    fn parse_logical_or(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_logical_and()?;
        while self.check_punct(Punct::OrOr) {
            self.advance();
            let right = self.parse_logical_and()?;
            left = Expr::Logical {
                op: LogicalOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_bitwise_or()?;
        while self.check_punct(Punct::AndAnd) {
            self.advance();
            let right = self.parse_bitwise_or()?;
            left = Expr::Logical {
                op: LogicalOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitwise_or(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_bitwise_xor()?;
        while self.check_punct(Punct::Pipe) {
            self.advance();
            let right = self.parse_bitwise_xor()?;
            left = Expr::Binary {
                op: BinaryOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_bitwise_and()?;
        while self.check_punct(Punct::Caret) {
            self.advance();
            let right = self.parse_bitwise_and()?;
            left = Expr::Binary {
                op: BinaryOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_equality()?;
        while self.check_punct(Punct::Amp) {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary {
                op: BinaryOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_relational()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Punct(Punct::EqEq) => BinaryOp::Eq,
                TokenKind::Punct(Punct::NotEq) => BinaryOp::Ne,
                TokenKind::Punct(Punct::StrictEq) => BinaryOp::StrictEq,
                TokenKind::Punct(Punct::StrictNe) => BinaryOp::StrictNe,
                _ => break,
            };
            self.advance();
            let right = self.parse_relational()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_relational(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_shift()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Punct(Punct::Lt) => BinaryOp::Lt,
                TokenKind::Punct(Punct::Gt) => BinaryOp::Gt,
                TokenKind::Punct(Punct::Le) => BinaryOp::Le,
                TokenKind::Punct(Punct::Ge) => BinaryOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Punct(Punct::Shl) => BinaryOp::Shl,
                TokenKind::Punct(Punct::Shr) => BinaryOp::Shr,
                TokenKind::Punct(Punct::UShr) => BinaryOp::UShr,
                _ => break,
            };
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Punct(Punct::Plus) => BinaryOp::Add,
                TokenKind::Punct(Punct::Minus) => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_exponent()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Punct(Punct::Star) => BinaryOp::Mul,
                TokenKind::Punct(Punct::Slash) => BinaryOp::Div,
                TokenKind::Punct(Punct::Percent) => BinaryOp::Rem,
                _ => break,
            };
            self.advance();
            let right = self.parse_exponent()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_exponent(&mut self) -> Result<Expr, SyntaxError> {
        let left = self.parse_unary()?;
        if self.check_punct(Punct::StarStar) {
            self.advance();
            let right = self.parse_exponent()?;
            return Ok(Expr::Binary {
                op: BinaryOp::Pow,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, SyntaxError> {
        self.with_depth(|parser| {
            let op = match parser.peek_kind() {
                TokenKind::Punct(Punct::Not) => Some(UnaryOp::Not),
                TokenKind::Punct(Punct::Minus) => Some(UnaryOp::Neg),
                TokenKind::Punct(Punct::Plus) => Some(UnaryOp::Pos),
                TokenKind::Punct(Punct::Tilde) => Some(UnaryOp::BitNot),
                TokenKind::Keyword(Keyword::Typeof) => Some(UnaryOp::Typeof),
                TokenKind::Keyword(Keyword::Void) => Some(UnaryOp::Void),
                _ => None,
            };
            if let Some(op) = op {
                parser.advance();
                let argument = parser.parse_unary()?;
                return Ok(Expr::Unary {
                    op,
                    argument: Box::new(argument),
                });
            }
            if parser.check_punct(Punct::PlusPlus) || parser.check_punct(Punct::MinusMinus) {
                let op = if parser.check_punct(Punct::PlusPlus) {
                    UpdateOp::Inc
                } else {
                    UpdateOp::Dec
                };
                let token = parser.advance();
                let target = parser.parse_unary()?;
                if !is_assignment_target(&target) {
                    return Err(SyntaxError::new(
                        "invalid update target",
                        token.line,
                        token.column,
                    ));
                }
                return Ok(Expr::Update {
                    op,
                    target: Box::new(target),
                    prefix: true,
                });
            }
            parser.parse_postfix()
        })
    }

    fn parse_postfix(&mut self) -> Result<Expr, SyntaxError> {
        let mut expr = self.parse_call_member()?;
        loop {
            let op = if self.check_punct(Punct::PlusPlus) {
                Some(UpdateOp::Inc)
            } else if self.check_punct(Punct::MinusMinus) {
                Some(UpdateOp::Dec)
            } else {
                None
            };
            let Some(op) = op else {
                break;
            };
            let token = self.advance();
            if !is_assignment_target(&expr) {
                return Err(SyntaxError::new(
                    "invalid update target",
                    token.line,
                    token.column,
                ));
            }
            expr = Expr::Update {
                op,
                target: Box::new(expr),
                prefix: false,
            };
        }
        Ok(expr)
    }

    fn parse_call_member(&mut self) -> Result<Expr, SyntaxError> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check_punct(Punct::Dot) {
                self.advance();
                let name = self.expect_identifier()?;
                expr = Expr::Member {
                    object: Box::new(expr),
                    property: Box::new(Expr::String(name)),
                    computed: false,
                };
            } else if self.check_punct(Punct::LBracket) {
                self.advance();
                let property = self.parse_expression()?;
                self.expect_punct(Punct::RBracket)?;
                expr = Expr::Member {
                    object: Box::new(expr),
                    property: Box::new(property),
                    computed: true,
                };
            } else if self.check_punct(Punct::LParen) {
                let args = self.parse_arguments()?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_arguments(&mut self) -> Result<Vec<Expr>, SyntaxError> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        if !self.check_punct(Punct::RParen) {
            loop {
                args.push(self.parse_expression()?);
                if !self.check_punct(Punct::Comma) {
                    break;
                }
                self.advance();
                if self.check_punct(Punct::RParen) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, SyntaxError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Number(value) => {
                self.advance();
                Ok(Expr::Number(value))
            }
            TokenKind::String(value) => {
                self.advance();
                Ok(Expr::String(value))
            }
            TokenKind::Identifier(name) => {
                self.advance();
                Ok(Expr::Identifier(name))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            TokenKind::Keyword(Keyword::Null) => {
                self.advance();
                Ok(Expr::Null)
            }
            TokenKind::Keyword(Keyword::Undefined) => {
                self.advance();
                Ok(Expr::Undefined)
            }
            TokenKind::Keyword(Keyword::This) => {
                self.advance();
                Ok(Expr::This)
            }
            TokenKind::Keyword(Keyword::Function) => self.parse_function_expression(),
            TokenKind::Punct(Punct::LParen) => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect_punct(Punct::RParen)?;
                Ok(expr)
            }
            TokenKind::Punct(Punct::LBracket) => self.parse_array_literal(),
            TokenKind::Punct(Punct::LBrace) => self.parse_object_literal(),
            TokenKind::Keyword(Keyword::Unsupported(word)) => {
                self.error(format!("unsupported keyword '{word}'"))
            }
            other => self.error(format!("expected expression but found {}", other.describe())),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, SyntaxError> {
        self.expect_punct(Punct::LBracket)?;
        let mut elements = Vec::new();
        loop {
            if self.check_punct(Punct::RBracket) {
                break;
            }
            if self.check_punct(Punct::Comma) {
                elements.push(Expr::Undefined);
                self.advance();
                continue;
            }
            elements.push(self.parse_expression()?);
            if !self.check_punct(Punct::Comma) {
                break;
            }
            self.advance();
        }
        self.expect_punct(Punct::RBracket)?;
        Ok(Expr::Array(elements))
    }

    fn parse_object_literal(&mut self) -> Result<Expr, SyntaxError> {
        self.expect_punct(Punct::LBrace)?;
        let mut properties = Vec::new();
        loop {
            if self.check_punct(Punct::RBrace) {
                break;
            }
            let (key, shorthand_name) = self.parse_property_key()?;
            if self.check_punct(Punct::Colon) {
                self.advance();
                let value = self.parse_expression()?;
                properties.push((key, value));
            } else if let Some(name) = shorthand_name {
                properties.push((key, Expr::Identifier(name)));
            } else {
                return self.error("expected ':' after object property name");
            }
            if !self.check_punct(Punct::Comma) {
                break;
            }
            self.advance();
        }
        self.expect_punct(Punct::RBrace)?;
        Ok(Expr::Object(properties))
    }

    fn parse_property_key(&mut self) -> Result<(String, Option<String>), SyntaxError> {
        match self.peek_kind().clone() {
            TokenKind::Identifier(name) => {
                self.advance();
                Ok((name.clone(), Some(name)))
            }
            TokenKind::String(value) => {
                self.advance();
                Ok((value, None))
            }
            TokenKind::Number(value) => {
                self.advance();
                Ok((format_number(value), None))
            }
            TokenKind::Keyword(keyword) => {
                self.advance();
                Ok((keyword.as_str().to_string(), None))
            }
            other => self.error(format!(
                "expected object property name but found {}",
                other.describe()
            )),
        }
    }
}

fn is_assignment_target(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Identifier(_) | Expr::Member { .. }
    )
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() && value.abs() < 1e21 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_core_statement_forms() {
        let program = parse(
            "var a = 1, b; let c = \"x\"; const d = true;\n\
             function add(x, y) { return x + y; }\n\
             if (a) { a++; } else { a--; }\n\
             while (a < 3) { a += 1; }\n\
             for (let i = 0; i < 3; i++) { if (i === 1) continue; b = i; }\n\
             for (;;) { break; }",
        )
        .expect("parse");
        assert_eq!(program.statements.len(), 8);
        assert!(matches!(program.statements[3], Stmt::Function(_)));
        assert!(matches!(program.statements[4], Stmt::If { .. }));
        assert!(matches!(program.statements[7], Stmt::For { .. }));
    }

    #[test]
    fn parses_expressions_and_objects() {
        let program = parse(
            "let o = { a: 1, \"b\": [2, 3,], c, f: function (x) { return x; } };\n\
             let v = o.a + o[\"b\"][0] * 2 ** 3 ? -o.a : 4;\n\
             o.a = 5; o.b += 2; o.a++; ++o.a; o.x = typeof v;",
        )
        .expect("parse");
        assert_eq!(program.statements.len(), 7);
    }

    #[test]
    fn reports_line_and_column_diagnostics() {
        let err = parse("let x = 1;\nlet y = ;").expect_err("syntax error");
        assert_eq!(err.line, 2);
        assert_eq!(err.column, 9);
        assert!(err.to_string().contains("line 2, column 9"));

        let err = parse("if (true) {\n  let x = 1;\n").expect_err("unterminated");
        assert_eq!(err.line, 3);

        let err = parse("break;").expect_err("break outside loop");
        assert_eq!(err.line, 1);
        assert_eq!(err.column, 1);

        let err = parse("function f() {} return 1;").expect_err("return outside");
        assert_eq!(err.line, 1);
        assert_eq!(err.column, 17);
    }

    #[test]
    fn rejects_unsupported_and_invalid_targets() {
        let err = parse("new Date();").expect_err("unsupported");
        assert!(err.message.contains("unsupported keyword 'new'"));
        let err = parse("1 = 2;").expect_err("target");
        assert!(err.message.contains("invalid assignment target"));
        let err = parse("const x;").expect_err("const");
        assert!(err.message.contains("initializer"));
    }

    #[test]
    fn supports_asi_before_brace_and_eof() {
        parse("let x = 1\nlet y = 2").expect("ASI at EOF");
        parse("function f() { let x = 1\nreturn x }").expect("ASI before brace");
    }
}
