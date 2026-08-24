//! Drawing an equation: the face's own glyph outlines, with the characters
//! kept behind them as invisible, selectable text.
//!
//! **Shared by both backends, and that is the point.** `--format html` has
//! drawn math this way since the outline change; `--format markdown` now does
//! too, by default. Two copies of the geometry below would be two copies of
//! the y-flip, the units-per-em scale, the pen-offset accumulation, the
//! stretchy-delimiter dedup and the space-from-a-gap rule — each of which was
//! got wrong at least once before it was got right, and each of which is
//! invisible when it drifts (a glyph 4/3 too big still renders). So the
//! machinery is here and each backend supplies only its own `<svg>` wrapper,
//! which is the one thing they genuinely disagree about:
//!
//! - the HTML backend positions its `<svg>` ABSOLUTELY inside a
//!   `position:relative` wrapper `<span>` it also uses for the run's nested
//!   flow content, and lets its stylesheet make the phantom text invisible;
//! - Markdown has no ancestor to position against and no stylesheet at all,
//!   so [`math_block`] emits ONE self-contained, intrinsically-sized,
//!   single-line `<svg>` and writes `fill:none` inline.
//!
//! Everything between those two wrappers — [`emit_glyph_layer`] — is one
//! function producing one byte string.
//!
//! ## Why an outline and not `<text>`
//!
//! A `<text>` names a face and hopes. Where the reader does not have it, the
//! substitute's advances are not the ones the equation was laid out against,
//! and math is the one place in this crate where that is fatal rather than
//! untidy: every glyph is positioned ABSOLUTELY (`MathGlyph::dx`/`dy`), so
//! there is no flow to absorb the difference. Measured on
//! `\forall \epsilon \: \exists \delta` at 12pt in Latin Modern Math, the port
//! reserves 7.992pt for `∀` and a substituted face draws 12.000 — so `ε` lands
//! inside the quantifier. Drawing the outline also makes the two backends
//! agree with the PDF rather than approximately agree.
//!
//! `<text>` remains the FALLBACK, for a render with no font store (base-14
//! mode) and for a face that will not parse or has no outline for a glyph.
//!
//! ## Why the characters have to survive anyway
//!
//! A `<path>` is a shape: it cannot be selected, copied, found with the
//! browser's in-page search, or announced by a screen reader. Outlining
//! without [`Phantom`] would trade one fidelity bug for four accessibility
//! ones. The technique is the one PDF viewers use for a scanned page with an
//! OCR layer — paint the picture, and put the text behind it where the
//! machinery that reads text can still find it. That it WORKS is a fact about
//! browsers rather than about the standard, and is measured in a real one by
//! `crates/rustyfi/tests/html_math_selection.rs`.

use std::fmt::Write as _;

use rustyfi_backend::{Color, GraphicsElem, MathGlyph};
use rustyfi_pdf::TtfFontStore;

/// What [`math_glyph_outline`] resolves one `MathGlyph` to: the face's
/// `units_per_em` — the unit every `d` below is written in — and the run's
/// inked glyphs in pen order.
///
/// `parts` is a LIST rather than one path because a `MathGlyph`'s `text` is
/// not always a single character: `primitives::math_boxes_of_inline_boxes`
/// folds a whole `text-in-math` `InnerString` into one glyph record, so a
/// `\text{if and only if}` inside an equation arrives here as one `MathGlyph`
/// holding sixteen characters. Each entry's `f64` is that character's pen
/// offset from the record's own `dx`, in POINTS (the enclosing `<svg>`'s user
/// unit), so the caller adds it and needs no unit conversion of its own.
pub(crate) struct GlyphOutline {
    pub(crate) upem: f64,
    pub(crate) parts: Vec<(String, f64)>,
}

/// One math glyph's drawn form as the FACE'S OWN OUTLINES, resolved against
/// `fonts`: the `units_per_em` the `d` numbers are expressed in, and one
/// `(d, dx_pt)` per inked glyph — `dx_pt` an offset in POINTS from the
/// `MathGlyph`'s own pen position.
///
/// Two shapes, because `MathGlyph` has two:
///
/// - `gid: Some(_)` — a MATH-table variant (a display-size big operator, a
///   stretchy delimiter, one part of a `GlyphAssembly`, an `ssty` script
///   form). The id is drawn directly; no cmap is consulted, because that is
///   exactly the case where `text` does NOT cmap to the glyph the document
///   laid out.
/// - `gid: None` — an ordinary run, whose glyphs ARE the ones `text` cmaps
///   to. Each character is looked up through the face's cmap and the pen
///   advances by that glyph's own `hmtx` advance. That reproduces the port's
///   own measurement exactly: `measure_run` (`rustyfi-lang`) is purely
///   additive per character with no kerning or ligatures, and
///   `FontMetrics::advance` is this same `hmtx / units_per_em` ratio.
///
/// `None` — leaving the caller on its `<text>` path — in base-14 mode (no
/// store, so no face to ask), when the face will not parse, when a character
/// has no cmap entry or no `hmtx` advance (the port measured that one through
/// a fallback face this function cannot see, so advancing by anything here
/// would misplace the rest of the run), and when nothing in the run has an
/// outline at all (a lone space).
///
/// A character the face maps but draws blank — a space inside a
/// `text-in-math` run — contributes no `d` and still advances, so it leaves a
/// real gap rather than closing one up.
pub(crate) fn math_glyph_outline(
    fonts: Option<&TtfFontStore>,
    glyph: &MathGlyph,
) -> Option<GlyphOutline> {
    let face = fonts?.face(glyph.info.font)?;
    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 {
        return None;
    }
    if let Some(gid) = glyph.gid {
        let d = crate::svg::glyph_outline_d(&face, gid)?;
        return Some(GlyphOutline {
            upem,
            parts: vec![(d, 0.0)],
        });
    }
    let scale = glyph.info.size.0 / upem;
    let mut parts = Vec::new();
    let mut pen = 0.0;
    for c in glyph.text.chars() {
        let gid = face.glyph_index(c)?;
        if let Some(d) = crate::svg::glyph_outline_d(&face, gid.0) {
            parts.push((d, pen));
        }
        pen += f64::from(face.glyph_hor_advance(gid)?) * scale;
    }
    (!parts.is_empty()).then_some(GlyphOutline { upem, parts })
}

/// Resolve `font` to a CSS `font-family` VALUE — the real family name the font
/// file declares, followed by generic fallbacks
/// ([`crate::fonts::reflow_font_stack`]). `None` in base-14 mode, and for a
/// file whose `name` table declares no usable family, in which case the
/// caller's own stack (or the reader's default) applies.
///
/// This NAMES the face rather than embedding it — see
/// `fonts::reflow_font_stack` for the argument. A free function rather than a
/// method because both backends' `Ctx` need it and neither owns the other.
pub(crate) fn font_family_for(
    fonts: Option<&TtfFontStore>,
    font: rustyfi_backend::FontKey,
) -> Option<String> {
    let store = fonts?;
    let file_idx = store.file_index(font);
    let family = store.file_family_name(file_idx)?;
    Some(crate::fonts::reflow_font_stack(&family))
}

/// Whether `rules` draws anything at all — as opposed to being nothing but
/// `draw-text` positioning wrappers, whose contents are HTML and are emitted
/// by the caller outside the `<svg>` (see [`crate::svg`]'s module comment on
/// why an HTML child ejects the rest of a drawing from its `<svg>`).
pub(crate) fn rules_have_ink(rules: &[GraphicsElem]) -> bool {
    rules.iter().any(|e| match e {
        GraphicsElem::Text { .. } | GraphicsElem::Destination { .. } => false,
        GraphicsElem::Group(inner) | GraphicsElem::Clip(_, inner) => rules_have_ink(inner),
        _ => true,
    })
}

/// The glyph layer of a math `<svg>`: one `<path>` per inked glyph where the
/// face gives an outline, a `<text>` where it does not, and the whole run's
/// characters once more as a single invisible [`Phantom`] `<text>`.
///
/// `height` is the box's own height, and it is the ONLY geometry this needs:
/// `MathGlyph.dx`/`dy` are box-local y-**up** offsets from the box's baseline
/// (the same convention `GraphicsElem::Path` points use, confirmed by the PDF
/// writer's own `anchor_y + glyph.dy` arithmetic in its y-up space,
/// `rustyfi-pdf`'s `place_math`), so a local `(dx, dy)` lands at SVG-native
/// `(dx, height - dy)`. That is computed BY HAND here rather than with a `<g
/// transform>` flip, specifically so `<text>` glyphs are never inside a
/// `scale(1,-1)` group, which would render them MIRRORED upside-down (SVG text
/// has no orientation-independence the way a filled path does).
///
/// `phantom_fill_none` writes `fill:none` into the phantom `<text>`'s own
/// style instead of leaving it to a stylesheet. The HTML backend has one
/// (`css.rs`'s `.math-glyphs .mphantom`) and passes `false`; a Markdown file
/// has nowhere to put a rule, and without this the phantom characters would be
/// painted in black ON TOP of the outlines they stand behind — every equation
/// drawn twice, once as shapes and once as the reader's own substitute face.
pub(crate) fn emit_glyph_layer(
    out: &mut String,
    glyphs: &[MathGlyph],
    height: f64,
    fonts: Option<&TtfFontStore>,
    phantom_fill_none: bool,
) {
    let mut phantom = Phantom::default();
    for (i, g) in glyphs.iter().enumerate() {
        let x = g.dx.0;
        let y = height - g.dy.0 - g.info.rising.0;
        // Every glyph is drawn from the face's own outline where one can be
        // had, so the equation does not depend on the reader having the face
        // — see [`emit_math_glyph_path`] and [`math_glyph_outline`]. The
        // characters themselves survive as invisible, selectable text
        // ([`Phantom`]); without it a `<path>` would be uncopyable,
        // unsearchable and unreadable to a screen reader.
        if let Some(outline) = math_glyph_outline(fonts, g) {
            emit_math_glyph_path(out, &outline, g, x, y);
            if let Some(text) = phantom_text(glyphs, i) {
                phantom.push(text, g, x, y);
            }
            continue;
        }
        let mut style = format!("font-size:{};", math_font_size_uu(g.info.size.0));
        if let Some(stack) = font_family_for(fonts, g.info.font) {
            style.push_str(&format!("font-family:{stack};"));
        }
        if g.info.color != Color::Gray(0.0) {
            style.push_str(&format!("fill:{};", crate::svg::css_color(g.info.color)));
        }
        let _ = writeln!(
            out,
            "<text x=\"{x}\" y=\"{y}\" style=\"{style}\">{}</text>",
            crate::escape_html(&g.text),
        );
    }
    phantom.finish(out, phantom_fill_none);
}

/// One drawing of a whole math box as a SELF-CONTAINED `<svg>`, on a single
/// line, in normal flow — Markdown's counterpart to the HTML backend's
/// absolutely-positioned pair of `<svg>`s.
///
/// **Not [`crate::svg::emit_graphics`]-shaped, for the reason
/// [`crate::svg::graphics_block`] is not either**: that helper writes
/// `position:absolute` with a `left`/`top` computed against an ancestor the
/// reflow backend makes `position:relative`, and a Markdown file has no such
/// ancestor — the equation would be positioned against the page and land on
/// top of the text.
///
/// **The viewport is the BOX's, not the ink's**, which is the one place this
/// deliberately differs from `graphics_block`. A drawing is judged by what it
/// draws; an equation is judged by the space the document reserved for it, so
/// that `x` and `x²` sit on one baseline and a fraction's bar is where the
/// line above it expects. `overflow:visible` keeps the ink that exceeds the
/// box — math routinely does — from being clipped, exactly as the HTML
/// backend's own math `<svg>` does, and `vertical-align` drops the box by its
/// depth so an inline equation sits ON the text baseline rather than hanging
/// from it.
///
/// **Single-line by construction**, as `graphics_block` is: a Markdown
/// paragraph is one line, and a raw `<svg>` broken across lines would have
/// blank lines and `nl2br` inserted into the middle of it by the reader's own
/// parser.
///
/// `None` when the box draws nothing this can name — no glyphs and no inked
/// rules, which is what a `\paren`-style decoration built entirely out of
/// `draw-text` looks like. The caller has already emitted those contents as
/// text; an empty `<svg>` here would reserve the box's size a second time on
/// top of them.
pub(crate) fn math_block(
    width: f64,
    height: f64,
    depth: f64,
    glyphs: &[MathGlyph],
    rules: &[GraphicsElem],
    fonts: Option<&TtfFontStore>,
) -> Option<String> {
    if glyphs.is_empty() && !rules_have_ink(rules) {
        return None;
    }
    let total_h = height + depth;
    if !(width.is_finite() && total_h.is_finite()) || width <= 0.0 || total_h <= 0.0 {
        return None;
    }
    let mut out = String::new();
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"math\" role=\"img\" \
         width=\"{width}pt\" height=\"{total_h}pt\" viewBox=\"0 0 {width} {total_h}\" \
         style=\"overflow:visible; vertical-align:{}pt;\">",
        -depth,
    );
    // The fraction bars and radical signs, which are y-UP paths and so need
    // the flip the glyph layer does by hand. Emitted first so a glyph is never
    // painted under its own bar.
    if rules_have_ink(rules) {
        crate::svg::emit_flipped_group(&mut out, rules, 0.0, height);
    }
    emit_glyph_layer(&mut out, glyphs, height, fonts, true);
    out.push_str("</svg>");
    // `emit_glyph_layer` ends each element with a newline; a Markdown
    // paragraph is one line, so they are folded out rather than left to be
    // reflowed by whatever reads the file. Nothing else in the buffer can
    // contain one: a `MathGlyph`'s `text` is characters the math layout
    // placed, and the phantom layer writes no whitespace of its own.
    Some(out.replace('\n', ""))
}

/// One `MathGlyph`'s ink, as SVG `<path>`s of the face's own outlines —
/// placed at the same `(x, y)` the `<text>` branch would have used, which is
/// the glyph's ORIGIN (pen position), not its top-left.
///
/// **What this fixes, beyond font independence.** `MathGlyph::gid` is `Some`
/// exactly when the glyph the document laid out is not the one its `text`
/// cmaps to: an OpenType MATH `MathVariants` record — a display-size big
/// operator (`push_big_char_glyph`), a stretchy delimiter or one part of a
/// `GlyphAssembly` (`push_delimiter_glyph`) — or an `ssty` script form
/// (`push_char_glyph`). The PDF writer emits the id straight into the content
/// stream (`cid.rs`'s `encode_glyph_run`); an SVG `<text>` can only address
/// the CHARACTER, so a backend without this draws the base glyph and there is
/// no spelling of `∑` that would produce the display one.
///
/// **It was two symptoms of one bug.** The size was the visible half; the
/// misplacement was the consequence. Measured at 12pt in Latin Modern Math:
/// `∑` is `summation` (advance 1.056 em) and the display variant is
/// `summation.v1` (advance 1.444 em, ink 0.056..1.387 em).
/// `layout_math_list`'s `UpperLimit`/`LowerLimit` arms centre each limit on
/// the base's own width (`center_offsets`) — 17.328pt, the VARIANT's advance,
/// because the variant is what the document laid out — so `n` and `k = 1`
/// were both centred on x = 8.664, while a base-size `∑` has its ink centred
/// on x = 6.330. Every limit sat 2.334pt right of the operator it belonged
/// to. `∫` shows the same arithmetic without the centring: its scripts are
/// set to the RIGHT at the base's width, so the subscript began at x = 11.988
/// (again the variant's advance) with a 4.008pt gap after the 7.980pt base
/// glyph. Drawing the variant closes both, because every one of those offsets
/// was already right about a glyph that was not being drawn.
///
/// **Why the outline and not a scaled `<text>`**, the cheaper repair. Scaling
/// the base glyph by the advance ratio fixes the horizontal centring by
/// construction but not the ink: for `∑` the ratio is 1.367 against a true
/// height+depth ratio of 1.400 (2.5% short — fine), but for `∫` it is 1.502
/// against 2.000, leaving the integral 25% too short. The display forms are
/// separately drawn glyphs, not scalings of the base, and the two operators
/// disagree by enough that no single scale factor serves both.
///
/// **Geometry.** `d` is in design units, y-up (`svg::glyph_outline_d`); the
/// surrounding viewport is y-DOWN at 1 user unit = 1 pt. So the per-element
/// transform is the whole conversion: translate to the pen position, then
/// `scale(s, -s)` with `s = size / units_per_em` — the y-flip that
/// [`emit_glyph_layer`] deliberately does NOT apply to `<text>` (it would
/// mirror the letters) is exactly right for a filled path, which is
/// orientation-independent.
///
/// A record holding several characters emits one `<path>` per inked one, each
/// translated by the pen offset [`math_glyph_outline`] accumulated for it —
/// already in points, so it simply adds to `x`.
///
/// **No `fill-rule`**, unlike every other path this crate writes. Glyph
/// outlines are defined under NONZERO winding — SVG's default — and CFF faces
/// in particular use overlapping contours that even-odd would punch holes in.
/// `svg.rs`'s `Fill`/`Clip` arms say `evenodd` because they are reproducing
/// PDF's `f*`; this is reproducing a font.
fn emit_math_glyph_path(out: &mut String, outline: &GlyphOutline, g: &MathGlyph, x: f64, y: f64) {
    let s = g.info.size.0 / outline.upem;
    let mut attrs = String::new();
    if g.info.color != Color::Gray(0.0) {
        attrs.push_str(&format!(" fill=\"{}\"", crate::svg::css_color(g.info.color)));
    }
    for (d, pen) in &outline.parts {
        let _ = writeln!(
            out,
            "<path d=\"{d}\" transform=\"translate({} {y}) scale({s} {})\"{attrs}/>",
            x + pen,
            -s,
        );
    }
}

/// The characters `glyphs[i]` should contribute to the document's TEXT, or
/// `None` when it should contribute none.
///
/// Almost always the record's own `text`. The exception is a stretchy
/// delimiter grown from a `GlyphAssembly`: `push_delimiter_glyph` emits one
/// `MathGlyph` per PART — a top, some extenders, a bottom — and gives every
/// one of them the same `text` and the same `dx`, since they are stacked in a
/// single column. Copying that verbatim would put `(((((` in the clipboard
/// where the page shows one tall bracket. So a record whose `text` and `dx`
/// both repeat its predecessor's is a continuation part and stays silent;
/// the first part already carries the character.
///
/// Nothing else in the corpus produces two glyph records at an identical `dx`
/// with identical text — that would be one character painted on top of
/// another, which is a layout bug rather than a construction.
fn phantom_text(glyphs: &[MathGlyph], i: usize) -> Option<&str> {
    let g = &glyphs[i];
    if g.text.is_empty() {
        return None;
    }
    if let Some(prev) = i.checked_sub(1).map(|p| &glyphs[p]) {
        if prev.text == g.text && prev.dx == g.dx {
            return None;
        }
    }
    Some(&g.text)
}

/// The invisible, SELECTABLE text that rides with a run of outlined glyphs,
/// carrying the characters the `<path>`s beside them draw. See this module's
/// doc comment for why it is not a nicety.
///
/// **`fill: none`, and specifically NOT `visibility: hidden` or
/// `display: none`.** The latter two remove the element from the accessibility
/// tree and from the selection along with the paint, which is exactly the
/// thing being avoided; `fill: none` removes only the paint. Verified in
/// headless chromium rather than assumed — see
/// `crates/rustyfi/tests/html_math_selection.rs`, which drives a real browser
/// over a real render and includes both losing spellings as controls.
///
/// **It steals no hit-testing from the paths.** SVG's default
/// `pointer-events: visiblePainted` tests the FILL only where a fill is
/// actually painted, and none is — so `elementFromPoint` over an equation
/// returns the wrapper, not this. Selection is unaffected by that, because it
/// walks text nodes rather than hit-testing paint. It changes no layout
/// either: SVG text contributes nothing to the flow.
///
/// **ONE `<text>` per run, one `<tspan>` per glyph**, rather than a `<text>`
/// each. Chrome serialises a selection that spans several `<text>` elements
/// with a newline between every one, so a reader copying `∀ε : ∃δ` got each
/// character on its own line; `<tspan>`s inside a single `<text>` are inline
/// and copy as `∀𝜀:∃𝛿`. It is also where the wrapper's `class` and the run's
/// shared `font-size` are paid for once instead of per glyph.
///
/// No whitespace is written between the `<tspan>`s or inside the `<text>`:
/// under SVG's default `xml:space` a newline there collapses to a real space
/// and would show up in the copied text.
///
/// **Document order is reading order**, because [`emit_glyph_layer`]'s loop
/// walks `glyphs` in the order the math layout produced them — a base before
/// its scripts, a numerator before its denominator — and this preserves that
/// order.
///
/// The only property carried is `font-size`, and only where a glyph departs
/// from the run's first: it sizes the selection highlight the browser paints
/// over an invisible glyph. The FAMILY is deliberately not repeated — this
/// text is never drawn, so naming a face would buy nothing and cost ~110
/// bytes on every glyph in the document.
#[derive(Default)]
struct Phantom {
    /// The `<tspan>`s so far, concatenated with no separator but the
    /// occasional deliberate space (see [`Phantom::push`]).
    spans: String,
    /// The first glyph's size, hoisted onto the enclosing `<text>`.
    size: Option<f64>,
    /// The previous glyph's right edge (`dx + width`) and baseline `y`, which
    /// is what decides whether a space belongs between it and the next.
    prev: Option<(f64, f64)>,
}

/// Two phantom glyphs are on the SAME ROW when their baselines agree to
/// within this many points — a threshold rather than equality because the
/// baselines are arithmetic on `Length`s, not copies of one value.
const PHANTOM_ROW_EPS: f64 = 0.5;

/// A horizontal gap of at least this fraction of the font size becomes a
/// space in the copied text. A word space is 0.25–0.33 em in the faces this
/// port bundles and the widest math space (`\;`, 5/18 em) is 0.28, so this
/// takes both and leaves italic correction and the sub-0.1 em inter-atom
/// kerns alone.
const PHANTOM_SPACE_EM: f64 = 0.2;

impl Phantom {
    /// Add one glyph record's characters, at the pen position the `<path>`
    /// beside it uses.
    ///
    /// **Why a gap can become a space.** Nothing else can put one there:
    /// `primitives::math_boxes_of_inline_boxes` turns the glue inside a
    /// `text-in-math` body into bare ADVANCE and keeps no character for it,
    /// so `${x \text!{ if and only if } y}` reaches this layer as four
    /// glyph records reading `if`, `and`, `only`, `if` and nothing between
    /// them. Concatenating those verbatim copies as `ifandonlyif`. The gap is
    /// the only surviving evidence that a space was set, and reading it back
    /// is what a PDF text extractor does with the same absolutely-placed
    /// glyphs — `place_math` writes one `Tj` per glyph at its own point, and
    /// poppler reconstructs the spaces the same way.
    ///
    /// **Same row only, and only forwards.** A script or a big operator's
    /// limit sits on its own baseline and at an `x` that may run BACKWARDS
    /// relative to the glyph before it (`∑` at 0, its subscript at 0.46, its
    /// superscript back at 5.70), so a gap across rows means nothing about
    /// reading order and must not manufacture a space.
    fn push(&mut self, text: &str, g: &MathGlyph, x: f64, y: f64) {
        let size = g.info.size.0;
        if let Some((prev_right, prev_y)) = self.prev {
            if (prev_y - y).abs() < PHANTOM_ROW_EPS && x - prev_right >= size * PHANTOM_SPACE_EM {
                self.spans.push(' ');
            }
        }
        self.prev = Some((x + g.width.0, y));
        let attr = match self.size {
            None => {
                self.size = Some(size);
                String::new()
            }
            Some(run) if (run - size).abs() < 1e-9 => String::new(),
            Some(_) => format!(" style=\"font-size:{};\"", math_font_size_uu(size)),
        };
        let _ = write!(
            self.spans,
            "<tspan x=\"{x}\" y=\"{y}\"{attr}>{}</tspan>",
            crate::escape_html(text),
        );
    }

    /// Write the run's phantom layer, if it has one.
    ///
    /// `fill_none` prepends the invisibility to the element's own `style`
    /// instead of relying on a stylesheet — see [`emit_glyph_layer`]. It comes
    /// FIRST in the declaration list so that the `false` case produces exactly
    /// the byte string the HTML backend emitted before this was shared, which
    /// is what keeps `--format html` output identical.
    fn finish(self, out: &mut String, fill_none: bool) {
        let Some(size) = self.size else { return };
        let _ = writeln!(
            out,
            "<text class=\"mphantom\" style=\"{}font-size:{};\">{}</text>",
            if fill_none { "fill:none;" } else { "" },
            math_font_size_uu(size),
            self.spans,
        );
    }
}

/// A math glyph's `pt` font size, spelled for the inside of a math viewport —
/// i.e. in SVG USER UNITS, as a `px` length.
///
/// **The bug this exists to prevent.** A math `<svg>` is
/// `width="{w}pt" viewBox="0 0 {w} {h}"`, so one user unit renders as exactly
/// one `pt` — the deliberate "1 user unit = 1 pt" contract `svg.rs`'s module
/// comment states, and what makes `MathGlyph::dx`/`dy` and every `rules` path
/// coordinate emittable as a bare `Length` with no conversion. An ABSOLUTE
/// CSS length inside that viewport does NOT get the same treatment: `pt`
/// resolves against the CSS reference pixel *before* the viewBox transform
/// (SVG fixes 1px = 1 user unit for absolute-unit conversion), so
/// `font-size:12pt` becomes 16 user units and then renders at 16pt. Every
/// glyph came out 4/3 too big while its POSITION stayed right, so glyphs
/// overlapped each other, overflowed the fraction bars and radical overbars
/// (which, being `rules` paths, were correctly scaled), and — because the
/// wrapper reserves only the box's own `height`/`depth` while the `<svg>` is
/// `overflow:visible` — spilled ink into the lines above and below. The PDF
/// was never affected: it positions each glyph absolutely and sets the size in
/// the content stream's own points.
///
/// **Why `px` and not a bare number**, which is what "user units" literally
/// means. A unitless length is legal in SVG only as a PRESENTATION
/// ATTRIBUTE (`font-size="12"`); this size goes into `style="…"`, which is
/// CSS, and CSS requires a unit on a non-zero `<length>`. Measured in
/// chromium inside a `viewBox="0 0 100 100"`/`width="100pt"` viewport, four
/// spellings of "12" on the same `<text>`:
///
/// | written                     | computed | user units |
/// |-----------------------------|----------|-----------:|
/// | `style="font-size:12pt"`    | `16px`   |         16 |
/// | `style="font-size:12px"`    | `12px`   |         12 |
/// | `style="font-size:12"`      | `12px`   |         12 |
/// | `font-size="12"` (attribute)| `12px`   |         12 |
///
/// So the bare `style` spelling happens to work in Blink — Blink runs the
/// SVG presentation-attribute grammar over the declaration — but it is
/// invalid CSS and Gecko drops it, which would leave the glyph at the
/// INHERITED body size with no error anywhere. `px` is the portable
/// spelling of one user unit (SVG fixes 1px = 1 user unit), so the number
/// is unchanged and only the unit is corrected: 12pt of document size ->
/// `font-size:12px` -> 12 user units -> 12pt rendered.
///
/// Every other length inside this viewport is already unitless, because
/// every other one is an attribute rather than CSS: `x`/`y` in the glyph
/// layer, and `svg.rs`'s `d`, `stroke-width`, `stroke-dasharray` and
/// `stroke-dashoffset`. `font-size` is the only one that had to be a
/// declaration, which is why it was the only one that got this wrong.
pub(crate) fn math_font_size_uu(size_pt: f64) -> String {
    format!("{size_pt}px")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mathrec::tests::glyph;

    /// With no font store there is no face to outline, so every glyph takes
    /// the `<text>` fallback — and the phantom layer must then NOT be written,
    /// or the characters would be painted twice.
    #[test]
    fn base_14_mode_falls_back_to_text_and_writes_no_phantom_layer() {
        let mut out = String::new();
        emit_glyph_layer(&mut out, &[glyph("x", 0.0, 0.0, 10.0)], 10.0, None, true);
        assert!(out.contains("<text x="), "{out}");
        assert!(!out.contains("mphantom"), "{out}");
        assert!(!out.contains("<path"), "{out}");
    }

    /// A box that draws nothing declines rather than reserving its own size a
    /// second time on top of the nested text the caller already emitted.
    #[test]
    fn a_box_with_no_ink_produces_no_svg() {
        assert!(math_block(10.0, 10.0, 2.0, &[], &[], None).is_none());
        // …and a degenerate viewport is declined too, since a zero-width
        // `<svg>` renders as nothing anyway.
        let g = [glyph("x", 0.0, 0.0, 10.0)];
        assert!(math_block(0.0, 10.0, 2.0, &g, &[], None).is_none());
        assert!(math_block(10.0, 0.0, 0.0, &g, &[], None).is_none());
    }

    /// The Markdown wrapper's three load-bearing properties, and its
    /// single-line-ness.
    #[test]
    fn the_markdown_wrapper_is_one_line_and_sits_on_the_baseline() {
        let svg = math_block(30.0, 10.0, 2.0, &[glyph("x", 0.0, 0.0, 10.0)], &[], None)
            .expect("a glyph is ink");
        assert!(!svg.contains('\n'), "must be one line: {svg}");
        assert!(!svg.contains("position:absolute"), "{svg}");
        assert!(svg.contains("viewBox=\"0 0 30 12\""), "{svg}");
        assert!(svg.contains("vertical-align:-2pt"), "{svg}");
        assert!(svg.contains("overflow:visible"), "{svg}");
    }

    /// The one difference the two wrappers make to the shared layer: with no
    /// stylesheet, the phantom text must carry its own invisibility, and the
    /// HTML spelling must be unchanged so its output stays byte-identical.
    #[test]
    fn the_phantom_layer_carries_fill_none_only_when_asked() {
        let mut inline = String::new();
        Phantom {
            spans: "<tspan>x</tspan>".into(),
            size: Some(10.0),
            prev: None,
        }
        .finish(&mut inline, true);
        assert!(inline.contains("style=\"fill:none;font-size:10px;\""), "{inline}");

        let mut styled = String::new();
        Phantom {
            spans: "<tspan>x</tspan>".into(),
            size: Some(10.0),
            prev: None,
        }
        .finish(&mut styled, false);
        assert!(styled.contains("style=\"font-size:10px;\""), "{styled}");
        assert!(!styled.contains("fill:none"), "{styled}");
    }
}
