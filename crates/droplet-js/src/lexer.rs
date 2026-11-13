use crate::error::SyntaxError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Var,
    Let,
    Const,
    Function,
    If,
    Else,
    While,
    For,
    Break,
    Continue,
    Return,
    True,
    False,
    Null,
    Undefined,
    Typeof,
    Void,
    This,
    Unsupported(&'static str),
}

impl Keyword {
    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::Var => "var",
            Keyword::Let => "let",
            Keyword::Const => "const",
            Keyword::Function => "function",
            Keyword::If => "if",
            Keyword::Else => "else",
            Keyword::While => "while",
            Keyword::For => "for",
            Keyword::Break => "break",
            Keyword::Continue => "continue",
            Keyword::Return => "return",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Null => "null",
            Keyword::Undefined => "undefined",
            Keyword::Typeof => "typeof",
            Keyword::Void => "void",
            Keyword::This => "this",
            Keyword::Unsupported(word) => word,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punct {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dot,
    Colon,
    Question,
    Assign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    StarStarAssign,
    EqEq,
    NotEq,
    StrictEq,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
    AndAnd,
    OrOr,
    Not,
    PlusPlus,
    MinusMinus,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    UShr,
    ShlAssign,
    ShrAssign,
    UShrAssign,
    AmpAssign,
    PipeAssign,
    CaretAssign,
}

impl Punct {
    pub fn as_str(self) -> &'static str {
        match self {
            Punct::LBrace => "{",
            Punct::RBrace => "}",
            Punct::LParen => "(",
            Punct::RParen => ")",
            Punct::LBracket => "[",
            Punct::RBracket => "]",
            Punct::Semicolon => ";",
            Punct::Comma => ",",
            Punct::Dot => ".",
            Punct::Colon => ":",
            Punct::Question => "?",
            Punct::Assign => "=",
            Punct::Plus => "+",
            Punct::Minus => "-",
            Punct::Star => "*",
            Punct::Slash => "/",
            Punct::Percent => "%",
            Punct::StarStar => "**",
            Punct::PlusAssign => "+=",
            Punct::MinusAssign => "-=",
            Punct::StarAssign => "*=",
            Punct::SlashAssign => "/=",
            Punct::PercentAssign => "%=",
            Punct::StarStarAssign => "**=",
            Punct::EqEq => "==",
            Punct::NotEq => "!=",
            Punct::StrictEq => "===",
            Punct::StrictNe => "!==",
            Punct::Lt => "<",
            Punct::Gt => ">",
            Punct::Le => "<=",
            Punct::Ge => ">=",
            Punct::AndAnd => "&&",
            Punct::OrOr => "||",
            Punct::Not => "!",
            Punct::PlusPlus => "++",
            Punct::MinusMinus => "--",
            Punct::Amp => "&",
            Punct::Pipe => "|",
            Punct::Caret => "^",
            Punct::Tilde => "~",
            Punct::Shl => "<<",
            Punct::Shr => ">>",
            Punct::UShr => ">>>",
            Punct::ShlAssign => "<<=",
            Punct::ShrAssign => ">>=",
            Punct::UShrAssign => ">>>=",
            Punct::AmpAssign => "&=",
            Punct::PipeAssign => "|=",
            Punct::CaretAssign => "^=",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Number(f64),
    String(String),
    Keyword(Keyword),
    Punct(Punct),
    Eof,
}

impl TokenKind {
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Identifier(name) => format!("identifier '{name}'"),
            TokenKind::Number(value) => format!("number '{value}'"),
            TokenKind::String(_) => "string literal".to_string(),
            TokenKind::Keyword(keyword) => format!("keyword '{}'", keyword.as_str()),
            TokenKind::Punct(punct) => format!("'{}'", punct.as_str()),
            TokenKind::Eof => "end of input".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn error<T>(&self, message: impl Into<String>, line: usize, column: usize) -> Result<T, SyntaxError> {
        Err(SyntaxError::new(message, line, column))
    }

    fn skip_trivia(&mut self) -> Result<(), SyntaxError> {
        loop {
            match self.peek() {
                Some(ch) if ch.is_whitespace() || ch == '\u{FEFF}' => {
                    self.advance();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    while let Some(ch) = self.peek() {
                        if ch == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    let line = self.line;
                    let column = self.column;
                    self.advance();
                    self.advance();
                    let mut closed = false;
                    while let Some(ch) = self.peek() {
                        if ch == '*' && self.peek_at(1) == Some('/') {
                            self.advance();
                            self.advance();
                            closed = true;
                            break;
                        }
                        self.advance();
                    }
                    if !closed {
                        return self.error("unterminated block comment", line, column);
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn lex_all(mut self) -> Result<Vec<Token>, SyntaxError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia()?;
            let line = self.line;
            let column = self.column;
            let offset = self.pos;
            let Some(ch) = self.peek() else {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    line,
                    column,
                    offset,
                });
                return Ok(tokens);
            };

            let kind = if is_identifier_start(ch) {
                self.lex_identifier()
            } else if ch.is_ascii_digit() || (ch == '.' && self.peek_at(1).is_some_and(|c| c.is_ascii_digit())) {
                self.lex_number()?
            } else if ch == '"' || ch == '\'' {
                self.lex_string(ch)?
            } else {
                self.lex_punct()?
            };
            tokens.push(Token {
                kind,
                line,
                column,
                offset,
            });
        }
    }

    fn lex_identifier(&mut self) -> TokenKind {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if is_identifier_continue(ch) {
                out.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        if let Some(keyword) = keyword_for(&out) {
            TokenKind::Keyword(keyword)
        } else {
            TokenKind::Identifier(out)
        }
    }

    fn lex_number(&mut self) -> Result<TokenKind, SyntaxError> {
        let line = self.line;
        let column = self.column;
        if self.peek() == Some('0') && matches!(self.peek_at(1), Some('x') | Some('X')) {
            self.advance();
            self.advance();
            let mut digits = String::new();
            while let Some(ch) = self.peek() {
                if ch.is_ascii_hexdigit() {
                    digits.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
            if digits.is_empty() {
                return self.error("hexadecimal literal needs at least one digit", line, column);
            }
            let value = u64::from_str_radix(&digits, 16).map_err(|_| {
                SyntaxError::new("hexadecimal literal is too large", line, column)
            })?;
            return Ok(TokenKind::Number(value as f64));
        }

        let mut text = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                text.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        if self.peek() == Some('.') {
            text.push('.');
            self.advance();
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    text.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            text.push('e');
            self.advance();
            if matches!(self.peek(), Some('+') | Some('-')) {
                text.push(self.advance().unwrap_or('+'));
            }
            let mut exponent_digits = 0;
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    text.push(ch);
                    self.advance();
                    exponent_digits += 1;
                } else {
                    break;
                }
            }
            if exponent_digits == 0 {
                return self.error("exponent needs at least one digit", line, column);
            }
        }
        text.parse::<f64>()
            .map(TokenKind::Number)
            .map_err(|_| SyntaxError::new("invalid numeric literal", line, column))
    }

    fn lex_string(&mut self, quote: char) -> Result<TokenKind, SyntaxError> {
        let start_line = self.line;
        let start_column = self.column;
        self.advance();
        let mut out = String::new();
        loop {
            let Some(ch) = self.advance() else {
                return self.error("unterminated string literal", start_line, start_column);
            };
            if ch == quote {
                return Ok(TokenKind::String(out));
            }
            if ch == '\n' || ch == '\r' {
                return self.error(
                    "string literal cannot contain an unescaped newline",
                    start_line,
                    start_column,
                );
            }
            if ch != '\\' {
                out.push(ch);
                continue;
            }

            let Some(escaped) = self.advance() else {
                return self.error("unterminated escape sequence", start_line, start_column);
            };
            match escaped {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000C}'),
                'v' => out.push('\u{000B}'),
                '0' => out.push('\0'),
                '\\' => out.push('\\'),
                '\'' => out.push('\''),
                '"' => out.push('"'),
                '\n' => {}
                '\r' => {
                    if self.peek() == Some('\n') {
                        self.advance();
                    }
                }
                'x' => {
                    let value = self.read_hex_digits(2, "\\x escape", start_line, start_column)?;
                    out.push(value);
                }
                'u' => {
                    let value = if self.peek() == Some('{') {
                        self.advance();
                        let mut digits = String::new();
                        while let Some(ch) = self.peek() {
                            if ch == '}' {
                                break;
                            }
                            if ch.is_ascii_hexdigit() && digits.len() < 6 {
                                digits.push(ch);
                                self.advance();
                            } else {
                                return self.error(
                                    "invalid \\u{...} escape",
                                    start_line,
                                    start_column,
                                );
                            }
                        }
                        if self.peek() != Some('}') || digits.is_empty() {
                            return self.error(
                                "unterminated \\u{...} escape",
                                start_line,
                                start_column,
                            );
                        }
                        self.advance();
                        u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32)
                    } else {
                        Some(self.read_hex_digits(4, "\\u escape", start_line, start_column)?)
                    };
                    let Some(value) = value else {
                        return self.error(
                            "unicode escape is not a valid scalar value",
                            start_line,
                            start_column,
                        );
                    };
                    out.push(value);
                }
                other => out.push(other),
            }
        }
    }

    fn read_hex_digits(
        &mut self,
        count: usize,
        what: &str,
        line: usize,
        column: usize,
    ) -> Result<char, SyntaxError> {
        let mut value = 0u32;
        for _ in 0..count {
            let Some(ch) = self.advance() else {
                return self.error(format!("unterminated {what}"), line, column);
            };
            let Some(digit) = ch.to_digit(16) else {
                return self.error(format!("invalid {what}"), line, column);
            };
            value = value * 16 + digit;
        }
        char::from_u32(value)
            .ok_or_else(|| SyntaxError::new(format!("invalid {what}"), line, column))
    }

    fn lex_punct(&mut self) -> Result<TokenKind, SyntaxError> {
        let line = self.line;
        let column = self.column;
        let ch = self.advance().unwrap_or('\0');
        let next = self.peek();
        let next2 = self.peek_at(1);
        let next3 = self.peek_at(2);

        let (punct, consumed) = match ch {
            '{' => (Punct::LBrace, 0),
            '}' => (Punct::RBrace, 0),
            '(' => (Punct::LParen, 0),
            ')' => (Punct::RParen, 0),
            '[' => (Punct::LBracket, 0),
            ']' => (Punct::RBracket, 0),
            ';' => (Punct::Semicolon, 0),
            ',' => (Punct::Comma, 0),
            '.' => (Punct::Dot, 0),
            ':' => (Punct::Colon, 0),
            '?' => (Punct::Question, 0),
            '~' => (Punct::Tilde, 0),
            '+' => match (next, next2) {
                (Some('+'), _) => (Punct::PlusPlus, 1),
                (Some('='), _) => (Punct::PlusAssign, 1),
                _ => (Punct::Plus, 0),
            },
            '-' => match (next, next2) {
                (Some('-'), _) => (Punct::MinusMinus, 1),
                (Some('='), _) => (Punct::MinusAssign, 1),
                _ => (Punct::Minus, 0),
            },
            '*' => match (next, next2) {
                (Some('*'), Some('=')) => (Punct::StarStarAssign, 2),
                (Some('*'), _) => (Punct::StarStar, 1),
                (Some('='), _) => (Punct::StarAssign, 1),
                _ => (Punct::Star, 0),
            },
            '/' => match (next, next2) {
                (Some('='), _) => (Punct::SlashAssign, 1),
                _ => (Punct::Slash, 0),
            },
            '%' => match (next, next2) {
                (Some('='), _) => (Punct::PercentAssign, 1),
                _ => (Punct::Percent, 0),
            },
            '=' => match (next, next2) {
                (Some('='), Some('=')) => (Punct::StrictEq, 2),
                (Some('='), _) => (Punct::EqEq, 1),
                _ => (Punct::Assign, 0),
            },
            '!' => match (next, next2) {
                (Some('='), Some('=')) => (Punct::StrictNe, 2),
                (Some('='), _) => (Punct::NotEq, 1),
                _ => (Punct::Not, 0),
            },
            '<' => match (next, next2) {
                (Some('<'), Some('=')) => (Punct::ShlAssign, 2),
                (Some('<'), _) => (Punct::Shl, 1),
                (Some('='), _) => (Punct::Le, 1),
                _ => (Punct::Lt, 0),
            },
            '>' => match (next, next2, next3) {
                (Some('>'), Some('>'), Some('=')) => (Punct::UShrAssign, 3),
                (Some('>'), Some('>'), _) => (Punct::UShr, 2),
                (Some('>'), Some('='), _) => (Punct::ShrAssign, 2),
                (Some('>'), _, _) => (Punct::Shr, 1),
                (Some('='), _, _) => (Punct::Ge, 1),
                _ => (Punct::Gt, 0),
            },
            '&' => match (next, next2) {
                (Some('&'), _) => (Punct::AndAnd, 1),
                (Some('='), _) => (Punct::AmpAssign, 1),
                _ => (Punct::Amp, 0),
            },
            '|' => match (next, next2) {
                (Some('|'), _) => (Punct::OrOr, 1),
                (Some('='), _) => (Punct::PipeAssign, 1),
                _ => (Punct::Pipe, 0),
            },
            '^' => match (next, next2) {
                (Some('='), _) => (Punct::CaretAssign, 1),
                _ => (Punct::Caret, 0),
            },
            other => {
                return self.error(
                    format!("unexpected character '{other}'"),
                    line,
                    column,
                )
            }
        };
        for _ in 0..consumed {
            self.advance();
        }
        Ok(TokenKind::Punct(punct))
    }
}

pub fn lex(source: &str) -> Result<Vec<Token>, SyntaxError> {
    Lexer::new(source).lex_all()
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, SyntaxError> {
    lex(source)
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphanumeric()
}

fn keyword_for(word: &str) -> Option<Keyword> {
    Some(match word {
        "var" => Keyword::Var,
        "let" => Keyword::Let,
        "const" => Keyword::Const,
        "function" => Keyword::Function,
        "if" => Keyword::If,
        "else" => Keyword::Else,
        "while" => Keyword::While,
        "for" => Keyword::For,
        "break" => Keyword::Break,
        "continue" => Keyword::Continue,
        "return" => Keyword::Return,
        "true" => Keyword::True,
        "false" => Keyword::False,
        "null" => Keyword::Null,
        "undefined" => Keyword::Undefined,
        "typeof" => Keyword::Typeof,
        "void" => Keyword::Void,
        "this" => Keyword::This,
        "do" => Keyword::Unsupported("do"),
        "switch" => Keyword::Unsupported("switch"),
        "case" => Keyword::Unsupported("case"),
        "default" => Keyword::Unsupported("default"),
        "try" => Keyword::Unsupported("try"),
        "catch" => Keyword::Unsupported("catch"),
        "finally" => Keyword::Unsupported("finally"),
        "throw" => Keyword::Unsupported("throw"),
        "class" => Keyword::Unsupported("class"),
        "new" => Keyword::Unsupported("new"),
        "delete" => Keyword::Unsupported("delete"),
        "in" => Keyword::Unsupported("in"),
        "instanceof" => Keyword::Unsupported("instanceof"),
        "of" => Keyword::Unsupported("of"),
        "async" => Keyword::Unsupported("async"),
        "await" => Keyword::Unsupported("await"),
        "yield" => Keyword::Unsupported("yield"),
        "import" => Keyword::Unsupported("import"),
        "export" => Keyword::Unsupported("export"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source)
            .expect("lex")
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn identifiers_numbers_and_comments() {
        let tokens = kinds(
            "let пример = 0x2a; // строка\nconst _x$ = 1.5e2; /* block */ var y = .25;",
        );
        assert_eq!(
            &tokens[..5],
            &[
                TokenKind::Keyword(Keyword::Let),
                TokenKind::Identifier("пример".into()),
                TokenKind::Punct(Punct::Assign),
                TokenKind::Number(42.0),
                TokenKind::Punct(Punct::Semicolon),
            ]
        );
        assert!(tokens.contains(&TokenKind::Keyword(Keyword::Const)));
        assert!(tokens.contains(&TokenKind::Identifier("_x$".into())));
        assert!(tokens.contains(&TokenKind::Number(150.0)));
        assert!(tokens.contains(&TokenKind::Number(0.25)));
    }

    #[test]
    fn strings_with_escapes_and_cyrillic() {
        let tokens = kinds(r#""Привет\n\t\x41\u0042\u{43}мир""#);
        assert_eq!(tokens[0], TokenKind::String("Привет\n\tABCмир".into()));
    }

    #[test]
    fn utf8_columns_count_characters() {
        let err = lex("let имя = \"незакрытая").expect_err("unterminated");
        assert_eq!(err.line, 1);
        assert_eq!(err.column, 11);
        assert!(err.to_string().contains("line 1, column 11"));
    }

    #[test]
    fn lexer_diagnostics_have_positions() {
        let err = lex("let x = ;\n/* ?? ").expect_err("comment");
        assert_eq!(err.line, 2);
        assert_eq!(err.column, 1);
        assert!(err.message.contains("unterminated block comment"));

        let err = lex("\"abc\n\"").expect_err("newline");
        assert_eq!(err.line, 1);
        assert_eq!(err.column, 1);
    }

    #[test]
    fn operators_are_longest_match() {
        let tokens = kinds(">>>= === !== **= >>> >>= <<= && || ++ --");
        assert!(tokens.contains(&TokenKind::Punct(Punct::UShrAssign)));
        assert!(tokens.contains(&TokenKind::Punct(Punct::StrictEq)));
        assert!(tokens.contains(&TokenKind::Punct(Punct::StrictNe)));
        assert!(tokens.contains(&TokenKind::Punct(Punct::StarStarAssign)));
        assert!(tokens.contains(&TokenKind::Punct(Punct::UShr)));
        assert!(tokens.contains(&TokenKind::Punct(Punct::ShrAssign)));
        assert!(tokens.contains(&TokenKind::Punct(Punct::ShlAssign)));
    }
}
