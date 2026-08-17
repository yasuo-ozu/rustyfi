//! PDF output backend: base-14 Type1 fonts, uncompressed content streams
//! (the milestone-1 replacement for handlePdf.ml on top of `pdf-writer`),
//! plus (phase 5) ttf-parser-backed metrics and CID-keyed TrueType embedding,
//! and (Slice 1, `docs/plans/math-images.md`) raster Image XObjects.

pub mod base14;
pub mod cid;
pub mod fonts;
pub mod ttf;

pub use base14::Base14Metrics;
pub use cid::{render_pdf_ttf, render_pdf_ttf_with};
pub use fonts::{FontConfigError, FontFlags, FontRegistry, FontSource};
pub use ttf::{FontError, TtfFontStore};

use std::collections::{BTreeMap, BTreeSet};

use pdf_writer::types::{ActionType, AnnotationType};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str, TextStr};
use satysfi_backend::{
    place_block_at, Annot, AnnotAction, Closing, Color, DocExtras, DocInfo, GraphicsElem,
    ImageResource, Length, MathGlyph, NamedDest, OutlineEntry, Page, PageGeometry, Path, PathSeg,
    PureHorzBox, VertBox,
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
                scan_box_images(bx, &mut used);
            }
        }
    }
    used
}

/// Recursive box scan for `used_images`: an `Image` can also hide inside a
/// `Tabular` cell, an `EmbeddedBlock`'s stacked lines, (§D) a `Frame`'s
/// contents, or (roadmap C1) a `draw-text` run's `GraphicsElem::Text`
/// contents (`read-inline`d text passed to `draw-text` can itself carry a
/// `use-image-by-width` box) — a pre-existing gap for the first two, closed
/// alongside the new `Frame`/`Graphics` recursion this slice adds.
fn scan_box_images(bx: &PureHorzBox, used: &mut BTreeSet<usize>) {
    match bx {
        PureHorzBox::Image { image, .. } => {
            used.insert(image.0);
        }
        PureHorzBox::Frame { contents, .. } => {
            for (_, b) in contents {
                scan_box_images(b, used);
            }
        }
        PureHorzBox::Graphics { elems, .. } => {
            for elem in elems {
                if let GraphicsElem::Text { contents, .. } = elem {
                    for (_, b) in contents {
                        scan_box_images(b, used);
                    }
                }
            }
        }
        PureHorzBox::Tabular(tab) => {
            for cell in &tab.cells {
                for (_, b) in &cell.contents {
                    scan_box_images(b, used);
                }
            }
        }
        PureHorzBox::EmbeddedBlock { block, .. } => {
            for vb in block {
                if let VertBox::Line { contents, .. } = vb {
                    for (_, b) in contents {
                        scan_box_images(b, used);
                    }
                }
            }
        }
        _ => {}
    }
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
/// `encode` turns one glyph into engine-specific `Tj` bytes: WinAnsi over
/// `glyph.text` for `render_pdf` (below), a glyph-id run for
/// `render_pdf_ttf` (`cid.rs`) that additionally special-cases
/// `glyph.gid.is_some()` (§B3: a raw MATH-table variant glyph, emitted
/// directly rather than re-derived from `text`) — the whole glyph (not just
/// `info`/`text`) is threaded through so `encode` can see `gid`.
pub(crate) fn place_math(
    content: &mut Content,
    glyphs: &[MathGlyph],
    anchor_x: f32,
    anchor_y: f32,
    name_for: &dyn Fn(satysfi_backend::FontKey) -> String,
    mut encode: impl FnMut(&MathGlyph) -> Result<Vec<u8>, PdfError>,
) -> Result<(), PdfError> {
    for glyph in glyphs {
        let encoded = encode(glyph)?;
        let res_name = name_for(glyph.info.font);
        content.begin_text();
        content.set_font(Name(res_name.as_bytes()), glyph.info.size.0 as f32);
        content.next_line(
            anchor_x + glyph.dx.0 as f32,
            anchor_y + glyph.dy.0 as f32 + glyph.info.rising.0 as f32,
        );
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

/// Write one indirect Link annotation object per entry, returning
/// page-index -> the refs that page's `/Annots` array must list.
/// Upstream: `Annotation.of_annotation` + `add_to_pdf` (annotation.ml).
pub(crate) fn write_annotations(
    pdf: &mut Pdf,
    mut next_ref: impl FnMut() -> Ref,
    annots: &[Annot],
    n_pages: usize,
) -> BTreeMap<usize, Vec<Ref>> {
    let mut by_page: BTreeMap<usize, Vec<Ref>> = BTreeMap::new();
    for a in annots {
        if a.page >= n_pages {
            continue; // out-of-range page: skip gracefully (mirrors write_image_xobjects's stance)
        }
        let r = next_ref();
        let mut ann = pdf.annotation(r);
        ann.subtype(AnnotationType::Link);
        let (x1, y1, x2, y2) = a.rect;
        ann.rect(Rect::new(x1.0 as f32, y1.0 as f32, x2.0 as f32, y2.0 as f32));
        // Upstream always writes a border — width 0 when None
        // (annotation.ml's `(Length.zero, None)` arm) — which suppresses the
        // PDF default 1pt border. Match it.
        let width = a.border.as_ref().map(|(w, _)| w.0 as f32).unwrap_or(0.0);
        ann.border(0.0, 0.0, width, None);
        if let Some((_, color)) = &a.border {
            match *color {
                Color::Gray(g) => {
                    ann.color_gray(g as f32);
                }
                Color::Rgb(r, g, b) => {
                    ann.color_rgb(r as f32, g as f32, b as f32);
                }
                Color::Cmyk(c, m, y, k) => {
                    ann.color_cmyk(c as f32, m as f32, y as f32, k as f32);
                }
            }
        }
        let mut act = ann.action();
        match &a.action {
            AnnotAction::Uri(uri) => {
                act.action_type(ActionType::Uri);
                act.uri(Str(uri.as_bytes()));
            }
            AnnotAction::GotoName(name) => {
                act.action_type(ActionType::GoTo);
                act.destination_named(Name(name.as_bytes()));
            }
        }
        act.finish();
        ann.finish();
        by_page.entry(a.page).or_default().push(r);
    }
    by_page
}

/// Write the `/Dests` name dictionary (PDF-1.1-style, exactly upstream
/// namedDest.ml's `Pdf.Dictionary` in the catalog — not a 1.2 name tree).
/// Each value is `[page /XYZ x y 0]`. Returns None when there is nothing to
/// write. Duplicate names: last registration wins (BTreeMap dedupe).
pub(crate) fn write_named_dests(
    pdf: &mut Pdf,
    mut next_ref: impl FnMut() -> Ref,
    dests: &[NamedDest],
    page_ids: &[Ref],
) -> Option<Ref> {
    let mut dedup: BTreeMap<&str, &NamedDest> = BTreeMap::new();
    for d in dests {
        if d.page < page_ids.len() {
            dedup.insert(d.name.as_str(), d);
        }
    }
    if dedup.is_empty() {
        return None;
    }
    let id = next_ref();
    let mut dict = pdf.destinations(id); // TypedDict<Destination>, chunk.rs:236
    for (name, d) in dedup {
        dict.insert(Name(name.as_bytes()))
            .page(page_ids[d.page])
            .xyz(d.x.0 as f32, d.y.0 as f32, None);
    }
    dict.finish();
    Some(id)
}

/// Write the whole `/Outlines` tree from the flat `(level, …)` list
/// (upstream outline.ml via camlpdf's Pdfmarks.add_bookmarks). Nesting is
/// derived from `level` exactly like Pdfmarks: an entry is a child of the
/// nearest preceding entry with a smaller level. `/Count` is the number of
/// descendants, negated when the item is closed (`is_open == false`);
/// the root `/Count` counts top-level items. Returns None when empty.
pub(crate) fn write_outline(
    pdf: &mut Pdf,
    mut next_ref: impl FnMut() -> Ref,
    entries: &[OutlineEntry],
) -> Option<Ref> {
    if entries.is_empty() {
        return None;
    }
    let root_id = next_ref();
    let ids: Vec<Ref> = entries.iter().map(|_| next_ref()).collect();

    // parent[i] / children / prev / next, all by index, from the level walk.
    let mut parent: Vec<Option<usize>> = vec![None; entries.len()];
    let mut stack: Vec<usize> = Vec::new(); // indices of open ancestors
    for i in 0..entries.len() {
        while let Some(&top) = stack.last() {
            if entries[top].level < entries[i].level {
                break;
            }
            stack.pop();
        }
        parent[i] = stack.last().copied();
        stack.push(i);
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut top_level: Vec<usize> = Vec::new();
    for i in 0..entries.len() {
        match parent[i] {
            Some(p) => children[p].push(i),
            None => top_level.push(i),
        }
    }
    // descendants(i) for /Count
    fn descendants(children: &[Vec<usize>], i: usize) -> i32 {
        children[i].iter().map(|&c| 1 + descendants(children, c)).sum()
    }

    {
        let mut root = pdf.outline(root_id);
        root.first(ids[*top_level.first().unwrap()]);
        root.last(ids[*top_level.last().unwrap()]);
        root.count(top_level.len() as i32);
    }
    // Emit each item with Title/Parent/Prev/Next/First/Last/Count/Dest.
    for (i, e) in entries.iter().enumerate() {
        let mut item = pdf.outline_item(ids[i]);
        item.title(TextStr(&e.text));
        item.parent(parent[i].map(|p| ids[p]).unwrap_or(root_id));
        let sibs: &Vec<usize> = match parent[i] {
            Some(p) => &children[p],
            None => &top_level,
        };
        let pos = sibs.iter().position(|&x| x == i).unwrap();
        if pos > 0 {
            item.prev(ids[sibs[pos - 1]]);
        }
        if pos + 1 < sibs.len() {
            item.next(ids[sibs[pos + 1]]);
        }
        if let (Some(&f), Some(&l)) = (children[i].first(), children[i].last()) {
            item.first(ids[f]);
            item.last(ids[l]);
            let n = descendants(&children, i);
            item.count(if e.is_open { n } else { -n });
        }
        item.dest_name(Name(e.dest_name.as_bytes()));
    }
    Some(root_id)
}

/// Emit the PDF `/Info` dictionary at `id` (`prim-retype-sweep §2.4 step
/// 5`) from `register-document-information`'s registered value — shared by
/// both writers (`render_pdf_with` above and `cid::render_pdf_ttf_with`).
/// `pdf.document_info(id)` self-registers with the file trailer (pdf-writer
/// `structure.rs`'s doc comment), so the caller only needs to allocate
/// `id` and call this once, gated on `extras.doc_info.is_some()` (an
/// unregistered document emits no `/Info` object at all, keeping every
/// pre-L5a PDF byte-identical). `/Title`/`/Subject`/`/Author` are written
/// only when `Some`; `/Keywords` only when non-empty, joined with a single
/// space (upstream `String.concat " "`,
/// `documentInformationDictionary.ml`). DOCUMENTED DEVIATION:
/// `/Creator`/`/Producer` are written unconditionally *once this function
/// runs* (i.e. only when the dict is registered at all) — upstream instead
/// emits them on EVERY document (the `/Info` dict always exists there);
/// gating the whole dict on registration is what keeps this slice's
/// byte-identity floor for every existing 0.0.6 fixture (§7 acceptance 4).
pub(crate) fn write_document_info(pdf: &mut Pdf, id: Ref, info: &DocInfo) {
    let mut w = pdf.document_info(id);
    if let Some(title) = &info.title {
        w.title(TextStr(title));
    }
    if let Some(subject) = &info.subject {
        w.subject(TextStr(subject));
    }
    if let Some(author) = &info.author {
        w.author(TextStr(author));
    }
    if !info.keywords.is_empty() {
        let joined = info.keywords.join(" ");
        w.keywords(TextStr(&joined));
    }
    w.creator(TextStr("SATySFi"));
    w.producer(TextStr("SATySFi"));
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
    render_pdf_with(geometry, pages, images, &DocExtras::default())
}

/// Same as [`render_pdf`], but also emits the §B/§C/§D extras (`/Annots`,
/// `/Dests`, `/Outlines`, per-page deco-graphics underlays) accumulated
/// while evaluating the document (`DocumentValue::extras`). `render_pdf`
/// above is a thin wrapper over this with `&DocExtras::default()`, which
/// emits none of the new catalog keys/annotations — every pre-Slice-A
/// document (and every existing test call site) stays byte-identical.
pub fn render_pdf_with(
    geometry: &PageGeometry,
    pages: &[Page],
    images: &[ImageResource],
    extras: &DocExtras,
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

    // §B/§C: link annotations, named destinations, the outline tree.
    let annot_refs = write_annotations(&mut pdf, &mut next_ref, &extras.annotations, pages.len());
    let dests_id = write_named_dests(&mut pdf, &mut next_ref, &extras.destinations, &page_ids);
    let outline_id = write_outline(&mut pdf, &mut next_ref, &extras.outline);
    // prim-retype-sweep §2.4 step 5: `/Info` dict, gated on `Some` so a
    // document that never called `register-document-information` (every
    // pre-L5a document included) emits byte-identical bytes to before this
    // slice.
    if let Some(info) = &extras.doc_info {
        let info_id = next_ref();
        write_document_info(&mut pdf, info_id, info);
    }

    {
        let mut cat = pdf.catalog(catalog_id);
        cat.pages(page_tree_id);
        if let Some(d) = dests_id {
            cat.destinations(d); // /Dests, structure.rs:55
        }
        if let Some(o) = outline_id {
            cat.outlines(o); // /Outlines, structure.rs:62
        }
    }
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

    for (i, ((page, &page_id), &content_id)) in
        pages.iter().zip(&page_ids).zip(&content_ids).enumerate()
    {
        let overlay = extras.page_graphics.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        let content = page_content(page, paper_h, overlay)?;
        pdf.stream(content_id, &content);

        let mut p = pdf.page(page_id);
        p.media_box(media_box);
        p.parent(page_tree_id);
        p.contents(content_id);
        if let Some(refs) = annot_refs.get(&i) {
            p.annotations(refs.iter().copied()); // structure.rs:1227
        }
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
/// page coordinates (downward) to PDF (upward). `overlay` (§D deco
/// graphics, already in absolute PDF y-up page coordinates — see
/// `fire_hooks`) is drawn FIRST, so it sits under the page's text/images
/// (background fills/borders).
fn page_content(page: &Page, paper_h: f32, overlay: &[GraphicsElem]) -> Result<Vec<u8>, PdfError> {
    let mut content = Content::new();
    // `place_graphics` emits its `q`/`cm`/`Q` wrapper UNCONDITIONALLY, even
    // for an empty slice — guard it so an extras-free (or hook-free) page's
    // content stream stays byte-identical to before this slice (§A9's
    // byte-identity floor).
    if !overlay.is_empty() {
        place_graphics(&mut content, overlay, 0.0, 0.0, &mut |c, bx, x, y| emit_box(c, bx, x, y))?;
    }
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
            content.next_line(tx, ty + info.rising.0 as f32);
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
            place_graphics(content, elems, tx, ty, &mut |c, bx, x, y| emit_box(c, bx, x, y))?;
        }
        PureHorzBox::Math { glyphs, rules, .. } => {
            // base-14 never sees `gid: Some(_)` (no provider here overrides
            // `math_vertical_variant`, §B3's zero-regression contract), so
            // this ignores `g.gid` entirely and always encodes `g.text`.
            let name_for = |k: satysfi_backend::FontKey| {
                FONT_RES_NAMES[(k.0 as usize).min(FONT_RES_NAMES.len() - 1)].to_string()
            };
            place_math(content, glyphs, tx, ty, &name_for, |g| winansi(&g.text))?;
            // §B2 (`docs/plans/math-engine.md`): the fraction bar/radical
            // sign+overbar are `Fill`s, not glyphs — placed through the SAME
            // `place_graphics` an `inline-graphics`/`Tabular` box uses, at
            // the SAME already-flipped anchor `place_math` just used for the
            // glyphs (see `place_graphics`'s own doc comment on why no
            // second y-flip belongs here).
            place_graphics(content, rules, tx, ty, &mut |c, bx, x, y| emit_box(c, bx, x, y))?;
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
            place_graphics(content, &tab.rules, tx, ty, &mut |c, bx, x, y| emit_box(c, bx, x, y))?;
        }
        PureHorzBox::EmbeddedBlock { block, .. } => {
            place_embedded_block(block, tx, ty, |cbx, x, y| emit_box(content, cbx, x, y))?;
        }
        // §D: an inline frame's contents, on the frame's own baseline —
        // mirrors the `Tabular` recursion above. The frame's deco graphics
        // are NOT emitted here — they were fired lang-side into
        // `DocExtras::page_graphics` and drawn as the page underlay
        // (`page_content`'s `overlay` prologue).
        PureHorzBox::Frame { contents, .. } => {
            for (dx, cbx) in contents {
                emit_box(content, cbx, tx + dx.0 as f32, ty)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Callback `place_graphics` invokes for each box a `GraphicsElem::Text` run
/// carries, at BOX-LOCAL coordinates — the surrounding `q; cm` translate maps
/// them onto the page, so implementations just call their own `emit_box`
/// unchanged (PDF text/image/path operators all compose with the CTM).
pub(crate) type NestedEmitter<'a> =
    &'a mut dyn FnMut(&mut Content, &PureHorzBox, f32, f32) -> Result<(), PdfError>;

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
///
/// **`GraphicsElem::Text` and the CTM.** A `draw-text` run's boxes are
/// emitted via `emit_nested` at BOX-LOCAL coordinates `(pt.x + dx, pt.y)` —
/// *inside* the `q; cm` translate below, exactly like every other element
/// here — so PDF text ops (`BT`/`Td`/`Tj`) compose with the CTM the same way
/// a filled path does. Adding an absolute anchor or a second y-flip here
/// would double-place the run — don't.
pub(crate) fn place_graphics(
    content: &mut Content,
    elems: &[GraphicsElem],
    tx: f32,
    ty: f32,
    emit_nested: NestedEmitter<'_>,
) -> Result<(), PdfError> {
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
            // `draw-text` (roadmap C1): re-enter the writer's own per-box
            // emission at box-local coordinates `pt + dx` for each box the
            // run carries — see `GraphicsElem::Text`'s doc comment
            // (satysfi-backend/src/graphics.rs) and this function's own
            // "Text and the CTM" note above.
            GraphicsElem::Text { pt, contents, .. } => {
                for (dx, bx) in contents {
                    emit_nested(content, bx, (pt.0 + *dx).0 as f32, pt.1 .0 as f32)?;
                }
            }
            // L5b (prim-retype-sweep.md §3.3): 0.1's `graphics` collection
            // container nodes. Never reached by any 0.0.6 program (no
            // 0.0.6-visible prim constructs `Group`/`Clip` — see
            // `GraphicsElem`'s doc comment); the §4.3 golden-PDF
            // byte-compare is the tripwire that proves it.
            GraphicsElem::Group(inner) => {
                // Recurse with a zero anchor: the outer q/cm(tx,ty) frame
                // above is already active; a nested translate of (0,0) is
                // what upstream's flat `List.concat` renders to.
                place_graphics(content, inner, 0.0, 0.0, &mut *emit_nested)?;
            }
            GraphicsElem::Clip(path, inner) => {
                // `graphicD.ml:323-336`: q; path; W' n; contents; Q — the
                // per-element q…Q wrapper above already provides the q/Q.
                emit_path(content, path);
                content.clip_even_odd();
                content.end_path();
                place_graphics(content, inner, 0.0, 0.0, &mut *emit_nested)?;
            }
        }
        content.restore_state();
    }
    content.restore_state();
    Ok(())
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
