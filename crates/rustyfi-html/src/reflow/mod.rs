//! Reflowable/semantic HTML output mode (Slice 1: "reflowing paragraphs + inline
//! text + CSS"). Alongside the existing FAITHFUL twin
//! ([`crate::render_html_fixed`]/[`crate::render_html_fixed_ttf_with`], which serializes the
//! same post-page-break placed-box model the PDF writer consumes, one
//! absolutely-positioned `<span>` per glyph run), this mode branches at the
//! pre-page-break flat `Vec<VertBox>` (`DocumentValue::reflow_source` in
//! `rustyfi-lang`, the design doc's "Option B") and emits REAL flowing HTML: `<p>`
//! paragraphs the browser re-breaks, nested `<div>` frames, styled inline
//! `<span>`s — no `position`/`top`/`left` anywhere in this module's own output
//! (the defining difference from the faithful twin).
//!
//! **Slice 1 scope** (see the design doc §6): paragraphs (`Line`-runs
//! coalesced by `Skip`/frame boundaries), inline text (`InnerString`,
//! escaped + styled by font/size/color/rising), block nesting
//! (`FrameStart`/`FrameEnd`, `EmbeddedBlock`), and a clean semantic
//! stylesheet. Math/graphics/images/tables/footnotes were rendered as inert
//! placeholder `<span>`s.
//!
//! **Slice 2 scope** (design doc §6 "S2"): `Math`/`Graphics` now render as
//! real inline `<svg>` (reusing [`crate::svg::emit_graphics`] verbatim for
//! graphics content, §4's "reuse verbatim"), and `\href`-style links
//! (`annot.satyh`'s `register-link-to-uri`/`-to-location`, fired from a
//! `PureHorzBox::Frame`'s deco) become real `<a href>` elements — see
//! `Ctx::links`'s doc comment for HOW a page-absolute `Annot` gets matched
//! back to a specific pre-page-break `Frame` (the `DecoId` both carry, not
//! a geometry guess). (`Image` and `Footnote` were placeholders in this
//! slice; both are real now — see "What a continuous document does with the
//! things a page had", below.)
//!
//! **Slice 3 scope** (design doc §6 "S3", the "above-flat structure" slice
//! — see `reflow/structure.rs` for the implementation and its own doc
//! comment on exactly what is/isn't recoverable):
//! - `extras.outline` → a `<nav class="toc">` nested list (`structure::
//!   render_toc`), plus BEST-EFFORT promotion of the matching in-flow
//!   paragraph to `<h1>`..`<h6>` (`structure::find_heading_level`,
//!   `block.rs`'s `Para::heading_level`) — correlated to the outline entry
//!   by `dest_name`, the SAME string both `register-outline` and
//!   `register-location-frame`/`register-destination` resolve a label
//!   through (`Interp::dest_name`), so this is a structural match via the
//!   existing `Ctx::dests` `DecoId` map, not a text/geometry heuristic.
//! - `PureHorzBox::Tabular` now renders as a real `<table>`/`<tr>`/`<td>`
//!   (`structure::render_table`), replacing the Slice 1/2 `table-placeholder`
//!   `<span>`.
//! - List structure (`itemize`/`enumerate`) is NOT promoted to `<ul>`/`<ol>`
//!   here — see `structure.rs`'s doc comment for why it was judged not
//!   reliably recoverable from the box tree, unlike outline/tabular. (S4,
//!   below, resolves this with a new lever.)
//!
//! **Slice 4 scope**: the box tree genuinely has no recoverable
//! list/emphasis structure (S3's verdict above), so S4 adds a NEW lever —
//! inert marker boxes (`VertBox::ListMark`/`PureHorzBox::InlineMark`)
//! emitted positionally by a modified `itemize.satyh` (list/item boundaries,
//! ordered-vs-unordered) and by the repo-controlled `\emph`/`\bold`
//! (`v01-mini.satyh`, `std-ja.satyh`) — rather than trying to infer
//! structure from the existing flat stream.
//! - `block.rs`'s `walk_vboxes` gains a `VertBox::ListMark` arm: a small
//!   stack of open `<ul>`/`<ol>` tags makes nesting fall out automatically
//!   from how the markers are nested in the box stream (no depth payload
//!   needed).
//! - `inline.rs`'s `emit_inline` gains a `PureHorzBox::InlineMark` arm: an
//!   `<em>`/`<strong>` tag stack (`Ctx::emph_stack`) and a bullet-suppression
//!   counter (`Ctx::bullet_suppress`) that drops the drawn bullet/number
//!   glyph run between a `BulletStart`/`BulletEnd` fence.
//! - The markers are proven INERT for PDF/faithful HTML (design doc §4.3):
//!   `chop_page`/`place_block_at`/`measure_block` (rustyfi-backend) skip
//!   `VertBox::ListMark` with zero contribution before it can ever reach a
//!   `PlacedLine`, and `PureHorzBox::InlineMark` contributes zero advance
//!   everywhere it's measured and renders nothing (both writers' wildcard
//!   `emit_box` arm) wherever it still rides inside a placed line's
//!   `contents` — so this module is the ONLY consumer.
//! - Emphasis is opt-in and per-command (§5's honesty verdict): only
//!   `v01-mini.satyh`'s/`std-ja.satyh`'s `\emph`/`\bold` are wired: a
//!   third-party or `md-ja.satyh` `\emph` degrades to today's plain text,
//!   by design (never a font/size/color heuristic).
//!
//! ## What a continuous document does with the things a page had
//!
//! This mode is what `--format html` now means (`rustyfi`'s
//! `format::OutputFormat`, whose doc comment says why). Three page-shaped
//! constructs have no page to live on any more, and each is answered here
//! rather than dropped:
//!
//! - **Footnotes** become a numbered `<sup>` reference in the text and an
//!   `<aside class="footnote" role="doc-footnote">` immediately after the
//!   paragraph that referenced them, linked both ways
//!   (`block.rs`'s `flush_para`/`drain_footnotes`). *Just after the
//!   paragraph* was chosen over *collected at the end*: a footnote is an
//!   aside to a specific sentence, and in a document with no pages "the
//!   end" can be an arbitrarily long scroll away, which turns every note
//!   into a round trip. It is also what a reader of a web page expects the
//!   `<aside>` to mean. Collecting them would be a one-line change to
//!   `drain_footnotes`' call sites if that judgement is ever reversed.
//!   Nothing is silently dropped: `walk_vboxes` drains the queue at the end
//!   of every block, so a footnote referenced from a table cell or a bare
//!   frame — somewhere no paragraph ever opens — still lands.
//! - **Headers, footers and page numbers** are absent, and that is not a
//!   gap: `page_break_core` captures `reflow_source` BEFORE
//!   `pagepartsf` runs, so the running head a document repeats 27 times
//!   never enters this backend at all.
//! - **`\clearpage`** becomes an `<hr class="clearpage">` — a thematic break
//!   is the honest remainder of a page break once pages are gone.
//!
//! Ink stays ink. Math, diagrams and rules are drawings, and each becomes
//! one inline `<svg>` with its own intrinsic size sitting on the text
//! baseline (`inline.rs`'s `emit_math_svg`/`emit_graphics_box`). MathML is
//! not an option and never was: `read_math`/`layout_math_value` flatten a
//! formula to positioned glyphs at eval time, so no structure survives to
//! mark up.
//!
//! **Additivity** (design doc §8): this module is reached only through the
//! two `pub fn`s below. Nothing here changes the behavior of
//! [`crate::render_html_fixed`]/[`crate::render_html_fixed_ttf_with`] or
//! `rustyfi_pdf::render_pdf*` — it only reuses their already-`pub(super)`
//! (crate-visible) helpers ([`crate::escape_html`], [`crate::svg::css_color`],
//! [`crate::svg::emit_graphics`], [`crate::fonts`], [`crate::image::data_uri`])
//! read-only.

mod block;
mod css;
mod inline;
mod structure;
mod text;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use rustyfi_backend::{
    AnnotAction, DecoId, DocExtras, FontKey, ImageResource, PageGeometry, VertBox,
};

use rustyfi_pdf::TtfFontStore;

use crate::HtmlError;

pub(crate) use text::BodyStyle;

/// Render-time state shared by every `emit_*` function in this module — the
/// reflow twin of `crate::Ctx` (kept as a separate type rather than reused
/// directly: this mode has no `images` byte-serving need yet, S1 renders
/// `Image` as a placeholder, see `inline.rs`; keeping a distinct type avoids
/// coupling the two modes' evolution).
pub(crate) struct Ctx<'a> {
    pub(crate) fonts: Option<&'a TtfFontStore>,
    pub(crate) used_fonts: RefCell<BTreeSet<usize>>,
    /// S2 ("Links/metadata"): `DecoId -> action` for every
    /// `register-link-to-uri`/`-to-location` call the compile driver
    /// observed firing (`DocumentValue:: reflow_links`) — built once per
    /// render from the flat slice passed in, so `inline::emit_inline`'s
    /// `Frame` arm can look up "is THIS Frame's deco a link" in O(1) by the
    /// exact same `DecoId` the Frame box itself carries (a structural match,
    /// not a geometry guess — see that field's doc comment on
    /// `rustyfi_lang::value::DocumentValue`).
    pub(crate) links: HashMap<DecoId, &'a AnnotAction>,
    /// Same idea as `links`, for `register-destination`
    /// (`DocumentValue::reflow_dests`) — `DecoId -> the named-destination
    /// key`, consulted by `block::walk_vboxes`'s `FrameStart`/`FrameEnd` arm
    /// and `inline::emit_inline`'s `Frame` arm to place an `id="…"` anchor.
    pub(crate) dests: HashMap<DecoId, &'a str>,
    /// S3 (design doc §6 "S3" / this module's doc comment): `dest_name ->
    /// outline level`, built once per render from `extras.outline`
    /// (`DocExtras::outline`) — consulted by `structure::find_heading_level`
    /// to promote the paragraph whose `Frame` `DecoId` resolves (via
    /// `dests`, above) to a `register-outline`-registered destination name.
    /// Owned (`String`, not `&'a str`) rather than borrowed from `extras`:
    /// keeps `Ctx`'s lifetime parameter tied only to the `links`/`dests`
    /// slices it already had, avoiding a second lifetime bound on `extras`.
    pub(crate) outline_by_dest: HashMap<String, i64>,
    /// S4 ("Inline level"): the stack of currently-open `<em>`/`<strong>`
    /// spans, keyed by their `InlineMarkKind::EmphStart::strong` bit —
    /// `EmphEnd` carries no payload of its own, so the matching open tag
    /// has to be remembered somewhere; `RefCell` (not a threaded `&mut`)
    /// mirrors `used_fonts` above, keeping `inline::emit_inline`'s
    /// `&Ctx`-only signature (no caller needs to change to thread a stack
    /// through).
    pub(crate) emph_stack: RefCell<Vec<bool>>,
    /// S4 (design doc §4.1 "BulletStart/End fence"): a nesting counter
    /// (not a bare flag — `BulletStart`/`BulletEnd` pairs are never nested
    /// in practice, but a counter is exactly as cheap and can't go
    /// negative-then-wrong on a stray unmatched marker) that, while
    /// non-zero, makes `inline::emit_inline` render nothing for any box
    /// OTHER than an `InlineMark` itself — the drawn bullet/number glyph
    /// run between the fence is dropped, since the real `<ul>`/`<ol>`
    /// marker replaces it (R2, design doc §6.4).
    pub(crate) bullet_suppress: RefCell<u32>,
    /// The stack of wrappers opened by an `InlineFrameMarker` start and not
    /// yet closed, each stored as the literal closing tag to emit. The end
    /// marker carries only `end: true` — it does not say whether the start
    /// opened an `<a>` or a `<span>` — so, exactly like `emph_stack` above,
    /// the matching closer has to be remembered rather than recomputed.
    pub(crate) iframe_stack: RefCell<Vec<&'static str>>,
    /// The document's image table, so an `Image` box can resolve its
    /// `ImageId` to an `ImageResource` and become a real `<img>` data URI
    /// (`crate::image::data_uri`, shared verbatim with the faithful
    /// backend). Slices 1-4 rendered an inert placeholder here; a document
    /// like `figbox`'s manual is 39 figures, so the placeholder was most of
    /// what the document is about.
    pub(crate) images: &'a [ImageResource],
    /// The `(font, size)` pair most of the document's characters are set in
    /// — see [`BodyStyle`]. `css.rs` puts it on `body`; `inline.rs` omits it
    /// from every run that matches, and most runs do.
    pub(crate) body: BodyStyle,
    /// The natural width (pt) of glue seen since the last thing that was
    /// actually written, awaiting the character that follows it before
    /// `text::wants_space` can judge whether it is a space, a kern, or a
    /// bare break opportunity. Consecutive glues merge by taking the widest
    /// — two adjacent glues are still at most one space.
    pub(crate) pending_glue: Cell<Option<f64>>,
    /// The last character actually written into the flow, the `prev` half of
    /// [`text::wants_space`]'s decision. Deliberately NOT reset by the
    /// transparent wrappers (`Frame`, `InlineFrameMarker`, `InlineMark`), so
    /// a CJK/CJK pair straddling a `\ref`'s `<a>` still suppresses its
    /// space; reset to `None` by opaque boxes (`<svg>`, `<img>`, `<table>`),
    /// which have no last character to speak of.
    pub(crate) last_char: Cell<Option<char>>,
    /// Footnote bodies whose reference marker has been emitted but whose
    /// text has not yet been placed. `block.rs`'s `flush_para` drains this
    /// immediately after closing the referencing paragraph — see this
    /// crate's `reflow` module doc comment on why "just after the
    /// paragraph" is where a footnote belongs once there is no page foot to
    /// put it at.
    pub(crate) footnotes: RefCell<Vec<(usize, String)>>,
    /// Monotonic footnote number, shared by the `<sup>` reference and the
    /// `<aside>` body so the two can link to each other.
    pub(crate) footnote_seq: Cell<usize>,
    /// Already-rendered BLOCK-level HTML that turned up while an inline
    /// paragraph was open — a `<table>` or a `<div class="embed">` reached
    /// through a `Frame`'s contents or out of a `draw-text`, rather than at
    /// the top level of a `Line` where `block.rs` can flush the paragraph
    /// around it first. `<table>` inside `<p>` is not valid HTML (a parser
    /// closes the `<p>` at the `<table>` and leaves the `</p>` stray), so
    /// these are queued exactly like [`Ctx::footnotes`] and emitted by
    /// `flush_para` immediately after the paragraph closes.
    pub(crate) deferred_blocks: RefCell<Vec<String>>,
}

impl Ctx<'_> {
    /// Resolve `font`'s CSS `font-family`, recording its backing physical
    /// file as used (mirrors `crate::Ctx::font_family_for`) — `None` in
    /// base-14 mode.
    pub(crate) fn font_family_for(&self, font: FontKey) -> Option<String> {
        let store = self.fonts?;
        let file_idx = store.file_index(font);
        self.used_fonts.borrow_mut().insert(file_idx);
        Some(crate::fonts::font_family_name(file_idx))
    }

    /// Record that a glue box of `natural_pt` natural width stands here.
    /// Nothing is written yet: whether it becomes a space depends on the
    /// character that follows (`text::wants_space`), which is not known
    /// until the next run arrives.
    pub(crate) fn note_glue(&self, natural_pt: f64) {
        let merged = match self.pending_glue.get() {
            Some(prev) if prev >= natural_pt => prev,
            _ => natural_pt,
        };
        self.pending_glue.set(Some(merged));
    }

    /// Resolve the pending glue against the character about to be written
    /// (`next`, `None` before an opaque box or at a paragraph edge),
    /// appending a space to `out` if one is warranted.
    pub(crate) fn resolve_glue(&self, out: &mut String, next: Option<char>) {
        if let Some(width) = self.pending_glue.take() {
            if text::wants_space(self.last_char.get(), next, width) {
                out.push(' ');
            }
        }
    }

    /// Drop any pending glue and forget the last character — used at a hard
    /// boundary (a new paragraph, a table cell, a footnote body) where a
    /// space carried over from the previous context would be wrong.
    pub(crate) fn reset_flow(&self) {
        self.pending_glue.set(None);
        self.last_char.set(None);
    }
}

/// Serialize the pre-page-break `Vec<VertBox>` (`source` —
/// `DocumentValue::reflow_source`, `None` when unavailable, e.g. a
/// hand-built `DocumentValue` in a test) to a single, self-contained,
/// REFLOWABLE HTML document, using generic system-font fallback (no
/// `@font-face` block) — the base-14 twin of [`render_html_reflow_ttf_with`],
/// exactly mirroring [`crate::render_html_fixed`]'s relationship to
/// [`crate::render_html_fixed_ttf_with`].
///
/// `images`/`extras` are accepted for argument-for-argument symmetry with
/// the faithful backend; Slice 1 did not read them, Slice 2 reads `images`
/// for `Image` `<img>` data-URIs (TODO: still deferred, see `inline.rs`) and
/// `extras` is superseded here by the more precise `links`/`dests` slices
/// (`DocumentValue::reflow_links`/`reflow_dests` — `DecoId`-keyed, not
/// `extras.annotations`/`destinations`'s page-absolute rects, see
/// `Ctx::links`'s doc comment on why).
#[allow(clippy::too_many_arguments)]
pub fn render_html_reflow(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> Result<String, HtmlError> {
    render_html_reflow_impl(source, geometry, images, extras, links, dests, None)
}

/// Same as [`render_html_reflow`], but rendering under a real
/// [`TtfFontStore`] — every inline run's `<span>` gets an explicit
/// `font-family` naming the `@font-face` this function's `<style>` block
/// embeds for every physical font file actually referenced, exactly
/// [`crate::render_html_fixed_ttf_with`]'s Slice-3 fidelity mitigation.
#[allow(clippy::too_many_arguments)]
pub fn render_html_reflow_ttf_with(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    store: &TtfFontStore,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> Result<String, HtmlError> {
    render_html_reflow_impl(source, geometry, images, extras, links, dests, Some(store))
}

#[allow(clippy::too_many_arguments)]
fn render_html_reflow_impl(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    font_store: Option<&TtfFontStore>,
) -> Result<String, HtmlError> {
    // One read-only pass over the flow before anything is written: which
    // `(font, size)` most of the text is in, and how much of it is CJK. Both
    // are document-wide facts the per-run emitter needs BEFORE it emits its
    // first run, so they cannot be accumulated as it goes.
    let body_style = BodyStyle::dominant(source);
    let ctx = Ctx {
        fonts: font_store,
        used_fonts: RefCell::new(BTreeSet::new()),
        links: links.iter().map(|(id, action)| (*id, action)).collect(),
        dests: dests
            .iter()
            .map(|(id, name)| (*id, name.as_str()))
            .collect(),
        outline_by_dest: structure::outline_levels(&extras.outline),
        emph_stack: RefCell::new(Vec::new()),
        bullet_suppress: RefCell::new(0),
        iframe_stack: RefCell::new(Vec::new()),
        images,
        body: body_style,
        pending_glue: Cell::new(None),
        last_char: Cell::new(None),
        footnotes: RefCell::new(Vec::new()),
        footnote_seq: Cell::new(0),
        deferred_blocks: RefCell::new(Vec::new()),
    };

    let mut body = String::new();
    // S3 (design doc §6 "S3"): a navigable table of contents, built once
    // from `extras.outline` regardless of whether any heading in the flow
    // ends up promoted (the nav's own `<a href="#dest_name">`s work off the
    // SAME `id="dest_name"` anchors `block.rs`/`inline.rs` already place via
    // `ctx.dests`, S2) — a no-op (`<nav>` never emitted) when the doc class
    // never called `register-outline`.
    structure::render_toc(&mut body, &extras.outline);
    body.push_str("<div class=\"doc\">\n");
    if let Some(vboxes) = source {
        block::walk_vboxes(&mut body, vboxes, &ctx);
    } else {
        // No captured pre-page-break flow (e.g. a hand-built `DocumentValue`
        // in a unit test that never populated `reflow_source`) — an empty
        // document body rather than a panic; still valid, well-formed HTML.
        body.push_str("<p class=\"para reflow-empty\">(no reflow source captured)</p>\n");
    }
    body.push_str("</div>\n");

    let mut out = String::new();
    // `hyphens: auto` is inert without a language — a browser will not guess
    // one — so the root carries the language the text actually is. The
    // threshold is deliberately low: a Japanese document interleaves enough
    // Latin (code, package names, math) that "mostly Japanese" is well under
    // half, while an English document with a few kana in an example is well
    // under a tenth.
    let lang = if ctx.body.cjk_ratio > 0.1 { "ja" } else { "en" };
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
    );
    out.push_str("<style>\n");
    out.push_str(&css::stylesheet(geometry, &ctx));
    if let Some(store) = font_store {
        let used = ctx.used_fonts.borrow();
        out.push_str(&crate::fonts::font_face_rules(store, &used));
    }
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(&body);
    out.push_str("</body>\n</html>\n");
    Ok(out)
}
