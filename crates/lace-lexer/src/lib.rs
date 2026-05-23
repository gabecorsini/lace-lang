pub mod lexer;
pub mod token;

pub use lexer::{lex, LexError};
pub use token::{DurationUnit, Span, Token, TokenKind};
