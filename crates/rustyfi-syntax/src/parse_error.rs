//! Turning a failed parse into one diagnostic a human can act on.
//!
//! [`locate`] is the whole module: given the source, the [`AtomStream`] the
//! parse ran on and syan's error tree, it produces one [`ParseFileError`] with
//! a position that names the construct the author actually got wrong.
//!
//! It lives beside [`crate::cst::parse_file`] and [`crate::cst_v1::
//! parse_file_v1`] rather than inside either, because both need it and both
//! used to carry a private `render_parse_error` that was
//! `format!("{err:?}")` — a `Debug` dump of the whole tree, at the aggregate's
//! span. Two things were wrong with that, and they are worth stating because
//! either alone would have looked like a small cosmetic bug:
//!
//! - **The span was not the failure's.** `ParseError::from_cause` documents
//!   that an `Alternatives` aggregate "takes the FIRST alternative's span:
//!   with no record of how far each alternative got, that is the only
//!   deterministic choice available". For a whole-file rule the first
//!   alternative typically dies on the first token, so the aggregate points at
//!   byte 0.
//! - **Even the right leaf can be stale.** A repetition discards the failure
//!   that ended it: `Vec<TopBinding>` rolls back to the start of the binding
//!   that would not parse, and what surfaces is "expected end of input" *at
//!   that binding's start*. Under 0.1 a whole library is one `module` binding,
//!   so every error in the file reported on line 1.
//!
//! The first is fixed by the standard furthest-failure rule over the tree
//! ([`best_failure`]); the second only by the stream's own high-water mark
//! (`AtomStream::furthest`), because by then the tree has forgotten. [`locate`]
//! uses both, and prefers the tree when the two agree on how far the parse got
//! — the tree's message names the expected alternatives, which the mark cannot.

use crate::span::{floor_char_boundary, Span};
use crate::stream::AtomStream;
use syan::error::ParseError;

/// What kind of failure a [`ParseFileError`] reports.
///
/// The distinction that earns this type is [`Self::GaveUp`]: a parse stopped
/// by [`crate::stream::Budget`] has established nothing about the source, and
/// must not be presented as though it had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseFailureKind {
    /// The lexer rejected the characters. These already carry a hand-written
    /// message and a tight span, and are passed through untouched.
    Lex,
    /// The grammar rejected the token stream. [`ParseFileError::span`] is the
    /// token the parse could not get past.
    Syntax,
    /// The parse ran out of backtracking budget before reaching any verdict.
    /// [`ParseFileError::span`] is the furthest token it reached, which is
    /// where an unfinished construct usually is — but the file may well be
    /// valid, and this is not a claim that it is not.
    GaveUp,
}

/// A parse failure, positioned at the construct that caused it.
///
/// [`Self::message`] is the bare reason, one line, with no position and no
/// severity word in it, so that a caller can frame it for its own medium; the
/// [`Display`](std::fmt::Display) impl is the terminal's framing, and
/// [`Self::render`] is that framing without the position.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{span}: {}", self.render())]
pub struct ParseFileError {
    pub span: Span,
    pub message: String,
    pub kind: ParseFailureKind,
}

impl ParseFileError {
    /// A lexer failure, passed through unchanged.
    pub fn from_lex(e: crate::lexer::LexError) -> Self {
        ParseFileError {
            span: e.span,
            message: e.msg,
            kind: ParseFailureKind::Lex,
        }
    }

    /// The one-line message, with the framing the kind calls for and without
    /// the position — what a language server puts in a diagnostic, and what
    /// [`Display`](std::fmt::Display) prints after the span.
    pub fn render(&self) -> String {
        match self.kind {
            ParseFailureKind::Lex | ParseFailureKind::Syntax => {
                format!("parse error: {}", self.message)
            }
            // NOT "parse error": nothing here says the source is wrong.
            ParseFailureKind::GaveUp => format!("gave up: {}", self.message),
        }
    }
}

/// The message [`ParseFailureKind::GaveUp`] carries.
///
/// Says three things, in this order, because that is the order a reader needs
/// them: that no verdict was reached, where the parser was when it stopped,
/// and what that usually means.
const GAVE_UP: &str = "this file needs more backtracking than the parser allows. \
                       The parse got this far and no further, which usually means a \
                       construct at or above this point is unfinished";

/// Reduce a failed parse to one position and one message.
///
/// `stream` must be the stream the parse ran on — that is where the high-water
/// mark is. See the module doc for why the error tree alone is not enough.
pub fn locate(source: &str, stream: &AtomStream, err: &ParseError<Span>) -> ParseFileError {
    let furthest = stream.furthest();
    let stalled = stream.furthest_span();

    // The parse never reached a verdict — the budget cut it off. The
    // `ParseError` in hand says only "the input ended", which the budget
    // caused and the source did not, so it must not be repeated as if it
    // described the source.
    if stream.exhausted() {
        return ParseFileError {
            span: stalled.unwrap_or(*err.span()),
            message: GAVE_UP.to_string(),
            kind: ParseFailureKind::GaveUp,
        };
    }

    let (span, message) = best_failure(err);
    // Where the error TREE points, versus how far the parse actually got. The
    // mark is >= any leaf's end by construction (a leaf's span comes from a
    // token that was served), so this partitions cleanly: EQUAL means the tree
    // knows as much as the stream, and its message is strictly more
    // informative because it names the expected alternatives; LESS means a
    // repetition swallowed the real failure and the tree is stale, and then
    // the mark is the only signal there is.
    if span.end.byte >= furthest {
        return ParseFileError {
            span,
            message,
            kind: ParseFailureKind::Syntax,
        };
    }
    let stalled = stalled.unwrap_or(span);
    ParseFileError {
        message: stalled_message(source, stalled),
        span: stalled,
        kind: ParseFailureKind::Syntax,
    }
}

/// Message for a failure located by the high-water mark rather than by the
/// error tree.
///
/// The tree's own message cannot be reused here: it describes the *outermost*
/// alternative that failed ("expected end of input", for a 0.1 file whose one
/// top-level `module` binding did not parse), which paired with an inner
/// position would read as a claim about that position that is not true. What
/// is known for certain is which token the parse could not get past, so that
/// is what the message says — quoting the source's own text, so the reader
/// sees exactly the characters involved.
fn stalled_message(source: &str, span: Span) -> String {
    const MAX: usize = 24;
    let start = floor_char_boundary(source, span.start.byte);
    let end = floor_char_boundary(source, span.end.byte.max(start));
    let raw = source[start..end].trim();
    if raw.is_empty() {
        // The end-of-input sentinel has a zero-width span past the last
        // character, so there is nothing to quote; say what that means rather
        // than pointing wordlessly at a position.
        return match start >= source.trim_end().len() {
            true => "unexpected end of input".to_string(),
            false => "unexpected input here".to_string(),
        };
    }
    // One line, so a token spanning a whole `'<...>` block does not paste a
    // paragraph into a diagnostics pane.
    let text: String = raw
        .chars()
        .take_while(|c| *c != '\n' && *c != '\r')
        .collect();
    let text = text.trim_end();
    if text.chars().count() > MAX {
        let cut: String = text.chars().take(MAX).collect();
        return format!("unexpected `{cut}...`");
    }
    format!("unexpected `{text}`")
}

/// Reduce syan's error tree to one position and one message, by the standard
/// furthest-failure rule: walk to the leaves, keep the ones that got deepest
/// into the token stream, and report their position with their (short,
/// `Display`-rendered) reasons joined. The depth measure is the leaf span's
/// **end** byte, because that is how far the parser had consumed when it gave
/// up.
fn best_failure(err: &ParseError<Span>) -> (Span, String) {
    let mut deepest: Option<Span> = None;
    let mut reasons: Vec<String> = Vec::new();
    visit_leaves(err, &mut |leaf| {
        let span = *leaf.span();
        let depth = span.end.byte;
        let best = deepest.map(|s| s.end.byte);
        if best.is_none_or(|b| depth > b) {
            deepest = Some(span);
            reasons.clear();
        }
        if deepest.map(|s| s.end.byte) == Some(depth) {
            let reason = leaf_reason(leaf);
            if !reasons.contains(&reason) {
                reasons.push(reason);
            }
        }
    });

    let span = deepest.unwrap_or(*err.span());
    (span, render_reasons(&reasons))
}

/// Join the furthest-failure reasons into one message.
///
/// Two touches beyond a plain join:
///
/// - **The list is capped.** A grammar this size offers dozens of
///   continuations at a single position, and "expected A, B, C, … and 40 more"
///   is no more useful than the first few.
/// - **A shared `expected` is factored out.** Every leaf renders as
///   `"expected 'let'"`, so a naive join gives "expected 'let', expected 'if',
///   expected 'fun'", which reads like three separate complaints. Factoring
///   only happens when *every* reason has the prefix — a mixed list (an
///   `expected` beside a hand-written `ParseError::Other`) is joined verbatim
///   rather than mangled into a false parallel.
fn render_reasons(reasons: &[String]) -> String {
    const MAX_REASONS: usize = 4;
    const PREFIX: &str = "expected ";

    if reasons.is_empty() {
        return "the input does not parse here".to_string();
    }
    let (kept, extra) = match reasons.len() > MAX_REASONS {
        true => (&reasons[..MAX_REASONS], reasons.len() - MAX_REASONS),
        false => (reasons, 0),
    };
    let all_expected = kept.iter().all(|r| r.starts_with(PREFIX));
    let body = if all_expected {
        let stripped: Vec<String> = kept.iter().map(|r| r[PREFIX.len()..].to_string()).collect();
        format!("expected {}", join_alternatives(&stripped))
    } else {
        join_alternatives(kept)
    };
    match extra {
        0 => body,
        n => format!("{body} (and {n} more)"),
    }
}

/// Depth-first walk over the non-`Alternatives` leaves of an error tree. An
/// `Alternatives` node with no children is itself a leaf (syan builds one for
/// an empty alternative set).
fn visit_leaves(err: &ParseError<Span>, f: &mut impl FnMut(&ParseError<Span>)) {
    let alts = err.alternatives();
    if alts.is_empty() {
        f(err);
        return;
    }
    for alt in alts {
        visit_leaves(alt, f);
    }
}

/// One leaf's reason, without syan's `Display` position suffix.
///
/// `ParseError`'s own `Display` appends `" at {span:?}"` for a spanned error,
/// which would put a `Span { start: Loc { line: .., col: .., byte: .. } }`
/// dump in the middle of the message. The span is already the diagnostic's
/// position, so it is dropped here rather than repeated in words.
fn leaf_reason(leaf: &ParseError<Span>) -> String {
    let rendered = leaf.to_string();
    match rendered.rfind(" at Span {") {
        Some(cut) => rendered[..cut].to_string(),
        None => rendered,
    }
}

/// `["a", "b", "c"]` → `"a, b, or c"`.
fn join_alternatives(reasons: &[String]) -> String {
    match reasons {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{}, or {last}", head.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_alternatives_reads_as_english() {
        assert_eq!(join_alternatives(&[]), "");
        assert_eq!(join_alternatives(&["a".into()]), "a");
        assert_eq!(join_alternatives(&["a".into(), "b".into()]), "a, or b");
        assert_eq!(
            join_alternatives(&["a".into(), "b".into(), "c".into()]),
            "a, b, or c"
        );
    }

    #[test]
    fn render_reasons_factors_out_a_shared_expected() {
        assert_eq!(render_reasons(&[]), "the input does not parse here");
        assert_eq!(
            render_reasons(&["expected 'let'".into(), "expected 'if'".into()]),
            "expected 'let', or 'if'"
        );
        // Mixed with a non-`expected` reason: joined verbatim, because
        // factoring would attach `expected` to something that is not a thing
        // the parser expected.
        assert_eq!(
            render_reasons(&["expected 'let'".into(), "unexpected end of input".into()]),
            "expected 'let', or unexpected end of input"
        );
    }

    #[test]
    fn render_reasons_caps_a_long_alternative_list() {
        let many: Vec<String> = (0..9).map(|i| format!("expected '{i}'")).collect();
        assert_eq!(
            render_reasons(&many),
            "expected '0', '1', '2', or '3' (and 5 more)"
        );
    }

    #[test]
    fn leaf_reason_drops_syans_span_suffix() {
        let span = Span::default();
        let leaf = ParseError::expected(span, "end of input");
        let rendered = leaf.to_string();
        assert!(
            rendered.contains("at Span {"),
            "syan changed its Display: {rendered}"
        );
        assert_eq!(leaf_reason(&leaf), "expected end of input");
    }

    /// A give-up is framed as a give-up, and a syntax error as a syntax
    /// error. The whole point of [`ParseFailureKind`] is that these two do not
    /// read alike.
    #[test]
    fn the_two_kinds_are_framed_differently() {
        let syntax = ParseFileError {
            span: Span::default(),
            message: "expected 'in'".to_string(),
            kind: ParseFailureKind::Syntax,
        };
        assert_eq!(syntax.render(), "parse error: expected 'in'");
        let gave_up = ParseFileError {
            span: Span::default(),
            message: GAVE_UP.to_string(),
            kind: ParseFailureKind::GaveUp,
        };
        assert!(gave_up.render().starts_with("gave up: "), "{gave_up}");
        assert!(!gave_up.render().contains("parse error"), "{gave_up}");
    }
}
