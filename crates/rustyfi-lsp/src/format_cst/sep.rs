//! When two copied byte ranges may be written adjacently.
//!
//! The renderer's safety argument is that every content byte is copied from the
//! input, so no token can be added, dropped or reordered. That argument has
//! exactly one hole: two adjacent copied ranges with no separator between them
//! can lex as **one** token. Real pairs in this grammar:
//!
//! | written adjacently | lexes as |
//! |---|---|
//! | `:` `:` | `Cons` `::` |
//! | `:` `>` | `Coerce` `:>` (0.1) |
//! | `<` `[` | `BPath` `<[` |
//! | `(` `\|` | `BRecord` `(\|` |
//! | `\|` `)` | `ERecord` `\|)` |
//! | `?` `:` | `Optional` |
//! | `?` `'r` | `RowVar` (0.1) |
//! | `-` `-` | `PathLine` `--` |
//! | `.` `.` | `PathCurve` `..` |
//! | `&` `&` | one binop — the staging hazard, `&&x` being a quote of a quote |
//! | `<` `-` | `OverwriteEq` `<-` |
//! | `1` `pt` | one `Length` |
//! | `x` `y`, `1` `2` | one `Var` / one `Int` |
//! | `+` `p` | one `VertCmd` — a `+` binop swallows the name after it |
//! | `1` `.5` | one `Float` — and `100.` `5`, from the other side |
//! | `\cmd` `@` | one `HorzMacro` |
//! | `` ` `` `` ` `` | one literal, or a "closed with too many `` ` ``" error |
//! | `#` `` `x` `` | one literal, with `omit_pre` flipped |
//! | `` `x` `` `#` | one literal, with `omit_post` flipped |
//!
//! Note the sign of the error: a false `true` costs one unnecessary space, a
//! false `false` corrupts a document. So the rule is written conservatively and
//! validated **exhaustively** by `tests/format_cst_sep.rs`, over every ordered
//! pair of the 11770 distinct token spellings the corpus contains — 55.4 M pairs
//! where this function answers `false`, none of which fuses — rather than by
//! inspection. The bottom half of the table above is what that sweep found and
//! the hand-written top half had missed; two of them (`+` `p` and `1` `.5`)
//! change what a document *means* rather than merely breaking its lexing.
//!
//! # The shape of the rule
//!
//! Three character classes and one pair table, in that order of confidence:
//!
//! 1. [`is_word`] against itself — one identifier, number, length or hex
//!    constant continuing into the next range.
//! 2. [`is_opsymbol`] against itself — `lexer.rs`'s own predicate, and the only
//!    class that is *provable* rather than enumerated: every program-mode
//!    operator token is a maximal `scan_while(is_opsymbol)` run.
//! 3. [`is_word_glue`] against [`is_word`], both ways — the three characters a
//!    word-shaped scanner absorbs although they are not word characters.
//! 4. [`FUSED_DELIMITERS`] — the multi-character delimiters the lexer matches by
//!    hand, where neither of the run rules applies.
//!
//! It used to be (1) plus "any two characters from a hand-listed symbol
//! alphabet", which was both unsound (it missed every row in the bottom half of
//! the table above) and needlessly coarse: it separated `)` from `)`, `)` from
//! `,` and `}` from `{`, none of which can fuse. Measured on one fixed sweep,
//! the old rule over-separated in **558** boundary classes and this one in
//! **101** — and the ones that remain are `1` from `+` and `>'` from a name,
//! where a formatter wants the space anyway, rather than `f(g(x)` from `)`. The
//! *rate* barely moves (26.5% -> 20.1% of separated pairs); the classes are what
//! a reader of the output would notice.
//!
//! One hazard this function deliberately does **not** cover: a `%` comment's
//! bytes swallow the rest of their line, so a comment may never be followed on
//! the same line by anything at all. No separator fixes that — only a line
//! break — so it belongs to whoever emits comments, not here.
//!
//! # The domain: program-area adjacency only
//!
//! This function answers for two ranges that meet **in a program area**. It
//! cannot answer for two ranges inside a text area and does not try: `is_str_char`
//! swallows most of ASCII, so `prose` ++ `!more` is one `Char` token there, and a
//! rule that separated them would also have to separate `f` from `(` in program
//! mode. Nothing needs it to: a text area is copied through as one
//! `Doc::Verbatim` whose interior the renderer never re-spaces, and a space
//! inserted into SATySFi prose changes the typeset line rather than just looking
//! different. `format_cst_sep.rs` pins that precondition with a test of its own,
//! so it cannot be forgotten while this rule is being tightened.
//!
//! That is also the answer to the CJK question. [`is_word`] is
//! `char::is_alphanumeric`, which is true for Han and Kana, so two adjacent runs
//! of Japanese *would* be separated here — and that space would be visible in the
//! PDF. It cannot arise: a CJK run is not a program-mode token at all (the
//! program lexer rejects the character outright), so it never reaches a
//! program-area join. Measured, not assumed — the sweep places every spelling in
//! the mode it needs and reports which ones reach program mode.
//!
//! # The lookahead-driven fusions, which are the ones that surprise
//!
//! `1` `pt` fuses because of `lexer.rs`'s `length_lookahead`, and it is not the
//! only rule of that shape. The others, all confirmed by the sweep: `+`'s
//! `name_len_at` (a name after a `+` binop makes it a `VertCmd`), `\cmd`'s and
//! `+cmd`'s trailing `@` (`HorzMacro`/`VertMacro`), `#` before a backtick and a
//! backtick run before a `#` (the literal's `omit_pre`/`omit_post` flags), `?`
//! before `'name` (0.1's `RowVar`), `Mod` before `.(` (`OpenModule`), and — in an
//! inline-text area, hence outside this function's domain — a run of whitespace
//! before `}`/`{`/`<`/`|`/`*`.
//!
//! `format.rs` refuses to insert whitespace at all, for exactly these reasons
//! (`format.rs:120-125`). A formatter that lays out lines must insert it, so it
//! pays for the privilege with this table and that test.

/// Characters that can continue an identifier, a number or a length.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '\''
}

/// `lexer.rs`'s own `is_opsymbol` (`lexer.rs:104-110`), verbatim.
///
/// This is the one *provable* fusion class: every operator token in program mode
/// is `scan_while(is_opsymbol)`, a maximal run, so **any** two of these adjacent
/// are one token. `:` `:`, `-` `-`, `&` `&`, `<` `-`, `?` `:`, `?` `'r`, `:` `>`
/// and `?` `->` are all this one rule rather than eight special cases.
fn is_opsymbol(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '^' | '&' | '|' | '!' | ':' | '=' | '<' | '>' | '~' | '\'' | '.'
            | '?'
    )
}

/// The multi-character tokens the lexer matches character by character instead
/// of through a run, where the first character is **not** covered by the run
/// rule above. Read off `lexer.rs`, one entry per site, rather than accumulated
/// from test failures — the sweep is what checks the list is complete, not what
/// wrote it.
///
/// The four `!`-headed rows are math mode's escapes into another area
/// (`lexer.rs:1359-1381`). `!` is an operator symbol, so `!` `{` is only
/// reachable if a `!`-final token could sit at a *program-mode* join, which
/// today it cannot; they are here because the cost is one space in a case that
/// does not arise and the cost of being wrong is a corrupted document.
const FUSED_DELIMITERS: [(char, char); 12] = [
    ('(', '|'),  // BRecord      `lexer.rs:524`, and again at :1280, :1372
    ('|', ')'),  // ERecord      `lexer.rs:740`
    ('<', '['),  // BPath        `lexer.rs:710`
    (']', '>'),  // EPath        `lexer.rs:548`
    ('$', '{'),  // BMathGrp     `lexer.rs:585`, :1199
    ('#', '`'),  // Literal, omit_pre = false   `lexer.rs:600`, :1160
    ('`', '#'),  // Literal, omit_post = false  `lexer.rs:422`
    ('`', '`'),  // one literal's quote run     `lexer.rs:597`
    ('!', '{'),  // math escape into inline text
    ('!', '<'),  // math escape into block text
    ('!', '('),  // math escape into a program area
    ('!', '['),  // math escape into a list
];

/// Characters that glue to a *word* although they are not word characters
/// themselves — in either direction, because each of them does it in at least
/// one:
///
/// - `+` swallows a following name: `+` ++ `p` is one `VertCmd`, so `1+p` is
///   `1` applied to a block command rather than an addition. The reverse
///   (`x` ++ `+p`) is safe, and separating it anyway costs only the space a
///   formatter would want around a binary `+`.
/// - `.` continues a number in both directions: `1` ++ `.5` and `100.` ++ `5`
///   are each one `Float`, and `100.` ++ `pt` is one `Length`.
/// - `@` after a command name makes a macro: `\cmd` ++ `@` is one `HorzMacro`.
///   The reverse (`\cmd@` ++ `x`) is safe, and `@` only ever begins a header,
///   which a formatter puts on a line of its own regardless.
fn is_word_glue(c: char) -> bool {
    matches!(c, '+' | '.' | '@')
}

/// Must a separator be written between `prev` and `next`?
///
/// Both arguments are the tokens' own source text, and only the last character
/// of `prev` and the first of `next` are read — which is what makes the
/// exhaustive sweep in `tests/format_cst_sep.rs` finite: it can decide a whole
/// group of right-hand sides with one call. Empty text on either side answers
/// `false`: there is nothing to fuse with.
///
/// Precondition: `prev` and `next` meet in a **program area**. See the module
/// header.
pub(crate) fn must_separate(prev: &str, next: &str) -> bool {
    let Some(a) = prev.chars().next_back() else {
        return false;
    };
    let Some(b) = next.chars().next() else {
        return false;
    };
    // A newline or a space already separates.
    if a.is_whitespace() || b.is_whitespace() {
        return false;
    }
    (is_word(a) && is_word(b))
        || (is_opsymbol(a) && is_opsymbol(b))
        || (is_word_glue(a) && is_word(b))
        || (is_word(a) && is_word_glue(b))
        || FUSED_DELIMITERS.contains(&(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_fusing_pairs_are_all_separated() {
        for (a, b) in [
            (":", ":"),
            (":", ">"),
            ("<", "["),
            ("(", "|"),
            ("|", ")"),
            ("?", ":"),
            ("?", "'r"),
            ("-", "-"),
            (".", "."),
            ("&", "&"),
            ("<", "-"),
            ("1", "pt"),
            ("x", "y"),
            ("1", "2"),
            // Found by the generated sweep, not by inspection.
            ("+", "p"),
            ("*+", "p"),
            ("1", ".5"),
            ("100.", "5"),
            ("100.", "pt"),
            ("\\cmd", "@import: x\n"),
            ("+h1", "@require: y\n"),
            ("`x`", "#"),
            ("#", "`x`"),
            ("`x`", "`y`"),
        ] {
            assert!(
                must_separate(a, b),
                "{a:?} followed by {b:?} can fuse and must be separated"
            );
        }
    }

    #[test]
    fn an_empty_side_never_needs_a_separator() {
        assert!(!must_separate("", "x"));
        assert!(!must_separate("x", ""));
    }

    #[test]
    fn existing_whitespace_is_already_a_separator() {
        assert!(!must_separate("x ", "y"));
        assert!(!must_separate("x", "\ny"));
    }

    #[test]
    fn a_word_beside_a_symbol_does_not_need_one() {
        // The common case, and the one where a false `true` would add noise:
        // `f(x)` must not become `f ( x )`.
        assert!(!must_separate("f", "("));
        assert!(!must_separate(")", "in"));
        assert!(!must_separate("x", ","));
        // `#` heads a record access and a backtick literal; only the second
        // fuses, so the first must stay tight.
        assert!(!must_separate("#", "field"));
        assert!(!must_separate("~", "x"));
        assert!(!must_separate("!", "x"));
    }
}
