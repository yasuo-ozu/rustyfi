//! The parse source for the SATySFi surface grammar.
//!
//! The parse source is the eagerly lexed `Vec<Atom>`, wrapped by
//! [`AtomStream`]: syan core has no `IntoParseStream for Vec<_>`, so the
//! buffering lives here.
//!
//! Two things this module used to carry are gone, both obsoleted by syan:
//!
//! * `EraseStream`/`InfallibleAdapter` — a `&mut dyn ParseStream` tower that
//!   pinned the grammar to one monomorphization. `Parse::parse_stream` now
//!   takes `&mut S` and recursion reborrows, so `S` is a genuine fixed point
//!   and the instantiation set is finite without erasing anything. Every
//!   stream operation in the parser stops being a virtual call.
//! * the failure high-water mark — `ParseError` is span-generic now and every
//!   variant carries the position it failed at, so the error reports itself.

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
