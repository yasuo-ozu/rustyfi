//! Lexer, token stream, and surface grammar for SATySFi.
//!
//! Port of the v0.0.6 SATySFi frontend (`lexer.mll` / `parser.mly`) on top of
//! the syan parser framework.

pub mod cst;
pub mod leaf;
pub mod lexer;
pub mod span;
pub mod stream;
pub mod token;

pub use cst::{parse_file, ParseFileError};
pub use lexer::{lex, LexError};
pub use stream::TokenStream;
pub use span::{Loc, Span};
pub use token::{Atom, Token};
