//! Reflowable/semantic HTML output mode (Slice 1: "reflowing paragraphs + inline
//! text + CSS"). Alongside the existing FAITHFUL twin
//! ([`crate::render_html`]/[`crate::render_html_ttf_with`], which serializes the
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
//! a geometry guess). `Image`/`Footnote` remain placeholders (no dedicated
//! recovery lever exists for either — out of scope for this backend).
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
//! **Additivity** (design doc §8): this module is reached only through the
//! two `pub fn`s below, themselves reached only via the CLI's
//! `--format html-reflow` (`rustyfi`). Nothing here changes the
//! behavior of [`crate::render_html`]/[`crate::render_html_ttf_with`] or
//! `rustyfi_pdf::render_pdf*` — it only reuses their already-`pub(super)`
//! (crate-visible) helpers ([`crate::escape_html`], [`crate::svg::css_color`],
//! [`crate::svg::emit_graphics`], [`crate::fonts`], [`crate::image::data_uri`])
//! read-only.

mod block;
mod css;
mod inline;
mod structure;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use rustyfi_backend::{
    AnnotAction, DecoId, DocExtras, FontKey, ImageResource, PageGeometry, VertBox,
};

use rustyfi_pdf::TtfFontStore;

use crate::HtmlError;

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
}

/// Serialize the pre-page-break `Vec<VertBox>` (`source` —
/// `DocumentValue::reflow_source`, `None` when unavailable, e.g. a
/// hand-built `DocumentValue` in a test) to a single, self-contained,
/// REFLOWABLE HTML document, using generic system-font fallback (no
/// `@font-face` block) — the base-14 twin of [`render_html_reflow_ttf_with`],
/// exactly mirroring [`crate::render_html`]'s relationship to
/// [`crate::render_html_ttf_with`].
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
/// [`crate::render_html_ttf_with`]'s Slice-3 fidelity mitigation.
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
    _images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    font_store: Option<&TtfFontStore>,
) -> Result<String, HtmlError> {
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
    out.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<style>\n");
    out.push_str(&css::stylesheet(geometry));
    if let Some(store) = font_store {
        let used = ctx.used_fonts.borrow();
        out.push_str(&crate::fonts::font_face_rules(store, &used));
    }
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(&body);
    out.push_str("</body>\n</html>\n");
    Ok(out)
}
