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

/// A type-erasing stream adapter: forwards to any underlying SATySFi token
/// stream behind one fixed concrete type.
///
/// `Parse` implementations are generic over the stream, and syan's
/// backtracking wraps the stream in a fresh `Dup<&mut _, _>` layer at every
/// enum/`Vec`/`Option` boundary, so the set of concrete stream types grows
/// with grammar nesting depth. Each `#[recurse]` engine is monomorphized
/// once per incoming stream type — a cross-SCC grammar reference (e.g. the
/// expression grammar embedding the pattern grammar at every `match` arm)
/// would therefore re-instantiate the entire inner engine at every distinct
/// stream type of the outer one, a multiplicative codegen blowup measured in
/// minutes of compile time and gigabytes of rustc memory. Parsing the inner
/// grammar through `&mut EraseStream` pins it to exactly one instantiation
/// crate-wide (see `cst::PatErased`).
pub struct EraseStream<'a> {
    inner: &'a mut dyn ParseStream<Atom = Atom, Error = Infallible>,
}

impl<'a> EraseStream<'a> {
    pub fn new(inner: &'a mut dyn ParseStream<Atom = Atom, Error = Infallible>) -> Self {
        EraseStream { inner }
    }
}

impl ParseStream for EraseStream<'_> {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        self.inner.next()
    }

    fn peek(&mut self) -> Option<&Self::Atom> {
        self.inner.peek()
    }

    fn push(&mut self, atom: Self::Atom) {
        self.inner.push(atom);
    }

    fn get_error(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn skip_sep(&mut self) -> bool {
        // Same eagerly-lexed token vector underneath: nothing to skip.
        false
    }
}

/// Adapts a stream with any error type to `Error = Infallible` so it can sit
/// behind [`EraseStream`]'s trait object. Our streams are always backed by
/// the eagerly lexed token vector, so the error channel is genuinely unused.
pub struct InfallibleAdapter<S>(pub S);

impl<S: ParseStream<Atom = Atom>> ParseStream for InfallibleAdapter<S> {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        self.0.next()
    }

    fn peek(&mut self) -> Option<&Self::Atom> {
        self.0.peek()
    }

    fn push(&mut self, atom: Self::Atom) {
        self.0.push(atom);
    }

    fn get_error(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn skip_sep(&mut self) -> bool {
        false
    }
}
