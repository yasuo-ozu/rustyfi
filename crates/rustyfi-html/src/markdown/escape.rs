//! Making document text survive as text.
//!
//! Markdown's whole design is that ordinary punctuation is markup, so a
//! typeset document — which is full of `*`, `_`, `[`, `|` and backticks that
//! the author meant literally — cannot simply be written out. Every
//! character of document text goes through [`inline`] on its way into a
//! paragraph, and every assembled line through [`line_start`] on its way out.
//!
//! **Backslash escapes, not entities.** CommonMark lets a backslash escape
//! any ASCII punctuation character, and that is what is used here: `\*`
//! renders as `*` in every conforming renderer, and — the reason it is
//! preferred to `&ast;` — a reader looking at the raw `.md` still sees the
//! character. The whole point of this format is that the source is legible.
//!
//! **Two passes, because Markdown has two grammars.** Some characters are
//! markup anywhere on a line (`*`, `_`, backtick, `[`); others only in the
//! LEADING position (`#` is a heading, `-` a bullet, `>` a quote, `1.` an
//! ordered item) and are perfectly ordinary in the middle of a sentence.
//! Escaping the second group everywhere would litter a document like
//! `latexcmds`', which is nothing but hyphenated compounds and version
//! numbers, with backslashes nobody needs. So [`inline`] escapes the first
//! group as text is accumulated, and [`line_start`] escapes the second group
//! once, on the finished line, where "leading" is finally a decidable
//! question.

/// Escape the characters that are markup ANYWHERE on a line.
///
/// - `` \ `` — the escape character itself, first, or every escape below
///   would be ambiguous;
/// - ``` ` ``` — a code span;
/// - `*`, `_` — emphasis. Intraword `_` is not emphasis in CommonMark, but
///   `_` is escaped unconditionally anyway: the paragraph this text lands in
///   is assembled from runs whose word boundaries the box stream has already
///   thrown away, so "intraword" is not a question this layer can answer;
/// - `[`, `]` — a link or a footnote reference. `(`/`)` need no escaping:
///   they are only special immediately after a `]`, which cannot happen
///   because the `]` is escaped;
/// - `<` — an autolink or raw HTML. Escaping it is what keeps a document
///   that types `<html>` from having it vanish into the renderer;
/// - `|` — a table cell separator. Only meaningful inside a table, but a
///   paragraph line full of pipes directly above one is read as its header
///   row, so the safe place to decide is nowhere;
/// - `~` — GFM strikethrough;
/// - `$` — a math delimiter. Not CommonMark, but every renderer that
///   understands `--katex`'s output understands it, and this backend now
///   EMITS `$…$`. So a document's own `$100` sitting beside an equation would
///   pair with that equation's opening delimiter and swallow the prose
///   between them; under a `dollars`-convention reader (`markdown-it-texmath`,
///   Pandoc's `tex_math_dollars`) the whole span disappears into a formula.
///   Escaped unconditionally, in every math mode: whether a later paragraph
///   emits a delimiter is not a property of this run;
/// - `&` — only when it opens something entity-shaped (`&amp;`, `&#8212;`),
///   since a bare ampersand between words is literal in CommonMark and
///   escaping every one of them is pure noise.
pub(super) fn inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '|' | '~' | '$' => {
                out.push('\\');
                out.push(c);
            }
            '&' if chars
                .peek()
                .is_some_and(|n| n.is_ascii_alphanumeric() || *n == '#') =>
            {
                out.push_str("\\&");
            }
            _ => out.push(c),
        }
    }
    out
}

/// Escape the characters that are markup only in a line's LEADING position,
/// on an already-[`inline`]-escaped line.
///
/// Applied to each line of a finished paragraph rather than to each run,
/// because until the paragraph is assembled there is no such thing as the
/// start of a line: the box stream's own line breaks are the port's wrapping
/// decisions and get collapsed away.
///
/// `=` and `-` alone on a line would make the paragraph ABOVE into a setext
/// heading, which is why they are escaped even though neither is a bullet
/// when it stands by itself.
pub(super) fn line_start(line: &str) -> String {
    // Up to three leading spaces still count as "the start of the line" to a
    // Markdown parser; four or more make an indented code block, which is
    // handled by never indenting a paragraph in the first place.
    let indent_len = line.len() - line.trim_start_matches(' ').len();
    let (indent, rest) = line.split_at(indent_len.min(3));
    let Some(first) = rest.chars().next() else {
        return line.to_string();
    };
    let needs = match first {
        // A heading, a bullet, a blockquote, a setext underline.
        '#' | '-' | '+' | '>' | '=' => true,
        // An ordered-list marker: digits then `.` or `)`.
        '0'..='9' => {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            matches!(
                rest[digits.len()..].chars().next(),
                Some('.') | Some(')')
            )
        }
        _ => false,
    };
    if !needs {
        return line.to_string();
    }
    // For the digit case the marker is the punctuation, not the digits, so
    // the backslash goes in front of the `.`/`)`; for the rest it goes in
    // front of the character itself.
    if first.is_ascii_digit() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        format!("{indent}{}\\{}", &rest[..digits], &rest[digits..])
    } else {
        format!("{indent}\\{rest}")
    }
}

/// A table cell's text: [`inline`] already escaped the `|`, but a cell may
/// not contain a raw newline at all — GFM's row grammar is one line per row —
/// so any that survived collapse to a space.
pub(super) fn table_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.chars() {
        let c = if c == '\n' || c == '\r' { ' ' } else { c };
        if c == ' ' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// Wrap `body` in the shortest backtick fence that cannot be closed by
/// anything inside it, for an inline code span.
///
/// A run of N backticks in the content needs a fence of N+1, and a span whose
/// content starts or ends with a backtick needs a space of padding that
/// CommonMark then strips. Both cases are real: `code-printer`'s own manual
/// typesets backticks as code.
pub(super) fn code_span(body: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in body.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    let pad = if body.starts_with('`') || body.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{body}{pad}{fence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metacharacters_a_reader_would_lose_are_escaped() {
        assert_eq!(inline("a*b_c"), "a\\*b\\_c");
        assert_eq!(inline("[x]"), "\\[x\\]");
        assert_eq!(inline("a|b"), "a\\|b");
        assert_eq!(inline("<html>"), "\\<html>");
        assert_eq!(inline("back\\slash"), "back\\\\slash");
        assert_eq!(inline("`tick`"), "\\`tick\\`");
    }

    /// A bare `&` between words is literal in CommonMark; only an
    /// entity-shaped one can disappear, so only that one is escaped.
    #[test]
    fn only_an_entity_shaped_ampersand_is_escaped() {
        assert_eq!(inline("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(inline("&amp;"), "\\&amp;");
        assert_eq!(inline("&#8212;"), "\\&#8212;");
    }

    /// Mid-sentence these are ordinary punctuation and must NOT collect a
    /// backslash — that is the whole reason the two passes are separate.
    #[test]
    fn line_start_only_escapes_the_leading_position() {
        assert_eq!(inline("issue #3 and 2. of the list"), "issue #3 and 2. of the list");
        assert_eq!(line_start("# not a heading"), "\\# not a heading");
        assert_eq!(line_start("- not a bullet"), "\\- not a bullet");
        assert_eq!(line_start("1. not an item"), "1\\. not an item");
        assert_eq!(line_start("12) not an item"), "12\\) not an item");
        assert_eq!(line_start("issue #3"), "issue #3");
        assert_eq!(line_start("2024 was a year"), "2024 was a year");
    }

    /// A lone `-` or `=` line would turn the paragraph above it into a setext
    /// heading, which is a much bigger wound than the character itself.
    #[test]
    fn a_setext_underline_cannot_form_by_accident() {
        assert_eq!(line_start("--- "), "\\--- ");
        assert_eq!(line_start("==="), "\\===");
    }

    #[test]
    fn a_code_span_fence_always_outgrows_its_content() {
        assert_eq!(code_span("plain"), "`plain`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("a``b"), "```a``b```");
        // Leading/trailing backticks need the padding CommonMark strips.
        assert_eq!(code_span("`x`"), "`` `x` ``");
    }

    #[test]
    fn a_table_cell_can_never_break_its_row() {
        assert_eq!(table_cell("a\nb"), "a b");
        assert_eq!(table_cell("  spaced   out  "), "spaced out");
    }
}
