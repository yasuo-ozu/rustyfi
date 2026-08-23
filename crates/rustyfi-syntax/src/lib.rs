//! Lexer, token stream, and surface grammar for SATySFi.
//!
//! Port of the v0.0.6 SATySFi frontend (`lexer.mll` / `parser.mly`) on top of
//! the syan parser framework.

pub mod cst;
pub mod cst_v1;
#[macro_use]
mod leaf_macro;
pub mod leaf;
pub mod lexer;
pub mod parse_error;
pub mod span;
pub mod stream;
pub mod token;
pub mod version;

pub use cst::{parse_file, ParseFileError};
pub use cst_v1::{parse_file_v1, FileV1};
pub use lexer::{lex, lex_partial, lex_with_version, LexError};
pub use parse_error::ParseFailureKind;
pub use span::{Loc, Span};
pub use token::{Atom, Token};
pub use version::{sniff_headers, sniff_version, HeaderSniff, ParseVersionError, RustyfiVersion};
