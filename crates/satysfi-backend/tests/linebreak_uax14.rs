//! UAX#14 segmenter (`break_opportunities`) and the discretionary-break DP
//! it feeds (docs/plans/text-rendering.md §3 + its step-0 upgrade).

use satysfi_backend::*;

#[test]
fn latin_breaks_only_at_the_space() {
    let breaks = break_opportunities("hello world");
    let allowed: Vec<usize> = breaks
        .iter()
        .filter(|(_, k)| *k == BreakKind::Allowed)
        .map(|(i, _)| *i)
        .collect();
    // The only optional break is right after the space (byte 6, start of
    // "world"); the remaining entry is the always-present end-of-text
    // marker (byte 11), which is Mandatory but not content-driven.
    assert_eq!(allowed, vec![6]);
}

#[test]
fn no_break_inside_a_word() {
    let breaks = break_opportunities("hello");
    // Nothing but the always-present end-of-text marker: no break
    // opportunity anywhere inside the word itself.
    assert_eq!(breaks, vec![(5, BreakKind::Mandatory)]);
}

#[test]
fn cjk_breaks_between_every_ideograph() {
    let text = "日本語版";
    let breaks = break_opportunities(text);
    let offsets: Vec<usize> = breaks.iter().map(|(i, _)| *i).collect();
    // Each ideograph is 3 UTF-8 bytes; a break opportunity between every
    // pair of the 4 characters, plus the end-of-text marker.
    assert_eq!(offsets, vec![3, 6, 9, 12]);
    assert!(breaks[..3].iter().all(|(_, k)| *k == BreakKind::Allowed));
}

#[test]
fn no_break_before_close_punctuation() {
    let breaks = break_opportunities("a)");
    // No break opportunity between 'a' and ')'; only the end-of-text
    // marker survives.
    assert_eq!(breaks, vec![(2, BreakKind::Mandatory)]);
}

#[test]
fn mandatory_break_at_newline() {
    let breaks = break_opportunities("a\nb");
    // A real, content-driven mandatory break right after the newline...
    assert!(breaks.contains(&(2, BreakKind::Mandatory)));
    // ...distinct from the always-present end-of-text marker at the end.
    assert!(breaks.contains(&(3, BreakKind::Mandatory)));
}

// ---- DP over discretionaries ----------------------------------------------

/// Every glyph is half an em wide, matching `tests/linebreak.rs`'s `Mono`.
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

fn ideograph(m: &Mono, c: &Context, ch: char) -> HorzBox {
    let text = ch.to_string();
    HorzBox::Pure(PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: c.font,
            size: c.font_size,
            rising: Length::ZERO,
        },
        width: m.text_width(c.font, &text, c.font_size).unwrap(),
        height: m.ascender(c.font, c.font_size),
        depth: m.descender(c.font, c.font_size),
        text,
    })
}

fn discretionary() -> HorzBox {
    HorzBox::Pure(PureHorzBox::Discretionary {
        penalty: 0,
        pre_break: Vec::new(),
        post_break: Vec::new(),
        no_break: Vec::new(),
    })
}

fn lines_of(v: &[VertBox]) -> Vec<String> {
    v.iter()
        .map(|vb| match vb {
            VertBox::Line { contents, .. } => contents
                .iter()
                .filter_map(|(_, b)| match b {
                    PureHorzBox::InnerString { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
            VertBox::Skip(_) => "<skip>".into(),
            VertBox::ClearPage => "<clear-page>".into(),
            VertBox::HookPageBreak(_) => "<hook>".into(),
            VertBox::FrameStart(_) => "<frame-start>".into(),
            VertBox::FrameEnd(_) => "<frame-end>".into(),
        })
        .collect()
}

/// CJK has no glue at all: a run of ideographs joined only by zero-width,
/// zero-penalty discretionaries (exactly what §3's `text_to_boxes` wiring
/// produces at UAX#14 `Allowed` boundaries) must still wrap, one ideograph
/// per line, once the measure is too narrow for two.
#[test]
fn narrow_measure_wraps_cjk_at_ideograph_discretionaries() {
    let mono = Mono;
    // Each ideograph is 6pt (12pt * 0.5) wide; 8pt fits exactly one but not
    // two (12pt, no stretch/shrink on a discretionary to absorb the rest).
    let ctx = Context::initial(Length::pt(8.0));
    let chars: Vec<char> = "日本語版".chars().collect();
    let mut boxes = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 {
            boxes.push(discretionary());
        }
        boxes.push(ideograph(&mono, &ctx, c));
    }

    let lines = break_into_lines(&ctx, boxes);
    assert_eq!(
        lines_of(&lines),
        vec!["日", "本", "語", "版"],
        "each ideograph must land on its own line via the discretionary path"
    );
}
