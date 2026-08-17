//! PDF output backend: base-14 Type1 fonts, uncompressed content streams
//! (the milestone-1 replacement for handlePdf.ml on top of `pdf-writer`),
//! plus (phase 5) ttf-parser-backed metrics and CID-keyed TrueType embedding,
//! and (Slice 1, `docs/plans/math-images.md`) raster Image XObjects.

pub mod base14;
pub mod cid;
pub mod fonts;
pub mod ttf;

pub use base14::Base14Metrics;
pub use cid::render_pdf_ttf;
pub use fonts::{FontConfigError, FontFlags, FontRegistry, FontSource};
pub use ttf::{FontError, TtfFontStore};

use std::collections::{BTreeMap, BTreeSet};

use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use satysfi_backend::{
    place_block_at, Closing, Color, GraphicsElem, HorzStringInfo, ImageResource, Length,
    MathGlyph, Page, PageGeometry, Path, PathSeg, PureHorzBox, VertBox,
};

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("text {0:?} is not encodable in WinAnsi (milestone-1 base fonts)")]
    Unencodable(String),
    #[error("no glyph for {0:?} in the embedded font")]
    NoGlyph(char),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Resource names for the three base fonts, indexed by `FontKey`.
const FONT_RES_NAMES: [&str; 3] = ["F0", "F1", "F2"];

// ============================================================================
// Shared Image XObject support (Slice 1: raster images,
// `docs/plans/math-images.md`). `render_pdf` (base-14, below) and
// `render_pdf_ttf` (CID-keyed TrueType, `cid.rs`) are otherwise entirely
// separate writers, but an `Image` box is rendered *identically* by both —
// only text rendering differs between them — so that one path lives here,
// once, and `cid.rs` imports it (`use crate::{..}`), the same way it already
// shares `FONT_RES_NAMES` with this module.
// ============================================================================

/// Every `ImageId` (as its raw `usize` index into a `DocumentValue::images`-
/// shaped table) that appears in at least one placed line across `pages` —
/// the writer only emits an XObject for an image actually placed on a page,
/// not merely decoded (a document can `load-image` something it never
/// places).
fn used_images(pages: &[Page]) -> BTreeSet<usize> {
    let mut used = BTreeSet::new();
    for page in pages {
        for line in &page.lines {
            for (_, bx) in &line.contents {
                if let PureHorzBox::Image { image, .. } = bx {
                    used.insert(image.0);
                }
            }
        }
    }
    used
}

/// The PDF resource name for image `id` (e.g. `Im3`) — shared verbatim by
/// the page's `/Resources /XObject` dictionary entry and the content
/// stream's `Do` operand, which must agree.
fn image_res_name(id: usize) -> String {
    format!("Im{id}")
}

/// Write one Image XObject per id in `used`, returning each id's freshly
/// allocated indirect reference for the caller's `/XObject` resource
/// dictionaries. Slice 1 always flattens to flat, uncompressed 8-bit
/// `DeviceRGB` samples (`ImageResource`'s doc comment in satysfi-backend
/// covers the alpha-dropping/JPEG-`DCTDecode`-passthrough caveats) — no
/// `/Filter` at all, matching this crate's existing "uncompressed content
/// streams" style (see this module's doc comment) rather than adding a
/// `FlateDecode`/`DCTDecode` encoding step.
fn write_image_xobjects(
    pdf: &mut Pdf,
    mut next_ref: impl FnMut() -> Ref,
    images: &[ImageResource],
    used: &BTreeSet<usize>,
) -> BTreeMap<usize, Ref> {
    let mut refs = BTreeMap::new();
    for &id in used {
        let Some(im) = images.get(id) else {
            // A box referenced an id past the end of the document's image
            // table — should not happen (every `ImageId` a box carries came
            // from a successful `load-image` push), but a page silently
            // missing one image is a far more graceful failure than a panic.
            continue;
        };
        let r = next_ref();
        refs.insert(id, r);
        let mut xo = pdf.image_xobject(r, &im.samples);
        xo.width(im.px_w as i32);
        xo.height(im.px_h as i32);
        xo.color_space().device_rgb();
        xo.bits_per_component(8);
        xo.finish();
    }
    refs
}

/// Emit the content-stream operators that place one Image box: `q  w 0 0 h
/// tx ty cm  /ImN Do  Q` (v0.0.6 `graphicD.ml`'s `pdfops_of_image`).
///
/// `ty` is the same already-flipped (page-down to PDF-up) baseline
/// y-coordinate a text run on this line uses for its `Td`/`next_line` — not
/// a separate computation — because a `pdf-writer` image XObject's unit
/// square is placed with its *bottom-left* corner at the `cm` matrix's
/// translation, and `PureHorzBox::Image` sits entirely above the baseline
/// (all height, zero depth, per `linebreak.rs`'s `layout_line`): the
/// baseline *is* the image's bottom edge.
fn place_image(content: &mut Content, id: usize, tx: f32, ty: f32, width: f32, height: f32) {
    content.save_state();
    content.transform([width, 0.0, 0.0, height, tx, ty]);
    content.x_object(Name(image_res_name(id).as_bytes()));
    content.restore_state();
}

/// Emit one laid-out `PureHorzBox::Math` run's glyphs as a `BT / Tf / Td /
/// Tj / ET` group per glyph — the whole point of the `Math` box
/// (`docs/plans/math-engine.md` §Slice 1) being that each glyph already
/// carries its own font/size (`glyph.info`) and offset (`glyph.dx`/`dy`)
/// relative to the box's placed anchor `(anchor_x, anchor_y)`, the same
/// already-flipped `(line.x + dx, paper_h - baseline_y)` a text run's `Td`
/// uses. `glyph.dy > 0` raises it (a superscript) since PDF y is up — no
/// second flip here, only an add.
///
/// `encode` turns one glyph's `text` into engine-specific `Tj` bytes: WinAnsi
/// for `render_pdf` (below), a glyph-id run for `render_pdf_ttf` (`cid.rs`) —
/// the one thing that differs between the two writers.
pub(crate) fn place_math(
    content: &mut Content,
    glyphs: &[MathGlyph],
    anchor_x: f32,
    anchor_y: f32,
    mut encode: impl FnMut(&HorzStringInfo, &str) -> Result<Vec<u8>, PdfError>,
) -> Result<(), PdfError> {
    for glyph in glyphs {
        let encoded = encode(&glyph.info, &glyph.text)?;
        let font_idx = (glyph.info.font.0 as usize).min(FONT_RES_NAMES.len() - 1);
        content.begin_text();
        content.set_font(
            Name(FONT_RES_NAMES[font_idx].as_bytes()),
            glyph.info.size.0 as f32,
        );
        content.next_line(anchor_x + glyph.dx.0 as f32, anchor_y + glyph.dy.0 as f32);
        content.show(Str(&encoded));
        content.end_text();
    }
    Ok(())
}

/// Stack an `EmbeddedBlock`'s already-broken `block` lines from its placed
/// anchor `(tx, ty)` (`docs/plans/context-box-prims.md` §3) — shared by both
/// writers the same way `place_graphics`/`place_math` are, with the *text*
/// emission (the one thing that differs between them) threaded through as
/// the `emit_line` callback (exactly `emit_box`'s own `Tabular` recursion,
/// one level up).
///
/// **Top-aligned stand-in.** `place_block_at` (satysfi-backend; also used
/// for page headers/footers) lays `block` out from a fixed `(0, 0)`
/// page-y-down origin; this shifts that whole stack so the FIRST content
/// line's baseline sits exactly at the anchor `ty` (mirroring how every
/// other `PureHorzBox` reports its own first line's ascent as `height`),
/// with every later line falling further down the page (subtracted from
/// `ty`, since PDF y is up). `embed-block-top`'s `adjust_to_first_line`
/// (exact upstream baseline alignment) is the faithful refinement; see that
/// primitive's doc comment (`satysfi-lang`).
pub(crate) fn place_embedded_block(
    block: &[VertBox],
    tx: f32,
    ty: f32,
    mut emit_line: impl FnMut(&PureHorzBox, f32, f32) -> Result<(), PdfError>,
) -> Result<(), PdfError> {
    let placed = place_block_at((Length::ZERO, Length::ZERO), block.to_vec());
    let Some(first) = placed.first() else {
        return Ok(());
    };
    let first_offset = first.baseline_y;
    for line in &placed {
        let y = ty - (line.baseline_y - first_offset).0 as f32;
        for (dx, cbx) in &line.contents {
            emit_line(cbx, tx + (line.x + *dx).0 as f32, y)?;
        }
    }
    Ok(())
}

/// Serialize typeset pages into a complete PDF document. `images` is the
/// document-wide image table (`DocumentValue::images`); pass `&[]` for a
/// text-only document (nothing in `pages` can reference an id past the end
/// of an empty table, so this is exactly as cheap and byte-identical to the
/// pre-Slice-1 output as before this parameter existed).
pub fn render_pdf(
    geometry: &PageGeometry,
    pages: &[Page],
    images: &[ImageResource],
) -> Result<Vec<u8>, PdfError> {
    let mut pdf = Pdf::new();
    let mut alloc = 1;
    let mut next_ref = || {
        let r = Ref::new(alloc);
        alloc += 1;
        r
    };

    let catalog_id = next_ref();
    let page_tree_id = next_ref();
    let font_ids: Vec<Ref> = (0..3).map(|_| next_ref()).collect();

    // One Image XObject per image actually placed on a page (Slice 1:
    // raster images, docs/plans/math-images.md).
    let used = used_images(pages);
    let img_refs = write_image_xobjects(&mut pdf, &mut next_ref, images, &used);

    let page_ids: Vec<Ref> = pages.iter().map(|_| next_ref()).collect();
    let content_ids: Vec<Ref> = pages.iter().map(|_| next_ref()).collect();

    pdf.catalog(catalog_id).pages(page_tree_id);
    {
        let mut tree = pdf.pages(page_tree_id);
        tree.kids(page_ids.iter().copied());
        tree.count(page_ids.len() as i32);
    }

    for (i, name) in base14::BASE_FONT_NAMES.iter().enumerate() {
        let mut font = pdf.type1_font(font_ids[i]);
        font.base_font(Name(name.as_bytes()));
        font.encoding_predefined(Name(b"WinAnsiEncoding"));
    }

    let paper_h = geometry.paper_height.0 as f32;
    let media_box = Rect::new(0.0, 0.0, geometry.paper_width.0 as f32, paper_h);

    for ((page, &page_id), &content_id) in pages.iter().zip(&page_ids).zip(&content_ids) {
        let content = page_content(page, paper_h)?;
        pdf.stream(content_id, &content);

        let mut p = pdf.page(page_id);
        p.media_box(media_box);
        p.parent(page_tree_id);
        p.contents(content_id);
        let mut resources = p.resources();
        let mut fonts = resources.fonts();
        for (i, res_name) in FONT_RES_NAMES.iter().enumerate() {
            fonts.pair(Name(res_name.as_bytes()), font_ids[i]);
        }
        fonts.finish();
        // Registered on every page uniformly, the same simplification the
        // three base fonts above already make (a page that never actually
        // shows a given image just leaves its resource entry unused, which
        // PDF permits).
        if !img_refs.is_empty() {
            let mut x_objects = resources.x_objects();
            for (&id, &r) in &img_refs {
                x_objects.pair(Name(image_res_name(id).as_bytes()), r);
            }
            x_objects.finish();
        }
        resources.finish();
        p.finish();
    }

    Ok(pdf.finish())
}

/// Build one page's content stream: `BT … Tf … Td … Tj … ET` runs for text
/// and `q … cm /ImN Do Q` runs for images, with the y axis flipped from
/// page coordinates (downward) to PDF (upward).
fn page_content(page: &Page, paper_h: f32) -> Result<Vec<u8>, PdfError> {
    let mut content = Content::new();
    for line in &page.lines {
        let y = paper_h - line.baseline_y.0 as f32;
        for (dx, bx) in &line.contents {
            emit_box(&mut content, bx, (line.x + *dx).0 as f32, y)?;
        }
    }
    Ok(content.finish().into_vec())
}

/// Emit one already-placed `PureHorzBox` at absolute PDF-space coordinates
/// `(tx, ty)` — `tx` the box's left edge, `ty` its baseline, both already in
/// PDF's y-**up** space (the page-level flip, `y = paper_h - baseline_y`,
/// already happened in the caller's `ty`). Factored out of `page_content`
/// (docs/plans/table-subsystem.md §4) so it is **reentrant**: a `Tabular`
/// box's cells hold their own already-laid-out `PureHorzBox` runs, and this
/// is the same path a top-level line uses to emit them, recursively (so a
/// nested table inside a cell just works).
///
/// **The three y-frames, reconciled in one expression.** Page layout is
/// y-down (flipped into `ty` by the caller, once); a `Tabular` box's own
/// coordinate frame is y-**up** from its own baseline-left origin; a cell's
/// `baseline_y` is measured y-up from *that* origin. Since `ty` is already
/// the box's *own* placed baseline in PDF y-up space, `ty + cell.baseline_y`
/// is exactly the cell's absolute baseline — no second flip. The rules
/// (`tab.rules`) go through the **existing** `place_graphics`, whose own
/// `cm` translate to `(tx, ty)` positions its box-local y-up path
/// coordinates the identical way a standalone `inline-graphics` box does —
/// cell text and rules land in the same frame this way.
fn emit_box(content: &mut Content, bx: &PureHorzBox, tx: f32, ty: f32) -> Result<(), PdfError> {
    match bx {
        PureHorzBox::InnerString { info, text, .. } => {
            let encoded = winansi(text)?;
            let font_idx = (info.font.0 as usize).min(FONT_RES_NAMES.len() - 1);
            content.begin_text();
            content.set_font(
                Name(FONT_RES_NAMES[font_idx].as_bytes()),
                info.size.0 as f32,
            );
            content.next_line(tx, ty);
            content.show(Str(&encoded));
            content.end_text();
        }
        PureHorzBox::Image {
            width,
            height,
            image,
        } => {
            place_image(content, image.0, tx, ty, width.0 as f32, height.0 as f32);
        }
        PureHorzBox::Graphics { elems, .. } => {
            place_graphics(content, elems, tx, ty);
        }
        PureHorzBox::Math { glyphs, .. } => {
            place_math(content, glyphs, tx, ty, |_info, text| winansi(text))?;
        }
        PureHorzBox::Tabular(tab) => {
            for cell in &tab.cells {
                for (cdx, cbx) in &cell.contents {
                    emit_box(
                        content,
                        cbx,
                        tx + (cell.x + *cdx).0 as f32,
                        ty + cell.baseline_y.0 as f32,
                    )?;
                }
            }
            place_graphics(content, &tab.rules, tx, ty);
        }
        PureHorzBox::EmbeddedBlock { block, .. } => {
            place_embedded_block(block, tx, ty, |cbx, x, y| emit_box(content, cbx, x, y))?;
        }
        _ => {}
    }
    Ok(())
}

/// Emit `elems` (already resolved to box-local coordinates — see
/// `PureHorzBox::Graphics`) into `content`, wrapped in one `save_state`/
/// `transform`/`restore_state` that translates the whole box to its placed
/// PDF-space anchor `(tx, ty)` — the **same** already-flipped
/// `(line.x + dx, paper_h - baseline_y)` anchor a text run on that line uses
/// (`page_content` above), so element coordinates stay box-local (exactly
/// `graphicD.ml`'s per-box `cm` wrapping). Shared by both `render_pdf`
/// (base-14, this module) and `render_pdf_ttf` (CID TrueType, `cid.rs`) — a
/// graphics box renders identically regardless of which font backend the
/// rest of the page uses, since path ops carry no font/text state.
///
/// **Coordinate space.** SATySFi graphics are y-**up** (PDF-native) inside a
/// `Path`/`Subpath`'s own coordinates, but *page* layout is y-**down**
/// (`y = paper_h - baseline_y`); that flip already happened in the anchor
/// `(tx, ty)` passed in here, via the `cm` translate below — never per
/// coordinate — so a naive per-coordinate re-flip inside `emit_path` would
/// mirror every path vertically. Don't add one.
pub(crate) fn place_graphics(content: &mut Content, elems: &[GraphicsElem], tx: f32, ty: f32) {
    content.save_state();
    content.transform([1.0, 0.0, 0.0, 1.0, tx, ty]);
    for elem in elems {
        content.save_state();
        match elem {
            // Upstream fills with the even-odd rule (`op_f'`,
            // `graphicD.ml:246`), not nonzero-winding — matters for
            // self-intersecting/nested subpaths (e.g. a frame = outer ⊕
            // inner rectangle).
            GraphicsElem::Fill(color, path) => {
                set_fill_color(content, *color);
                emit_path(content, path);
                content.fill_even_odd();
            }
            GraphicsElem::Stroke(width, color, path) => {
                set_stroke_color(content, *color);
                content.set_line_width(width.0 as f32);
                emit_path(content, path);
                content.stroke();
            }
            // `dashed-stroke`: identical to `Stroke` plus a `d` dash-array op
            // (upstream `pdfops_of_dashed_stroke`, `graphicD.ml:231`).
            GraphicsElem::DashedStroke(width, dash, color, path) => {
                set_stroke_color(content, *color);
                content.set_line_width(width.0 as f32);
                content.set_dash_pattern([dash.0 .0 as f32, dash.1 .0 as f32], dash.2 .0 as f32);
                emit_path(content, path);
                content.stroke();
            }
            // `draw-text` STAND-IN (see `GraphicsElem::Text`'s doc comment,
            // satysfi-backend/src/graphics.rs): the anchor point carries no
            // renderable content, so this emits nothing.
            GraphicsElem::Text(_) => {}
        }
        content.restore_state();
    }
    content.restore_state();
}

fn set_fill_color(content: &mut Content, color: Color) {
    match color {
        Color::Gray(g) => content.set_fill_gray(g as f32),
        Color::Rgb(r, g, b) => content.set_fill_rgb(r as f32, g as f32, b as f32),
        Color::Cmyk(c, m, y, k) => content.set_fill_cmyk(c as f32, m as f32, y as f32, k as f32),
    };
}

fn set_stroke_color(content: &mut Content, color: Color) {
    match color {
        Color::Gray(g) => content.set_stroke_gray(g as f32),
        Color::Rgb(r, g, b) => content.set_stroke_rgb(r as f32, g as f32, b as f32),
        Color::Cmyk(c, m, y, k) => {
            content.set_stroke_cmyk(c as f32, m as f32, y as f32, k as f32)
        }
    };
}

/// Emit one `Path`'s subpaths as `m`/`l`/`c`/`h` operators (per
/// `graphicD.ml`'s `pdfops_of_path`): `move_to(start)`, then each `PathSeg`
/// as `line_to`/`cubic_to`, then the closing — `Open` emits nothing,
/// `Line` emits `close_path` (`h`), `Bezier(c1, c2)` emits a final
/// `cubic_to(c1, c2, start)` then `close_path`.
fn emit_path(content: &mut Content, path: &Path) {
    for sub in &path.subpaths {
        content.move_to(sub.start.0 .0 as f32, sub.start.1 .0 as f32);
        for seg in &sub.segs {
            match seg {
                PathSeg::Line(pt) => {
                    content.line_to(pt.0 .0 as f32, pt.1 .0 as f32);
                }
                PathSeg::Bezier(c1, c2, dest) => {
                    content.cubic_to(
                        c1.0 .0 as f32,
                        c1.1 .0 as f32,
                        c2.0 .0 as f32,
                        c2.1 .0 as f32,
                        dest.0 .0 as f32,
                        dest.1 .0 as f32,
                    );
                }
            }
        }
        match sub.closing {
            Closing::Open => {}
            Closing::Line => {
                content.close_path();
            }
            Closing::Bezier(c1, c2) => {
                content.cubic_to(
                    c1.0 .0 as f32,
                    c1.1 .0 as f32,
                    c2.0 .0 as f32,
                    c2.1 .0 as f32,
                    sub.start.0 .0 as f32,
                    sub.start.1 .0 as f32,
                );
                content.close_path();
            }
        }
    }
}

/// Encode to WinAnsi. Milestone 1 accepts ASCII 32..=126 (what the metrics
/// tables cover); anything else is a polite error rather than mojibake.
fn winansi(text: &str) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        let code = c as u32;
        if (32..=126).contains(&code) {
            out.push(code as u8);
        } else {
            return Err(PdfError::Unencodable(text.to_string()));
        }
    }
    Ok(out)
}
