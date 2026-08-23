//! How much backtracking this crate will spend on one buffer.
//!
//! The *mechanism* — a furthest-position-reached mark and a serve counter —
//! used to live here, in a `HighWaterStream` wrapping `AtomStream`. It is now
//! in [`rustyfi_syntax::stream::AtomStream`] itself, because the compiler
//! needed it just as badly: `rustyfi doc.saty` used to point at line 1 for an
//! error on line 5, and to hang outright on a 35-line file.
//!
//! What stays here is the *policy*, which is genuinely different for an
//! editor. [`rustyfi_syntax::stream::Budget::for_atoms`] scales the allowance
//! with the file so that no honest parse of any size can hit it, and lets a
//! pathological one run for as long as that takes — right for a compiler,
//! which is asked once and must answer about the file it was given. A language
//! server is asked again on every keystroke, about a file that is *expected*
//! to be incomplete, and a wrong-but-instant answer beats a right one that
//! arrives after the user has typed three more lines. So it spends a flat
//! [`BUDGET`] and gives up sooner.
//!
//! Measured, release build, on prefixes of the bundled 0.1
//! `dist-v01/packages/std-ja.satyh` — the shape that forced this:
//!
//! | prefix | 0.1 grammar | 0.0.6 grammar |
//! |---|---|---|
//! | 13,484 B | 13 ms | 0.2 ms |
//! | 13,669 B | 69 ms | 0.2 ms |
//! | 13,853 B | 334 ms | 0.2 ms |
//! | 14,223 B | **11.5 s** | 0.3 ms |
//!
//! Roughly ×5 per 200 bytes typed, and it does not stop there.

use rustyfi_syntax::stream::{AtomStream, Budget};
use rustyfi_syntax::token::Atom;

/// How many atoms one parse may consume — counting every backtracked re-read
/// — before the stream declares end of input.
///
/// Calibrated from measurement, not guessed. A *clean* parse costs 14–20
/// serves per token; the largest file in the bundled corpus
/// (`dist/packages/math.satyh`, 9,698 tokens) finishes in about 190,000, and
/// the worst *ratio* anywhere in the corpus is 34.7 serves per token
/// (`dist-v01/packages/tabular.satyh`, re-measured on every run by
/// `rustyfi-syntax`'s `parse_errors.rs`). This is an order of magnitude above
/// the largest and about two orders of magnitude below the 78 million the
/// pathological prefix in the module doc wanted — so no real file can reach
/// it, and the worst case a user can provoke is a fraction of a second rather
/// than eleven seconds.
///
/// A *flat* count, unlike the compiler's, and that is the whole difference
/// between the two policies: this one is a promise about latency, so it must
/// not grow with the file.
///
/// A count, not a clock: every entry point of this crate must be a pure
/// function of its input — the same buffer has to produce the same answer in a
/// test, on a fast machine and on a slow one, and in a browser.
pub const BUDGET: Budget = Budget::exactly(2_000_000);

/// A parse source over `atoms` under this crate's [`BUDGET`].
///
/// Every parse this crate starts goes through here, so diagnostics, the symbol
/// outline and the semantic model cannot drift into three different
/// responsiveness policies.
pub(crate) fn stream(atoms: Vec<Atom>) -> AtomStream {
    AtomStream::with_budget(atoms, BUDGET)
}
