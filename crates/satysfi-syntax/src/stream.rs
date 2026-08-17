//! Stream adapters for the `#[recurse]` monomorphization machinery of the
//! SATySFi surface grammar (see `cst::erased_leaf!`).
//!
//! The parse source itself is now just the eagerly lexed `Vec<Atom>`: syan
//! core provides `impl IntoParseStream for Vec<A: Clone>` (a `BufStream` with
//! LIFO pushback), and `ParseError` carries the failure span, so no bespoke
//! token stream (nor a high-water mark) is needed here anymore.

use crate::token::Atom;
use std::convert::Infallible;
use syan::parse::ParseStream;

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
