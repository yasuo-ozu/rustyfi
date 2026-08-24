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

/// The tag `--katex` wraps an equation in. One constant, because two places
/// have to agree about it exactly: [`inline::emit_math_svg`] writes it and
/// [`sole_math_tex`] reads it back.
///
/// [`inline::emit_math_svg`]: crate::reflow::inline
pub(crate) const MATH_TEX_OPEN: &str = "<span class=\"math-tex\">\\(";
/// The closing half of [`MATH_TEX_OPEN`].
pub(crate) const MATH_TEX_CLOSE: &str = "\\)</span>";

/// The LaTeX of a flushed paragraph that holds `--katex` equations and
/// nothing else — i.e. an equation that was DISPLAYED — or `None`.
///
/// **Why this reads the buffer back instead of being decided where the box
/// was walked.** Whether an equation is inline or displayed is a property of
/// the paragraph, not of the equation: nothing in the box stream says
/// "display style", and what makes one displayed is that its `line-break`
/// holds nothing else. The inline emitters take `&mut String`, not `&mut
/// Para`, so at the point the math box is seen there is no paragraph to ask —
/// and the answer is not known yet anyway, since the rest of the line has not
/// arrived. `block.rs`'s flush is the first place the whole paragraph exists.
///
/// **Why sniffing the string is safe here.** The markup being matched is this
/// backend's OWN, written a few lines away from a shared constant, and its
/// body is `escape_html`'d LaTeX — which by construction contains no `<`, no
/// `&` and therefore no nested element. So "the trimmed paragraph is exactly
/// one `math-tex` span" is decidable by looking at its two ends. It is the
/// same layer, and the same kind of reasoning, as [`has_visible_content`]
/// just above.
///
/// The distinction is worth the trouble: `\(…\)` and `\[…\]` are not two
/// spellings of one thing. In inline style KaTeX sets `\sum`'s limits BESIDE
/// the operator and shrinks the operator itself; in display style it sets
/// them above and below at full size. Getting it wrong turns every displayed
/// equation in a document into a cramped inline one.
///
/// **Several spans still make ONE displayed equation.** A formula is not one
/// box: `latexcmds`' Schrödinger equation reaches this backend as four,
/// because each `\underset`-style construction splits the run. They are pieces
/// of one equation, so their bodies are joined into a single `\[…\]` — four
/// separate display blocks would be four centred lines where the document has
/// one.
pub(crate) fn sole_math_tex(html: &str) -> Option<String> {
    let mut rest = html.trim();
    let mut parts: Vec<&str> = Vec::new();
    while !rest.is_empty() {
        rest = strip_hskip(rest);
        if rest.is_empty() {
            break;
        }
        let after_open = rest.strip_prefix(MATH_TEX_OPEN)?;
        let end = after_open.find(MATH_TEX_CLOSE)?;
        let inner = &after_open[..end];
        // The body is `escape_html`'d LaTeX, so it can hold no element of its
        // own; a `<` here means the shape is not what it looks like.
        if inner.contains('<') {
            return None;
        }
        parts.push(inner);
        rest = after_open[end + MATH_TEX_CLOSE.len()..].trim_start();
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// [`sole_math_tex`]'s question for `--mathml`: a flushed paragraph that holds
/// MathML equations and nothing else, as the CHILDREN of the one
/// `display="block"` element they should become, plus the combined verdict on
/// their ink.
///
/// **Why the shape can be decided by scanning, when the body is full of
/// markup.** [`sole_math_tex`] can rely on its body containing no `<` at all;
/// this one cannot, so it matches the two ENDS instead. That is sound for a
/// narrower reason, and the reason is structural rather than lucky: this
/// backend never nests a `<math>` inside another one — `crate::mathml` writes
/// no `<annotation-xml>` and no `<semantics>`, the only Core elements that
/// could contain one — so the first `</math>` after an opening `<math ` is
/// unambiguously that element's end. Everything between them is by
/// construction one element's children and is copied through untouched.
///
/// The `approx` verdict is read back OFF THE OPEN TAG rather than being
/// threaded through the buffer, because by this point the paragraph is a
/// string and the boxes are gone. `crate::mathml::APPROX_CLASS` is the one
/// name both sides use.
pub(crate) fn sole_math_ml(html: &str) -> Option<(String, crate::mathml::Approx)> {
    const OPEN: &str = "<math ";
    let mut rest = html.trim();
    let mut parts: Vec<&str> = Vec::new();
    let mut approx = crate::mathml::Approx::Exact;
    while !rest.is_empty() {
        rest = strip_hskip(rest);
        if rest.is_empty() {
            break;
        }
        let after_open = rest.strip_prefix(OPEN)?;
        // An attribute value here is written by `crate::mathml::open_tag` and
        // holds no `>`, so the first one ends the open tag.
        let tag_end = after_open.find('>')?;
        if after_open[..tag_end].contains(crate::mathml::APPROX_CLASS) {
            approx = crate::mathml::Approx::Approx;
        }
        let body = &after_open[tag_end + 1..];
        let end = body.find(crate::mathml::CLOSE_TAG)?;
        parts.push(&body[..end]);
        rest = body[end + crate::mathml::CLOSE_TAG.len()..].trim_start();
    }
    (!parts.is_empty()).then(|| (parts.concat(), approx))
}

/// Drop any leading spacing struts, and the whitespace after them.
///
/// **Without this the display upgrade essentially never fires**, which is how
/// it shipped: a DISPLAYED equation is centred, and `\align-center` is a pair
/// of `inline-fil`s that reach this backend as `<span class="hskip">`. So the
/// paragraph does not START with the math span, `strip_prefix` fails, and
/// every displayed equation in the document is written with inline
/// delimiters — the exact failure [`sole_math_tex`]'s own doc comment warns
/// about. Measured across ten corpus documents: `\[` fired once.
///
/// The same strut is what [`has_visible_content`] skips, and for the same
/// reason: it carries no ink, so it cannot be evidence that the paragraph
/// holds anything besides the equation.
fn strip_hskip(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        let Some(after) = s.strip_prefix("<span class=\"hskip\"") else {
            return s;
        };
        match after.find("></span>") {
            Some(end) => s = &after[end + "></span>".len()..],
            None => return s,
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
