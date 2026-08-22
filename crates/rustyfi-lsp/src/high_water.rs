//! A [`ParseStream`] that remembers how far the parser ever got.
//!
//! # Why this exists
//!
//! syan reports a failure as a tree of alternatives, each carrying the span it
//! failed at, and for the 0.0.6 grammar that is enough: the top level is a
//! flat run of bindings, so a failure inside one of them surfaces as
//! "expected end of input" at the first token the parser could not consume,
//! which is the right place to draw a squiggle.
//!
//! The 0.1 grammar is shaped differently, and there it is not enough. A 0.1
//! library file is *one* top-level binding — `module M :> sig … end = struct …
//! end` — spanning the whole file. When something inside the `struct` fails,
//! the enclosing alternative fails as a whole, and
//! `ParseError::from_cause` (whose own comment concedes it has "no record of
//! how far each alternative got") keeps only the *first* alternative's span.
//! The inner failure is not merely mis-ranked, it is **absent from the tree**
//! — verified by dumping it. The diagnostic therefore lands on the `module`
//! keyword on line 1 of every 0.1 file, whatever the real error was, which is
//! very nearly useless.
//!
//! The furthest-position-reached mark is the standard answer for a
//! backtracking parser, and the stream is the only place it can be observed:
//! it is a property of the *parse*, not of any one error value. This wrapper
//! records the end of the furthest atom ever handed out and never lowers it,
//! so backtracking cannot erase the evidence.
//!
//! `rustyfi-syntax`'s own `stream` module explicitly declines to carry one
//! ("Neither stream erasure … nor a failure high-water mark belongs here,
//! obsoleted by syan on both counts"), which is why it lives here, in the one
//! consumer whose whole job is turning failures into positions, rather than
//! being pushed back into the shared crate.

use std::convert::Infallible;

use rustyfi_syntax::span::Span;
use rustyfi_syntax::stream::AtomStream;
use rustyfi_syntax::token::Atom;
use syan::parse::ParseStream;

/// [`AtomStream`] plus a monotone record of the furthest byte the parse
/// reached.
pub struct HighWaterStream {
    inner: AtomStream,
    furthest: usize,
}

impl HighWaterStream {
    /// Wrap an eagerly lexed atom vector.
    pub fn new(atoms: Vec<Atom>) -> Self {
        HighWaterStream {
            inner: AtomStream::new(atoms),
            furthest: 0,
        }
    }

    /// End byte of the furthest atom the parser ever consumed; `0` if it
    /// consumed nothing.
    ///
    /// Consumed, not peeked: a lookahead that rejects a token has not made
    /// progress through it, and counting it would push every diagnostic one
    /// token to the right.
    pub fn furthest(&self) -> usize {
        self.furthest
    }

    fn observe(&mut self, span: Span) {
        // Monotone by construction: a rollback re-serves atoms already seen,
        // and the point of the mark is that backtracking does not lower it.
        self.furthest = self.furthest.max(span.end.byte);
    }
}

impl ParseStream for HighWaterStream {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        let atom = self.inner.next()?;
        self.observe(atom.span);
        Some(atom)
    }

    fn peek(&mut self) -> Option<&Self::Atom> {
        self.inner.peek()
    }

    fn push(&mut self, atom: Self::Atom) {
        self.inner.push(atom);
    }

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
        self.inner.get_error()
    }

    fn skip_sep(&mut self) -> bool {
        self.inner.skip_sep()
    }
}
