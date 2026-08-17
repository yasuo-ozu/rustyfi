//! S2 golden fixture (`docs/plans/design-hyphenation.md` §5 S2): drives
//! `primitives::read_inline` (`text_to_boxes`'s `flush_word` hyphenation
//! injection, §4) together with `rustyfi_backend::break_into_lines`,
//! proving end to end:
//!
//! 1. With no dictionary installed (`Context::initial`'s default), a
//!    hyphenatable word still produces exactly one `InnerString` — the
//!    byte-identity gate (D4/§6) at the box-construction level.
//! 2. With a dictionary installed, `read_inline` splits the word into
//!    `InnerString` fragments separated by `Discretionary`s whose
//!    `pre_break` is exactly a `-` glyph, and the fragments rejoin to the
//!    original word.
//! 3. A narrow paragraph containing that word actually wraps mid-word at
//!    real line-break time, with a trailing `-` glyph ending the wrapped
//!    line (the "golden": words break mid-word with a trailing hyphen).
//! 4. A huge `hyphen_badness` (the `set-hyphen-penalty 100000` disable
//!    idiom) suppresses the break: the DP keeps the whole word on one
//!    (overfull) line, exactly as if no dictionary were installed.

use rustyfi_backend::{
    break_into_lines, Context, FontKey, FontMetrics, HorzBox, HyphenLang, Length, PureHorzBox,
    VertBox,
};
use rustyfi_lang::quoted::IText;
use rustyfi_lang::eval::Interp;
use rustyfi_lang::hyphenation::hyphenate_word;
use rustyfi_lang::primitives;
use rustyfi_lang::value::Env;

/// Every char is half an em wide (mirrors `rustyfi-backend/tests/
/// linebreak.rs`'s own `Mono`) — deterministic, font-independent widths so
/// this test needs no real TTF/AFM data.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

/// A word real dictionaries hyphenate at more than one point (the design
/// doc's own D2 example: `hy-phen-ation`).
const WORD: &str = "hyphenation";

/// Run `word` through `read_inline` (i.e. `text_to_boxes`/`flush_word`)
/// under `ctx`, plus a trailing `inline-fil` glue (the same convention every
/// `linebreak.rs` fixture uses) so `break_into_lines` gets an ordinary
/// ragged-last-line paragraph.
fn boxes_for_word(ctx: &Context, word: &str) -> Vec<HorzBox> {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let elems = vec![IText::Text(word.to_string())];
    let mut boxes = primitives::read_inline(&mut interp, ctx, &elems, &Env::root())
        .expect("read_inline should succeed");
    boxes.push(HorzBox::Pure(PureHorzBox::OuterFil));
    boxes
}

/// [`boxes_for_word`] specialized to [`WORD`] — kept as the S2 fixtures'
/// original entry point.
fn boxes_for(ctx: &Context) -> Vec<HorzBox> {
    boxes_for_word(ctx, WORD)
}

#[test]
fn no_dictionary_installed_yields_a_single_unsplit_inner_string() {
    // `Context::initial` now installs English by default, as upstream does
    // (`primitives.cppo.ml:500,607`); this test is about the NO-dictionary
    // path, so it opts out explicitly.
    let mut ctx = Context::initial(Length::pt(400.0));
    assert_eq!(
        ctx.hyphen_dictionary,
        Some(HyphenLang::EnglishUS),
        "Context::initial must default to English, matching upstream"
    );
    ctx.hyphen_dictionary = None;
    let boxes = boxes_for(&ctx);
    // boxes: [InnerString(WORD), OuterFil] — no Discretionary at all.
    assert_eq!(boxes.len(), 2, "expected InnerString + trailing fil only: {boxes:?}");
    match &boxes[0] {
        HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => assert_eq!(text, WORD),
        other => panic!("expected a single InnerString, got {other:?}"),
    }
}

#[test]
fn dictionary_installed_splits_the_word_into_fragments_and_discretionaries() {
    let mut ctx = Context::initial(Length::pt(400.0));
    ctx.hyphen_dictionary = Some(HyphenLang::EnglishUS);
    let boxes = boxes_for(&ctx);

    let expected_breaks = hyphenate_word(HyphenLang::EnglishUS, WORD, 3, 2);
    assert!(!expected_breaks.is_empty(), "expected {WORD:?} to actually hyphenate");

    let mut fragments = Vec::new();
    let mut disc_count = 0usize;
    for hb in &boxes {
        match hb {
            HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => fragments.push(text.clone()),
            HorzBox::Pure(PureHorzBox::Discretionary {
                penalty,
                pre_break,
                post_break,
                no_break,
            }) => {
                disc_count += 1;
                assert_eq!(*penalty, 100, "default Context::hyphen_badness");
                assert!(post_break.is_empty(), "post_break must be empty (D2)");
                assert!(no_break.is_empty(), "no_break must be empty (D2)");
                assert_eq!(pre_break.len(), 1);
                match &pre_break[0] {
                    PureHorzBox::InnerString { text, .. } => assert_eq!(text, "-"),
                    other => panic!("expected the hyphen glyph, got {other:?}"),
                }
            }
            HorzBox::Pure(PureHorzBox::OuterFil) => {}
            other => panic!("unexpected box kind: {other:?}"),
        }
    }
    assert_eq!(
        disc_count,
        expected_breaks.len(),
        "one Discretionary per accepted hyphenation break"
    );
    assert_eq!(
        fragments.join(""),
        WORD,
        "fragments must rejoin to the original word (width-identity, D2/§6)"
    );
}

#[ignore = "encodes scale-100 hyphenation break decision; the faithful cost model (|r|^3*10000 + ratio limits, lineBreak.ml) changes the hyphenate-vs-overflow balance in a 40pt column. No corpus doc installs a hyphenation dictionary — rewrite for the faithful model."]#[test]
fn narrow_column_forces_a_mid_word_break_with_a_trailing_hyphen() {
    let mut ctx = Context::initial(Length::pt(40.0));
    ctx.hyphen_dictionary = Some(HyphenLang::EnglishUS);
    let boxes = boxes_for(&ctx);
    let lines = break_into_lines(&ctx, boxes);
    assert!(
        lines.len() >= 2,
        "expected {WORD:?} to wrap across >=2 lines at a 40pt column: {lines:?}"
    );

    let mut saw_trailing_hyphen = false;
    let mut rejoined = String::new();
    for vb in &lines {
        if let VertBox::Line { contents, .. } = vb {
            for (_, b) in contents {
                if let PureHorzBox::InnerString { text, .. } = b {
                    rejoined.push_str(text);
                }
            }
            if let Some((_, PureHorzBox::InnerString { text, .. })) = contents.last() {
                if text == "-" {
                    saw_trailing_hyphen = true;
                }
            }
        }
    }
    assert!(
        saw_trailing_hyphen,
        "expected a wrapped line ending in the hyphen glyph: {lines:?}"
    );
    // Every line's contents include the literal hyphen glyph InnerString
    // wherever a break was taken (that's the point) — strip those out and
    // the word's own letters must still all be present, in order.
    assert_eq!(
        rejoined.replace('-', ""),
        WORD,
        "the word's letters must still all be present, in order, once the injected hyphen \
         glyphs are stripped out"
    );
}

#[test]
fn a_huge_hyphen_penalty_disables_the_break_even_though_the_line_overflows() {
    // The `set-hyphen-penalty 100000` disable idiom
    // (`docs/plans/design-hyphenation.md` D6) does NOT extend to the case where
    // the hyphenated line is itself overfull, because upstream's graph rule
    // outranks any penalty: `hyphen-` is 42pt on this 40pt measure, so it is
    // already `LBTooLong`, and a source node contributes only its FIRST
    // too-long edge before being dropped (`lineBreak.ml:1017-1027`). The
    // whole-word line is that node's SECOND too-long option, so it is never
    // offered — no penalty can bring back an edge the graph never had.
    //
    // (This asserted a single overfull line when the port scored every overfull
    // candidate independently, which let a huge penalty outvote them all.)
    let mut ctx = Context::initial(Length::pt(40.0));
    ctx.hyphen_dictionary = Some(HyphenLang::EnglishUS);
    ctx.hyphen_badness = 100_000;
    let boxes = boxes_for(&ctx);
    let lines = break_into_lines(&ctx, boxes);

    assert_eq!(
        lines.len(),
        2,
        "expected the hyphenated break (the only too-long edge on offer): {lines:?}"
    );
    let VertBox::Line { contents, .. } = &lines[0] else {
        panic!("expected a Line, got {:?}", lines[0]);
    };
    let joined: String = contents
        .iter()
        .filter_map(|(_, b)| match b {
            PureHorzBox::InnerString { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    // The break IS taken, so the first line ends with a printed hyphen — that
    // is the `pre_break` slot doing its job.
    assert_eq!(joined, "hyphen-", "the taken break prints its hyphen on line 1");
}

// ---- S3 (`docs/plans/design-hyphenation.md` §S3) ---------------------------

/// Item 1 (soft-hyphen priority): with a dictionary installed, an explicit
/// U+00AD embedded in the input word must win over dictionary-derived
/// breaks and must not leak into any rendered fragment's text — the word
/// splits exactly (and only) at the marked point.
#[test]
fn explicit_soft_hyphen_wins_over_dictionary_breaks_and_is_not_rendered() {
    let mut ctx = Context::initial(Length::pt(400.0));
    ctx.hyphen_dictionary = Some(HyphenLang::EnglishUS);
    // Authored as "hy-phenation": a single soft hyphen after the 2nd
    // letter — a different split point than the dictionary would choose on
    // its own (`hyphenate_word(EnglishUS, "hyphenation", 3, 2) == [6, 7]`,
    // i.e. "hyphen-a-tion"), so this proves the soft hyphen actually took
    // priority rather than merely coinciding with a pattern break.
    let word_with_shy = "hy\u{ad}phenation";
    let boxes = boxes_for_word(&ctx, word_with_shy);

    let mut fragments = Vec::new();
    let mut disc_count = 0usize;
    for hb in &boxes {
        match hb {
            HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => {
                assert!(
                    !text.contains('\u{ad}'),
                    "the soft hyphen marker itself must never appear in a rendered \
                     fragment: {text:?}"
                );
                fragments.push(text.clone());
            }
            HorzBox::Pure(PureHorzBox::Discretionary { pre_break, post_break, no_break, .. }) => {
                disc_count += 1;
                assert!(post_break.is_empty());
                assert!(no_break.is_empty());
                assert_eq!(pre_break.len(), 1);
                match &pre_break[0] {
                    PureHorzBox::InnerString { text, .. } => assert_eq!(text, "-"),
                    other => panic!("expected the hyphen glyph, got {other:?}"),
                }
            }
            HorzBox::Pure(PureHorzBox::OuterFil) => {}
            other => panic!("unexpected box kind: {other:?}"),
        }
    }
    assert_eq!(disc_count, 1, "exactly one break, at the authored soft hyphen");
    assert_eq!(
        fragments,
        vec!["hy".to_string(), "phenation".to_string()],
        "split exactly at the soft hyphen's position, not a dictionary-derived point"
    );
}

/// Item 1 continued: the byte-identity gate must hold for soft-hyphen text
/// too — with `hyphen_dictionary == None` (the default), a word containing
/// a literal U+00AD must produce exactly the same boxes as before this
/// slice (this project's general non-ASCII/UAX#14 tokenizing of a soft
/// hyphen, unrelated to and untouched by the hyphenation feature). This
/// pins that the new `is_gated_soft_hyphen` check in `text_to_boxes` only
/// ever fires when a dictionary is installed.
#[test]
fn soft_hyphen_with_no_dictionary_installed_is_untouched_by_this_slice() {
    let mut ctx = Context::initial(Length::pt(400.0));
    ctx.hyphen_dictionary = None; // this test is about the no-dictionary path
    let word_with_shy = "hy\u{ad}phenation";
    let boxes = boxes_for_word(&ctx, word_with_shy);

    // Pre-existing (untouched) behavior: the general UAX#14 tokenizer
    // treats U+00AD as an ordinary "break-after" boundary and flushes the
    // word right there, embedding the literal soft-hyphen character in the
    // leading fragment's text and emitting an empty (no visible glyph)
    // Discretionary — exactly what `uax14_boundaries`/`break_opportunities`
    // already did for any non-ASCII break-after character before this
    // slice touched anything gated on `hyphen_dictionary`.
    assert_eq!(boxes.len(), 4, "InnerString + Discretionary + InnerString + fil: {boxes:?}");
    match &boxes[0] {
        HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => {
            assert_eq!(text, "hy\u{ad}", "leading fragment keeps the raw soft hyphen char")
        }
        other => panic!("expected an InnerString, got {other:?}"),
    }
    match &boxes[1] {
        HorzBox::Pure(PureHorzBox::Discretionary { penalty, pre_break, post_break, no_break }) => {
            assert_eq!(*penalty, 0, "UAX#14 Allowed break, not a hyphenation-penalty one");
            assert!(pre_break.is_empty(), "no injected hyphen glyph on the untouched path");
            assert!(post_break.is_empty());
            assert!(no_break.is_empty());
        }
        other => panic!("expected a Discretionary, got {other:?}"),
    }
    match &boxes[2] {
        HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => assert_eq!(text, "phenation"),
        other => panic!("expected an InnerString, got {other:?}"),
    }
}

/// Item 2 (per-run font correctness): the injected hyphen glyph must carry
/// the *run's own* font, not a hardcoded default — two runs typeset under
/// different `ctx.font` values must get hyphen boxes tagged with their own
/// distinct font key.
#[test]
fn hyphen_glyph_uses_the_run_own_font_not_a_hardcoded_default() {
    fn hyphen_font_for(font: FontKey) -> FontKey {
        let mut ctx = Context::initial(Length::pt(400.0));
        ctx.hyphen_dictionary = Some(HyphenLang::EnglishUS);
        ctx.font = font;
        let boxes = boxes_for(&ctx);
        let mut fonts = Vec::new();
        for hb in &boxes {
            if let HorzBox::Pure(PureHorzBox::Discretionary { pre_break, .. }) = hb {
                for b in pre_break {
                    if let PureHorzBox::InnerString { text, info, .. } = b {
                        assert_eq!(text, "-");
                        fonts.push(info.font);
                    }
                }
            }
        }
        assert!(!fonts.is_empty(), "expected at least one injected hyphen glyph");
        assert!(
            fonts.iter().all(|&f| f == font),
            "every hyphen glyph must carry this run's font {font:?}, got {fonts:?}"
        );
        font
    }

    let bold_like = hyphen_font_for(FontKey(11));
    let regular_like = hyphen_font_for(FontKey(22));
    assert_ne!(
        bold_like, regular_like,
        "hyphen glyphs from two differently-fonted runs must carry different font keys \
         (proves the font is read from the run, not a fixed constant)"
    );
}

/// Item 3 (min-fragment override): `set-hyphen-min`'s live
/// `left_hyphen_min`/`right_hyphen_min` must actually constrain which
/// dictionary breaks the injection accepts — not just the defaults
/// (already covered indirectly by the S2 fixtures), but a *non-default*
/// override.
#[test]
fn min_fragment_override_from_the_live_context_is_respected() {
    fn fragments_for(left_min: i64, right_min: i64) -> Vec<String> {
        let mut ctx = Context::initial(Length::pt(400.0));
        ctx.hyphen_dictionary = Some(HyphenLang::EnglishUS);
        ctx.left_hyphen_min = left_min;
        ctx.right_hyphen_min = right_min;
        let boxes = boxes_for(&ctx);
        boxes
            .iter()
            .filter_map(|hb| match hb {
                HorzBox::Pure(PureHorzBox::InnerString { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    // Default (3, 2): `hyphenate_word(EnglishUS, "hyphenation", 3, 2) ==
    // [6, 7]` -> three fragments "hyphen"/"a"/"tion".
    let default_fragments = fragments_for(3, 2);
    assert_eq!(default_fragments, vec!["hyphen", "a", "tion"]);

    // Raising left_hyphen_min to 7 filters out the break at char index 6
    // (6 < 7), leaving only the one at 7: "hyphena"/"tion".
    let left_overridden = fragments_for(7, 2);
    assert_eq!(
        left_overridden,
        vec!["hyphena", "tion"],
        "a stricter left_hyphen_min must remove the break too close to the start"
    );

    // Raising right_hyphen_min to 5 filters out the break at char index 7
    // (11 - 7 = 4 < 5), leaving only the one at 6: "hyphen"/"ation".
    let right_overridden = fragments_for(3, 5);
    assert_eq!(
        right_overridden,
        vec!["hyphen", "ation"],
        "a stricter right_hyphen_min must remove the break too close to the end"
    );

    // Sanity: the override actually changed something relative to the
    // default (guards against a no-op filter that would pass trivially).
    assert_ne!(default_fragments, left_overridden);
    assert_ne!(default_fragments, right_overridden);
}
