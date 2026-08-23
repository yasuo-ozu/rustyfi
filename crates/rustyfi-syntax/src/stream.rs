//! The parse source for the SATySFi surface grammar.
//!
//! The parse source is the eagerly lexed `Vec<Atom>`, wrapped by
//! [`AtomStream`]: syan core has no `IntoParseStream for Vec<_>`, so the
//! buffering lives here.
//!
//! Neither stream erasure (a `&mut dyn ParseStream` tower) nor a failure
//! high-water mark belongs here, obsoleted by syan on both counts:
//! `Parse::parse_stream` takes `&mut S` and recursion reborrows, so `S` is a
//! genuine fixed point and the instantiation set is finite without erasing
//! anything (and no stream operation is a virtual call); and `ParseError` is
//! span-generic, every variant carrying the position it failed at, so the
//! error reports itself.
//!
//! **The second half of that is now known to be false, and the mark exists
//! elsewhere because of it.** `ParseError` does carry a position, but not a
//! useful one for a failure inside a repetition: `Vec<TopBinding>` stops on
//! the binding that would not parse and rolls the stream back, and its error
//! is discarded rather than aggregated, so what surfaces is the enclosing
//! rule's "expected end of input" at the binding's START. Measured, a 0.0.6
//! error sixty bytes into a top-level `let` reports at byte 3; a 0.1 error
//! anywhere in a file reports on the `module` keyword on line 1, because a
//! 0.1 library IS one binding. `rustyfi-lsp`'s `high_water` module therefore
//! wraps this type in its own `ParseStream` to recover the furthest position
//! reached, and reduces the tree itself; folding both back in here — a
//! `usize` and one `max` in `next()`, plus a real `render_parse_error` beside
//! [`crate::cst::parse_file`] — would fix the compiler's own parse errors,
//! which are the worse of the two. Not done yet; do not add a THIRD mark
//! somewhere on the strength of the paragraph above.

use crate::token::Atom;
use std::convert::Infallible;
use syan::parse::tape::Tape;
use syan::parse::ParseStream;

/// A parse source over an eagerly lexed token vector.
///
/// Backtracking runs through syan's [`Tape`], which owns the pushback and the
/// checkpoint scopes, so this is a thin forwarding shim and nothing more.
pub struct AtomStream {
    tape: Tape<std::vec::IntoIter<Atom>>,
}

impl AtomStream {
    pub fn new(atoms: Vec<Atom>) -> Self {
        AtomStream {
            tape: Tape::new(atoms.into_iter()),
        }
    }
}

impl ParseStream for AtomStream {
    type Atom = Atom;
    type Error = Infallible;

    fn next(&mut self) -> Option<Self::Atom> {
        self.tape.next()
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
