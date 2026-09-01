//! The layout IR.
//!
//! A `Doc` is what a builder produces and [`super::render`] consumes. The
//! design constraint that matters is on the *content* variants, and it is the
//! whole safety argument:
//!
//! > Every byte of output is either copied from a byte range of the input, or
//! > is one space or one line terminator that the renderer decided to insert.
//!
//! There is deliberately **no constructor that takes a `Token`**. Printing a
//! leaf by `Display`-ing its token would be silent corruption for six token
//! kinds, because the lexer normalises their payloads: `0x1F` lexes to
//! `IntConst(31)` and prints back as `31` (`lexer.rs:969-984`, `token.rs:264`);
//! ``` ``x`` ``` and `` #`x` `` both lex to `Literal { body: "x", .. }` and
//! print back as `` `x` `` (`lexer.rs:440-468`, `token.rs:271`); `\&` in inline
//! text lexes to `Char("&")` (`lexer.rs:1190-1194`); a header loses its `@`,
//! its keyword and the spaces after the colon (`lexer.rs:899-965`). Three of
//! those change what the program *means*. Slicing the source at a span has no
//! such failure mode for any token kind, ever — so that is the only way in.

/// How a group lays itself out when it does not fit flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Flat if the whole group fits, otherwise every `Line` in it breaks.
    /// The ordinary Wadler group.
    Auto,
    /// Always broken, whatever the width. `match` arms, `sig`/`struct` items:
    /// joining them would be legal and unreadable.
    Break,
    /// Break only where needed, keeping as many items per line as fit.
    /// Command argument runs.
    Fill,
}

/// One node of the layout IR.
///
/// `'s` is the source text's lifetime: every content leaf borrows from it
/// rather than owning a rendered string, which is what makes the "copied, never
/// re-rendered" property structural instead of a rule to remember.
#[derive(Debug, Clone)]
pub(crate) enum Doc<'s> {
    Nil,
    /// One token's own source bytes, plus its index in the atom stream.
    ///
    /// The index is what [`super::sep::must_separate`] consults when two of
    /// these end up adjacent in flat mode — the one hole in the
    /// copied-bytes-only argument is that two adjacent copied ranges can lex
    /// as a *single* token (`:` `:` -> `::`, `1` `pt` -> one length).
    Token { text: &'s str, atom: usize },
    /// A byte range copied through untouched: a text/math area the current
    /// policy does not format, a `%` comment's own bytes, or — in slice 0 —
    /// the trivia between two tokens.
    ///
    /// Distinct from [`Doc::Token`] because it is never a separation candidate:
    /// nothing fuses across it, and its interior is not the renderer's business.
    Verbatim(&'s str),
    /// **The indentation of the line about to be opened, verbatim**, in place
    /// of the one the enclosing [`Doc::Nest`] chain owes it.
    ///
    /// Slice 1 emits it for exactly one thing: a `%` comment that sits on a
    /// line of its own keeps the author's own leading whitespace instead of
    /// being pulled to the block's depth. That is not a nicety — `%`-disabled
    /// code parked at column 0 is the single largest source of *indent* moves
    /// in the corpus, and re-indenting it says the comment belongs to a block
    /// it was deliberately kept out of.
    ///
    /// The empty string is meaningful and is what a column-0 comment emits:
    /// it *cancels* the owed indent rather than adding nothing to it.
    ///
    /// Idempotent by construction — the bytes written are the bytes read, so
    /// the second pass reads back what the first wrote. The renderer honours
    /// it only while an indent is still owed (or the line is still empty); a
    /// mid-line occurrence would be intra-line spacing, which is
    /// [`Doc::Verbatim`]'s job, and the builder never emits one there.
    VerbatimIndent(&'s str),
    Concat(Vec<Doc<'s>>),
    /// Indent everything inside by `n` further columns when it breaks.
    Nest(i32, Box<Doc<'s>>),
    Group(Mode, Box<Doc<'s>>),
    /// `" "` flat, a newline plus the current indent when broken.
    Line,
    /// `""` flat, a newline plus the current indent when broken.
    SoftLine,
    /// Always a newline, and forces every enclosing [`Mode::Auto`] group open.
    HardLine,
    /// **A break opportunity inside inline text**: one space, or a newline
    /// plus the current indent, decided greedily by the renderer against
    /// what follows it up to the next break opportunity.
    ///
    /// Not a [`Doc::Line`], and the difference is the whole of slice 6. A
    /// `Line` asks its enclosing [`Doc::Group`] — every `Line` in a group
    /// breaks or none does, which is right for a record literal and wrong for
    /// a paragraph, where the answer has to be different at every gap. This
    /// is the `Mode::Fill` behaviour, expressed as a POINT rather than as a
    /// group, because an inline area's break points are not all break points:
    /// a gap whose two sides are both CJK is frozen exactly as the author
    /// wrote it and reaches the renderer as ordinary content, so "the group
    /// breaks" is not a decision anything can act on.
    ///
    /// The renderer never emits one where the source had no whitespace token
    /// and never omits one where it had — a run is neither invented nor
    /// emptied, only re-spelled — which is what keeps the `Space`/`Break`
    /// slot the only thing this can change in the token stream. See
    /// [`super::inline`] for the predicate that decides which gaps become
    /// one.
    FillLine,
    /// A preserved paragraph break: one blank line, on top of the newline that
    /// ends the previous line.
    ///
    /// Normalised by the renderer against the *final* line structure rather
    /// than by the builder. That ordering is not a detail — `format.rs:466-483`
    /// records the bug from doing it the other way round: the blank-line cap ran
    /// before the final newline was added, so a whitespace-only last line became
    /// a blank line that only the *next* format capped away, and two
    /// consecutive saves of an untouched file produced two different files.
    BlankLine,
}

impl<'s> Doc<'s> {
    pub(crate) fn concat(parts: Vec<Doc<'s>>) -> Doc<'s> {
        match parts.len() {
            0 => Doc::Nil,
            _ => Doc::Concat(parts),
        }
    }

    /// Whether this subtree contains a [`Doc::HardLine`] or a multi-line
    /// [`Doc::Verbatim`], either of which forces an enclosing `Auto` group open.
    ///
    /// A multiline `Verbatim` counts because a text area's *re-wrappable* width
    /// does not exist: `manual.saty:36-40` is a four-line `+p{…}` that must not
    /// be re-wrapped at any width, so the enclosing group's fit question is
    /// "does its first line fit, and is it multiline", not "how wide is it".
    pub(crate) fn forces_break(&self) -> bool {
        match self {
            Doc::HardLine | Doc::BlankLine => true,
            // A break OPPORTUNITY, not a break: the renderer may well render
            // it flat, so it says nothing about whether an enclosing group
            // fits. (Neither builder emits a `Group`, so this arm is the
            // statement of intent rather than a live decision.)
            Doc::FillLine => false,
            Doc::Verbatim(s) => s.contains('\n'),
            // Spaces and tabs only (`trivia::classify`), so never.
            Doc::VerbatimIndent(_) => false,
            Doc::Token { text, .. } => text.contains('\n'),
            Doc::Concat(parts) => parts.iter().any(Doc::forces_break),
            Doc::Nest(_, inner) | Doc::Group(_, inner) => inner.forces_break(),
            Doc::Nil | Doc::Line | Doc::SoftLine => false,
        }
    }
}
