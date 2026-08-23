//! HTML output backends. There are two, and they answer different
//! questions.
//!
//! **This module is the LAYOUT-FAITHFUL one** (`--format html-fixed`,
//! [`render_html_fixed`]): it serializes the SAME post-page-break
//! `Page`/`PlacedLine` model the PDF writer (`rustyfi-pdf`'s `lib.rs`)
//! consumes — the design doc's "Option A", a non-reflowing "PDF-in-a-div"
//! twin of the PDF output. Its use is visual diffing: putting this port's
//! layout in a browser where a run's coordinates can be inspected, rather
//! than eyeballing two renderings side by side. It is not a web page and
//! is not meant to be read as one.
//!
//! **The [`reflow`] submodule is the readable one** (`--format html`,
//! [`render_html_reflow`]): one continuous, semantic document with no pages
//! in it, built from the flat block stream as it stood BEFORE page
//! breaking. See its own doc comment.
//!
//! Everything below concerns the faithful backend.
//!
//! **Slice 1** (§Slice 1, "text + block layout of a single-page document"):
//! `InnerString` runs as positioned `<span>`s, plus the `<div class="page">`
//! wrapper. **Slice 2** (§Slice 2, "graphics (inline SVG)"), this revision:
//! `Graphics` as inline SVG (`svg.rs`) and the `Tabular`/`EmbeddedBlock`/
//! `Frame` composite recursions, mirroring the PDF writer's own `emit_box`
//! (`lib.rs:646-671`). `Image`/`Math` and `DocExtras::page_graphics` remain
//! Slice 3+ territory — see `emit_box`'s doc comment below.
//!
//! **Slice 3** (§Slice 3, "real fonts + math"): `@font-face` data-URI
//! embedding (`fonts.rs`) so text/math runs use the SAME TrueType face
//! the [`rustyfi_pdf::TtfFontStore`] PDF path embeds (metric-faithful
//! positioning — see this module's `Ctx`/`render_html_fixed_ttf_with`), `Image`
//! boxes as `<img>` data URIs (`image.rs`, a hand-rolled uncompressed
//! BMP container — no PNG/image-codec dependency), and `Math` glyphs as
//! positioned `<span>`s (reusing the same run-emission path as
//! `InnerString`, per the design doc's math row) with `Math.rules` (the
//! fraction bar/radical) through the Slice-2 SVG path. The base-14 (no font
//! store) path is UNCHANGED from Slice 1/2: [`render_html_fixed`] still emits the
//! generic `.run` CSS default font-family, no `@font-face` block at all.
//!
//! **Slice 4** (§Slice 4, "multi-page + print pagination"), this revision:
//! print pagination CSS (a `@page { size: …; margin: 0 }` rule matching
//! `geometry.paper_width`/`paper_height`, plus `.page:not(:last-child) {
//! page-break-after: always; break-after: page }` so a browser print/
//! print-to-PDF paginates 1:1 with the document — a single-page document has
//! no non-last `.page`, so this selector matches nothing and its output is
//! byte-identical to Slice 1-3's, per the design doc's "keep single-page
//! docs looking identical" requirement) and `DocExtras::page_graphics` (the
//! per-page deco-graphics underlay this doc comment had flagged as deferred
//! since Slice 1/2 — see `render_html_impl`'s per-page loop below for the
//! coordinate-frame reconciliation).
//!
//! **Location.** This is its own `rustyfi-html` crate, a peer of
//! `rustyfi-pdf` (per the design doc's original spec, survey #6). It depends
//! on `rustyfi-backend` for every box/geometry type used below, plus
//! `rustyfi-pdf` for [`rustyfi_pdf::TtfFontStore`] (the one type this module
//! reuses rather than re-implements — only its `pub` `file_index`/
//! `file_bytes` accessors are used, so this is a plain one-way dependency,
//! not a cycle: `rustyfi-pdf` does not depend on `rustyfi-html`). Nothing
//! here touches `pdf_writer` or any other PDF-specific type, only
//! `rustyfi_backend`/`rustyfi_pdf::TtfFontStore` types and `String`
//! building.

mod base64;
mod fonts;
mod image;
mod reflow;
mod svg;

// Reflowable/semantic HTML output mode (`reflow/mod.rs`'s doc comment) —
// re-exported at the crate root so CLI dispatch calls it exactly like the
// faithful `render_html_fixed`/`render_html_fixed_ttf_with` pair above (argument-for-
// argument symmetry, not a new API shape to learn).
pub use reflow::{
    render_html_reflow, render_html_reflow_ttf_with, render_html_reflow_ttf_with_decos,
    render_html_reflow_with_decos,
};

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use rustyfi_backend::{
    place_block_at, Color, DocExtras, FontKey, HorzStringInfo, ImageResource, Length, Page,
    PageGeometry, PureHorzBox, VertBox,
};

use rustyfi_pdf::TtfFontStore;

/// Slice 1 never actually constructs this — every text run is valid
/// UTF-8/HTML-escapable, and no font/image embedding (the error-prone parts,
/// per the design doc's later slices) happens yet. The `Result` return
/// shape is kept anyway so `render_html_fixed` is argument-for-argument (module
/// signature, not module fallibility) with `render_pdf_with`
/// (`lib.rs:459`), and so Slices 2/3 (SVG graphics, real fonts/`@font-face`,
/// image data-URIs) can surface a real error without a breaking signature
/// change.
#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Shared render-time state threaded through every `emit_*` function below
/// (Slice 3): the document's image table (so `Image` boxes can resolve an
/// `ImageId` to its `ImageResource`) and, when rendering under a real
/// [`TtfFontStore`] (`render_html_fixed_ttf_with`), the store itself plus a
/// running set of which physical font FILES (`TtfFontStore::file_index`,
/// not `FontKey` slots — bold/oblique with no configured face dedup to the
/// regular file exactly like the CID PDF writer's own `FontUsage`,
/// `cid.rs`) were actually referenced by an emitted run, so the caller only
/// writes `@font-face` for fonts the document actually used.
///
/// `used_fonts` is a `RefCell` rather than threaded as an extra `&mut`
/// parameter through every recursive call because [`svg::NestedEmitter`]'s
/// callback type has no `Ctx` slot — every callsite instead closes over
/// `ctx: &Ctx` by value (a `&Ctx` copy). This module is single-threaded, so
/// interior mutability here is exactly as safe as (and far less invasive to
/// plumb than) a threaded `&mut BTreeSet`.
struct Ctx<'a> {
    images: &'a [ImageResource],
    fonts: Option<&'a TtfFontStore>,
    used_fonts: RefCell<BTreeSet<usize>>,
}

impl Ctx<'_> {
    /// Resolve `font`'s CSS `font-family`, marking its backing physical file
    /// as used so [`fonts::font_face_rules`] later emits an `@font-face` for
    /// it. Returns `None` in base-14 mode (no store configured) — callers
    /// then fall back to the `.run` CSS class's generic default (a system
    /// serif), exactly Slice 1/2's unmodified behavior.
    fn font_family_for(&self, font: FontKey) -> Option<String> {
        let store = self.fonts?;
        let file_idx = store.file_index(font);
        self.used_fonts.borrow_mut().insert(file_idx);
        Some(fonts::font_family_name(file_idx))
    }
}

/// Serialize typeset pages to a single, self-contained HTML document, using
/// generic system-font fallback (Slice 1/2 behavior, unchanged): no
/// `@font-face` block, every run styled by the plain `.run` CSS class. This
/// is the base-14 twin of [`rustyfi_pdf::render_pdf_with`] — pass
/// [`render_html_fixed_ttf_with`] a real [`TtfFontStore`] instead when the
/// document was typeset against real embedded fonts, for metric-faithful
/// output (Slice 3, see this module's doc comment).
///
/// Argument-for-argument with [`rustyfi_pdf::render_pdf_with`] (`lib.rs:459`):
/// `geometry` (reads only `paper_width`/`paper_height`, exactly like the PDF
/// writer), `pages` (the post-page-break `Vec<PlacedLine>` per `Page`),
/// `images` (the document-wide image table — `Image` boxes resolve their
/// `ImageId` against it, Slice 3), and `extras` (Slice 4: `page_graphics` is
/// now rendered as a per-page `<svg>` underlay, see `render_html_impl`;
/// `annotations`/`outline`/`doc_info` remain a documented gap — there is no
/// HTML analogue of a PDF `/Annots`/`/Outlines` tree in this Option-A
/// serializer).
///
/// One `<div class="page">` per `Page`, sized to `paper_width`/`paper_height`
/// in CSS `pt` (1:1 with SATySFi's own point unit). Inside, every
/// `PureHorzBox::InnerString` run on a `PlacedLine` becomes one
/// absolutely-positioned `<span>` at its resolved `(x, y)` — SATySFi's page
/// coordinates are already y-**down** from the paper top
/// (`PlacedLine`'s own doc comment, `pagebreak.rs:13`), which is exactly
/// CSS's `top` convention, so unlike the PDF writer this needs **no y-flip**.
pub fn render_html_fixed(
    geometry: &PageGeometry,
    pages: &[Page],
    images: &[ImageResource],
    extras: &DocExtras,
) -> Result<String, HtmlError> {
    render_html_impl(geometry, pages, images, extras, None)
}

/// Same as [`render_html_fixed`], but rendering under a real [`TtfFontStore`] —
/// the HTML twin of [`rustyfi_pdf::render_pdf_ttf_with`] (`cid.rs`). Every text
/// and math run's `<span>` gets an explicit `font-family` naming the
/// `@font-face` embedding this function adds to the `<style>` block for
/// every physical font file the document actually referenced, so the
/// browser lays text out in the SAME face whose metrics the layout was
/// computed with (the design doc's §Risks "font-metric fidelity" mitigation,
/// Slice 3's whole point).
pub fn render_html_fixed_ttf_with(
    geometry: &PageGeometry,
    pages: &[Page],
    store: &TtfFontStore,
    images: &[ImageResource],
    extras: &DocExtras,
) -> Result<String, HtmlError> {
    render_html_impl(geometry, pages, images, extras, Some(store))
}

fn render_html_impl(
    geometry: &PageGeometry,
    pages: &[Page],
    images: &[ImageResource],
    extras: &DocExtras,
    font_store: Option<&TtfFontStore>,
) -> Result<String, HtmlError> {
    let paper_w = geometry.paper_width.0;
    let paper_h = geometry.paper_height.0;

    let ctx = Ctx {
        images,
        fonts: font_store,
        used_fonts: RefCell::new(BTreeSet::new()),
    };

    // Pass 1: emit every page's markup, recording (via `ctx.used_fonts`,
    // Slice 3) which physical font files were actually referenced — mirrors
    // the CID PDF writer's own two-pass shape (`cid.rs`'s `page_content`
    // pass populating `usage` before `write_font` runs).
    let mut body = String::new();
    for (i, page) in pages.iter().enumerate() {
        body.push_str(&format!(
            "<div class=\"page\" style=\"width:{paper_w}pt; height:{paper_h}pt;\">\n"
        ));
        // Slice 4: `DocExtras::page_graphics` — one overlay per page (missing
        // = empty, mirrors `render_pdf_with`'s own `extras.page_graphics
        // .get(i)...unwrap_or(&[])`, `lib.rs:528`), drawn FIRST so it sits
        // UNDER the page's text/images (background fills/borders), exactly
        // `page_content`'s own overlay-first order (`lib.rs:566-577`).
        //
        // **Coordinate-frame reconciliation.** Unlike every other
        // `emit_graphics` call in this module, `page_graphics` elements are
        // NOT box-local — `fire_hooks` (`rustyfi-lang/src/lib.rs:280`) fills
        // them in ABSOLUTE PDF y-up page coordinates (`doc.rs:76`, and the
        // PDF writer feeds them to `place_graphics` at anchor `(0.0, 0.0)`,
        // `lib.rs:576`, confirming "absolute" — no per-box translate). Reuse
        // `svg::emit_graphics`'s existing box-local-to-page-space formula
        // (`page = (tx + px, ty - py)`, that module's doc comment) by
        // choosing `(tx, ty, height) = (0.0, paper_h, paper_h)`: `ty - py`
        // becomes exactly the y-flip `paper_h - py` this absolute-coordinate
        // convention needs (PDF y-up, paper-bottom origin -> CSS y-down,
        // paper-top origin), and the viewport top `ty - height = 0`/total
        // height `height + depth = paper_h` cover the full page exactly
        // (`depth = 0.0`).
        let overlay = extras
            .page_graphics
            .get(i)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        svg::emit_graphics(
            &mut body,
            overlay,
            paper_w,
            paper_h,
            0.0,
            0.0,
            paper_h,
            &mut |out, cbx, x, y| emit_box(out, cbx, x, y, &ctx),
        );
        for line in &page.lines {
            for (dx, bx) in &line.contents {
                let tx = (line.x + *dx).0;
                let ty = line.baseline_y.0;
                emit_box(&mut body, bx, tx, ty, &ctx);
            }
        }
        body.push_str("</div>\n");
    }

    let mut out = String::new();
    out.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<style>\n");
    out.push_str("body { margin: 0; padding: 12pt; background: #999; }\n");
    out.push_str(
        ".page { position: relative; background: #fff; margin: 0 auto 12pt auto; \
         overflow: hidden; box-shadow: 0 0 4pt rgba(0,0,0,0.4); }\n",
    );
    out.push_str(
        ".run { position: absolute; margin: 0; padding: 0; white-space: pre; \
         font-family: serif; line-height: 1; }\n",
    );
    // Slice 4: print pagination. `@page` pins the printed sheet size to
    // EXACTLY the document's own paper size (no browser default-margin
    // letterhead), and `.page:not(:last-child)` forces a hard page break
    // after every page except the last — so printing/PDF-from-browser
    // reproduces the document's own page count 1:1. `:not(:last-child)`
    // rather than an unconditional rule on every `.page` avoids a trailing
    // blank page after the final one, and — the design's "keep single-page
    // docs looking identical" requirement — a document with exactly one
    // `.page` div has no non-last sibling, so this selector matches nothing
    // there: single-page output is byte-identical to Slice 1-3's aside from
    // this now-always-present (but inert) rule text itself. Screen stacking
    // (the visible gap/border between pages) is unchanged from Slice 1 — the
    // `.page` margin/box-shadow above already provides it.
    out.push_str(&format!(
        "@page {{ size: {paper_w}pt {paper_h}pt; margin: 0; }}\n"
    ));
    out.push_str(".page:not(:last-child) { page-break-after: always; break-after: page; }\n");
    // Slice 3: one `@font-face` per physical font file the document
    // referenced (empty when `font_store` is `None`, or when a store was
    // given but nothing was ever emitted through it — e.g. an empty
    // `pages`), keeping the base-14 path's `<style>` block byte-identical
    // to Slice 1/2.
    if let Some(store) = font_store {
        let used = ctx.used_fonts.borrow();
        out.push_str(&fonts::font_face_rules(store, &used));
    }
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str(&body);
    out.push_str("</body>\n</html>\n");
    Ok(out)
}

/// Emit one already-placed `PureHorzBox` at absolute page coordinates
/// `(tx, ty)` — `tx` the box's left edge, `ty` its **baseline**, both in
/// SATySFi's own y-down page space (no flip needed for HTML/CSS, unlike the
/// PDF writer's `emit_box`, `lib.rs:604`, which this mirrors in shape).
///
/// Slice 1 handled only `InnerString` (a positioned `<span>`, via
/// [`emit_run`]) and `OuterEmpty`/`FixedEmpty` (inter-word glue/skips —
/// already fully accounted for by the caller's `dx` offsets, so they render
/// nothing extra). Slice 2 added `Graphics` (inline SVG, via
/// [`svg::emit_graphics`]) and the three composite recursions the PDF
/// writer's own `emit_box` has (`Tabular`/`EmbeddedBlock`/`Frame`,
/// `lib.rs:646-671`) so nested content inside them renders too. Slice 3
/// adds `Image` (an `<img>` data URI, via [`image::data_uri`]) and `Math`
/// (per-glyph `<span>`s through the same [`emit_run`] path, plus `rules`
/// through [`svg::emit_graphics`] — see the design doc's math row: the
/// semantic tree is already flattened by `read_math` by the time a box
/// exists, so this needs no math-specific rendering beyond reusing the
/// text/SVG paths). The remaining zero-width markers still hit the wildcard
/// arm — exactly `emit_box`'s own `_ => {}` (`lib.rs:672`).
fn emit_box(out: &mut String, bx: &PureHorzBox, tx: f64, ty: f64, ctx: &Ctx) {
    match bx {
        PureHorzBox::InnerString {
            info, text, height, ..
        } => {
            // `info.rising` raises the run (a positive rising moves it UP
            // the page, i.e. DECREASES the y-down `ty` — the mirror image
            // of the PDF writer's `ty + rising` in its y-**up** space,
            // `lib.rs:614`). `height` is the run's ascent (height above its
            // own baseline, `hbox.rs:88`), so the span's CSS `top` (its
            // TOP edge) is the effective baseline minus that ascent.
            let baseline = ty - info.rising.0;
            let top = baseline - height.0;
            emit_run(out, info, text, tx, top, ctx);
        }
        PureHorzBox::OuterEmpty { .. } | PureHorzBox::FixedEmpty { .. } => {
            // Interword glue / a fixed skip: no visible content of its own —
            // its width already went into every LATER box's `dx` on this
            // line (the same reasoning as the PDF writer, which also emits
            // nothing for these two, `lib.rs:670`'s wildcard).
        }
        // §Slice 3 (`Image` sub-step): an `<img>` sized/positioned exactly
        // like the PDF writer's `place_image` (`lib.rs:165`) — `ty` is the
        // box's BASELINE, and an `Image` box is all height/zero depth (it
        // sits entirely above the baseline, `linebreak.rs`'s
        // `layout_line`), so the baseline IS the image's bottom edge and
        // `top = ty - height` is its top edge, the same "baseline minus
        // ascent" arithmetic every other box here uses. Silently skips an
        // out-of-range `ImageId` (mirrors `write_image_xobjects`'s own
        // graceful skip, `lib.rs:136-142` — should not happen, but a page
        // missing one image beats a panic).
        PureHorzBox::Image {
            width,
            height,
            image,
        } => {
            if let Some(res) = ctx.images.get(image.0) {
                let top = ty - height.0;
                let w = width.0;
                let h = height.0;
                if res.pdf.is_some() {
                    // `load-pdf-image`: a raster `<img>`/BMP data URI has no
                    // samples to encode for an imported PDF page
                    // (`res.samples` is empty). A faithful HTML rendering
                    // would need to rasterize the page — out of scope here —
                    // so this emits a bordered placeholder box at the box's
                    // resolved dimensions instead of silently producing a
                    // degenerate 0x0 image.
                    let _ = write!(
                        out,
                        "<div style=\"position:absolute; left:{tx}pt; top:{top}pt; \
                         width:{w}pt; height:{h}pt; box-sizing:border-box; \
                         border:1px solid #888;\" title=\"PDF page image\"></div>\n",
                    );
                } else {
                    let uri = image::data_uri(res);
                    let _ = write!(
                        out,
                        "<img style=\"position:absolute; left:{tx}pt; top:{top}pt; \
                         width:{w}pt; height:{h}pt;\" src=\"{uri}\" alt=\"\">\n",
                    );
                }
            }
        }
        // §Slice 2: a box carrying resolved `graphics` elements — one
        // inline `<svg>`, sized/positioned from this box's own outer
        // metrics (see `svg::emit_graphics`'s doc comment for the
        // coordinate-frame reconciliation). `GraphicsElem::Text` (a
        // `draw-text` run) re-enters `emit_box` itself via the callback.
        PureHorzBox::Graphics {
            width,
            height,
            depth,
            elems,
            origin_independent: _,
        } => {
            svg::emit_graphics(
                out,
                elems,
                width.0,
                height.0,
                depth.0,
                tx,
                ty,
                &mut |out, cbx, x, y| emit_box(out, cbx, x, y, ctx),
            );
        }
        // §Slice 3 (`Math` row): each already-positioned `MathGlyph` is
        // rendered through the SAME run path as `InnerString` (a `<span>`,
        // via `emit_run`) — `glyph.dx`/`dy` are box-local offsets from this
        // box's own placed anchor `(tx, ty)`, y-**up** (mirroring
        // `place_math`'s `anchor_y + glyph.dy` in PDF's y-up space,
        // `lib.rs:187-208`), so both `dy` and `info.rising` SUBTRACT from
        // the page-down `ty` here — the same sign flip `InnerString`'s own
        // `rising` handling uses above. `glyph.gid` (a raw MATH-table
        // variant glyph id, not necessarily reachable from `glyph.text` via
        // cmap — §B3) has no HTML/CSS analogue (there is no way to address a
        // bare glyph id from markup), so this renders `glyph.text`
        // regardless — a documented, Option-A-inherent approximation for
        // that one construction (stretchy delimiters/big operators), not a
        // regression for the overwhelmingly common cmap-reachable glyph
        // case. `rules` (the fraction bar/radical) are `GraphicsElem`s in
        // the SAME box-local convention as `Tabular.rules`, so they route
        // through the identical `(tx, ty)` anchor via `svg::emit_graphics`.
        PureHorzBox::Math {
            width,
            height,
            depth,
            glyphs,
            rules,
        } => {
            for g in glyphs {
                let baseline = ty - g.dy.0 - g.info.rising.0;
                let top = baseline - g.height.0;
                let x = tx + g.dx.0;
                emit_run(out, &g.info, &g.text, x, top, ctx);
            }
            svg::emit_graphics(
                out,
                rules,
                width.0,
                height.0,
                depth.0,
                tx,
                ty,
                &mut |out, cbx, x, y| emit_box(out, cbx, x, y, ctx),
            );
        }
        // §Slice 2 (mirrors `emit_box`'s `Tabular` arm, `lib.rs:646-658`):
        // each cell's already-laid-out boxes at their resolved offset —
        // `cell.x`/`cdx` are page-down-frame-agnostic horizontal offsets
        // (added straight through, like every other horizontal offset),
        // but `cell.baseline_y` is box-local y-**up** from the tabular
        // box's own baseline-left origin (`TabularCellBox`'s doc comment,
        // `tabular.rs:60`) — exactly `GraphicsElem`'s convention — so it
        // SUBTRACTS from the page-down `ty` (the mirror image of the PDF
        // writer's `ty + cell.baseline_y` in its y-up space). `tab.rules`
        // are `GraphicsElem`s in that SAME box-local convention, so they
        // route through the identical `(tx, ty)` anchor via
        // `svg::emit_graphics`, just like a standalone `Graphics` box.
        PureHorzBox::Tabular(tab) => {
            for cell in &tab.cells {
                for (cdx, cbx) in &cell.contents {
                    emit_box(
                        out,
                        cbx,
                        tx + (cell.x + *cdx).0,
                        ty - cell.baseline_y.0,
                        ctx,
                    );
                }
            }
            svg::emit_graphics(
                out,
                &tab.rules,
                tab.width.0,
                tab.height.0,
                tab.depth.0,
                tx,
                ty,
                &mut |out, cbx, x, y| emit_box(out, cbx, x, y, ctx),
            );
        }
        // §Slice 2 (mirrors `emit_box`'s `EmbeddedBlock` arm, `lib.rs:659-
        // 661`, via `place_embedded_block`'s HTML twin below): stack the
        // block's already-broken lines from the box's placed anchor.
        PureHorzBox::EmbeddedBlock { block, .. } => {
            emit_embedded_block(out, block, tx, ty, ctx);
        }
        // §Slice 2 (mirrors `emit_box`'s `Frame` arm, `lib.rs:667-671`): an
        // inline frame's contents, all on the frame's OWN baseline (only
        // `dx` varies per child — no y offset, unlike `Tabular`'s cells).
        // The frame's deco graphics are NOT emitted here, same as the PDF
        // writer — they are fired lang-side into `DocExtras::page_graphics`,
        // a page-level underlay rendered once per page by `render_html_impl`
        // (Slice 4), not per-`Frame`-box here — same split the PDF writer
        // itself has (`page_content`'s overlay vs. `emit_box`'s per-box
        // arms, `lib.rs:566-671`).
        PureHorzBox::Frame { contents, .. } => {
            for (dx, cbx) in contents {
                emit_box(out, cbx, tx + dx.0, ty, ctx);
            }
        }
        _ => {}
    }
}

/// Stack an `EmbeddedBlock`'s already-broken `block` lines from its placed
/// anchor `(tx, ty)`, the HTML twin of `rustyfi-pdf`'s `place_embedded_block`
/// (`lib.rs:226`). **Sign differs from the PDF version on purpose**: a
/// `VertBox`/`PlacedLine`'s own `baseline_y` grows DOWNWARD (the same
/// page-down convention this whole module uses, `pagebreak.rs:13`), so here
/// each later line's growing `baseline_y` delta is ADDED to the page-down
/// `ty` (moving further down the page) — the PDF version SUBTRACTS the same
/// delta because ITS `ty` lives in PDF's y-**up** space, where "further
/// down" means a smaller value.
fn emit_embedded_block(out: &mut String, block: &[VertBox], tx: f64, ty: f64, ctx: &Ctx) {
    let placed = place_block_at((Length::ZERO, Length::ZERO), block.to_vec());
    let Some(first) = placed.first() else {
        return;
    };
    let first_offset = first.baseline_y;
    for line in &placed {
        let y = ty + (line.baseline_y - first_offset).0;
        for (dx, cbx) in &line.contents {
            emit_box(out, cbx, tx + (line.x + *dx).0, y, ctx);
        }
    }
}

/// One `InnerString`/`Math`-glyph-shaped run: an absolutely-positioned
/// `<span>` at its top-left corner `(tx, top)`, sized in CSS `pt` (1:1 with
/// SATySFi points) via `info.size`, with `text` HTML-escaped. Slice 3: when
/// `ctx` carries a real [`TtfFontStore`] (`render_html_fixed_ttf_with`), the span
/// gets an explicit inline `font-family` naming the `@font-face` this
/// document's `<style>` block embeds for `info.font`'s backing file
/// (`Ctx::font_family_for`, which also records the file as used); in
/// base-14 mode (`ctx.fonts` is `None`) this stays Slice 1's behavior
/// exactly — no inline `font-family`, so the `.run` CSS class's generic
/// system serif default applies.
fn emit_run(out: &mut String, info: &HorzStringInfo, text: &str, tx: f64, top: f64, ctx: &Ctx) {
    let size = info.size.0;
    let family_style = match ctx.font_family_for(info.font) {
        Some(family) => format!(" font-family:\"{family}\";"),
        None => String::new(),
    };
    // Non-black only, mirroring both PDF writers' `q…Q`-scoped fill-color
    // guard, so an all-black document's HTML output is unchanged.
    let color_style = if info.color != Color::Gray(0.0) {
        format!(" color:{};", svg::css_color(info.color))
    } else {
        String::new()
    };
    let _ = write!(
        out,
        "<span class=\"run\" style=\"left:{tx}pt; top:{top}pt; font-size:{size}pt;{family_style}{color_style}\">{}</span>\n",
        escape_html(text),
    );
}

/// Escape the five HTML/attribute-hostile characters. Slice 1's `<span>`
/// text is never re-parsed as markup, so this is the standard minimal set
/// (no need for a full entity table).
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
