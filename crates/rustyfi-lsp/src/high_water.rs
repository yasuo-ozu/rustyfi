//! A [`ParseStream`] that remembers how far the parser ever got, and stops it
//! if it goes on too long.
//!
//! # The high-water mark
//!
//! syan reports a failure as a tree of alternatives, each carrying the span it
//! failed at — but **a repetition rule discards the failure that ended it**.
//! `Vec<TopBinding>` stops on the first binding that does not parse and rolls
//! the stream back, and nothing in the error it eventually reports remembers
//! how far that attempt got. What survives is the enclosing rule's own
//! complaint ("expected end of input") at the *start* of the binding, plus
//! whatever `ParseError::from_cause` kept — and its own comment concedes it
//! has "no record of how far each alternative got", so it keeps the FIRST
//! alternative's span.
//!
//! The consequence is worst under the 0.1 grammar, where a whole library file
//! is *one* top-level binding (`module M :> sig … end = struct … end`): every
//! error in the file, wherever it is, would be reported on the `module`
//! keyword on line 1. But it is not 0.1-specific — measured on 0.0.6, an
//! error sixty bytes into a long top-level `let` reports at the `let`, byte 3.
//! Both generations need this.
//!
//! The furthest-position-reached mark is the standard answer for a
//! backtracking parser, and the stream is the only place it can be observed:
//! it is a property of the *parse*, not of any one error value. This wrapper
//! records the furthest atom ever handed out and never forgets it, so
//! backtracking cannot erase the evidence.
//!
//! # The budget
//!
//! The 0.1 grammar backtracks exponentially on some incomplete inputs, and an
//! editor buffer is incomplete on most keystrokes. Measured, release build,
//! on prefixes of the bundled `dist-v01/packages/std-ja.satyh`:
//!
//! | prefix | 0.1 grammar | 0.0.6 grammar |
//! |---|---|---|
//! | 13,484 B | 13 ms | 0.2 ms |
//! | 13,669 B | 69 ms | 0.2 ms |
//! | 13,853 B | 334 ms | 0.2 ms |
//! | 14,223 B | **11.5 s** | 0.3 ms |
//!
//! Roughly ×5 per 200 bytes typed, and it does not stop there. A language
//! server that inherits that freezes the editor's diagnostics for the rest of
//! the session, on a file the user is in the middle of writing — which is
//! worse than having no language server at all.
//!
//! So the stream counts what it serves and reports end-of-input past
//! [`BUDGET`]. The parser reads that as a file that stopped early and unwinds
//! promptly; nothing has to know the budget exists, because
//! `analysis::parse_failure` already locates a failure from the mark. The
//! caller can distinguish the two outcomes with [`Self::exhausted`], and does,
//! so that a give-up is reported as a give-up rather than as a confident claim
//! about the token the parse happened to stop at.
//!
//! A count, not a clock: `analyze` must be a pure function of its input — the
//! same buffer has to produce the same diagnostics in a test, on a fast
//! machine and on a slow one, and in a browser.
//!
//! # Why here and not in `rustyfi-syntax`
//!
//! Because it is a scope call, not a design one. `rustyfi-syntax`'s `stream`
//! module explicitly declines to carry a mark — "Neither stream erasure … nor
//! a failure high-water mark belongs here, obsoleted by syan on both counts,
//! … `ParseError` is span-generic, every variant carrying the position it
//! failed at, so the error reports itself" — and the evidence above shows
//! that claim does not hold: the error reports a position, but not the one
//! the user needs.
//!
//! So this belongs upstream, folded into `AtomStream` itself, with
//! `analysis`'s reducer moved beside `parse_file`. That would fix the
//! *compiler's* diagnostics too, which are worse than the editor's: `rustyfi
//! doc.saty` today points at line 1 for an error on line 5, and for a 0.1
//! file prints kilobytes of `Debug`-formatted `Loc { … }`, because `cst.rs`'s
//! `render_parse_error` is `format!("{err:?}")` over the whole tree. Left as
//! a follow-up rather than done here, so that adding a language server does
//! not also rewrite the shared parser's error path.

use std::convert::Infallible;

use rustyfi_syntax::span::Span;
use rustyfi_syntax::stream::AtomStream;
use rustyfi_syntax::token::Atom;
use syan::parse::ParseStream;

/// How many atoms one parse may consume — counting every backtracked re-read
/// — before the stream declares end of input.
///
/// Calibrated from measurement, not guessed. A *clean* parse costs 16–20
/// serves per token; the most expensive file in the bundled corpus
/// (`dist/packages/math.satyh`, 9,698 tokens) finishes in about 190,000. This
/// is an order of magnitude above that, and about two orders of magnitude
/// below the 78 million the pathological prefix in the table above wanted —
/// so no real file can reach it, and the worst case a user can provoke is a
/// fraction of a second rather than eleven seconds.
pub const BUDGET: u64 = 2_000_000;

/// [`AtomStream`], plus a monotone record of the furthest point the parse
/// reached and a cap on how long it may go on.
pub struct HighWaterStream {
    inner: AtomStream,
    /// End byte of the furthest atom ever served; `0` if none was.
    furthest: usize,
    /// The span of that atom — kept as it is observed, rather than recovered
    /// afterwards by scanning a copy of every token's span, which is what
    /// this replaces.
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
    /// the generated leaf parsers are `next()` → match → `push()`-back-on-
    /// mismatch (`rustyfi-syntax`'s `leaf.rs`), so the offending token has
    /// already been pulled through the stream by the time the leaf rejects
    /// it. Reporting the token after it would put every squiggle one token to
    /// the right.
    pub fn furthest_span(&self) -> Option<Span> {
        self.furthest_span
    }

    /// Whether the parse hit [`BUDGET`] rather than reaching a real verdict.
    ///
    /// When this is true, the failure the parser reported means only "the
    /// stream ended", which is this type's doing and not the buffer's — so
    /// the caller must not dress it up as a claim about the source.
    pub fn exhausted(&self) -> bool {
        self.served >= BUDGET
    }

    fn observe(&mut self, span: Span) {
        // Monotone by construction: a rollback re-serves atoms already seen,
        // and the point of the mark is that backtracking does not lower it.
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
