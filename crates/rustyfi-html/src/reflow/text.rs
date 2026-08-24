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

/// The glue rule this module's doc comment argues for lives in
/// [`crate::recover`], because `--format markdown` needs exactly the same
/// answer and a second copy of it would rot the next time either backend
/// corrects it. The reasoning stays here, where it is about HTML; the
/// decision is shared.
pub(crate) use crate::recover::{is_cjk, wants_space};

/// Does a rendered fragment hold anything a reader would see?
///
/// "Empty" has to mean "nothing but spacing struts", not "the empty
/// string", because a `<span class="hskip">` carries no ink and two very
/// different constructs produce fragments made of nothing else:
///
/// - the PHANTOM table. `easytable` builds every table TWICE
///   (`table-builder.satyh`'s `build`): once with the real cell text and no
///   rules, and once as the same grid of EMPTY cells carrying only the rule
///   callbacks, drawing both into one `inline-graphics`. The carrier is
///   invisible in the PDF — it is nothing but the lines — but rendered
///   literally it became an empty bordered grid above every real table in
///   the manual, forty of them. Dropping it loses nothing, since this
///   backend draws table rules from its own stylesheet rather than from
///   callbacks it cannot run;
/// - the leftover gap after a list bullet. `itemize`'s bullet is fenced by
///   `inline-mark` so the reflow walker can drop it in favour of the real
///   `<ul>` marker, but the `inline-skip` that separated it from the text
///   is outside the fence, and it was left stranded in a paragraph of its
///   own above every list item's content.
pub(crate) fn has_visible_content(html: &str) -> bool {
    let mut rest = html;
    loop {
        rest = rest.trim_start();
        let Some(after) = rest.strip_prefix("<span class=\"hskip\"") else {
            return !rest.is_empty();
        };
        match after.find("></span>") {
            Some(end) => rest = &after[end + "></span>".len()..],
            None => return true,
        }
    }
}

/// The document's body text style: the `(font, size)` pair the most
/// characters are set in. `css.rs` writes it onto `body`, and `inline.rs`
/// omits from each run's `<span>` every property that matches it — omitting
/// the `<span>` entirely when nothing is left to say.
#[derive(Clone, Debug, PartialEq)]
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
    /// `ImageId` -> how many times the flow places it. An image placed once
    /// stays a real `<img>`; one placed repeatedly has its bytes emitted
    /// ONCE as a CSS rule instead (see `inline.rs`'s `Image` arm), because a
    /// data URI repeated per placement is not a rounding error: `figbox`'s
    /// manual places two figures seventeen times between them and came out
    /// as a 13 MB file, 1.9 MB of which was the images.
    pub(crate) image_uses: HashMap<usize, usize>,
}

impl Default for BodyStyle {
    fn default() -> Self {
        BodyStyle {
            font: None,
            size: 12.0,
            cjk_ratio: 0.0,
            image_uses: HashMap::new(),
        }
    }
}

impl BodyStyle {
    /// Walk the whole flow once — nested blocks, frames, table cells,
    /// footnote bodies, `draw-text` runs — counting characters per
    /// `(font, size)`, CJK characters, and image placements. Sizes are
    /// bucketed at 0.01pt so two runs that differ only by float noise do not
    /// split the vote.
    ///
    /// Falls back to a 12pt unstyled default for a document with no text at
    /// all (a pure-graphics slide deck), which keeps `size` non-zero for the
    /// `em`-ratio division without a special case at the use site.
    pub fn dominant(source: Option<&[VertBox]>) -> BodyStyle {
        let mut t = Tally::default();
        if let Some(vboxes) = source {
            t.vboxes(vboxes);
        }
        let best = t.styles.iter().max_by_key(|(&(font, size), &n)| {
            // Ties broken deterministically (a `HashMap` iteration order is
            // not stable across runs, and this feeds the emitted CSS).
            (n, std::cmp::Reverse(size), std::cmp::Reverse(font))
        });
        let (font, size) = match best {
            Some((&(font, size_hundredths), _)) => {
                (Some(FontKey(font)), size_hundredths as f64 / 100.0)
            }
            None => (None, 12.0),
        };
        BodyStyle {
            font,
            size,
            cjk_ratio: if t.total == 0 {
                0.0
            } else {
                t.cjk as f64 / t.total as f64
            },
            image_uses: t.images,
        }
    }

    /// `true` when a run set in `(font, size)` needs no `<span>` of its own.
    pub(crate) fn matches(&self, font: FontKey, size: f64) -> bool {
        self.font == Some(font) && (size - self.size).abs() < 0.005
    }
}

/// The accumulator behind [`BodyStyle::dominant`], gathered in one pass so
/// three unrelated document-wide facts do not cost three walks.
#[derive(Default)]
struct Tally {
    /// `(font key, size in hundredths of a point) -> characters`.
    styles: HashMap<(u16, i64), usize>,
    cjk: usize,
    total: usize,
    /// `ImageId -> placements`.
    images: HashMap<usize, usize>,
}

impl Tally {
    fn vboxes(&mut self, vboxes: &[VertBox]) {
        for vb in vboxes {
            if let VertBox::Line { contents, .. } = vb {
                for (_, bx) in contents {
                    self.hbox(bx);
                }
            }
        }
    }

    fn hbox(&mut self, bx: &PureHorzBox) {
        match bx {
            PureHorzBox::InnerString { info, text, .. } => {
                let n = text.chars().count();
                if n == 0 {
                    return;
                }
                *self
                    .styles
                    .entry((info.font.0, (info.size.0 * 100.0).round() as i64))
                    .or_default() += n;
                self.total += n;
                self.cjk += text.chars().filter(|c| is_cjk(*c)).count();
            }
            PureHorzBox::Image { image, .. } => *self.images.entry(image.0).or_default() += 1,
            PureHorzBox::Frame { contents, .. } => {
                for (_, c) in contents {
                    self.hbox(c);
                }
            }
            PureHorzBox::EmbeddedBlock { block, .. } | PureHorzBox::Footnote { block } => {
                self.vboxes(block)
            }
            PureHorzBox::Tabular(tab) => self.tabular(tab),
            PureHorzBox::Graphics { elems, .. } => self.elems(elems),
            PureHorzBox::Discretionary { no_break, .. } => {
                for c in no_break {
                    self.hbox(c);
                }
            }
            // Math glyphs are drawn into an `<svg>` at their own absolute
            // sizes and never inherit the body style, so they get no vote.
            // Every remaining variant carries no text and places no image.
            _ => {}
        }
    }

    fn tabular(&mut self, tab: &TabularBox) {
        for cell in &tab.cells {
            for (_, bx) in &cell.contents {
                self.hbox(bx);
            }
        }
    }

    fn elems(&mut self, elems: &[GraphicsElem]) {
        for e in elems {
            match e {
                GraphicsElem::Text { contents, .. } => {
                    for (_, bx) in contents {
                        self.hbox(bx);
                    }
                }
                GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => self.elems(inner),
                _ => {}
            }
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
