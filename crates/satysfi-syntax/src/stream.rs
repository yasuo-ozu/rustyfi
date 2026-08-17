//! The syan `ParseStream` over the eagerly lexed token vector.

use crate::span::Span;
use crate::token::Atom;
use std::convert::Infallible;
use syan::parse::ParseStream;
use syan::span::Spanned;

/// A pushback-capable stream over pre-lexed tokens. Tracks a high-water mark
/// (the furthest token ever consumed) so that a parse failure can be reported
/// with a source position even though syan's `ParseError` carries no span.
pub struct TokenStream {
    toks: Vec<Atom>,
    pos: usize,
    buf: Vec<Atom>,
    hwm: usize,
}

impl TokenStream {
    pub fn new(toks: Vec<Atom>) -> Self {
        TokenStream {
            toks,
            pos: 0,
            buf: Vec::new(),
            hwm: 0,
        }
    }

    /// The span around the furthest point the parser ever reached.
    pub fn high_water_span(&self) -> Span {
        let idx = self.hwm.min(self.toks.len().saturating_sub(1));
        self.toks.get(idx).map(|a| a.span()).unwrap_or_default()
    }
}

impl ParseStream for TokenStream {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        if let Some(buffered) = self.buf.pop() {
            return Some(buffered);
        }
        let atom = self.toks.get(self.pos)?.clone();
        self.pos += 1;
        self.hwm = self.hwm.max(self.pos);
        Some(atom)
    }

    fn peek(&mut self) -> Option<&Self::Atom> {
        if let Some(buffered) = self.buf.last() {
            return Some(buffered);
        }
        self.toks.get(self.pos)
    }

    fn push(&mut self, atom: Self::Atom) {
        self.buf.push(atom);
    }

    fn get_error(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn skip_sep(&mut self) -> bool {
        // Whitespace and comments never survive the lexer; there is nothing to skip.
        false
    }
}
