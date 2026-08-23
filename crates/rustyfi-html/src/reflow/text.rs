//! The two things that decide whether this backend's output READS as prose:
//! what a glue box becomes, and which runs need a `<span>` at all.
//!
//! ## Why glue cannot just become a space
//!
//! The pre-page-break box stream is not a word stream. `convertText.ml`'s
//! port (`rustyfi-lang/src/primitives.rs`) splits every text run at each
//! UAX#14 chunk boundary and inserts a glue box between the pieces — a real
//! inter-word space, a zero-width break opportunity inside a word
//! (`Con|trib|u|tors`), a negative kern (the `\LaTeX` logo's four), and —
//! the one that matters most — the `adjacent_space`/JLreq class glue the
//! CJK work inserted between EVERY pair of Japanese characters, so that a
//! Japanese line has something to justify with.
//!
//! Rendering all of those as U+0020, which is what this module replaced,
//! turned every Japanese paragraph into `研 究 計 画` — one `<span>` per
//! character with a space between, which is neither readable, selectable,
//! nor line-breakable by the browser, and turned `\LaTeX` into `L AT EX`.
//!
//! [`wants_space`] decides instead from the two characters the glue sits
//! between plus its natural width, which is all the information the box
//! stream actually carries:
//!
//! - natural width ~0 → nothing. A zero-width glue is a break OPPORTUNITY,
//!   never spacing: inside a word (`Con|trib`), or between two CJK
//!   characters (`adjacent_space` is natural-0/stretch-only). Browsers
//!   already break between CJK characters on their own, and must NOT be
//!   allowed to break inside a Latin word, so emitting nothing is right in
//!   both directions.
//! - both neighbours CJK → nothing, whatever the width. The nonzero cases
//!   here are JLreq class spaces (the half-em after `。`, before `「`), and
//!   in HTML the full-width glyph already carries that side bearing; the
//!   elastic part is a justification device the browser's own
//!   `text-align: justify` supplies.
//! - otherwise → one space. Real inter-word spaces, and the inter-SCRIPT
//!   glue at a Japanese/Latin boundary, where a space is what HTML wants.
//!
//! ## Why most runs should carry no `<span>` at all
//!
//! [`BodyStyle::dominant`] counts characters per `(font, size)` pair over
//! the whole flow and names the winner the document's body style. `css.rs`
//! puts that pair on `body`, so the ~90% of runs that are body text can be
//! written as bare escaped text with no element around them — the output
//! stops being a wall of markup and starts being prose (13 592 `<span>`s in
//! the `enumitem` manual before, a few hundred after). Runs that DIFFER
//! still get a `<span>`, and only for the properties that actually differ,
//! with the size as a `em` RATIO of the body size so the whole document
//! scales with one number instead of freezing every run at an absolute
//! point size.

use std::collections::HashMap;

use rustyfi_backend::{FontKey, GraphicsElem, PureHorzBox, TabularBox, VertBox};

/// Anything below this (in pt) counts as a zero-width glue — a break
/// opportunity or a kern, never spacing. Deliberately a hair above 0 rather
/// than an exact comparison: the CJK pair glue's natural part is computed
/// through several `f64` multiplications, and a 0.01pt residue is not a
/// space.
const GLUE_EPSILON_PT: f64 = 0.05;

/// Is `c` set solid, with no inter-character space — Han, kana, Hangul, the
/// CJK punctuation block and the full-width forms? Used only by
/// [`wants_space`]; a false negative costs one spurious space and a false
/// positive one missing one, so the ranges are the broad blocks rather than
/// a Unicode script database.
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF     // Hangul Jamo
        | 0x2E80..=0x2EFF   // CJK radicals supplement
        | 0x3000..=0x303F   // CJK symbols and punctuation (incl. U+3000)
        | 0x3040..=0x30FF   // Hiragana + Katakana
        | 0x3100..=0x312F   // Bopomofo
        | 0x3190..=0x319F   // Kanbun
        | 0x31F0..=0x31FF   // Katakana phonetic extensions
        | 0x3400..=0x4DBF   // CJK ext A
        | 0x4E00..=0x9FFF   // CJK unified ideographs
        | 0xAC00..=0xD7AF   // Hangul syllables
        | 0xF900..=0xFAFF   // CJK compatibility ideographs
        | 0xFF00..=0xFF60   // full-width forms
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FA1F // CJK ext B..F + compatibility supplement
    )
}

/// Does a glue box of natural width `natural_pt`, sitting between `prev` and
/// `next`, become a space? See this module's doc comment for the reasoning
/// behind each clause. `prev`/`next` are `None` at a paragraph edge and
/// either side of an opaque box (an `<svg>`, an `<img>`, a `<table>`), which
/// count as non-CJK: a formula or figure set into Japanese prose takes the
/// same space its inter-script glue was asking for.
pub(crate) fn wants_space(prev: Option<char>, next: Option<char>, natural_pt: f64) -> bool {
    if natural_pt <= GLUE_EPSILON_PT {
        return false;
    }
    // Nothing to separate FROM: a leading space is trimmed by `flush_para`
    // anyway, and emitting one here would defeat that trim inside a `<td>`.
    let Some(p) = prev else { return false };
    !(is_cjk(p) && next.is_some_and(is_cjk))
}

/// The document's body text style: the `(font, size)` pair the most
/// characters are set in. `css.rs` writes it onto `body`, and `inline.rs`
/// omits from each run's `<span>` every property that matches it — omitting
/// the `<span>` entirely when nothing is left to say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyStyle {
    pub(crate) font: Option<FontKey>,
    /// Points. Never zero (see [`BodyStyle::dominant`]'s fallback), so it is
    /// always safe to divide a run's size by it for the `em` ratio.
    pub(crate) size: f64,
    /// Fraction of the document's characters that are CJK — `css.rs`/
    /// `mod.rs` turn this into the root `lang` attribute, which is what
    /// makes `hyphens: auto` do anything at all (a browser hyphenates only
    /// a language it has patterns for, and will not guess one).
    pub(crate) cjk_ratio: f64,
}

impl Default for BodyStyle {
    fn default() -> Self {
        BodyStyle {
            font: None,
            size: 12.0,
            cjk_ratio: 0.0,
        }
    }
}

impl BodyStyle {
    /// Count characters per `(font, size)` over the whole flow — nested
    /// blocks, frames, table cells, footnote bodies, `draw-text` runs — and
    /// return the winner. Sizes are bucketed at 0.01pt so two runs that
    /// differ only by float noise do not split the vote.
    ///
    /// Falls back to a 12pt unstyled default for a document with no text at
    /// all (a pure-graphics slide deck), which keeps `size` non-zero for the
    /// `em`-ratio division without a special case at the use site.
    pub fn dominant(source: Option<&[VertBox]>) -> BodyStyle {
        let mut tally: HashMap<(u16, i64), usize> = HashMap::new();
        let mut cjk = 0usize;
        let mut total = 0usize;
        if let Some(vboxes) = source {
            tally_vboxes(vboxes, &mut tally, &mut cjk, &mut total);
        }
        let best = tally.into_iter().max_by_key(|&((font, size), n)| {
            // Ties broken deterministically (a `HashMap` iteration order is
            // not stable across runs, and this feeds the emitted CSS).
            (n, std::cmp::Reverse(size), std::cmp::Reverse(font))
        });
        match best {
            Some(((font, size_hundredths), _)) => BodyStyle {
                font: Some(FontKey(font)),
                size: size_hundredths as f64 / 100.0,
                cjk_ratio: if total == 0 {
                    0.0
                } else {
                    cjk as f64 / total as f64
                },
            },
            None => BodyStyle::default(),
        }
    }

    /// `true` when a run set in `(font, size)` needs no `<span>` of its own.
    pub(crate) fn matches(&self, font: FontKey, size: f64) -> bool {
        self.font == Some(font) && (size - self.size).abs() < 0.005
    }
}

fn tally_vboxes(
    vboxes: &[VertBox],
    tally: &mut HashMap<(u16, i64), usize>,
    cjk: &mut usize,
    total: &mut usize,
) {
    for vb in vboxes {
        if let VertBox::Line { contents, .. } = vb {
            for (_, bx) in contents {
                tally_hbox(bx, tally, cjk, total);
            }
        }
    }
}

fn tally_hbox(
    bx: &PureHorzBox,
    tally: &mut HashMap<(u16, i64), usize>,
    cjk: &mut usize,
    total: &mut usize,
) {
    match bx {
        PureHorzBox::InnerString { info, text, .. } => {
            let n = text.chars().count();
            if n == 0 {
                return;
            }
            *tally
                .entry((info.font.0, (info.size.0 * 100.0).round() as i64))
                .or_default() += n;
            *total += n;
            *cjk += text.chars().filter(|c| is_cjk(*c)).count();
        }
        PureHorzBox::Frame { contents, .. } => {
            for (_, c) in contents {
                tally_hbox(c, tally, cjk, total);
            }
        }
        PureHorzBox::EmbeddedBlock { block, .. } | PureHorzBox::Footnote { block } => {
            tally_vboxes(block, tally, cjk, total)
        }
        PureHorzBox::Tabular(tab) => tally_tabular(tab, tally, cjk, total),
        PureHorzBox::Graphics { elems, .. } => tally_elems(elems, tally, cjk, total),
        PureHorzBox::Discretionary { no_break, .. } => {
            for c in no_break {
                tally_hbox(c, tally, cjk, total);
            }
        }
        // Math glyphs are drawn into an `<svg>` at their own absolute sizes
        // and never inherit the body style, so they get no vote. Every
        // remaining variant carries no text.
        _ => {}
    }
}

fn tally_tabular(
    tab: &TabularBox,
    tally: &mut HashMap<(u16, i64), usize>,
    cjk: &mut usize,
    total: &mut usize,
) {
    for cell in &tab.cells {
        for (_, bx) in &cell.contents {
            tally_hbox(bx, tally, cjk, total);
        }
    }
}

fn tally_elems(
    elems: &[GraphicsElem],
    tally: &mut HashMap<(u16, i64), usize>,
    cjk: &mut usize,
    total: &mut usize,
) {
    for e in elems {
        match e {
            GraphicsElem::Text { contents, .. } => {
                for (_, bx) in contents {
                    tally_hbox(bx, tally, cjk, total);
                }
            }
            GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => {
                tally_elems(inner, tally, cjk, total)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_width_glue_is_never_a_space() {
        // `Con|trib`: a UAX#14 chunk boundary inside a word.
        assert!(!wants_space(Some('n'), Some('t'), 0.0));
        // `研|究`: `adjacent_space`, natural 0 with stretch only.
        assert!(!wants_space(Some('研'), Some('究'), 0.0));
    }

    #[test]
    fn cjk_pair_never_takes_a_space_even_when_the_glue_is_wide() {
        // A JLreq class space (half an em after `。`) is side bearing the
        // full-width glyph already carries in HTML.
        assert!(!wants_space(Some('。'), Some('研'), 5.28));
    }

    #[test]
    fn a_script_boundary_and_a_word_space_both_take_one() {
        assert!(wants_space(Some('を'), Some('L'), 2.64));
        assert!(wants_space(Some('X'), Some('を'), 2.64));
        assert!(wants_space(Some('o'), Some('w'), 3.5));
    }

    #[test]
    fn a_negative_kern_is_not_a_space() {
        // The `\LaTeX` logo's four kerns, which used to render as `L AT EX`.
        assert!(!wants_space(Some('L'), Some('A'), -1.5));
    }

    #[test]
    fn a_paragraph_edge_emits_nothing() {
        assert!(!wants_space(None, Some('a'), 3.5));
    }

    #[test]
    fn cjk_ranges_cover_kana_han_and_fullwidth_but_not_latin() {
        for c in ['研', 'ひ', 'カ', '。', '　', 'Ａ', '한'] {
            assert!(is_cjk(c), "{c} should be CJK");
        }
        for c in ['a', 'Z', '1', '.', ' ', 'é', 'α'] {
            assert!(!is_cjk(c), "{c} should not be CJK");
        }
    }
}
