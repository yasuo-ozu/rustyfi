//! Reflowable/semantic HTML — what `--format html` produces.
//!
//! Where the PDF writer consumes the post-page-break placed-box model, this
//! mode branches at the pre-page-break flat `Vec<VertBox>`
//! (`DocumentValue::reflow_source` in `rustyfi-lang`, the design doc's
//! "Option B") and emits REAL flowing HTML.
//!
//! **There are no pages here.** Reading the stream before page breaking is
//! what makes that true rather than merely stitched-together: nothing is cut
//! at a page boundary, and the page furniture — running headers, footers,
//! folios — is generated during page breaking and so never exists at all.
//! The output is one continuous document the browser re-breaks, hyphenates
//! and justifies at whatever width it is read.
//!
//! **No `position`/`top`/`left` anywhere in this module's own output.** The
//! one exception is deliberate and is not page positioning: math and
//! graphics are DRAWINGS, and each is an intrinsically-sized inline `<svg>`
//! whose own contents are positioned within its own tiny viewport (see
//! `inline.rs`'s `emit_math_svg`/`emit_graphics_box`).
//!
//! Three concerns big enough to have their own explanations:
//!
//! - **what a glue box becomes**, and why "glue means space" made Japanese
//!   unreadable — `text.rs`'s doc comment;
//! - **which runs need a `<span>` at all** — also `text.rs`; the document's
//!   dominant `(font, size)` goes on `body` so the bulk of the prose is
//!   written as bare text;
//! - **where a footnote goes** when there is no page foot — `inline.rs`'s
//!   `Footnote` arm and `block.rs`'s `drain_footnotes`. It becomes an
//!   `<aside>` immediately after the paragraph that referenced it, which is
//!   where a reader wants it in a continuous document; the in-text anchor is
//!   a zero-width link target, because the document has already typeset its
//!   own reference marker.
//!
//! **Slice 1 scope** (see the design doc §6): paragraphs (`Line`-runs
//! coalesced by `Skip`/frame boundaries), inline text (`InnerString`,
//! escaped + styled by font/size/color/rising), block nesting
//! (`FrameStart`/`FrameEnd`, `EmbeddedBlock`), and a clean semantic
//! stylesheet. Math/graphics/images/tables/footnotes were rendered as inert
//! placeholder `<span>`s.
//!
//! **Slice 2 scope** (design doc §6 "S2"): `Math`/`Graphics` render as real
//! inline `<svg>` (reusing [`crate::svg::emit_graphics`] verbatim for
//! graphics content, §4's "reuse verbatim"), and `\href`-style links
//! (`annot.satyh`'s `register-link-to-uri`/`-to-location`, fired from a
//! `PureHorzBox::Frame`'s deco) become real `<a href>` elements — see
//! `Ctx::links`'s doc comment for HOW a page-absolute `Annot` gets matched
//! back to a specific pre-page-break `Frame` (the `DecoId` both carry, not
//! a geometry guess). `Image` and `Footnote` were placeholders through
//! Slice 4 and are now real; see this module's doc comment above.
//!
//! **Slice 3 scope** (design doc §6 "S3", the "above-flat structure" slice
//! — see `reflow/structure.rs` for the implementation and its own doc
//! comment on exactly what is/isn't recoverable):
//! - `extras.outline` → BEST-EFFORT promotion of the matching in-flow
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
//! structure from the existing flat stream. BOTH generations' `itemize`
//! now emit them (`dist/packages/itemize.satyh` as well as
//! `dist-v01/`'s), so an ordinary 0.0.6 `+listing`/`+enumerate` gets a real
//! `<ul>`/`<ol>` too; a third-party list package (the corpus `enumitem`)
//! does not, and degrades to its own drawn bullets in flat paragraphs.
//! - `block.rs`'s `walk_vboxes` gains a `VertBox::ListMark` arm: a small
//!   stack of open `<ul>`/`<ol>` tags makes nesting fall out automatically
//!   from how the markers are nested in the box stream (no depth payload
//!   needed).
//! - `inline.rs`'s `emit_inline` gains a `PureHorzBox::InlineMark` arm: an
//!   `<em>`/`<strong>` tag stack (`Ctx::emph_stack`) and a bullet-suppression
//!   counter (`Ctx::bullet_suppress`) that drops the drawn bullet/number
//!   glyph run between a `BulletStart`/`BulletEnd` fence.
//! - The markers are proven INERT for the PDF path (design doc §4.3):
//!   `chop_page`/`place_block_at`/`measure_block` (rustyfi-backend) skip
//!   `VertBox::ListMark` with zero contribution before it can ever reach a
//!   `PlacedLine`, and `PureHorzBox::InlineMark` contributes zero advance
//!   everywhere it's measured and renders nothing (the PDF writer's wildcard
//!   `emit_box` arm) wherever it still rides inside a placed line's
//!   `contents` — so this module is the ONLY consumer.
//! - Emphasis is opt-in and per-command (§5's honesty verdict): only
//!   `v01-mini.satyh`'s/`std-ja.satyh`'s `\emph`/`\bold` are wired: a
//!   third-party or `md-ja.satyh` `\emph` degrades to today's plain text,
//!   by design (never a font/size/color heuristic).
//!
//! **Additivity** (design doc §8): this module is reached only through the
//! `pub fn`s below, themselves reached only via the CLI's
//! `--format html` (`rustyfi`). Nothing here changes the
//! behavior of `rustyfi_pdf::render_pdf*` — it only reuses the crate's own
//! `pub(super)` helpers ([`crate::escape_html`], [`crate::svg::css_color`],
//! [`crate::svg::emit_graphics`], [`crate::fonts`], [`crate::image::data_uri`])
//! read-only.

mod block;
mod css;
mod inline;
mod structure;
mod text;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;

use rustyfi_backend::{
    AnnotAction, DecoId, DocExtras, FontKey, FrameDecoration, GraphicsElem, ImageResource,
    MathGlyph, PageGeometry, VertBox,
};

use rustyfi_pdf::TtfFontStore;

use crate::HtmlError;

pub(crate) use text::BodyStyle;

/// Render-time state shared by every `emit_*` function in this module.
pub(crate) struct Ctx<'a> {
    pub(crate) fonts: Option<&'a TtfFontStore>,
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
    /// has to be remembered somewhere. `RefCell` rather than a threaded
    /// `&mut`, so `inline::emit_inline` keeps its `&Ctx`-only signature and
    /// no caller has to change to pass a stack through — the same bargain
    /// every other interior-mutable field here makes.
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
    /// yet closed, as `(tag to RE-open it with, tag to close it with)`. The
    /// end marker carries only `end: true` — it does not say whether the
    /// start opened an `<a>` or a `<span>` — so, exactly like `emph_stack`
    /// above, the matching closer has to be remembered rather than
    /// recomputed.
    ///
    /// The re-open tag exists because an `inline-frame-breakable` region can
    /// straddle a paragraph boundary: `\ref`-style markup opens its wrapper
    /// on one `Line` and closes it after a `Skip` has already flushed the
    /// paragraph, which would otherwise leave `<span class="iframe">` open
    /// across `</p>`. `block.rs` closes every open wrapper when it flushes
    /// and re-opens them on the next paragraph's first content — the same
    /// repair an HTML parser performs for a misnested inline element. It is
    /// a RE-open rather than the original tag because a wrapper carrying an
    /// `id=` must not repeat it; only the first fragment is the anchor.
    pub(crate) iframe_stack: RefCell<Vec<(String, &'static str)>>,
    /// The document's image table, so an `Image` box can resolve its
    /// `ImageId` to an `ImageResource` and become a real `<img>` data URI
    /// (`crate::image::data_uri`). Slices 1-4 rendered an inert placeholder
    /// here; a document like `figbox`'s manual is 39 figures, so the
    /// placeholder was most of what the document is about.
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
    /// Whether the last text run written was set in a fixed-pitch face.
    ///
    /// This is the only signal in the box stream that distinguishes a line
    /// boundary the browser should REDO from one it must KEEP. Both arrive as
    /// two consecutive `VertBox::Line`s with nothing between them: a wrapped
    /// paragraph and a `+code` block are structurally identical, because
    /// `code.satyh` calls `line-break` once per source line exactly as the
    /// line breaker does per wrapped line. Reset to `false` by any
    /// proportional run, so it means "still inside monospace text".
    pub(crate) mono_run: Cell<bool>,
    /// The line currently being built ends with a hyphen the LINE BREAKER
    /// inserted (`InlineMarkKind::BreakHyphen`), so rejoining it to the next
    /// line must drop that hyphen.
    ///
    /// Set per line and cleared by `block.rs` at each line boundary. Before
    /// this existed the rejoin guessed from the text's shape — "ends with
    /// letter+hyphen, next line starts lowercase" — and the guess deleted
    /// authored hyphens: a paragraph wrapping at `code-printer` rendered as
    /// `codeprinter`.
    pub(crate) break_hyphen: Cell<bool>,
    /// Rules belonging to a table whose own `TabularBox` does not carry them,
    /// as `(width, height, rules)`.
    ///
    /// `easytable` draws a table as TWO overlaid `tabular`s at one anchor:
    /// one holds the rules over PHANTOM cells, the other the real content and
    /// no rules at all (its own source shows the shape plainly — `ib-rule`
    /// and `ib-table`, both `draw-text` into one `inline-graphics`). Rendered
    /// independently, the rules land on a table with nothing in it — dropped
    /// as empty — and the visible table comes out with no rules. Pushed by
    /// `inline.rs`'s text-only graphics path, which is the only place the two
    /// halves are visible together, and matched back by geometry.
    pub(crate) tabular_rules: RefCell<Vec<(f64, f64, Vec<GraphicsElem>)>>,
    /// `DecoId -> the frame's own decoration`, from
    /// `DocumentValue::reflow_frame_decos`.
    ///
    /// A block frame's decoration is a lang-side callback, and this backend
    /// has no page grid to run it on — which is why `.frame` drew nothing at
    /// all, and every `stdjabook` title block, `+code` panel and framed
    /// figure arrived as bare text. `fire_hooks` already runs the callback
    /// for the PDF path; this is the same graphics, recorded box-local at the
    /// frame's natural size so it can be SCALED to whatever width the reader
    /// gives it rather than replayed at a fixed one.
    pub(crate) frame_decos: HashMap<DecoId, &'a FrameDecoration>,
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
    /// Canonical `ImageId`s of images placed more than once, in first-use
    /// order. Their bytes go into the stylesheet ONCE, as a
    /// `background-image` rule (`css.rs`'s `shared_image_rules`), instead of
    /// once per placement. See [`Ctx::image_sharing`].
    pub(crate) shared_images: RefCell<Vec<usize>>,
    /// Every `ImageId` mapped to the LOWEST `ImageId` holding identical
    /// pixels, and how many placements that canonical image has in total.
    ///
    /// Content, not identity, is what has to be deduplicated: each
    /// `include-image` call mints a fresh `ImageResource` even for a file
    /// already loaded, so `figbox`'s manual holds seventeen distinct
    /// `ImageId`s covering two actual pictures. Keying on the id alone found
    /// nothing to share.
    image_canon: HashMap<usize, (usize, usize)>,
    /// The `style` of the `<span class="run">` currently left OPEN, if any.
    /// A run whose style matches simply appends its text to it, so a word
    /// the box stream split into chunks — and a Japanese phrase it split
    /// into individual characters, which is every CJK run at any size other
    /// than the body's — comes out as ONE span of ordinary text rather than
    /// one span per chunk. Every emitter that writes something which is not
    /// part of the run (a tag, a strut, an `<svg>`) closes it first via
    /// `inline::close_run`; a space and a soft hyphen deliberately do not,
    /// since neither carries style and both belong inside the word.
    pub(crate) open_run: RefCell<Option<String>>,
}

impl Ctx<'_> {
    /// Resolve `font` to a CSS `font-family` VALUE — the real family name
    /// the font file declares, followed by generic fallbacks
    /// (`fonts::reflow_font_stack`). `None` in base-14 mode, and for a file
    /// whose `name` table declares no usable family, in which case the
    /// stylesheet's own stack applies.
    ///
    /// This NAMES the face rather than embedding it — see
    /// `fonts::reflow_font_stack` for the argument.
    pub(crate) fn font_family_for(&self, font: FontKey) -> Option<String> {
        let store = self.fonts?;
        let file_idx = store.file_index(font);
        let family = store.file_family_name(file_idx)?;
        Some(crate::fonts::reflow_font_stack(&family))
    }

    /// Whether `font` is a fixed-pitch face, read off the same family name
    /// [`Ctx::font_family_for`] builds its stack out of
    /// (`fonts::is_monospace_family` — a name heuristic, and labelled as one
    /// there). `false` in base-14 mode, where there is no file to ask.
    pub(crate) fn is_monospace(&self, font: Option<FontKey>) -> bool {
        let (Some(store), Some(font)) = (self.fonts, font) else {
            return false;
        };
        store
            .file_family_name(store.file_index(font))
            .is_some_and(|f| crate::fonts::is_monospace_family(&f))
    }

    /// The SVG `d` for a math glyph the document placed by GLYPH ID rather
    /// than by character (`MathGlyph::gid`), plus the face's
    /// `units_per_em` — `crate::svg::glyph_outline_d`'s two inputs resolved
    /// against this render's font store.
    ///
    /// `None` for every ordinary cmap-driven glyph (`gid: None`), which the
    /// caller renders as `<text>` exactly as before, and `None` in base-14
    /// mode — where `FontMetrics::math_vertical_variant`/`math_script_variant`
    /// answer `None` too, so no `Some(gid)` glyph can have been produced in
    /// the first place.
    pub(crate) fn math_glyph_outline(&self, glyph: &MathGlyph) -> Option<(String, f64)> {
        let gid = glyph.gid?;
        let face = self.fonts?.face(glyph.info.font)?;
        let upem = f64::from(face.units_per_em());
        let d = crate::svg::glyph_outline_d(&face, gid)?;
        Some((d, upem))
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

    /// For an `ImageId`: the canonical id of the image it holds, and whether
    /// that image is placed more than once (and so should be shared through
    /// the stylesheet rather than repeated inline). See `image_canon`.
    pub(crate) fn image_sharing(&self, id: usize) -> (usize, bool) {
        match self.image_canon.get(&id) {
            Some(&(canon, uses)) => (canon, uses > 1),
            None => (id, false),
        }
    }
}

/// Group `images` by CONTENT and fold in each group's total placement count
/// from the pre-pass, producing `Ctx::image_canon`. Two resources are the
/// same picture when their pixel dimensions and their bytes agree — the
/// original JPEG stream when there is one (which is also what
/// `image::data_uri` will emit), the decoded samples otherwise.
fn canonical_images(
    images: &[ImageResource],
    uses: &HashMap<usize, usize>,
) -> HashMap<usize, (usize, usize)> {
    let mut first_by_content: HashMap<(&[u8], u32, u32), usize> = HashMap::new();
    let mut canon_of: HashMap<usize, usize> = HashMap::new();
    for (idx, res) in images.iter().enumerate() {
        let bytes: &[u8] = match &res.jpeg_dct {
            Some(j) => &j.bytes,
            None => &res.samples,
        };
        // An imported PDF page has neither, so every one of them would hash
        // alike; they render as a labelled box rather than an image anyway,
        // so leave each as its own canonical self.
        if bytes.is_empty() {
            canon_of.insert(idx, idx);
            continue;
        }
        let canon = *first_by_content
            .entry((bytes, res.px_w, res.px_h))
            .or_insert(idx);
        canon_of.insert(idx, canon);
    }
    let mut total: HashMap<usize, usize> = HashMap::new();
    for (id, n) in uses {
        let canon = canon_of.get(id).copied().unwrap_or(*id);
        *total.entry(canon).or_default() += n;
    }
    canon_of
        .into_iter()
        .map(|(id, canon)| (id, (canon, total.get(&canon).copied().unwrap_or(0))))
        .collect()
}

/// Serialize the pre-page-break `Vec<VertBox>` (`source` —
/// `DocumentValue::reflow_source`, `None` when unavailable, e.g. a
/// hand-built `DocumentValue` in a test) to a single, self-contained,
/// REFLOWABLE HTML document, leaving every run's face to the stylesheet's
/// own generic stack — the base-14 twin of
/// [`render_html_reflow_ttf_with`], exactly mirroring
/// `rustyfi_pdf::render_pdf_with`'s relationship to
/// `rustyfi_pdf::render_pdf_ttf_with`.
///
/// `images` is read for real: each `Image` box resolves against it and
/// becomes an `<img>` data URI (`crate::image::data_uri`). `extras` is
/// accepted mostly for argument-for-argument symmetry with the PDF writer —
/// `extras.outline` drives heading promotion (`Ctx::outline_by_dest`), but
/// its `annotations`/`destinations` are superseded here by the more precise
/// `links`/`dests` slices (`DocumentValue::reflow_links`/`reflow_dests` —
/// `DecoId`-keyed, not page-absolute rects; see `Ctx::links`'s doc comment
/// on why).
#[allow(clippy::too_many_arguments)]
pub fn render_html_reflow(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> Result<String, HtmlError> {
    render_html_reflow_impl(source, geometry, images, extras, links, dests, &[], None)
}

/// [`render_html_reflow`] plus the frame decorations
/// (`DocumentValue::reflow_frame_decos`), so framed blocks draw their own
/// decoration instead of nothing.
pub fn render_html_reflow_with_decos(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    frame_decos: &[(DecoId, FrameDecoration)],
) -> Result<String, HtmlError> {
    render_html_reflow_impl(source, geometry, images, extras, links, dests, frame_decos, None)
}

/// Same as [`render_html_reflow`], but rendering under a real
/// [`TtfFontStore`] — the document's faces are then NAMED
/// (`crate::fonts::reflow_font_stack`) rather than left to the stylesheet's
/// generic stack: the dominant face goes on the `body` rule (`css.rs`), and
/// only a run that departs from it names a family of its own (`inline.rs`'s
/// `emit_run`) — so the bulk of the prose stays bare text with no `<span>`
/// at all. Nothing is embedded; see `crate::fonts` for why a reflowed
/// document does not pay for that.
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
    render_html_reflow_impl(source, geometry, images, extras, links, dests, &[], Some(store))
}

/// [`render_html_reflow_ttf_with`] plus the frame decorations — the
/// full-fidelity entry point the CLI uses.
#[allow(clippy::too_many_arguments)]
pub fn render_html_reflow_ttf_with_decos(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    store: &TtfFontStore,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    frame_decos: &[(DecoId, FrameDecoration)],
) -> Result<String, HtmlError> {
    render_html_reflow_impl(
        source,
        geometry,
        images,
        extras,
        links,
        dests,
        frame_decos,
        Some(store),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_html_reflow_impl(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    frame_decos: &[(DecoId, FrameDecoration)],
    font_store: Option<&TtfFontStore>,
) -> Result<String, HtmlError> {
    // One read-only pass over the flow before anything is written: which
    // `(font, size)` most of the text is in, and how much of it is CJK. Both
    // are document-wide facts the per-run emitter needs BEFORE it emits its
    // first run, so they cannot be accumulated as it goes.
    let body_style = BodyStyle::dominant(source);
    let image_canon = canonical_images(images, &body_style.image_uses);
    let ctx = Ctx {
        fonts: font_store,
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
        mono_run: Cell::new(false),
        break_hyphen: Cell::new(false),
        tabular_rules: RefCell::new(Vec::new()),
        frame_decos: frame_decos.iter().map(|(id, d)| (*id, d)).collect(),
        footnotes: RefCell::new(Vec::new()),
        footnote_seq: Cell::new(0),
        shared_images: RefCell::new(Vec::new()),
        image_canon,
        open_run: RefCell::new(None),
    };

    let mut body = String::new();
    // No generated table of contents. `extras.outline` still drives heading
    // promotion and the `id=` anchors that in-document links land on, but a
    // document that wants a contents page TYPESETS one (`stdjabook`'s
    // `\table-of-contents`), and emitting a second, differently-styled copy
    // above the title duplicated it in every real manual.
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
    // Reads state the body walk filled in, so it must come after it: which
    // images were placed often enough to be worth sharing. (No
    // `@font-face` counterpart — this backend names fonts rather than
    // embedding them; see `fonts::reflow_font_stack`.)
    out.push_str(&css::shared_image_rules(&ctx));
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(&body);
    out.push_str("</body>\n</html>\n");
    Ok(out)
}
