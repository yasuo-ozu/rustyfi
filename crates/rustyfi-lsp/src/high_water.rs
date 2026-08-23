//! A [`ParseStream`] that remembers how far the parser ever got, and stops it
//! if it goes on too long.
//!
//! # The high-water mark
//!
//! syan reports a failure as a tree of alternatives, each carrying the span it
//! failed at — but **a repetition rule discards the failure that ended it**.
//! `Vec<TopBinding>` stops on the first binding that does not parse and rolls
//! the stream back; what survives is the enclosing rule's complaint ("expected
//! end of input") at the *start* of that binding, and `ParseError::from_cause`
//! keeps the FIRST alternative's span because it has no record of how far each
//! alternative got. Under the 0.1 grammar a whole library file is *one*
//! top-level binding, so every error in it would be reported on line 1's
//! `module` keyword; 0.0.6 has the same problem inside a long top-level `let`.
//!
//! The furthest position reached is the standard answer, and the stream is the
//! only place it can be observed — it is a property of the *parse*, not of any
//! one error value. This wrapper records the furthest atom ever handed out and
//! never lowers it, so backtracking cannot erase the evidence.
//!
//! # The budget
//!
//! The 0.1 grammar backtracks exponentially on some incomplete inputs, and an
//! editor buffer is incomplete on most keystrokes: a few hundred more bytes of
//! a half-typed library can take the parse from milliseconds to tens of
//! seconds, which would freeze the editor's diagnostics for the rest of the
//! session.
//!
//! So the stream counts what it serves and reports end-of-input past
//! [`BUDGET`]. The parser reads that as a file that stopped early and unwinds;
//! nothing else has to know the budget exists, because `analysis` already
//! locates a failure from the mark. The caller distinguishes the two outcomes
//! with [`Self::exhausted`] so that a give-up is reported as a give-up rather
//! than as a confident claim about the token the parse stopped at.
//!
//! A count, not a clock: `analyze` must be a pure function of its input, so
//! the same buffer produces the same diagnostics on a fast machine, a slow one
//! and in a browser.

use std::convert::Infallible;

use rustyfi_syntax::span::Span;
use rustyfi_syntax::stream::AtomStream;
use rustyfi_syntax::token::Atom;
use syan::parse::ParseStream;

/// How many atoms one parse may consume — counting every backtracked re-read
/// — before the stream declares end of input.
///
/// An order of magnitude above what the most expensive file in the bundled
/// corpus needs, and far below what a pathological half-typed 0.1 buffer would
/// take, so no real file can reach it. Lowering it risks cutting a real parse
/// short; raising it gives back the multi-second stalls it exists to bound.
pub const BUDGET: u64 = 2_000_000;

/// [`AtomStream`], plus a monotone record of the furthest point the parse
/// reached and a cap on how long it may go on.
pub struct HighWaterStream {
    inner: AtomStream,
    /// End byte of the furthest atom ever served; `0` if none was.
    furthest: usize,
    /// The span of that atom, kept as it is observed.
    furthest_span: Option<Span>,
    served: u64,
}

impl HighWaterStream {
    /// Wrap an eagerly lexed atom vector.
    pub fn new(atoms: Vec<Atom>) -> Self {
        HighWaterStream {
            inner: AtomStream::new(atoms),
            furthest: 0,
            furthest_span: None,
            served: 0,
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

    /// The span of the atom that set [`Self::furthest`] — the token the parse
    /// stopped at.
    ///
    /// It is the token *ending* at the mark, not the one starting after it:
    /// the generated leaf parsers are `next()` → match → push-back-on-mismatch
    /// (`rustyfi-syntax`'s `leaf.rs`), so the offending token has already been
    /// pulled through the stream when the leaf rejects it. Reporting the token
    /// after it would put every squiggle one token to the right.
    pub fn furthest_span(&self) -> Option<Span> {
        self.furthest_span
    }

    /// Whether the parse hit [`BUDGET`] rather than reaching a real verdict.
    ///
    /// When true, the failure the parser reported means only "the stream
    /// ended", which is this type's doing and not the buffer's — so the caller
    /// must not dress it up as a claim about the source.
    pub fn exhausted(&self) -> bool {
        self.served >= BUDGET
    }

    fn observe(&mut self, span: Span) {
        // Monotone: a rollback re-serves atoms already seen, and the point of
        // the mark is that backtracking does not lower it.
        if span.end.byte > self.furthest || self.furthest_span.is_none() {
            self.furthest = span.end.byte;
            self.furthest_span = Some(span);
        }
    }
}

impl ParseStream for HighWaterStream {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        // Checked before the read, so an exhausted stream stays exhausted
        // however many times the parser retries.
        if self.served >= BUDGET {
            return None;
        }
        self.served += 1;
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
