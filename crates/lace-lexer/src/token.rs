#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationUnit {
    Ms,
    S,
    M,
    H,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    TypeIdent(String),
    IntLit(i64),
    FloatLit(String),
    StringLit(String),
    BoolLit(bool),
    DurationLit(i64, DurationUnit),

    Module,
    Use,
    Import,
    As,
    Fn,
    Tool,
    Record,
    Enum,
    Type,
    Const,
    Extern,
    From,
    Let,
    Mut,
    For,
    In,
    While,
    Pure,
    If,
    Else,
    Match,
    Return,
    Break,
    Continue,
    Retries,
    Timeout,
    Mock,

    Plus,
    Minus,
    Star,
    Slash,
    SlashSlash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Gt,
    Le,
    Ge,
    AndAnd,
    OrOr,
    PlusPlus,
    PipeGt,
    Question,
    Bang,
    Assign,
    Arrow,
    FatArrow,
    ColonColon,

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Colon,
    Semicolon,
    At,

    Eof,

    /// `## ...` doc-comment line (text after the `## ` prefix, trimmed)
    DocComment(String),
}
