//! Which of SATySFi's four lexical areas a point in the token stream is in.
//!
//! The one place this fold is written. Two features need it and they need it
//! for opposite reasons — [`crate::completions`] asks "which namespace may the
//! cursor name?", [`crate::format`] asks "may these bytes be rewritten?" — and
//! a second copy of the transition table would be a fork whose two halves
//! disagree silently, on exactly the constructs (`\cmd(…){…}`, `${… !{…} …}`)
//! that are rare enough not to be noticed.
//!
//! # Why a token fold is faithful
//!
//! It is not a re-derivation of the lexer's mode stack; it *is* that stack,
//! read off the tokens the stack itself produced. `rustyfi_syntax`'s lexer
//! decides the area with a `Vec<Mode>` (`lexer.rs`'s `push_mode`/`pop_mode`)
//! and the token it emits at every push and pop records which way it went: a
//! `{` in program text lexes as [`Token::BHorzGrp`] and a `{` inside math as
//! [`Token::BMathGrp`], the same character and a different token, because the
//! lexer already knew. So this replays the stack exactly rather than guessing
//! at it.
//!
//! Two places where the replay is *deliberately* not one-for-one with the
//! lexer, both sound:
//!
//! - **The lexer's `Active` mode has no counterpart here.** An inline command
//!   pushes `Mode::Active` (`\emph` → `[.., Horizontal, Active]`) and its `{`
//!   pops that and pushes `Horizontal`; here `\emph` pushes nothing and its `{`
//!   pushes [`Area::Inline`]. The stacks differ by one entry *while the
//!   command's arguments are being read* and are back in step the moment the
//!   argument list ends — a `;` (`Token::EndActive`, which pops `Active` in the
//!   lexer and is ignored here) or the argument group's own closer. The visible
//!   consequence is that the whitespace between a command and its first
//!   argument reports as `Inline`/`Block` rather than `Program`. That is the
//!   conservative direction for both callers, and it is what is wanted:
//!   completion offers prose nothing there, and the formatter leaves it alone.
//! - **`<[ … ]>`** (a path literal) pushes and pops here though the lexer does
//!   neither — it lexes `Token::BPath` without a mode change, because a path
//!   *is* program text. Pushing [`Area::Program`] onto [`Area::Program`] and
//!   popping it again is a no-op, and `<[` cannot appear anywhere else, so the
//!   two agree.

use rustyfi_syntax::{Atom, Token};

/// Which text area a point in the token stream sits in.
///
/// Derived from the token stream rather than the parse tree, because the
/// question has an answer even when the file does not parse — and a file being
/// typed into usually does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Area {
    /// Ordinary program text. The **only** area whose whitespace is
    /// insignificant: the lexer skips it and emits no token for it.
    Program,
    /// Inline text, `{ … }`. Spaces and newlines here are `Token::Space` /
    /// `Token::Break` — content, not layout.
    Inline,
    /// Block text, `'< … >`.
    Block,
    /// Math, `${ … }`.
    Math,
}

/// The lexer's mode stack, replayed one token at a time.
#[derive(Debug, Clone)]
pub(crate) struct AreaStack {
    /// Never empty: the bottom entry is the mode a file starts in, and
    /// `rustyfi_syntax`'s entry points always start one in program mode.
    stack: Vec<Area>,
}

impl AreaStack {
    pub(crate) fn new() -> Self {
        AreaStack {
            stack: vec![Area::Program],
        }
    }

    /// The area the tokens fed so far leave the stream in.
    pub(crate) fn current(&self) -> Area {
        *self.stack.last().expect("the stack always holds Program")
    }

    /// Fold one more token in.
    pub(crate) fn advance(&mut self, tok: &Token) {
        match tok {
            Token::BHorzGrp => self.stack.push(Area::Inline),
            Token::BVertGrp => self.stack.push(Area::Block),
            Token::BMathGrp => self.stack.push(Area::Math),
            Token::LParen | Token::BList | Token::BRecord | Token::BPath | Token::OpenModule(_) => {
                self.stack.push(Area::Program)
            }
            Token::EHorzGrp
            | Token::EVertGrp
            | Token::EMathGrp
            | Token::RParen
            | Token::EList
            | Token::ERecord
            | Token::EPath => {
                // A closer with nothing to close is not necessarily a broken
                // file: `]>` lexes as `Token::EPath` in program text with no
                // opener required (`lexer.rs`'s `']'` arm), so the guard is
                // load-bearing rather than mere defensiveness.
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
            }
            _ => {}
        }
    }
}

/// Which area the tokens `before` leave the stream in.
pub(crate) fn area_at(before: &[&Atom]) -> Area {
    let mut stack = AreaStack::new();
    for a in before {
        stack.advance(&a.slot);
    }
    stack.current()
}
