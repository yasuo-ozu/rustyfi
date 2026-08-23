//! The protocol-free half: source text in, [`Diag`]s out.
//!
//! See the crate doc comment for why this stops at parsing.

use rustyfi_syntax::{RustyfiVersion, Span};
use syan::error::ParseError;
use syan::parse::Parse;

use crate::high_water::HighWaterStream;
use crate::line_index::{floor_boundary, LineIndex};

/// How bad a [`Diag`] is. The numbering matches LSP's `DiagnosticSeverity`
/// (1 = `Error`) so the server can cast straight across, but the type itself
/// names no LSP crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard failure: the file does not lex or parse.
    Error,
    /// Something suspicious that does not stop the compile.
    Warning,
    /// Neutral information.
    Information,
    /// A gentle suggestion.
    Hint,
}

impl Severity {
    /// The LSP `DiagnosticSeverity` wire number.
    pub fn code(self) -> u8 {
        match self {
            Severity::Error => 1,
            Severity::Warning => 2,
            Severity::Information => 3,
            Severity::Hint => 4,
        }
    }
}

/// One diagnostic, positioned the way LSP wants it: **zero-based** lines, and
/// characters counted in **UTF-16 code units** (see [`crate::LineIndex`]).
///
/// The range is half-open, `[start, end)`, and is always non-degenerate —
/// [`analyze`] widens a zero-width span to cover at least one character so
/// the editor has something to underline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// Zero-based start line.
    pub line: u32,
    /// Zero-based start character, in UTF-16 code units.
    pub character: u32,
    /// Zero-based end line.
    pub end_line: u32,
    /// Zero-based end character, in UTF-16 code units.
    pub end_character: u32,
    /// Severity.
    pub severity: Severity,
    /// Human-readable message, one line, no trailing newline.
    pub message: String,
}

/// Analyse one buffer under an explicitly chosen generation.
///
/// Pure: no filesystem, no environment, no globals — which is what makes it
/// usable from `wasm32-unknown-unknown` and from a test without a temp
/// directory. `@require:`/`@import:` headers are *parsed* (they are part of
/// the grammar) but never *resolved*, so a document naming packages that are
/// not installed still gets clean parse diagnostics.
///
/// `lang` is taken literally — there is no fallback to the other generation.
/// Use [`analyze_auto`] to have the generation chosen (and, where the buffer
/// is genuinely ambiguous, cross-checked) from the text.
///
/// Returns at most one diagnostic today — this port's parser stops at the
/// first failure and has no error recovery — but the signature is a `Vec` so
/// that adding recovery, or a second analysis tier, is not a breaking change.
pub fn analyze(source: &str, lang: RustyfiVersion) -> Vec<Diag> {
    match parse_failure(source, lang) {
        None => Vec::new(),
        Some(f) => vec![f.into_diag(source)],
    }
}

/// [`analyze`], choosing the generation from the buffer itself.
///
/// The base rule is the CLI's own Axis-A ladder (`rustyfi`'s
/// `resolve_version_and_mode`): [`rustyfi_syntax::sniff_version`]'s verdict,
/// else [`RustyfiVersion::DEFAULT`]. Deliberately the CLI's rule and not
/// `rustyfi_loader`'s — the loader answers a different question, "which
/// generation is this *dependency* of an already-pinned entry?", and its
/// answer depends on the parent that reached the file and on whether the path
/// sits in a frozen corpus directory. An open buffer is an entry, not a
/// dependency: nothing has reached it, so only the entry rule applies.
///
/// On top of that, one addition that matters a great deal in practice:
/// **when the buffer carries no version signal at all, a failure under the
/// default generation is re-checked against the other one, and a clean
/// re-check wins.**
///
/// This is not hedging. [`rustyfi_syntax::sniff_version`] is decisive only
/// for a `use`/`val` head (0.1) or a `@stage:`/`let-*` head (0.0); a *library*
/// file — `.satyh`/`.satyg` — of either generation typically opens `module M
/// = struct` after its headers, which is deliberately no signal, and so
/// defaults to 0.0. Without the re-check, opening any file in this port's own
/// bundled 0.1 corpus (`lib-rustyfi/dist-v01/packages/`) paints a parse error
/// on a file that compiles: 32 of its 34 packages, measured. The full
/// compiler does not have this problem because `rustyfi_loader` knows which
/// *entry* reached a dependency and under which generation; a language server
/// holding one detached buffer knows neither.
///
/// The re-check runs only when the sniff was `None` and the default parse
/// failed, so a decisive signal is always obeyed, a clean file costs one
/// parse, and a genuinely broken file costs two.
pub fn analyze_auto(source: &str) -> Vec<Diag> {
    analyze_detected(source).1
}

/// [`analyze_auto`], also reporting which generation the result was produced
/// under — after the ambiguity re-check, so this is the generation the buffer
/// actually parses as, not merely the sniffed guess.
pub fn analyze_detected(source: &str) -> (RustyfiVersion, Vec<Diag>) {
    let (version, failure) = detect(source);
    (
        version,
        failure.into_iter().map(|f| f.into_diag(source)).collect(),
    )
}

/// The verdict alone — [`parse_detected`] with the tree dropped.
fn detect(source: &str) -> (RustyfiVersion, Option<Failure>) {
    let (version, parsed) = parse_detected(source);
    (version, parsed.err())
}

/// A buffer that lexed and parsed, tagged by the grammar that read it —
/// [`parse_detected`]'s success value.
///
/// `None` is a buffer with no tokens at all (see [`parse_source`]): nothing
/// was parsed, and nothing is wrong either.
///
/// Without the `typecheck` feature nothing reads the trees — [`analyze`] only
/// ever wants the verdict — so the payloads are dead code in the wasm build
/// specifically. Building them costs nothing there either: the parser
/// constructs the CST whether or not anyone keeps it.
#[cfg_attr(not(feature = "typecheck"), allow(dead_code))]
pub(crate) enum Parsed {
    None,
    V0_0(rustyfi_syntax::cst::File),
    V0_1(rustyfi_syntax::cst_v1::FileV1),
}

#[cfg_attr(not(feature = "typecheck"), allow(dead_code))]
impl Parsed {
    /// Whether this buffer is a *document* (has a body expression) rather
    /// than a library. Mirrors `rustyfi_loader::LoadedCst::is_document`, which
    /// cannot be reused because this half of the crate does not depend on the
    /// loader.
    pub(crate) fn is_document(&self) -> bool {
        match self {
            Parsed::None => false,
            Parsed::V0_0(f) => f.body.is_some(),
            Parsed::V0_1(f) => matches!(f, rustyfi_syntax::cst_v1::FileV1::Document { .. }),
        }
    }
}

/// [`parse_detected`], or a forced generation when the caller has one — the
/// parse-keeping counterpart of the server's `match self.opts.lang { Some =>
/// analyze, None => analyze_auto }`, written once so the two entry points
/// cannot drift.
#[cfg(feature = "typecheck")]
pub(crate) fn parse_with(
    source: &str,
    lang: Option<RustyfiVersion>,
) -> (RustyfiVersion, Result<Parsed, Failure>) {
    match lang {
        Some(lang) => (lang, parse_source(source, lang)),
        None => parse_detected(source),
    }
}

/// [`analyze_detected`]'s engine: the same generation ladder, keeping the
/// parse tree instead of throwing it away.
///
/// Whole-program analysis (`crate::project`) needs three things this returns
/// and [`analyze_detected`] cannot: the generation to hand the loader, the
/// buffer's own CST (so a library can be checked without being parsed a third
/// time), and whether it is a document at all.
///
/// This is the ONE place the ladder is written. Anything else that needs to
/// know a buffer's generation goes through it (`detect`, and hence
/// `analyze_detected`), so a second feature cannot drift into a second answer
/// for the same file.
pub(crate) fn parse_detected(source: &str) -> (RustyfiVersion, Result<Parsed, Failure>) {
    let sniffed = rustyfi_syntax::sniff_version(source);
    let primary = sniffed.unwrap_or(RustyfiVersion::DEFAULT);

    let failure = match parse_source(source, primary) {
        Ok(parsed) => return (primary, Ok(parsed)),
        Err(f) => f,
    };

    // A decisive signal is obeyed even when it does not parse: reporting the
    // 0.0 reading of a file whose first line is `use Foo` would be a lie.
    if sniffed.is_some() {
        return (primary, Err(failure));
    }

    let other = other_generation(primary);
    match parse_source(source, other) {
        Ok(parsed) => (other, Ok(parsed)),
        // Both readings fail, so the buffer is broken under either grammar
        // and the question is only which error to show. Show the one from the
        // grammar that got FURTHER through the text — the same
        // furthest-progress rule `parse_failure` uses to locate an error
        // within one parse, applied one level up to choose between two.
        //
        // It matters: `module M = struct … end` sniffs as no signal at all,
        // so a 0.1 library with a typo halfway down would otherwise report
        // 0.0.6's complaint about the `module` head on line 2, which names a
        // construct the author did not write and a line they did not touch.
        // The 0.1 reading reaches the typo; the 0.0.6 one dies at the head.
        //
        // A reading that ran out of backtracking budget competes on the same
        // terms: it got as far as it did by accounting for that much of the
        // file, which is exactly what is being compared. Preferring a
        // *verdict* over a give-up was tried and is wrong — the give-up is
        // overwhelmingly the RIGHT grammar struggling with an incomplete
        // buffer, while the verdict is the wrong grammar dying early, so that
        // rule reported 0.0.6's complaint about a half-typed 0.1 library.
        Err(alt) if alt.furthest > failure.furthest => (other, Err(alt)),
        Err(_) => (primary, Err(failure)),
    }
}

/// The generation that is not `v`. Written as a match rather than `!=` so
/// that adding a third variant to the `#[non_exhaustive]`
/// [`RustyfiVersion`] is a compile error here rather than a silent
/// mis-pairing.
fn other_generation(v: RustyfiVersion) -> RustyfiVersion {
    match v {
        RustyfiVersion::V0_1 => RustyfiVersion::V0_0,
        _ => RustyfiVersion::V0_1,
    }
}

/// A lex or parse failure, before it is turned into LSP coordinates.
pub(crate) struct Failure {
    span: Span,
    message: String,
    /// How far into the source the attempt got before giving up — the
    /// high-water mark for a parse failure, the failing position for a lex
    /// one. Used to choose between two generations' failures in
    /// [`analyze_detected`].
    furthest: usize,
}

impl Failure {
    pub(crate) fn into_diag(self, source: &str) -> Diag {
        let (start, end) = span_to_range(&LineIndex::new(source), self.span);
        Diag {
            line: start.line,
            character: start.character,
            end_line: end.line,
            end_character: end.character,
            severity: Severity::Error,
            message: self.message,
        }
    }
}

/// Lex and parse `source` under `lang`, returning the failure if there is
/// one.
///
/// A thin wrapper over [`parse_source`] for the callers that only want the
/// verdict; see there for why this is not `rustyfi_syntax::parse_file`.
fn parse_failure(source: &str, lang: RustyfiVersion) -> Option<Failure> {
    parse_source(source, lang).err()
}

/// Lex and parse `source` under `lang`, keeping the tree.
///
/// This deliberately re-implements [`rustyfi_syntax::parse_file`]'s two steps
/// rather than calling it, for one reason: `parse_file` flattens syan's error
/// tree with `format!("{err:?}")` into a `String`, and by then the structure
/// is gone. The tree is what makes a usable message — see [`best_failure`].
fn parse_source(source: &str, lang: RustyfiVersion) -> Result<Parsed, Failure> {
    let atoms = match rustyfi_syntax::lex_with_version(source, lang) {
        Ok(atoms) => atoms,
        // Lex errors already carry a hand-written message and a tight span.
        Err(e) => {
            return Err(Failure {
                furthest: e.span.end.byte,
                span: e.span,
                message: e.msg,
            })
        }
    };
    // A buffer with no tokens at all — empty, or nothing but whitespace and
    // `%` comments — has nothing to diagnose. (The lexer always appends an
    // end-of-input sentinel, so "no tokens" is "nothing but `Eoi`", not an
    // empty vector.) Said explicitly because the two grammars disagree about
    // it: 0.0.6's `File` accepts an empty token stream, 0.1's `FileV1`
    // demands at least a binding or a body, so without this a `--lang 0.1`
    // user gets a red squiggle on a file they have not started typing yet.
    // Declining to complain hides nothing — there is no construct present to
    // be wrong.
    if atoms.iter().all(|a| a.slot == rustyfi_syntax::Token::Eoi) {
        return Ok(Parsed::None);
    }
    let mut stream = HighWaterStream::new(atoms);
    let err = match lang {
        RustyfiVersion::V0_1 => match <rustyfi_syntax::cst_v1::FileV1 as Parse<_>>::parse(&mut stream) {
            Ok(file) => return Ok(Parsed::V0_1(file)),
            Err(e) => e,
        },
        // `RustyfiVersion` is `#[non_exhaustive]`; anything not explicitly
        // 0.1 is read with the 0.0.6 grammar, which is `DEFAULT`'s behaviour.
        _ => match <rustyfi_syntax::cst::File as Parse<_>>::parse(&mut stream) {
            Ok(file) => return Ok(Parsed::V0_0(file)),
            Err(e) => e,
        },
    };

    let furthest = stream.furthest();
    let stalled = stream.furthest_span();

    // The parse never reached a verdict — it was cut off by the stream's
    // budget (see `high_water`). The `ParseError` in hand says only "the
    // input ended", which this crate caused and the buffer did not, so it
    // must not be repeated as if it described the source.
    if stream.exhausted() {
        let span = stalled.unwrap_or(*err.span());
        return Err(Failure {
            span,
            message: "parse error: gave up analysing this file — it needs more \
                      backtracking than the language server allows, which usually \
                      means it is still incomplete here"
                .to_string(),
            furthest,
        });
    }

    let (span, message) = best_failure(&err);
    // Where the error TREE points, versus how far the parse actually got. The
    // mark is >= any leaf's end by construction (a leaf's span comes from a
    // token that was served), so this partitions cleanly: EQUAL means the
    // tree knows as much as the stream, and its message is strictly more
    // informative because it names the expected alternatives; LESS means a
    // repetition swallowed the real failure and the tree is stale, and then
    // the mark is the only signal there is. See `high_water`.
    if span.end.byte >= furthest {
        return Err(Failure {
            span,
            message,
            furthest,
        });
    }
    let stalled = stalled.unwrap_or(span);
    Err(Failure {
        message: stalled_message(source, stalled),
        span: stalled,
        furthest,
    })
}

/// Message for a failure located by the high-water mark rather than by the
/// error tree.
///
/// The tree's own message cannot be reused here: it describes the *outermost*
/// alternative that failed ("expected end of input", for a 0.1 file whose one
/// top-level `module` binding did not parse), which paired with an inner
/// position would read as a claim about that position that is not true. What
/// is known for certain is which token the parse could not get past, so that
/// is what the message says — quoting the buffer's own text, so the user sees
/// exactly the characters involved.
fn stalled_message(source: &str, span: Span) -> String {
    const MAX: usize = 24;
    let start = floor_boundary(source, span.start.byte);
    let end = floor_boundary(source, span.end.byte.max(start));
    let raw = source[start..end].trim();
    if raw.is_empty() {
        return "parse error here".to_string();
    }
    // One line, so a token spanning a whole `'<...>` block does not paste a
    // paragraph into the editor's diagnostics pane.
    let text: String = raw.chars().take_while(|c| *c != '\n' && *c != '\r').collect();
    let text = text.trim_end();
    if text.chars().count() > MAX {
        let cut: String = text.chars().take(MAX).collect();
        return format!("parse error: unexpected `{cut}...`");
    }
    format!("parse error: unexpected `{text}`")
}

/// Reduce syan's error tree to one position and one message.
///
/// Two problems with the aggregate node the parser hands back:
///
/// - **Its span is useless.** `ParseError::from_cause` documents that an
///   `Alternatives` aggregate "takes the FIRST alternative's span: with no
///   record of how far each alternative got, that is the only deterministic
///   choice available". For a top-level file rule the first alternative
///   typically fails on the very first token, so the aggregate points at
///   byte 0 — a squiggle on the first word of a 300-line document whose
///   error is on line 250.
/// - **Its `Debug` is a wall of `Span { start: Loc { .. } }`.** An editor
///   renders a diagnostic message on one line.
///
/// Both are fixed by the standard furthest-failure rule: walk the tree to its
/// leaves, keep the ones that got deepest into the token stream, and report
/// their position with their (short, `Display`-rendered) reasons joined. The
/// depth measure is the leaf span's **end** byte, because that is how far the
/// parser had consumed when it gave up.
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
///   continuations at a single position, and "expected A, B, C, … and 40
///   more" is no more useful than the first few.
/// - **A shared `expected` is factored out.** Every leaf renders as
///   `"expected 'let'"`, so a naive join gives "expected 'let', expected
///   'if', expected 'fun'", which reads like three separate complaints.
///   Factoring only happens when *every* reason has the prefix — a mixed
///   list (an `expected` beside a hand-written `ParseError::Other`) is joined
///   verbatim rather than mangled into a false parallel.
fn render_reasons(reasons: &[String]) -> String {
    const MAX_REASONS: usize = 4;
    const PREFIX: &str = "expected ";

    if reasons.is_empty() {
        return "parse error".to_string();
    }
    let (kept, extra) = match reasons.len() > MAX_REASONS {
        true => (&reasons[..MAX_REASONS], reasons.len() - MAX_REASONS),
        false => (reasons, 0),
    };
    let all_expected = kept.iter().all(|r| r.starts_with(PREFIX));
    let body = if all_expected {
        let stripped: Vec<String> = kept
            .iter()
            .map(|r| r[PREFIX.len()..].to_string())
            .collect();
        format!("expected {}", join_alternatives(&stripped))
    } else {
        join_alternatives(kept)
    };
    match extra {
        0 => format!("parse error: {body}"),
        n => format!("parse error: {body} (and {n} more)"),
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
/// range, so it is dropped here rather than repeated in words.
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

/// Turn a [`rustyfi_syntax::Span`] into a non-degenerate LSP range.
///
/// Two adjustments the raw span needs:
///
/// - **Byte offsets, not `Loc::line`/`Loc::col`.** `Loc::col` counts `char`s,
///   LSP counts UTF-16 code units; they differ on astral-plane characters.
///   `Loc::byte` is exact, so both coordinates are re-derived from it.
/// - **Zero-width spans are widened.** A parse error that fires with
///   `start == end` (an unexpected end of input, or a failure syan could only
///   attribute to a single point) would otherwise render as an invisible
///   caret. Widening to the next character — or, at end of file, to the
///   previous one — gives the editor something to draw.
pub(crate) fn span_to_range(
    index: &LineIndex<'_>,
    span: Span,
) -> (crate::Position, crate::Position) {
    // Taken from the index rather than as a second parameter, so the two can
    // never be handed a different buffer each.
    let source = index.source();
    let start_byte = floor_boundary(source, span.start.byte);
    let end_byte = floor_boundary(source, span.end.byte.max(start_byte));

    if start_byte < end_byte {
        return (index.position(start_byte), index.position(end_byte));
    }

    // Degenerate: widen by one character, forwards if there is one.
    if let Some(c) = source[start_byte..].chars().next() {
        return (
            index.position(start_byte),
            index.position(start_byte + c.len_utf8()),
        );
    }
    // At end of file: widen backwards instead, so the range is still visible.
    match source[..start_byte].chars().next_back() {
        Some(c) => (
            index.position(start_byte - c.len_utf8()),
            index.position(start_byte),
        ),
        // Empty file. A zero-width range at the origin is all there is.
        None => (index.position(0), index.position(0)),
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
        assert_eq!(render_reasons(&[]), "parse error");
        assert_eq!(
            render_reasons(&["expected 'let'".into(), "expected 'if'".into()]),
            "parse error: expected 'let', or 'if'"
        );
        // Mixed with a non-`expected` reason: joined verbatim, because
        // factoring would attach `expected` to something that is not a thing
        // the parser expected.
        assert_eq!(
            render_reasons(&["expected 'let'".into(), "unexpected end of input".into()]),
            "parse error: expected 'let', or unexpected end of input"
        );
    }

    #[test]
    fn render_reasons_caps_a_long_alternative_list() {
        let many: Vec<String> = (0..9).map(|i| format!("expected '{i}'")).collect();
        assert_eq!(
            render_reasons(&many),
            "parse error: expected '0', '1', '2', or '3' (and 5 more)"
        );
    }

    #[test]
    fn leaf_reason_drops_syans_span_suffix() {
        let span = Span::default();
        let leaf = ParseError::expected(span, "end of input");
        let rendered = leaf.to_string();
        assert!(rendered.contains("at Span {"), "syan changed its Display: {rendered}");
        assert_eq!(leaf_reason(&leaf), "expected end of input");
    }
}
