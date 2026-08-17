//! Stream adapters for the `#[recurse]` monomorphization machinery of the
//! SATySFi surface grammar (see `cst::erased_leaf!`).
//!
//! The parse source is the eagerly lexed `Vec<Atom>`, wrapped by
//! [`AtomStream`]. syan core has neither an `IntoParseStream for Vec<_>` nor a
//! span on `ParseError` (`error.rs`'s `ParseError::new` takes the span and
//! discards it), so both the buffering and the failure position live here: the
//! stream keeps a high-water mark of the furthest atom any attempt reached,
//! which is what a backtracking parser should report anyway.

use crate::span::Span;
use crate::token::Atom;
use std::convert::Infallible;
use syan::parse::tape::Tape;
use syan::parse::ParseStream;

/// A parse source over an eagerly lexed token vector.
///
/// Backtracking runs through syan's [`Tape`], so the stream sees the same atom
/// more than once; [`AtomStream::furthest`] is therefore a maximum, not a
/// cursor. It is the position to blame in an error message — the deepest point
/// the grammar managed to reach before every alternative failed.
pub struct AtomStream {
    tape: Tape<std::vec::IntoIter<Atom>>,
    furthest: Span,
}

impl AtomStream {
    pub fn new(atoms: Vec<Atom>) -> Self {
        AtomStream {
            tape: Tape::new(atoms.into_iter()),
            furthest: Span::default(),
        }
    }

    /// The furthest span consumed by any attempt, for error reporting.
    pub fn furthest(&self) -> Span {
        self.furthest
    }
}

impl ParseStream for AtomStream {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        let atom = self.tape.next()?;
        if atom.span.end.byte > self.furthest.end.byte {
            self.furthest = atom.span;
        }
        Some(atom)
    }

    fn peek(&mut self) -> Option<&Self::Atom> {
        self.tape.peek()
    }

    fn push(&mut self, atom: Self::Atom) {
        self.tape.push(atom);
    }

    fn checkpoint_raw(&mut self) -> u64 {
        self.tape.checkpoint()
    }

    fn rollback_raw(&mut self, raw: u64) {
        self.tape.rollback(raw);
    }

    fn commit_raw(&mut self, raw: u64) {
        self.tape.commit(raw);
    }

    fn get_error(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn skip_sep(&mut self) -> bool {
        // Already lexed: there is no separator atom to skip.
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

    // Forwarded, never defaulted: the trait's defaults would drop the
    // transaction on the floor, and a leaf parser backtracks through whatever
    // stream it was handed — which, for the erased grammar, is this one.
    fn checkpoint_raw(&mut self) -> u64 {
        self.inner.checkpoint_raw()
    }

    fn rollback_raw(&mut self, raw: u64) {
        self.inner.rollback_raw(raw);
    }

    fn commit_raw(&mut self, raw: u64) {
        self.inner.commit_raw(raw);
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

    fn checkpoint_raw(&mut self) -> u64 {
        self.0.checkpoint_raw()
    }

    fn rollback_raw(&mut self, raw: u64) {
        self.0.rollback_raw(raw);
    }

    fn commit_raw(&mut self, raw: u64) {
        self.0.commit_raw(raw);
    }

    fn get_error(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn skip_sep(&mut self) -> bool {
        false
    }
}
