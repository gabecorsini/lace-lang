use crate::token::{DurationUnit, Span, Token, TokenKind};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LexError {
    #[error("unexpected character '{ch}' at {span_start}..{span_end}")]
    UnexpectedChar {
        ch: char,
        span_start: usize,
        span_end: usize,
    },
    #[error("unterminated string literal at {span_start}..{span_end}")]
    UnterminatedString { span_start: usize, span_end: usize },
    #[error("invalid escape sequence at {span_start}..{span_end}")]
    InvalidEscape { span_start: usize, span_end: usize },
    #[error("invalid numeric literal '{literal}' at {span_start}..{span_end}")]
    InvalidNumber {
        literal: String,
        span_start: usize,
        span_end: usize,
    },
}

pub fn lex(input: &str) -> (Vec<Token>, Vec<LexError>) {
    let mut lexer = Lexer::new(input);
    lexer.lex_all();
    (lexer.tokens, lexer.errors)
}

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
    errors: Vec<LexError>,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            chars: src.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn lex_all(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.bump();
                continue;
            }
            if ch == '#' {
                if self.peek2() == Some('#') {
                    self.lex_doc_comment();
                } else {
                    self.skip_line_comment();
                }
                continue;
            }
            if ch == '/' && self.peek2() == Some('*') {
                self.skip_block_comment();
                continue;
            }
            let start = self.pos;
            match ch {
                'a'..='z' | 'A'..='Z' | '_' => self.lex_ident_or_keyword(start),
                '0'..='9' => self.lex_number(false, start),
                '-' => {
                    if matches!(self.peek2(), Some('0'..='9')) {
                        self.bump();
                        self.lex_number(true, start);
                    } else if self.peek2() == Some('>') {
                        self.bump();
                        self.bump();
                        self.push(TokenKind::Arrow, start);
                    } else {
                        self.bump();
                        self.push(TokenKind::Minus, start);
                    }
                }
                '"' => self.lex_string(start),
                '+' => {
                    self.bump();
                    if self.peek() == Some('+') {
                        self.bump();
                        self.push(TokenKind::PlusPlus, start);
                    } else {
                        self.push(TokenKind::Plus, start);
                    }
                }
                '*' => {
                    self.bump();
                    self.push(TokenKind::Star, start);
                }
                '/' => {
                    self.bump();
                    if self.peek() == Some('/') {
                        self.bump();
                        self.push(TokenKind::SlashSlash, start);
                    } else {
                        self.push(TokenKind::Slash, start);
                    }
                }
                '%' => {
                    self.bump();
                    self.push(TokenKind::Percent, start);
                }
                '=' => {
                    self.bump();
                    match self.peek() {
                        Some('=') => {
                            self.bump();
                            self.push(TokenKind::EqEq, start);
                        }
                        Some('>') => {
                            self.bump();
                            self.push(TokenKind::FatArrow, start);
                        }
                        _ => self.push(TokenKind::Assign, start),
                    }
                }
                '!' => {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        self.push(TokenKind::NotEq, start);
                    } else {
                        self.push(TokenKind::Bang, start);
                    }
                }
                '<' => {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        self.push(TokenKind::Le, start);
                    } else {
                        self.push(TokenKind::Lt, start);
                    }
                }
                '>' => {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        self.push(TokenKind::Ge, start);
                    } else {
                        self.push(TokenKind::Gt, start);
                    }
                }
                '&' if self.peek2() == Some('&') => {
                    self.bump();
                    self.bump();
                    self.push(TokenKind::AndAnd, start);
                }
                '|' => {
                    self.bump();
                    match self.peek() {
                        Some('|') => {
                            self.bump();
                            self.push(TokenKind::OrOr, start);
                        }
                        Some('>') => {
                            self.bump();
                            self.push(TokenKind::PipeGt, start);
                        }
                        _ => {
                            self.errors.push(LexError::UnexpectedChar {
                                ch,
                                span_start: start,
                                span_end: self.pos,
                            });
                        }
                    }
                }
                '?' => {
                    self.bump();
                    self.push(TokenKind::Question, start);
                }
                ':' => {
                    self.bump();
                    if self.peek() == Some(':') {
                        self.bump();
                        self.push(TokenKind::ColonColon, start);
                    } else {
                        self.push(TokenKind::Colon, start);
                    }
                }
                '(' => {
                    self.bump();
                    self.push(TokenKind::LParen, start);
                }
                ')' => {
                    self.bump();
                    self.push(TokenKind::RParen, start);
                }
                '{' => {
                    self.bump();
                    self.push(TokenKind::LBrace, start);
                }
                '}' => {
                    self.bump();
                    self.push(TokenKind::RBrace, start);
                }
                '[' => {
                    self.bump();
                    self.push(TokenKind::LBracket, start);
                }
                ']' => {
                    self.bump();
                    self.push(TokenKind::RBracket, start);
                }
                ',' => {
                    self.bump();
                    self.push(TokenKind::Comma, start);
                }
                '.' => {
                    self.bump();
                    self.push(TokenKind::Dot, start);
                }
                ';' => {
                    self.bump();
                    self.push(TokenKind::Semicolon, start);
                }
                '@' => {
                    self.bump();
                    self.push(TokenKind::At, start);
                }
                _ => {
                    self.bump();
                    self.errors.push(LexError::UnexpectedChar {
                        ch,
                        span_start: start,
                        span_end: self.pos,
                    });
                }
            }
        }

        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span {
                start: self.pos,
                end: self.pos,
            },
        });
    }

    fn lex_ident_or_keyword(&mut self, start: usize) {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                s.push(ch);
                self.bump();
            } else {
                break;
            }
        }

        let kind = match s.as_str() {
            "module" => TokenKind::Module,
            "use" => TokenKind::Use,
            "import" => TokenKind::Import,
            "as" => TokenKind::As,
            "fn" => TokenKind::Fn,
            "tool" => TokenKind::Tool,
            "record" => TokenKind::Record,
            "enum" => TokenKind::Enum,
            "type" => TokenKind::Type,
            "const" => TokenKind::Const,
            "extern" => TokenKind::Extern,
            "from" => TokenKind::From,
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "while" => TokenKind::While,
            "pure" => TokenKind::Pure,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "match" => TokenKind::Match,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "true" => TokenKind::BoolLit(true),
            "false" => TokenKind::BoolLit(false),
            "retries" => TokenKind::Retries,
            "timeout" => TokenKind::Timeout,
            "mock" => TokenKind::Mock,
            _ => {
                if s.chars().next().map(char::is_uppercase).unwrap_or(false) {
                    TokenKind::TypeIdent(s)
                } else {
                    TokenKind::Ident(s)
                }
            }
        };

        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        });
    }

    fn lex_number(&mut self, had_minus: bool, start: usize) {
        let mut text = String::new();
        if had_minus {
            text.push('-');
        }

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                text.push(ch);
                self.bump();
            } else {
                break;
            }
        }

        let mut is_float = false;
        if self.peek() == Some('.') && matches!(self.peek2(), Some('0'..='9')) {
            is_float = true;
            text.push('.');
            self.bump();
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    text.push(ch);
                    self.bump();
                } else {
                    break;
                }
            }
            if matches!(self.peek(), Some('e' | 'E')) {
                text.push(self.peek().unwrap());
                self.bump();
                if matches!(self.peek(), Some('+' | '-')) {
                    text.push(self.peek().unwrap());
                    self.bump();
                }
                let mut any_exp = false;
                while let Some(ch) = self.peek() {
                    if ch.is_ascii_digit() {
                        any_exp = true;
                        text.push(ch);
                        self.bump();
                    } else {
                        break;
                    }
                }
                if !any_exp {
                    self.errors.push(LexError::InvalidNumber {
                        literal: text,
                        span_start: start,
                        span_end: self.pos,
                    });
                    return;
                }
            }
        }

        if !is_float {
            let unit = if self.peek() == Some('m') && self.peek2() == Some('s') {
                self.bump();
                self.bump();
                Some(DurationUnit::Ms)
            } else if self.peek() == Some('s') {
                self.bump();
                Some(DurationUnit::S)
            } else if self.peek() == Some('m') {
                self.bump();
                Some(DurationUnit::M)
            } else if self.peek() == Some('h') {
                self.bump();
                Some(DurationUnit::H)
            } else {
                None
            };

            if let Ok(v) = text.parse::<i64>() {
                let kind = if let Some(u) = unit {
                    TokenKind::DurationLit(v, u)
                } else {
                    TokenKind::IntLit(v)
                };
                self.tokens.push(Token {
                    kind,
                    span: Span {
                        start,
                        end: self.pos,
                    },
                });
            } else {
                self.errors.push(LexError::InvalidNumber {
                    literal: text,
                    span_start: start,
                    span_end: self.pos,
                });
            }
            return;
        }

        self.tokens.push(Token {
            kind: TokenKind::FloatLit(text),
            span: Span {
                start,
                end: self.pos,
            },
        });
    }

    fn lex_string(&mut self, start: usize) {
        self.bump();
        let mut out = String::new();

        while let Some(ch) = self.peek() {
            match ch {
                '"' => {
                    self.bump();
                    self.tokens.push(Token {
                        kind: TokenKind::StringLit(out),
                        span: Span {
                            start,
                            end: self.pos,
                        },
                    });
                    return;
                }
                '\\' => {
                    self.bump();
                    match self.peek() {
                        Some('"') => {
                            out.push('"');
                            self.bump();
                        }
                        Some('\\') => {
                            out.push('\\');
                            self.bump();
                        }
                        Some('n') => {
                            out.push('\n');
                            self.bump();
                        }
                        Some('t') => {
                            out.push('\t');
                            self.bump();
                        }
                        Some('r') => {
                            out.push('\r');
                            self.bump();
                        }
                        Some('u') => {
                            self.bump();
                            let mut hex = String::new();
                            for _ in 0..4 {
                                if let Some(h) = self.peek() {
                                    if h.is_ascii_hexdigit() {
                                        hex.push(h);
                                        self.bump();
                                    } else {
                                        self.errors.push(LexError::InvalidEscape {
                                            span_start: start,
                                            span_end: self.pos,
                                        });
                                        return;
                                    }
                                } else {
                                    self.errors.push(LexError::InvalidEscape {
                                        span_start: start,
                                        span_end: self.pos,
                                    });
                                    return;
                                }
                            }
                            if let Ok(value) = u32::from_str_radix(&hex, 16) {
                                if let Some(c) = char::from_u32(value) {
                                    out.push(c);
                                } else {
                                    self.errors.push(LexError::InvalidEscape {
                                        span_start: start,
                                        span_end: self.pos,
                                    });
                                    return;
                                }
                            } else {
                                self.errors.push(LexError::InvalidEscape {
                                    span_start: start,
                                    span_end: self.pos,
                                });
                                return;
                            }
                        }
                        _ => {
                            self.errors.push(LexError::InvalidEscape {
                                span_start: start,
                                span_end: self.pos,
                            });
                            return;
                        }
                    }
                }
                _ => {
                    out.push(ch);
                    self.bump();
                }
            }
        }

        self.errors.push(LexError::UnterminatedString {
            span_start: start,
            span_end: self.pos,
        });
    }

    fn lex_doc_comment(&mut self) {
        let start = self.pos;
        // consume `##`
        self.bump();
        self.bump();
        // skip optional single space
        if self.peek() == Some(' ') {
            self.bump();
        }
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                self.bump();
                break;
            }
            text.push(ch);
            self.bump();
        }
        self.push(TokenKind::DocComment(text), start);
    }

    fn skip_line_comment(&mut self) {
        while let Some(ch) = self.peek() {
            self.bump();
            if ch == '\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) {
        self.bump();
        self.bump();
        while let Some(ch) = self.peek() {
            if ch == '*' && self.peek2() == Some('/') {
                self.bump();
                self.bump();
                return;
            }
            self.bump();
        }
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        });
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    #[allow(dead_code)]
    fn _slice(&self, start: usize, end: usize) -> String {
        self.src
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }
}
