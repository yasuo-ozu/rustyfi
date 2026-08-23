//! The parse source for the SATySFi surface grammar.
//!
//! The parse source is the eagerly lexed `Vec<Atom>`, wrapped by
//! [`AtomStream`]: syan core has no `IntoParseStream for Vec<_>`, so the
//! buffering lives here.
//!
//! Stream erasure (a `&mut dyn ParseStream` tower) does not belong here, and
//! is obsoleted by syan: `Parse::parse_stream` takes `&mut S` and recursion
//! reborrows, so `S` is a genuine fixed point and the instantiation set is
//! finite without erasing anything, and no stream operation is a virtual call.
//!
//! # The high-water mark
//!
//! This module used to decline a failure high-water mark too, on the grounds
//! that "`ParseError` is span-generic, every variant carrying the position it
//! failed at, so the error reports itself". **That was false**, and this type
//! now carries the mark because of it. `ParseError` does carry a position, but
//! not a useful one for a failure inside a repetition: `Vec<TopBinding>` stops
//! on the binding that would not parse and rolls the stream back, and its
//! error is discarded rather than aggregated, so what surfaces is the
//! enclosing rule's "expected end of input" at the binding's START. Measured,
//! a 0.0.6 error sixty bytes into a top-level `let` reported at byte 3; a 0.1
//! error anywhere in a file reported on the `module` keyword on line 1,
//! because a 0.1 library IS one binding.
//!
//! The furthest-position-reached mark is the standard answer for a
//! backtracking parser, and the stream is the only place it can be observed:
//! it is a property of the *parse*, not of any one error value. `next()`
//! records the furthest atom ever handed out and never forgets it, so
//! backtracking cannot erase the evidence, and
//! [`crate::parse_error::locate`] turns mark + error tree into one diagnostic.
//!
//! # The budget
//!
//! Both grammars are ordered-choice backtrackers, so an unfactored common
//! prefix costs a *factor* per nesting level rather than a constant. The one
//! such prefix that had been measured is gone (see [`Budget`]); the cap
//! remains, because it is what makes the next one a slow error instead of a
//! hang — see [`Budget`] for why a *compiler*, and not only a language
//! server, wants one.

use crate::span::Span;
use crate::token::Atom;
use std::convert::Infallible;
use syan::parse::tape::Tape;
use syan::parse::ParseStream;

/// How much backtracking one parse may do before [`AtomStream`] declares the
/// input unparseable and reports end of input.
///
/// The unit is a **serve**: one atom handed out by [`ParseStream::next`],
/// counting every re-read a rollback causes. A count and not a clock, so the
/// same source produces the same verdict on a fast machine, on a slow one, in
/// a test and in a browser.
///
/// # Why a compiler has one at all
///
/// Because without it the compiler does not report anything. Measured on a
/// release build, over chains of `let vN = N in` ending in a `let` with no
/// right-hand side:
///
/// | file | error on | before |
/// |---|---|---|
/// | 9 lines | line 6 | exit 1, 7 ms |
/// | 15 lines | line 12 | exit 1, 32 ms |
/// | 35 lines | line 32 | **still running after 100 s** |
///
/// The cause was a plain unfactored common prefix: `Expr::LetIn` and
/// `Expr::LetPatternIn` both began `let ‹target› = ‹expr› in ‹body›`, so a
/// failure in the innermost body was re-derived exactly twice per enclosing
/// `let`. Measured, serves against chain length: 1,115 at 3, 9,459 at 6,
/// 76,211 at 9, 610,227 at 12, 4,882,355 at 15 — ×2.000 each time.
///
/// **That prefix is now factored** and the table above is history:
/// `cst::PatNonVarErased` refuses a bare-variable destructuring target — the
/// restriction upstream 0.1 spells `pattern_non_var` — which makes the two
/// alternatives disjoint at the token after `let`. The same chain costs 411,
/// 750, 1,089, 1,428, 1,767: an arithmetic progression, +113 per `let`,
/// pinned as an exact equality by `parse_errors.rs`'s
/// `one_more_let_costs_a_constant_number_of_serves`. No input is known that
/// reaches this cap any more.
///
/// The cap stays anyway, and the row above is why it was worth having: the
/// 0.1 grammar was separately observed to blow up ×5 per 200 bytes on
/// truncated prefixes of the bundled `std-ja.satyh`, from a *different*
/// prefix that has not been chased down. What a budget buys, and a grammar
/// fix does not, is that the next such prefix is a slow error instead of a
/// hang.
///
/// The give-up is reported as a give-up
/// ([`crate::ParseFailureKind::GaveUp`]), never as a claim about the token
/// the parse happened to stop at — and it still carries the high-water mark's
/// position, which in every case measured is the line the author must look at.
///
/// # Why it scales with the input
///
/// A cap has to be unreachable by any honest parse of any honest file, and
/// "honest" is a property per token, not per file: a fixed ceiling that a
/// 300-line file cannot reach is one a generated 30,000-line file can. So the
/// cap is a per-atom allowance, and only *superlinear* backtracking can
/// outrun it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget(u64);

impl Budget {
    /// Serves per atom an honest parse is allowed.
    ///
    /// Calibrated from measurement, not guessed: a clean parse costs 14–17
    /// serves per atom, and the worst of the 77 files in the bundled corpus
    /// (`dist-v01/packages/tabular.satyh`) costs 34.7. This is roughly sixty
    /// times that, and `parse_errors.rs`'s
    /// `the_bundled_corpus_stays_far_under_the_per_atom_budget` re-measures
    /// the corpus on every run rather than trusting the figure.
    pub const PER_ATOM: u64 = 2_048;

    /// Floor, so that a small file still gets the allowance a mid-sized one
    /// would.
    ///
    /// Without it a ten-line file would be capped at a few thousand serves and
    /// would give up on constructs a hundred-line file resolves. At roughly
    /// 10M serves per second this is about a second of trying.
    ///
    /// **Do not raise this to fix a give-up.** While the `let` prefix above
    /// was unfactored, the floor bought a broken chain of fifteen `let`s a
    /// real verdict and each further doubling bought exactly one more `let` —
    /// which is the shape of the argument in general: against a superlinear
    /// grammar the budget cannot buy diagnostic quality, only bound the
    /// damage. A give-up means a production needs left-factoring; the number
    /// to change is in `cst.rs`, not here.
    pub const FLOOR: u64 = 8_000_000;

    /// The allowance for a token vector of `atoms` atoms.
    pub const fn for_atoms(atoms: usize) -> Self {
        let scaled = (atoms as u64).saturating_mul(Self::PER_ATOM);
        // `Ord::max` is not a `const fn`, hence the `if`.
        Budget(if scaled > Self::FLOOR {
            scaled
        } else {
            Self::FLOOR
        })
    }

    /// An explicit allowance, for a caller with its own responsiveness
    /// requirement — a language server spends less than a compiler, because a
    /// human is waiting on every keystroke.
    pub const fn exactly(serves: u64) -> Self {
        Budget(serves)
    }

    /// No cap at all: the parse runs to a verdict or forever.
    ///
    /// For a caller that has bounded the work some other way, and for pinning
    /// the unbounded behaviour in a test.
    pub const fn unlimited() -> Self {
        Budget(u64::MAX)
    }

    /// The allowance, in serves.
    pub const fn serves(self) -> u64 {
        self.0
    }
}

/// A parse source over an eagerly lexed token vector, which remembers how far
/// the parse ever got and stops it if it goes on too long.
///
/// Backtracking runs through syan's [`Tape`], which owns the pushback and the
/// checkpoint scopes, so the forwarding half of this is a thin shim; the mark
/// and the budget are the parts that are not.
pub struct AtomStream {
    tape: Tape<std::vec::IntoIter<Atom>>,
    /// End byte of the furthest atom ever served; `0` if none was.
    furthest: usize,
    /// The span of that atom — kept as it is observed, rather than recovered
    /// afterwards by scanning every token's span.
    furthest_span: Option<Span>,
    served: u64,
    budget: u64,
}

impl AtomStream {
    /// Wrap an eagerly lexed atom vector, with the budget [`Budget`]
    /// calibrates for its size.
    pub fn new(atoms: Vec<Atom>) -> Self {
        let budget = Budget::for_atoms(atoms.len());
        Self::with_budget(atoms, budget)
    }

    /// [`Self::new`] with the budget chosen by the caller.
    pub fn with_budget(atoms: Vec<Atom>, budget: Budget) -> Self {
        AtomStream {
            tape: Tape::new(atoms.into_iter()),
            furthest: 0,
            furthest_span: None,
            served: 0,
            budget: budget.serves(),
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
    /// mismatch (see [`crate::leaf`]), so the offending token has already been
    /// pulled through the stream by the time the leaf rejects it. Reporting
    /// the token after it would put every diagnostic one token to the right.
    pub fn furthest_span(&self) -> Option<Span> {
        self.furthest_span
    }

    /// Whether the parse hit its budget rather than reaching a real verdict.
    ///
    /// When this is true, the failure the parser reported means only "the
    /// stream ended", which is this type's doing and not the source's — so the
    /// caller must not dress it up as a claim about the source.
    /// [`crate::parse_error::locate`] does not.
    pub fn exhausted(&self) -> bool {
        self.served >= self.budget
    }

    /// How many atoms have been served, counting every re-read. Exposed for
    /// calibrating [`Budget`] against a real corpus.
    pub fn served(&self) -> u64 {
        self.served
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

impl ParseStream for AtomStream {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        // Checked before the read, so an exhausted stream stays exhausted
        // however many times the parser retries.
        if self.served >= self.budget {
            return None;
        }
        self.served += 1;
        let atom = self.tape.next()?;
        self.observe(atom.span);
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
