//! CID-keyed (Type0/CIDFontType2) TrueType embedding, with a required
//! `ToUnicode` CMap so `pdftotext`-style extraction keeps working. This is
//! the phase-5 sibling of `render_pdf` (base-14 Type1): it shares the page
//! geometry / line-layout plumbing but writes Identity-H glyph-index content
//! streams against real embedded font files instead of WinAnsi bytes against
//! the built-in base-14 fonts.
//!
//! Deferred to a later phase: subsetting (fonts are embedded whole — see the
//! comment on `write_font`), kerning/shaping (advances come straight from
//! `FontMetrics`, matching the milestone-1 line breaker), and CFF/OpenType-CFF
//! outlines (`CIDFontType0`) — only TrueType outlines (`CIDFontType2`) are
//! handled.

use std::collections::BTreeMap;

use pdf_writer::types::{CidFontType, FontFlags};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use pdf_writer::types::{SystemInfo, UnicodeCmap};
use satysfi_backend::{FontKey, ImageResource, Page, PageGeometry, PureHorzBox};
use ttf_parser::{Face, GlyphId};

use crate::ttf::TtfFontStore;
use crate::{
    image_res_name, place_embedded_block, place_graphics, place_image, place_math, used_images,
    write_image_xobjects, PdfError, FONT_RES_NAMES,
};

/// Which glyphs of one physical font file are referenced anywhere in the
/// document, and the (first-seen) source character for each — enough to
/// build both the `/W` widths array and the `ToUnicode` CMap without
/// embedding glyphs nobody uses.
#[derive(Default)]
struct FontUsage {
    /// gid -> first char that produced it.
    glyphs: BTreeMap<u16, char>,
}

/// Serialize typeset pages into a PDF that embeds real TrueType fonts as
/// CID-keyed (Type0/CIDFontType2) fonts, Identity-H encoded. `images` is the
/// document-wide image table (`DocumentValue::images`; Slice 1, raster
/// images) — pass `&[]` for a text-only document.
pub fn render_pdf_ttf(
    geometry: &PageGeometry,
    pages: &[Page],
    store: &TtfFontStore,
    images: &[ImageResource],
) -> Result<Vec<u8>, PdfError> {
    let paper_h = geometry.paper_height.0 as f32;

    // Pass 1: build each page's content stream (Identity-H glyph-id runs
    // plus `Do`-invoked images), recording which glyphs of which physical
    // font file were used.
    let mut usage: BTreeMap<usize, FontUsage> = BTreeMap::new();
    let mut page_contents = Vec::with_capacity(pages.len());
    for page in pages {
        page_contents.push(page_content(page, paper_h, store, &mut usage)?);
    }

    let mut pdf = Pdf::new();
    let mut alloc: i32 = 1;
    let next_ref = |alloc: &mut i32| {
        let r = Ref::new(*alloc);
        *alloc += 1;
        r
    };

    let catalog_id = next_ref(&mut alloc);
    let page_tree_id = next_ref(&mut alloc);

    // One Type0 font object per *used* physical font file (dedup point for
    // bold/oblique falling back to regular, and for skipping files that
    // never appear in the document at all).
    let mut type0_ids: BTreeMap<usize, Ref> = BTreeMap::new();
    for &file_idx in usage.keys() {
        type0_ids.insert(file_idx, next_ref(&mut alloc));
    }

    // One Image XObject per image actually placed on a page — shared with
    // `render_pdf` (base-14, `lib.rs`); see that module's doc comment on
    // this section.
    let used = used_images(pages);
    let img_refs = write_image_xobjects(&mut pdf, || next_ref(&mut alloc), images, &used);

    let page_ids: Vec<Ref> = pages.iter().map(|_| next_ref(&mut alloc)).collect();
    let content_ids: Vec<Ref> = pages.iter().map(|_| next_ref(&mut alloc)).collect();

    pdf.catalog(catalog_id).pages(page_tree_id);
    {
        let mut tree = pdf.pages(page_tree_id);
        tree.kids(page_ids.iter().copied());
        tree.count(page_ids.len() as i32);
    }

    for (&file_idx, file_usage) in &usage {
        write_font(
            &mut pdf,
            &mut alloc,
            type0_ids[&file_idx],
            store,
            file_idx,
            file_usage,
        )?;
    }

    let media_box = Rect::new(0.0, 0.0, geometry.paper_width.0 as f32, paper_h);

    for ((&page_id, &content_id), content_bytes) in
        page_ids.iter().zip(&content_ids).zip(&page_contents)
    {
        pdf.stream(content_id, content_bytes);

        let mut p = pdf.page(page_id);
        p.media_box(media_box);
        p.parent(page_tree_id);
        p.contents(content_id);
        let mut resources = p.resources();
        let mut fonts = resources.fonts();
        for (i, res_name) in FONT_RES_NAMES.iter().enumerate() {
            let file_idx = store.file_index(FontKey(i as u16));
            if let Some(&font_ref) = type0_ids.get(&file_idx) {
                fonts.pair(Name(res_name.as_bytes()), font_ref);
            }
        }
        fonts.finish();
        // Registered on every page uniformly, the same simplification the
        // per-`FontKey` loop above already makes — see `render_pdf`'s
        // (`lib.rs`) matching comment.
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

/// Build one page's content stream. Structurally identical to `base14`'s
/// (`BT … Tf … Td … Tj … ET` per text run, `q … cm /ImN Do Q` per image, y
/// flipped to PDF's upward axis), except each `Tj` operand is a run of
/// 2-byte big-endian glyph IDs (Identity-H) rather than WinAnsi bytes — the
/// backend's x-offsets are authoritative, so no kerning/shaping is applied
/// here beyond what `FontMetrics` already measured. Image placement
/// (`place_image`, `crate::lib`) is identical between the two writers.
fn page_content(
    page: &Page,
    paper_h: f32,
    store: &TtfFontStore,
    usage: &mut BTreeMap<usize, FontUsage>,
) -> Result<Vec<u8>, PdfError> {
    let mut content = Content::new();
    for line in &page.lines {
        let y = paper_h - line.baseline_y.0 as f32;
        for (dx, bx) in &line.contents {
            emit_box(&mut content, bx, (line.x + *dx).0 as f32, y, store, usage)?;
        }
    }
    Ok(content.finish().into_vec())
}

/// Emit one already-placed `PureHorzBox` at absolute PDF-space coordinates
/// `(tx, ty)` — the CID-writer twin of `crate::emit_box` (base-14, `lib.rs`),
/// factored out for the same reason (docs/plans/table-subsystem.md §4):
/// reentrant, so a `Tabular` box's cells emit through the same path a
/// top-level line uses, recursively. Text emission is the one thing that
/// differs between the two writers (an `encode_glyph_run` Identity-H run
/// with per-file `usage` tracking here, vs. base-14's WinAnsi `Tj`), so this
/// threads `store`/`usage` where `crate::emit_box` doesn't need to.
fn emit_box(
    content: &mut Content,
    bx: &PureHorzBox,
    tx: f32,
    ty: f32,
    store: &TtfFontStore,
    usage: &mut BTreeMap<usize, FontUsage>,
) -> Result<(), PdfError> {
    match bx {
        PureHorzBox::InnerString { info, text, .. } => {
            let file_idx = store.file_index(info.font);
            let face = store
                .face_by_file(file_idx)
                .ok_or_else(|| PdfError::NoGlyph(text.chars().next().unwrap_or('\u{FFFD}')))?;
            let file_usage = usage.entry(file_idx).or_default();
            let encoded = encode_glyph_run(&face, text, file_usage)?;

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
        PureHorzBox::Math { glyphs, rules, .. } => {
            place_math(content, glyphs, tx, ty, |g| {
                let file_idx = store.file_index(g.info.font);
                let file_usage = usage.entry(file_idx).or_default();
                match g.gid {
                    // §B3: a raw MATH-table variant glyph id
                    // (`push_big_char_glyph`/`push_delimiter_glyph`) — not
                    // necessarily cmap-reachable from `g.text`, so emit it
                    // directly (Identity-H: content bytes ARE gids) rather
                    // than re-deriving a gid through `glyph_index`.
                    Some(gid) => {
                        file_usage
                            .glyphs
                            .entry(gid)
                            .or_insert(g.text.chars().next().unwrap_or('\u{FFFD}'));
                        Ok(gid.to_be_bytes().to_vec())
                    }
                    None => {
                        let face = store.face_by_file(file_idx).ok_or_else(|| {
                            PdfError::NoGlyph(g.text.chars().next().unwrap_or('\u{FFFD}'))
                        })?;
                        encode_glyph_run(&face, &g.text, file_usage)
                    }
                }
            })?;
            // §B2: same fraction-bar/radical-sign `Fill`s as `render_pdf`'s
            // base-14 writer (`lib.rs`'s Math arm) — the CID writer shares
            // `place_graphics` unchanged, since a filled path carries no
            // font/text state either writer needs to specialize.
            place_graphics(content, rules, tx, ty);
        }
        PureHorzBox::Tabular(tab) => {
            for cell in &tab.cells {
                for (cdx, cbx) in &cell.contents {
                    emit_box(
                        content,
                        cbx,
                        tx + (cell.x + *cdx).0 as f32,
                        ty + cell.baseline_y.0 as f32,
                        store,
                        usage,
                    )?;
                }
            }
            place_graphics(content, &tab.rules, tx, ty);
        }
        PureHorzBox::EmbeddedBlock { block, .. } => {
            place_embedded_block(block, tx, ty, |cbx, x, y| {
                emit_box(content, cbx, x, y, store, usage)
            })?;
        }
        _ => {}
    }
    Ok(())
}

/// Map `text` to a run of 2-byte big-endian glyph IDs, recording each glyph
/// (and the first character that produced it) in `usage` for the `/W` array
/// and `ToUnicode` CMap built later.
fn encode_glyph_run(
    face: &Face<'_>,
    text: &str,
    usage: &mut FontUsage,
) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for c in text.chars() {
        let gid = face.glyph_index(c).ok_or(PdfError::NoGlyph(c))?;
        usage.glyphs.entry(gid.0).or_insert(c);
        out.extend_from_slice(&gid.0.to_be_bytes());
    }
    Ok(out)
}

/// Write the Type0 font, its CIDFontType2 descendant, FontDescriptor,
/// FontFile2 and ToUnicode CMap for one physical font file.
///
/// The font file is embedded in full (`FontFile2` holds the whole input
/// file's bytes): subsetting is deferred to a later phase, so a document
/// that uses only a handful of glyphs from e.g. a ~700 KB TrueType face still
/// pays for the whole face in the output PDF. Only the metrics tables
/// (`/W`, `ToUnicode`) are trimmed to the glyphs actually used.
fn write_font(
    pdf: &mut Pdf,
    alloc: &mut i32,
    type0_ref: Ref,
    store: &TtfFontStore,
    file_idx: usize,
    usage: &FontUsage,
) -> Result<(), PdfError> {
    let mut next_ref = || {
        let r = Ref::new(*alloc);
        *alloc += 1;
        r
    };
    let cid_font_ref = next_ref();
    let descriptor_ref = next_ref();
    let font_file_ref = next_ref();
    let to_unicode_ref = next_ref();

    let face = store
        .face_by_file(file_idx)
        .expect("file_idx came from a successfully-loaded TtfFontStore");
    let units_per_em = face.units_per_em() as f64;
    // PDF CID widths (and FontDescriptor metrics) are always expressed in
    // 1000-units-per-em glyph space, regardless of the font's own
    // `unitsPerEm` (DejaVu et al. use 2048).
    let scale = |v: f64| (v * 1000.0 / units_per_em) as f32;

    let base_name = base_font_name(&face, file_idx);

    // --- ToUnicode CMap (required: this is what keeps text extraction
    // working for an Identity-H-encoded, glyph-indexed content stream). ---
    let mut cmap = UnicodeCmap::new(
        Name(b"Custom-UCS"),
        SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"UCS"),
            supplement: 0,
        },
    );
    for (&gid, &ch) in &usage.glyphs {
        cmap.pair(gid, ch);
    }
    let cmap_bytes = cmap.finish();
    pdf.cmap(to_unicode_ref, &cmap_bytes);

    // --- Type0 (composite) font. ---
    {
        let mut t0 = pdf.type0_font(type0_ref);
        t0.base_font(Name(base_name.as_bytes()));
        t0.encoding_predefined(Name(b"Identity-H"));
        t0.descendant_font(cid_font_ref);
        t0.to_unicode(to_unicode_ref);
    }

    // --- CIDFontType2 descendant: CIDToGIDMap=Identity (we hand out raw
    // TrueType glyph indices as CIDs directly, since we never subset/remap
    // them), CIDSystemInfo Adobe-Identity-0 (no predefined CJK ordering —
    // CIDs are just glyph indices). ---
    let widths: BTreeMap<u16, f32> = usage
        .glyphs
        .keys()
        .map(|&gid| {
            let advance = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f64;
            (gid, scale(advance))
        })
        .collect();
    let default_width = if widths.is_empty() {
        1000.0
    } else {
        widths.values().sum::<f32>() / widths.len() as f32
    };
    {
        let mut cid = pdf.cid_font(cid_font_ref);
        cid.subtype(CidFontType::Type2);
        cid.base_font(Name(base_name.as_bytes()));
        cid.system_info(SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        });
        cid.font_descriptor(descriptor_ref);
        cid.default_width(default_width);
        cid.cid_to_gid_map_predefined(Name(b"Identity"));

        if !widths.is_empty() {
            let mut w = cid.widths();
            write_width_runs(&mut w, &widths);
            w.finish();
        }
    }

    // --- FontDescriptor. ---
    {
        let bbox_units = face.global_bounding_box();
        let bbox = Rect::new(
            scale(bbox_units.x_min as f64),
            scale(bbox_units.y_min as f64),
            scale(bbox_units.x_max as f64),
            scale(bbox_units.y_max as f64),
        );

        let mut flags = FontFlags::empty();
        if face.is_italic() {
            flags |= FontFlags::ITALIC;
        }
        if face.is_monospaced() {
            flags |= FontFlags::FIXED_PITCH;
        }
        // Descendant CID fonts are addressed by glyph index, not by a
        // standard Latin character encoding, so mark them Symbolic — the
        // same convention other PDF producers use for Identity-H CID fonts
        // regardless of the face's actual glyph repertoire.
        flags |= FontFlags::SYMBOLIC;

        let mut fd = pdf.font_descriptor(descriptor_ref);
        fd.name(Name(base_name.as_bytes()));
        fd.flags(flags);
        fd.bbox(bbox);
        fd.italic_angle(face.italic_angle());
        fd.ascent(scale(face.ascender() as f64));
        fd.descent(scale(face.descender() as f64));
        let cap_height = face.capital_height().unwrap_or(face.ascender());
        fd.cap_height(scale(cap_height as f64));
        // ttf-parser exposes no direct equivalent of Type1's /StemV; 80/120
        // is the regular/bold heuristic several PDF producers (e.g. pdfTeX)
        // fall back to when a face doesn't carry real stem-width data.
        fd.stem_v(if face.is_bold() { 120.0 } else { 80.0 });
        fd.font_file2(font_file_ref);
    }

    // --- FontFile2: the whole input font file, uncompressed (see the
    // module-level doc comment re: subsetting). ---
    pdf.stream(font_file_ref, store.file_bytes(file_idx));

    Ok(())
}

/// Write a `/W` array as a run of `Widths::consecutive` calls over
/// consecutive glyph IDs, which is both compact and simple to build from a
/// sorted `gid -> width` map.
fn write_width_runs(w: &mut pdf_writer::writers::Widths<'_>, widths: &BTreeMap<u16, f32>) {
    let mut iter = widths.iter().peekable();
    while let Some((&start, &start_w)) = iter.next() {
        let mut run = vec![start_w];
        let mut prev = start;
        while let Some(&(&next_gid, &next_w)) = iter.peek() {
            if next_gid == prev + 1 {
                run.push(next_w);
                prev = next_gid;
                iter.next();
            } else {
                break;
            }
        }
        w.consecutive(start, run);
    }
}

/// A PostScript-ish base font name for the `/BaseFont` entries. Real PDF
/// readers don't need this to resolve to anything (the font is embedded), it
/// is purely informational, so we use the face's own name table when present
/// and fall back to a synthetic tag derived from the physical file's slot.
fn base_font_name(face: &Face<'_>, file_idx: usize) -> String {
    for name in face.names() {
        if name.is_unicode() {
            if let Some(s) = name.to_string() {
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    format!("EmbeddedTTF{file_idx}")
}
