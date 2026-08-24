//! Making document text survive as text.
//!
//! LaTeX reserves ten of the characters a typeset document is full of, and
//! the failure modes are not cosmetic. A bare `%` comments out **the rest of
//! the line** — a paragraph that mentions `100%` silently loses its second
//! half, and the file still compiles, so nothing reports it. A bare `&` in a
//! paragraph is `Misplaced alignment tab character &`, a hard error. A bare
//! `_` outside math mode is `Missing $ inserted`, which is worse than an
//! error: TeX RECOVERS from it by opening math mode, and the rest of the
//! sentence comes out in italics with the spaces removed.
//!
//! So every character of document text goes through [`text`] on its way into
//! a paragraph, and nothing else does. LaTeX this backend GENERATED — a
//! `\section*{`, an `\item`, a `\href{`— never touches it; the two are kept
//! apart in the paragraph buffer (`para.rs`'s `Piece`) rather than escaped
//! eagerly and hoped about.
//!
//! ## The three that are not on the usual list
//!
//! `<`, `>` and `|` are ordinary letters in the source but are typeset as
//! `¡`, `¿` and `—` by the OT1 encoding LaTeX still defaults to for
//! pdfTeX. The preamble this backend writes asks for T1 (`mod.rs`'s
//! `preamble`), under which all three are correct — but the escapes are
//! emitted anyway, because a reader who lifts a paragraph out of the
//! generated file into their own document should not have that paragraph's
//! meaning depend on a `fontenc` line they did not copy.
//!
//! ## Why not a `verbatim`-style catch-all
//!
//! Escaping is per-character rather than per-run because a run is not
//! uniformly one thing: `\href{...}{text}` has escaped text inside generated
//! braces, and a table cell has escaped text inside a generated `&`
//! separator. The only level at which "this is the document's" is a decidable
//! question is the character.

/// Escape one string of the DOCUMENT's own text for LaTeX horizontal mode.
///
/// - `\` → `\textbackslash{}` — a backslash cannot escape itself, since
///   `\\` is a line break;
/// - `{` `}` → `\{` `\}`;
/// - `$` `&` `#` `_` `%` → the same with a backslash, which is exactly what
///   they mean;
/// - `~` `^` → `\textasciitilde{}` / `\textasciicircum{}`. A backslash in
///   front of either is an ACCENT command that would eat the next character
///   (`\~n` is `ñ`), so these two need their named form;
/// - `<` `>` `|` → their `\text…` names, see this module's doc comment.
///
/// The `{}` after the argument-less commands is not optional: without it TeX
/// swallows the following space, and `a \textbackslash b` comes out as
/// `a \bb`.
pub(super) fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '$' => out.push_str("\\$"),
            '&' => out.push_str("\\&"),
            '#' => out.push_str("\\#"),
            '_' => out.push_str("\\_"),
            '%' => out.push_str("\\%"),
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            '<' => out.push_str("\\textless{}"),
            '>' => out.push_str("\\textgreater{}"),
            '|' => out.push_str("\\textbar{}"),
            _ => out.push(c),
        }
    }
    out
}

/// A destination or label name, reduced to characters `hyperref` can carry
/// through a `\hypertarget`/`\hyperlink` pair.
///
/// The names come from `register-destination`, so they are whatever the
/// document's author typed — `sec:intro`, but also `第1章` or `fig (2)`.
/// Anything outside a conservative ASCII set becomes `-`, and the result is
/// prefixed so that a document whose label happens to collide with one
/// `hyperref` mints for itself cannot shadow it.
///
/// Two different names can collapse to the same sanitized form. That makes a
/// cross-reference point at the wrong section, which is bad but bounded; the
/// alternative — passing the raw name through — makes the document fail to
/// compile, since a `%` or a `#` in a `\hypertarget` argument is the same
/// hazard [`text`] exists for.
pub(super) fn dest_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    out.push_str("rustyfi:");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '-') {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    out
}

/// A URL for `\href{…}`.
///
/// `hyperref` reads its first argument almost verbatim, which is what makes
/// a URL containing `%` or `#` work at all — but `\`, `{`, `}` and the
/// comment character still have to be neutralised, and `hyperref` provides
/// `\%`, `\#`, `\&` and `\~` for exactly this inside a `\href`. A `%` in a
/// URL is a percent-encoded byte and is extremely common, so getting this
/// wrong truncates the link AND the line after it.
pub(super) fn url(u: &str) -> String {
    let mut out = String::with_capacity(u.len());
    for c in u.chars() {
        match c {
            '%' => out.push_str("\\%"),
            '#' => out.push_str("\\#"),
            '&' => out.push_str("\\&"),
            '~' => out.push_str("\\~"),
            '\\' => out.push_str("\\\\"),
            // A brace would end the argument early. There is no escape that
            // survives `hyperref`'s own re-reading, so it is percent-encoded
            // — which is what a brace in a URL should have been anyway.
            '{' => out.push_str("%7B"),
            '}' => out.push_str("%7D"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure this exists to prevent: `%` comments out the rest of the
    /// line, silently, and the document still compiles.
    #[test]
    fn a_percent_does_not_eat_the_rest_of_the_line() {
        assert_eq!(text("100% of it"), "100\\% of it");
        assert!(!text("100% of it").contains("0% "));
    }

    #[test]
    fn every_reserved_character_is_neutralised() {
        assert_eq!(
            text("# $ % & _ { }"),
            "\\# \\$ \\% \\& \\_ \\{ \\}"
        );
        // `~` and `^` may NOT take a bare backslash: `\~n` is `ñ`, so the
        // character after them would be eaten.
        assert_eq!(text("~^"), "\\textasciitilde{}\\textasciicircum{}");
        // A backslash cannot escape itself — `\\` is a line break.
        assert_eq!(text("a\\b"), "a\\textbackslash{}b");
    }

    /// The trailing `{}` is what keeps the following space: without it TeX
    /// reads the space as the command's terminator and `a \textbar b`
    /// becomes `a |b`.
    #[test]
    fn an_argumentless_command_keeps_the_space_after_it() {
        assert_eq!(text("a | b"), "a \\textbar{} b");
    }

    #[test]
    fn a_label_is_reduced_to_what_hyperref_can_carry() {
        assert_eq!(dest_name("sec:intro"), "rustyfi:sec:intro");
        // One `-` per CHARACTER, not per byte: a label is not a byte string,
        // and per-byte would make two different two-kanji labels collide at
        // six dashes apiece.
        assert_eq!(dest_name("第1章"), "rustyfi:-1-");
        assert_eq!(dest_name("fig (2)%"), "rustyfi:fig--2--");
    }

    /// A percent-encoded byte in a URL is the common case, not a corner —
    /// and unescaped it takes the rest of the line with it.
    #[test]
    fn a_url_keeps_its_percent_encoding() {
        assert_eq!(url("http://x/a%20b#frag"), "http://x/a\\%20b\\#frag");
    }
}
