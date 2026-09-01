//! Slice 6: **which gap inside `{ … }` may gain or lose a line break**.
//!
//! One predicate, in one place, because three consumers have to agree about
//! it exactly: the two builders decide what to emit, and
//! [`super::same_tokens`] decides what to accept. A verifier that re-derives
//! the rule from the lexer's own output is worth having — but only if it is
//! the same rule, and the way to guarantee that is one function.
//!
//! # Why inline text needs a predicate at all
//!
//! It is the one area whose whitespace is a **token**. `lex_vertical`
//! (`lexer.rs:1029-1032`), `lex_math` (`:1338-1340`) and `lex_active`
//! (`:1241-1243`) skip whitespace without emitting, so re-laying those out
//! cannot change the token stream — there is no token there to change. In
//! horizontal mode a run collapses to one `Space` or `Break`, and which one it
//! is is fixed by the run's FIRST character (`:1149-1155`). So joining two
//! lines is `Break` -> `Space` and splitting one is `Space` -> `Break`, and
//! for the typesetter those are two different documents wherever it can see
//! the difference.
//!
//! # The measured predicate
//!
//! From 123 fixture pairs, 801 in-process compiles and 221 vacuity probes
//! with 0 vacuous, recorded in `docs/plans/formatter-cst/README.md`'s rule 3
//! and re-verified for this slice by `crates/rustyfi/tests/ws_inline_rewrap.rs`:
//!
//! > A break may be inserted **or removed** at a gap iff NOT (the codepoint
//! > immediately before it is CJK **and** the codepoint immediately after it
//! > is CJK).
//!
//! Insert and remove are the same relation. Run length and indentation are
//! free everywhere (123/123), which is what slice 4 already ships.
//!
//! Two things about it are not obvious and both were measured rather than
//! reasoned:
//!
//! - **CJK on only ONE side is fully absorbed.** A Latin paragraph that names
//!   日本語 once re-wraps freely at any column. The first version of this rule
//!   was per AREA and would have refused it; of 1297 inline areas in the
//!   corpus, 246 are multi-line and only 51 are entirely safe, so an
//!   area-level rule reaches 3.9% of areas. Per gap it reaches 80.7% of gaps
//!   and all 246 areas.
//! - **The test is this port's RANGE classifier, not the Unicode Script
//!   property.** [`is_cjk`] mirrors `rustyfi-backend/src/font.rs:87-97`, which
//!   routes U+3000-303F and U+FF00-FFEF through `HanIdeographic`. `Ａ` U+FF21
//!   FULLWIDTH LATIN A has Script=Latin and is measurably unsafe (3.96 pt of
//!   displaced ink), and so are `：` `！` `（）` `。` `、` `「` `々` `・` and
//!   the ideographic space `　` U+3000, which is a `Zs`. Hangul, Thai, Lao,
//!   Greek, Cyrillic and emoji are all **safe** — this port routes them
//!   through `OtherScript`, so the UAX#14 `nonspaced`/class-SA framing does
//!   not apply.
//!
//! # "Immediately", and why the token stream answers it exactly
//!
//! "Immediately" means *within the same elaborated text run*: a command,
//! `${…}`, `#var;`, a backtick literal or a nested group edge ends the run and
//! counts as non-CJK. That is precisely "the neighbouring token is a
//! `Token::Char`", so [`gap_is_reflowable`] reads the neighbours' **payloads**
//! rather than their source bytes — and the payload is the elaborated text,
//! which is what the typesetter sees. `\&` lexes to `Char("&")`
//! (`lexer.rs:1190-1194`) and an escaped space `\ ` to `Char(" ")` — and that
//! last one is rule 3's escaped-space veto, which [`edge`] implements by
//! looking THROUGH the space to the character behind it. Reading it the other
//! way (the space itself decides, so `日本\ ⏎語` is Latin-adjacent and free) is
//! what the sentence most obviously says, and a compile refutes it:
//! `crates/rustyfi/tests/ws_inline_rewrap.rs`'s R19.
//!
//! Reading the payload is also the conservative choice in the one direction
//! that matters. Mistaking a CJK character for a non-CJK one reflows a gap
//! that must be frozen and silently re-typesets somebody's paragraph;
//! mistaking a non-CJK character for a CJK one only declines to reflow. The
//! source bytes of an escape are ASCII and would answer "non-CJK" for a
//! payload that is not — so the payload is read, always.

use rustyfi_syntax::token::Atom;
use rustyfi_syntax::Token;

/// Is `c` CJK **by this port's own classifier**?
///
/// The union of `rustyfi-backend`'s `char_script` arms that answer `Kana` or
/// `HanIdeographic` (`font.rs:83-92`), transcribed rather than called: the
/// analysis half of this crate promises nothing outside `rustyfi-syntax`
/// (`lib.rs:8-22`) and the browser playground links it into wasm, which is
/// the same reason [`super::render::width`] carries its own table.
///
/// `crates/rustyfi/tests/ws_inline_rewrap.rs` asserts this agrees with
/// `char_script` character for character over the boundaries of every range,
/// so the transcription cannot drift without a test saying so.
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        // Hiragana, Katakana (+ phonetic extensions).
        0x3040..=0x30FF | 0x31F0..=0x31FF
        // CJK symbols and punctuation — `。` `、` `「` `々` `・` and the
        // ideographic space U+3000, none of which is Han or Kana and every
        // one of which is measurably unsafe.
        | 0x3000..=0x303F
        // CJK Unified Ideographs and Ext-A, compatibility ideographs, Ext-B.
        | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
        // Halfwidth and fullwidth forms — including `Ａ` U+FF21, whose
        // Unicode Script is Latin.
        | 0xFF00..=0xFFEF
    )
}

/// The elaborated character on one side of a gap, or `None` for "the run ends
/// here".
///
/// `None` is the non-CJK answer: rule 3 says a command, a math escape, a
/// `#var;`, a backtick literal and a group edge all end the run and count as
/// non-CJK, and every one of those is a token that is not `Token::Char`.
///
/// # Whitespace inside the run is looked THROUGH, and that was measured
///
/// An escaped space `\ ` lexes to `Char(" ")` (`lexer.rs`'s escape table), so
/// `{日本\ ⏎語}` puts a literal SPACE immediately before the gap. Taking that
/// space as the deciding character makes the gap look Latin-adjacent and free
/// — and `ws_inline_rewrap.rs`'s case R19 compiles the two spellings and gets
/// **DIFFER**. So the escaped space is looked through and `本` decides, which
/// freezes the gap.
///
/// That is `README.md` rule 3's "an escaped space `\ ` joins the run, so the
/// run's LAST character decides" under the reading the measurement picks: the
/// character that decides is the run's last SIGNIFICANT one, not the escape
/// itself. The sentence admits both readings and this is the one that agrees
/// with a compile.
///
/// Looking through is also the only direction that is safe to be wrong in. It
/// can only ever find MORE CJK, so it can only ever FREEZE more gaps — it
/// cannot license one that the measurement refuses. The scan walks outward
/// across consecutive `Char` tokens, because `日本` and the `\ ` after it are
/// two separate ones.
fn edge(atoms: &[Atom], from: usize, last: bool) -> Option<char> {
    let mut i = from;
    loop {
        let Some(Token::Char(s)) = atoms.get(i).map(|a| &a.slot) else {
            return None;
        };
        let found = match last {
            true => s.chars().rev().find(|c| !c.is_whitespace()),
            false => s.chars().find(|c| !c.is_whitespace()),
        };
        if let Some(c) = found {
            return Some(c);
        }
        // The whole payload is whitespace — an escaped space on its own.
        // Keep walking away from the gap.
        i = match last {
            true => i.checked_sub(1)?,
            false => i + 1,
        };
    }
}

/// May the whitespace token at `atoms[i]` be re-spelled — a `Space` written as
/// a `Break`, or the other way round?
///
/// Answers for any index; a caller that hands it something other than a
/// `Space`/`Break` gets an answer about a gap that does not exist, which is
/// harmless because nothing acts on it. The two builders call it at the
/// cursor's own index and [`super::same_tokens`] at the index whose slot
/// differs.
pub(crate) fn gap_is_reflowable(atoms: &[Atom], i: usize) -> bool {
    let before = i.checked_sub(1).and_then(|j| edge(atoms, j, true));
    let after = edge(atoms, i + 1, false);
    !(before.is_some_and(is_cjk) && after.is_some_and(is_cjk))
}

/// The two freezes that are about the run's own BYTES rather than about the
/// script on either side of it.
///
/// Both are conservative and neither is in the measured predicate, because
/// neither is a question about the typeset output:
///
/// - **A `%` comment inside the run.** `{a  % c⏎  b}` is one `Space` token
///   whose span holds the comment (`lexer.rs:1149-1155` takes the whole run,
///   and `trivia.rs`'s module comment, trap 1, is about exactly this). Writing
///   the run as one space would DELETE the comment; writing it as a newline
///   would move it. Slice 4 already refuses to reflow a comment reached from
///   here for the same reason, and an inline comment can delete a space
///   besides (`{Alpha% c⏎beta}` sets `Alphabeta`, measured as I14/I15).
/// - **A blank line inside the run.** `{a⏎⏎b}` is one `Break`, so the
///   typesetter cannot tell it from `{a⏎b}` — but the author wrote a paragraph
///   break and filling it away is a change to the document's shape that no
///   width budget asked for. Slice 4 preserves it; slice 6 does not get to
///   undo that.
pub(crate) fn run_bytes_allow_reflow(text: &str) -> bool {
    !text.contains('%') && terminators(text) <= 1
}

/// How many line terminators `text` holds, counting a `\r\n` as **one**.
///
/// Not `matches('\n') + matches('\r')`, which counts a single CRLF as two and
/// would freeze every gap in a CRLF file — a whole-file behaviour difference
/// keyed on the line ending, which is the class of bug `format.rs` produced
/// four times.
fn terminators(text: &str) -> usize {
    let mut n = 0usize;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' => {
                n += 1;
                if it.peek() == Some(&'\n') {
                    it.next();
                }
            }
            '\n' => n += 1,
            _ => {}
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_syntax::{lex_with_version, RustyfiVersion};

    /// The classifier, at every range boundary and on the characters rule 3
    /// names as counterexamples.
    #[test]
    fn the_range_test_is_the_ports_own_and_not_the_script_property() {
        // Han and Kana, the obvious half.
        for c in ['日', '本', '語', 'あ', 'ア', '㐀', '豈'] {
            assert!(is_cjk(c), "{c:?}");
        }
        // The counterexamples: measurably unsafe, and NOT Han or Kana.
        for c in ['：', '！', '（', '）', '。', '、', '「', '々', '・', '\u{3000}', 'Ａ'] {
            assert!(is_cjk(c), "{c:?} is unsafe by measurement and must classify as CJK");
        }
        // Safe: this port routes all of these through `OtherScript`.
        for c in ['a', 'Z', '0', ' ', '한', 'ก', 'ລ', 'α', 'Ж', '🙂', '—', '…'] {
            assert!(!is_cjk(c), "{c:?}");
        }
        // The exact boundaries, so a transcription slip in either direction
        // fails rather than shifting a range quietly.
        for (lo, hi) in [
            (0x3000u32, 0x303F),
            (0x3040, 0x30FF),
            (0x31F0, 0x31FF),
            (0x3400, 0x4DBF),
            (0x4E00, 0x9FFF),
            (0xF900, 0xFAFF),
            (0xFF00, 0xFFEF),
            (0x20000, 0x2FA1F),
        ] {
            for u in [lo, hi] {
                assert!(is_cjk(char::from_u32(u).unwrap()), "U+{u:04X} inside a range");
            }
        }
        // U+2FFF is below the first range and U+FFF0 above the last.
        assert!(!is_cjk('\u{2FFF}'));
        assert!(!is_cjk('\u{FFF0}'));
    }

    /// The predicate, read off real token streams.
    fn verdicts(src: &str) -> Vec<bool> {
        let atoms = lex_with_version(src, RustyfiVersion::V0_0).expect("lexes");
        atoms
            .iter()
            .enumerate()
            .filter(|(_, a)| matches!(a.slot, Token::Space | Token::Break))
            .map(|(i, _)| gap_is_reflowable(&atoms, i))
            .collect()
    }

    #[test]
    fn a_gap_is_frozen_only_when_cjk_stands_on_both_sides_of_it() {
        // Latin either side.
        assert_eq!(verdicts("let x = {abc def} in x\n"), [true]);
        // CJK either side: the one refusal.
        assert_eq!(verdicts("let x = {日本\n語です} in x\n"), [false]);
        // CJK on ONE side is absorbed — the finding an area-level rule misses.
        assert_eq!(verdicts("let x = {日本語 and abc} in x\n"), [true, true]);
        // The counterexamples that are not Han or Kana.
        assert_eq!(verdicts("let x = {あ、\nＡ} in x\n"), [false]);
        assert_eq!(verdicts("let x = {語。\n「あ」} in x\n"), [false]);
    }

    #[test]
    fn a_run_that_ends_at_something_other_than_text_is_not_cjk_on_that_side() {
        // A command edge, a math escape and a nested group edge each end the
        // run, so the gap beside one is reflowable even between CJK.
        assert_eq!(verdicts("let x = {\\emph{日} 本} in x\n"), [true]);
        assert_eq!(verdicts("let x = {日 ${b} 本} in x\n"), [true, true]);
        // ONE gap, not two: the run in front of a nested `{` is swallowed
        // into the `BHorzGrp` token (`lexer.rs:1112-1147`) and is not a
        // whitespace token at all, so there is nothing there to reflow. The
        // gap after the `}` sees a group edge on its left and is safe.
        assert_eq!(verdicts("let x = {日 {本語} です} in x\n"), [true]);
        // An escaped space is LOOKED THROUGH: `本` decides, not the space,
        // so the gap freezes. Measured — `ws_inline_rewrap.rs`'s R19 compiles
        // the two spellings and they DIFFER. See [`edge`].
        assert_eq!(verdicts("let x = {日本\\ \n語} in x\n"), [false]);
        // The control: without the escape the same two sides freeze too, so
        // the escape changes nothing rather than licensing anything.
        assert_eq!(verdicts("let x = {日本\n語} in x\n"), [false]);
        // And it is not a blanket freeze — an escaped space between Latin and
        // CJK is still free, because only one side is CJK.
        assert_eq!(verdicts("let x = {abc\\ \n語} in x\n"), [true]);
        assert_eq!(verdicts("let x = {日本\\ \nabc} in x\n"), [true]);
    }

    #[test]
    fn a_comment_or_a_blank_line_inside_the_run_freezes_it_whatever_the_script() {
        assert!(!run_bytes_allow_reflow("  % c\n  "));
        assert!(!run_bytes_allow_reflow("\n\n  "));
        assert!(run_bytes_allow_reflow(" "));
        assert!(run_bytes_allow_reflow("\n    "));
        assert!(run_bytes_allow_reflow("   "));
        // CRLF is ONE terminator, not two.
        assert!(run_bytes_allow_reflow("\r\n  "));
        assert!(!run_bytes_allow_reflow("\r\n\r\n  "));
    }
}
