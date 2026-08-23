//! CID-keyed (Type0/CIDFontType2 for `glyf`, Type0/CIDFontType0 for CFF)
//! font embedding, with a required `ToUnicode` CMap so `pdftotext`-style
//! extraction keeps working. The sibling of `render_pdf` (base-14 Type1): it
//! shares the page geometry / line-layout plumbing but writes Identity-H
//! glyph-index content streams against real embedded font files.
//!
//! This subsets a `glyf`-outline face's `FontFile2` down to the glyphs the
//! document actually used, via a translating `/CIDToGIDMap` stream (see
//! `write_font`): raw original gid stays the CID, only the embedded font's
//! OWN internal glyph ids get renumbered.
//!
//! CFF/OpenType-CFF outlines take a second path
//! (`CIDFontType0`/`/FontFile3`): a CFF face's `FontFile2` embed is invalid
//! PDF.
//!
//! **Real CFF subsetting via `subsetter`.** `CIDFontType0` has no
//! `/CIDToGIDMap` to absorb subset renumbering, and `subsetter`'s CFF output
//! is CID-keyed with CID == new gid (crate docs), so the CONTENT STREAM
//! itself must emit the REMAPPED CID for a subsetted CFF file's runs. That
//! needs the per-file `GlyphRemapper` to exist *before* content is
//! generated, hence a two-pass split: `render_pdf_ttf_with` walks every page
//! once purely to collect `usage` (glyph-id sets per physical file,
//! discarding that pass's content bytes), builds a `subsetter::subset` +
//! `GlyphRemapper` for every *used* CFF file, then walks every page a
//! *second* time for the real content streams, threading a `cid_remaps:
//! &BTreeMap<usize, &subsetter::GlyphRemapper>` (populated only for CFF
//! files whose subset attempt succeeded) so a CFF-backed run emits
//! `remapper.get(original_gid)`. `usage` itself stays keyed by the ORIGINAL
//! face gid in both passes (what `face.glyph_index`/hmtx naturally give) —
//! `write_font_cff` re-keys `/W` and ToUnicode to the remapped CID at write
//! time, looking up each glyph's metrics via its original gid (metrics don't
//! change under renumbering). It falls back to the whole-OTF embed
//! (original gid as CID, matching the content pass's own fallback when
//! `cid_remaps` has no entry for that file) whenever `subsetter::subset`
//! fails (seac composites, CFF2).
//!
//! Still deferred: kerning/shaping (advances come straight from
//! `FontMetrics`). A `.ttc` member is never subset either
//! (`subsetter::subset`'s output is a standalone sfnt, which
//! `FontFile2`/`FontFile3` requires — embedding a raw TTC index directly
//! would be invalid).

use std::collections::{BTreeMap, BTreeSet};

use pdf_writer::types::{CidFontType, FontFlags};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use pdf_writer::types::{SystemInfo, UnicodeCmap};
use rustyfi_backend::{
    Color, DocExtras, FontKey, GraphicsElem, ImageResource, Page, PageGeometry, PureHorzBox,
};
use ttf_parser::{Face, GlyphId};

use crate::ttf::TtfFontStore;
use crate::{
    form_res_name, image_res_name, place_embedded_block, place_form, place_graphics, place_image,
    place_math, set_fill_color, used_images, write_annotations, write_document_info,
    write_form_xobjects, write_image_xobjects, write_named_dests, write_outline, PdfError,
};

/// The PDF resource name for physical font file `file_idx` — one name
/// per *file* actually embedded, so a registry slot beyond `FontKey(0/1/2)`
/// is nameable too. Both the page `/Resources /Font` dictionary and every
/// `Tf` operand go through this one function, so they always agree.
pub(crate) fn font_res_name(file_idx: usize) -> String {
    format!("F{file_idx}")
}

/// Which glyphs of one physical font file are referenced anywhere in the
/// document, and the (first-seen) source character for each — enough for both
/// the `/W` widths array and the `ToUnicode` CMap.
#[derive(Default)]
struct FontUsage {
    /// gid -> first char that produced it.
    glyphs: BTreeMap<u16, char>,
    /// Every character this file had no `cmap` entry for, and so emitted as
    /// gid 0. Collected rather than warned about inline so one unrenderable
    /// character repeated across a document produces one line, not hundreds
    /// — see [`report_missing_glyphs`].
    missing: BTreeSet<char>,
}

/// Warn, once per (font file, character), about every character that had to
/// be emitted as `.notdef`.
///
/// **Why this is not cosmetic.** Until it existed, an uncovered character was
/// a completely silent failure. `.notdef` is a visible tofu box in a TrueType
/// face, but in a CFF/OTF face it is usually EMPTY — and
/// `latinmodern-math.otf` is both this port's default math font and a CFF
/// face, so `\mathcal`-style Mathematical Alphanumerics simply vanished:
/// right advance, no ink, exit status 0. The bug report this fixes was
/// "some fonts are not drawn in PDF mode", which is precisely what that
/// looks like from outside.
///
/// Upstream warns per occurrence (`Logging.warn_no_glyph`,
/// `logging.ml:160-165`, "No glyph is provided for U+%04X by font `%s`");
/// this dedupes to one line per character per font, since a missing letter
/// in body text would otherwise produce a warning per occurrence, and prints
/// to STDERR rather than upstream's STDOUT — the same deliberate divergence
/// `primitives.rs`'s `display-message` documents.
fn report_missing_glyphs(usage: &BTreeMap<usize, FontUsage>, store: &TtfFontStore) {
    for (&file_idx, file_usage) in usage {
        if file_usage.missing.is_empty() {
            continue;
        }
        let label = store.file_label(file_idx);
        for &c in &file_usage.missing {
            eprintln!(
                "  [Warning] No glyph is provided for U+{:04X} ({}) by font `{label}`; \
                 it is drawn as .notdef, which this face may render as nothing at all.",
                c as u32,
                c.escape_debug(),
            );
        }
    }
}

/// Serialize typeset pages into a PDF that embeds real TrueType fonts as
/// CID-keyed (Type0/CIDFontType2) fonts, Identity-H encoded. `images` is the
/// document-wide image table (`DocumentValue::images`) — pass `&[]` for a
/// text-only document.
pub fn render_pdf_ttf(
    geometry: &PageGeometry,
    pages: &[Page],
    store: &TtfFontStore,
    images: &[ImageResource],
) -> Result<Vec<u8>, PdfError> {
    render_pdf_ttf_with(geometry, pages, store, images, &DocExtras::default())
}

/// Same as [`render_pdf_ttf`], but also emits the extras
/// (`/Annots`, `/Dests`, `/Outlines`, per-page deco-graphics underlays)
/// accumulated while evaluating the document (`DocumentValue::extras`).
pub fn render_pdf_ttf_with(
    geometry: &PageGeometry,
    pages: &[Page],
    store: &TtfFontStore,
    images: &[ImageResource],
    extras: &DocExtras,
) -> Result<Vec<u8>, PdfError> {
    let paper_h = geometry.paper_height.0 as f32;

    // Pass 1a: a throwaway walk of every page purely to collect `usage`; the
    // content bytes are discarded. This has to happen before any CFF file can
    // be subset, because the subset's glyph set (and hence its
    // `GlyphRemapper`) depends on what the WHOLE document uses.
    let mut usage: BTreeMap<usize, FontUsage> = BTreeMap::new();
    for (i, page) in pages.iter().enumerate() {
        let overlay = extras.page_graphics.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        let _ = page_content(page, paper_h, store, &mut usage, overlay, images, &BTreeMap::new())?;
    }

    // Pass 1b: subset every *used* CFF file now that `usage` is complete,
    // keeping both the subset sfnt bytes and its `GlyphRemapper` — pass 1c
    // needs the latter to translate each CFF run's original gid into the CID
    // the embedded subset expects (CID == new gid, per `subsetter`'s docs). A
    // `glyf` file gets no entry: its own subset renumbering is absorbed
    // by `/CIDToGIDMap`, not by rewriting the CID space. A subsetting failure
    // (`.ok()`, e.g. a seac composite or CFF2 face) leaves no entry either —
    // the whole-OTF fallback (`write_font_cff`) also keeps the original gid
    // as the CID, so `cid_remaps`'s absence is the right signal for both the
    // content pass and the writer.
    let mut cff_subsets: BTreeMap<usize, (Vec<u8>, subsetter::GlyphRemapper)> = BTreeMap::new();
    for (&file_idx, file_usage) in &usage {
        let Some(face) = store.face_by_file(file_idx) else {
            continue;
        };
        let tables = face.tables();
        if tables.glyf.is_none() && tables.cff.is_some() {
            let glyphs: Vec<u16> = file_usage.glyphs.keys().copied().collect();
            let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);
            if let Ok(bytes) = subsetter::subset(store.file_bytes(file_idx), 0, &remapper) {
                cff_subsets.insert(file_idx, (bytes, remapper));
            }
        }
    }
    let cid_remaps: BTreeMap<usize, &subsetter::GlyphRemapper> =
        cff_subsets.iter().map(|(&idx, (_, r))| (idx, r)).collect();

    // Reported off pass 1a, which has already seen every glyph in the
    // document, and before pass 1c re-runs the same walk — so each
    // unrenderable character is named once rather than once per page.
    report_missing_glyphs(&usage, store);

    // Pass 1c: the REAL content streams. `usage` ends up with the exact same
    // original-gid keys (remapping changes only the bytes emitted, not which
    // original gids a run resolves to), except a CFF-backed run whose file
    // has a `cid_remaps` entry now emits `remapper.get(original_gid)`.
    let mut usage: BTreeMap<usize, FontUsage> = BTreeMap::new();
    let mut page_contents = Vec::with_capacity(pages.len());
    for (i, page) in pages.iter().enumerate() {
        let overlay = extras.page_graphics.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        page_contents.push(page_content(page, paper_h, store, &mut usage, overlay, images, &cid_remaps)?);
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

    // One Type0 font object per *used* physical font file (the dedup point
    // for bold/oblique falling back to regular).
    let mut type0_ids: BTreeMap<usize, Ref> = BTreeMap::new();
    for &file_idx in usage.keys() {
        type0_ids.insert(file_idx, next_ref(&mut alloc));
    }

    // One Image XObject per image actually placed on a page — shared with
    // `render_pdf` (base-14, `lib.rs`); see that module's doc comment on
    // this section.
    let used = used_images(pages, &extras.page_graphics);
    let img_refs = write_image_xobjects(&mut pdf, || next_ref(&mut alloc), images, &used);
    // One Form XObject per imported PDF page (`load-pdf-image`) — shared
    // with `render_pdf` (base-14, `lib.rs`); see that module's doc
    // comment.
    let form_refs = write_form_xobjects(&mut pdf, || next_ref(&mut alloc), images, &used);

    let page_ids: Vec<Ref> = pages.iter().map(|_| next_ref(&mut alloc)).collect();
    let content_ids: Vec<Ref> = pages.iter().map(|_| next_ref(&mut alloc)).collect();

    // Link annotations, named destinations, the outline tree — refs
    // allocated right after `page_ids` (needed by `write_named_dests`); the
    // chunk model makes object-write order irrelevant, only ref
    // availability matters.
    let annot_refs =
        write_annotations(&mut pdf, || next_ref(&mut alloc), &extras.annotations, pages.len());
    let dests_id =
        write_named_dests(&mut pdf, || next_ref(&mut alloc), &extras.destinations, &page_ids);
    let outline_id = write_outline(&mut pdf, || next_ref(&mut alloc), &extras.outline);
    // See `lib.rs`'s matching comment.
    if let Some(info) = &extras.doc_info {
        let info_id = next_ref(&mut alloc);
        write_document_info(&mut pdf, info_id, info);
    }

    {
        let mut cat = pdf.catalog(catalog_id);
        cat.pages(page_tree_id);
        if let Some(d) = dests_id {
            cat.destinations(d);
        }
        if let Some(o) = outline_id {
            cat.outlines(o);
        }
    }
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
            cff_subsets.get(&file_idx),
        )?;
    }

    let media_box = Rect::new(0.0, 0.0, geometry.paper_width.0 as f32, paper_h);

    for (i, ((&page_id, &content_id), content_bytes)) in
        page_ids.iter().zip(&content_ids).zip(&page_contents).enumerate()
    {
        pdf.stream(content_id, content_bytes);

        let mut p = pdf.page(page_id);
        p.media_box(media_box);
        p.parent(page_tree_id);
        p.contents(content_id);
        if let Some(refs) = annot_refs.get(&i) {
            p.annotations(refs.iter().copied());
        }
        let mut resources = p.resources();
        let mut fonts = resources.fonts();
        // One resource name per *used* physical font FILE, not per
        // `FontKey` slot — that also collapses the bold-falls-back-to-regular
        // alias correctly, matching `emit_box`'s own `font_res_name`.
        let names: Vec<(String, Ref)> = type0_ids
            .iter()
            .map(|(&file_idx, &font_ref)| (font_res_name(file_idx), font_ref))
            .collect();
        for (name, font_ref) in &names {
            fonts.pair(Name(name.as_bytes()), *font_ref);
        }
        fonts.finish();
        // Registered on every page uniformly, the same simplification the
        // per-`FontKey` loop above already makes — see `render_pdf`'s
        // (`lib.rs`) matching comment.
        if !img_refs.is_empty() || !form_refs.is_empty() {
            let mut x_objects = resources.x_objects();
            for (&id, &r) in &img_refs {
                x_objects.pair(Name(image_res_name(id).as_bytes()), r);
            }
            for (&id, &r) in &form_refs {
                x_objects.pair(Name(form_res_name(id).as_bytes()), r);
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
/// `overlay` (deco graphics) is drawn FIRST, same as `lib.rs`'s
/// `page_content` — see that function's doc comment for the byte-identity
/// guard on an empty overlay.
///
/// `cid_remaps`: per-file `GlyphRemapper`s for CFF files already
/// subset (see this module's doc comment). A `glyf` file, or a CFF file with
/// no entry, keeps emitting the original gid.
fn page_content(
    page: &Page,
    paper_h: f32,
    store: &TtfFontStore,
    usage: &mut BTreeMap<usize, FontUsage>,
    overlay: &[GraphicsElem],
    images: &[ImageResource],
    cid_remaps: &BTreeMap<usize, &subsetter::GlyphRemapper>,
) -> Result<Vec<u8>, PdfError> {
    let mut content = Content::new();
    if !overlay.is_empty() {
        place_graphics(&mut content, overlay, 0.0, 0.0, &mut |c, bx, x, y| {
            emit_box(c, bx, x, y, store, usage, images, cid_remaps)
        })?;
    }
    for line in &page.lines {
        let y = paper_h - line.baseline_y.0 as f32;
        for (dx, bx) in &line.contents {
            emit_box(&mut content, bx, (line.x + *dx).0 as f32, y, store, usage, images, cid_remaps)?;
        }
    }
    Ok(content.finish().into_vec())
}

/// Emit one already-placed `PureHorzBox` at absolute PDF-space coordinates
/// `(tx, ty)` — the CID-writer twin of `crate::emit_box` (base-14, `lib.rs`),
/// factored out for the same reason: reentrant, so a `Tabular` box's cells
/// emit through the same path a top-level line uses, recursively. Text
/// emission is the one thing that differs between the two writers (an
/// `encode_glyph_run` Identity-H run with per-file `usage` tracking here, vs.
/// base-14's WinAnsi `Tj`), so this threads `store`/`usage` where
/// `crate::emit_box` doesn't need to.
fn emit_box(
    content: &mut Content,
    bx: &PureHorzBox,
    tx: f32,
    ty: f32,
    store: &TtfFontStore,
    usage: &mut BTreeMap<usize, FontUsage>,
    images: &[ImageResource],
    cid_remaps: &BTreeMap<usize, &subsetter::GlyphRemapper>,
) -> Result<(), PdfError> {
    match bx {
        PureHorzBox::InnerString { info, text, .. } => {
            let file_idx = store.file_index(info.font);
            let face = store
                .face_by_file(file_idx)
                .ok_or_else(|| PdfError::NoGlyph(text.chars().next().unwrap_or('\u{FFFD}')))?;
            let file_usage = usage.entry(file_idx).or_default();
            let encoded =
                encode_glyph_run(&face, text, file_usage, cid_remaps.get(&file_idx).copied())?;

            let colored = info.color != Color::Gray(0.0);
            if colored {
                content.save_state();
                set_fill_color(content, info.color);
            }
            content.begin_text();
            content.set_font(
                Name(font_res_name(file_idx).as_bytes()),
                info.size.0 as f32,
            );
            content.next_line(tx, ty + info.rising.0 as f32);
            content.show(Str(&encoded));
            content.end_text();
            if colored {
                content.restore_state();
            }
        }
        PureHorzBox::Image {
            width,
            height,
            image,
        } => {
            // load-pdf-image: see `crate::emit_box`'s (base-14, `lib.rs`)
            // matching arm.
            match images.get(image.0).and_then(|im| im.pdf.as_ref()) {
                Some(pdf_res) => place_form(
                    content,
                    image.0,
                    tx,
                    ty,
                    width.0 as f32,
                    height.0 as f32,
                    pdf_res.media_box,
                ),
                None => place_image(content, image.0, tx, ty, width.0 as f32, height.0 as f32),
            }
        }
        PureHorzBox::Graphics { elems, origin_independent, .. } => {
            // See `crate::emit_box`: a page-absolute (`origin_independent`)
            // callback's coords are final — anchor at (0,0), don't translate
            // by the box position (else a full-page frame background shifts
            // off the page).
            let (ax, ay) = if *origin_independent { (0.0, 0.0) } else { (tx, ty) };
            place_graphics(content, elems, ax, ay, &mut |c, bx, x, y| {
                emit_box(c, bx, x, y, store, usage, images, cid_remaps)
            })?;
        }
        PureHorzBox::Math { glyphs, rules, .. } => {
            let name_for = |k: FontKey| font_res_name(store.file_index(k));
            place_math(content, glyphs, tx, ty, &name_for, |g| {
                let file_idx = store.file_index(g.info.font);
                let remap = cid_remaps.get(&file_idx).copied();
                let file_usage = usage.entry(file_idx).or_default();
                match g.gid {
                    // A raw MATH-table variant glyph id
                    // (`push_big_char_glyph`/`push_delimiter_glyph`) — not
                    // necessarily cmap-reachable from `g.text`, so emit it
                    // directly (Identity-H: content bytes ARE gids) rather
                    // than re-deriving a gid through `glyph_index`. `usage`
                    // still records the ORIGINAL gid.
                    Some(gid) => {
                        file_usage
                            .glyphs
                            .entry(gid)
                            .or_insert(g.text.chars().next().unwrap_or('\u{FFFD}'));
                        let cid = remap.and_then(|r| r.get(gid)).unwrap_or(gid);
                        Ok(cid.to_be_bytes().to_vec())
                    }
                    None => {
                        let face = store.face_by_file(file_idx).ok_or_else(|| {
                            PdfError::NoGlyph(g.text.chars().next().unwrap_or('\u{FFFD}'))
                        })?;
                        encode_glyph_run(&face, &g.text, file_usage, remap)
                    }
                }
            })?;
            // Same fraction-bar/radical-sign `Fill`s as `render_pdf`'s
            // base-14 writer (`lib.rs`'s Math arm) — the CID writer shares
            // `place_graphics` unchanged, since a filled path carries no
            // font/text state either writer needs to specialize.
            place_graphics(content, rules, tx, ty, &mut |c, bx, x, y| {
                emit_box(c, bx, x, y, store, usage, images, cid_remaps)
            })?;
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
                        images,
                        cid_remaps,
                    )?;
                }
            }
            place_graphics(content, &tab.rules, tx, ty, &mut |c, bx, x, y| {
                emit_box(c, bx, x, y, store, usage, images, cid_remaps)
            })?;
        }
        PureHorzBox::EmbeddedBlock { block, anchor_last, .. } => {
            place_embedded_block(block, tx, ty, *anchor_last, |cbx, x, y| {
                emit_box(content, cbx, x, y, store, usage, images, cid_remaps)
            })?;
        }
        // See `crate::emit_box`'s (base-14, `lib.rs`) matching arm.
        PureHorzBox::Frame { contents, .. } => {
            for (dx, cbx) in contents {
                emit_box(content, cbx, tx + dx.0 as f32, ty, store, usage, images, cid_remaps)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Map `text` to a run of 2-byte big-endian glyph IDs, recording each glyph
/// (and the first character that produced it) in `usage` for the `/W` array
/// and `ToUnicode` CMap built later — `usage` is always keyed by the
/// ORIGINAL face gid, for every font format (glyph metrics/cmap lookups are
/// naturally in that space). `remap` is applied to the EMITTED CONTENT
/// BYTES only: `remap.get(gid).unwrap_or(gid)` becomes the CID written into
/// the `Tj` operand, and `write_font_cff` re-keys `/W`/ToUnicode to match at
/// write time.
fn encode_glyph_run(
    face: &Face<'_>,
    text: &str,
    usage: &mut FontUsage,
    remap: Option<&subsetter::GlyphRemapper>,
) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for c in text.chars() {
        // A character the face doesn't cover degrades to `.notdef` (gid 0)
        // rather than aborting the whole PDF — matching `measure_run`'s
        // half-em placeholder box. (Uncovered glyphs like `□`/`〚` in
        // satysfi-base docs; faithful per-glyph font-fallback is future work.)
        // Recorded so `report_missing_glyphs` can SAY so: `.notdef` is a tofu
        // box in a TrueType face but usually EMPTY in a CFF/OTF one, so on
        // e.g. `latinmodern-math.otf` this silently drops the character.
        let gid = face.glyph_index(c).unwrap_or_else(|| {
            usage.missing.insert(c);
            GlyphId(0)
        });
        usage.glyphs.entry(gid.0).or_insert(c);
        let cid = remap.and_then(|r| r.get(gid.0)).unwrap_or(gid.0);
        out.extend_from_slice(&cid.to_be_bytes());
    }
    Ok(out)
}

/// Write the Type0 font, its CIDFontType2 descendant, FontDescriptor,
/// FontFile2 and ToUnicode CMap for one physical font file.
///
/// **Subsetting.** Content-stream generation (`page_content`, pass 1)
/// runs before this function and bakes ORIGINAL face glyph ids into every
/// `Tj` as Identity-H CIDs — renumbering CIDs here would force a two-pass
/// restructure of the whole writer (and the math raw-gid channel,
/// `emit_box`'s `Math` arm). Instead: the content stream and `/W`/ToUnicode
/// all stay keyed by the original gid (= CID); only the
/// embedded `FontFile2` is subsetted (via the `subsetter` crate, `glyf`
/// faces only) and a translating `/CIDToGIDMap` stream maps each original
/// gid (CID) to its renumbered gid inside the subset font — the PDF spec's
/// documented mechanism for exactly this split (§9.7.4.3). Deviation from
/// upstream (`fontFormat.ml`'s `SubsetMap`, which interns original->subset
/// ids and renumbers the CIDs themselves): same rendered glyphs, different
/// CID space; documented, not a fidelity gap.
///
/// A `glyf`-less face (CFF/OpenType-CFF) is dispatched to `write_font_cff`
/// instead, which has its own (structurally different) subsetting/fallback
/// story; `cff_subset`, when `Some`, is that file's already-computed
/// subset bytes + `GlyphRemapper` (built by `render_pdf_ttf_with` before
/// content generation), passed straight through so `write_font_cff`
/// doesn't redo the subsetting work. For a `glyf` face, a subsetting
/// failure degrades gracefully to a whole-file embed with
/// `CIDToGIDMap=Identity` — subsetting is a size optimization, never a hard
/// requirement for a correct PDF.
fn write_font(
    pdf: &mut Pdf,
    alloc: &mut i32,
    type0_ref: Ref,
    store: &TtfFontStore,
    file_idx: usize,
    usage: &FontUsage,
    cff_subset: Option<&(Vec<u8>, subsetter::GlyphRemapper)>,
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
    // Allocated unconditionally (mirrors the other refs above); only
    // actually written as a PDF object when subsetting succeeds (see
    // `subset` below) — an allocated-but-unwritten ref number is harmless
    // (pdf-writer's chunk model doesn't require contiguous object use). For
    // a CFF file (`write_font_cff` below), this ref is likewise allocated
    // but never written — `CIDFontType0` has no `/CIDToGIDMap` at all.
    let c2g_ref = next_ref();

    let face = store
        .face_by_file(file_idx)
        .expect("file_idx came from a successfully-loaded TtfFontStore");

    // CFF/OpenType-CFF outlines take a structurally different embed path —
    // `CIDFontType0`/`/FontFile3`, no `/CIDToGIDMap` — so they're dispatched
    // to `write_font_cff` right after ref allocation, before any of the
    // glyf-specific logic below runs.
    let tables = face.tables();
    if tables.glyf.is_none() && tables.cff.is_some() {
        return write_font_cff(
            pdf,
            type0_ref,
            cid_font_ref,
            descriptor_ref,
            font_file_ref,
            to_unicode_ref,
            store,
            file_idx,
            usage,
            &face,
            cff_subset,
        );
    }

    let units_per_em = face.units_per_em() as f64;
    // PDF CID widths (and FontDescriptor metrics) are always expressed in
    // 1000-units-per-em glyph space, regardless of the font's own
    // `unitsPerEm` (DejaVu et al. use 2048).
    let scale = |v: f64| (v * 1000.0 / units_per_em) as f32;

    // --- Subset the font file down to the glyphs `usage` references.
    // `glyf`-only (CFF faces went to `write_font_cff` above); a subsetting
    // failure (`.ok()`) degrades to the whole-file embed rather than a hard
    // error — subsetting is a size optimization only. ---
    let glyphs: Vec<u16> = usage.glyphs.keys().copied().collect();
    let subset: Option<(Vec<u8>, subsetter::GlyphRemapper)> = if face.tables().glyf.is_some() {
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);
        subsetter::subset(store.file_bytes(file_idx), 0, &remapper)
            .ok()
            .map(|bytes| (bytes, remapper))
    } else {
        None
    };

    let base_name = match &subset {
        Some(_) => format!("{}+{}", subset_tag(&glyphs), base_font_name(&face, file_idx)),
        None => base_font_name(&face, file_idx),
    };

    // --- ToUnicode CMap (required: this is what keeps text extraction working for an Identity-H-encoded, glyph-indexed content stream). ---
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

    {
        let mut t0 = pdf.type0_font(type0_ref);
        t0.base_font(Name(base_name.as_bytes()));
        t0.encoding_predefined(Name(b"Identity-H"));
        t0.descendant_font(cid_font_ref);
        t0.to_unicode(to_unicode_ref);
    }

    // --- CIDFontType2 descendant: CIDs are raw ORIGINAL TrueType glyph
    // indices (never renumbered, see `write_font`'s doc comment);
    // CIDSystemInfo Adobe-Identity-0 (no predefined CJK ordering). The
    // CID->embedded-glyph-id translation is `/CIDToGIDMap`: a stream when
    // subsetted, the `Identity` predefined name otherwise. ---
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
        if subset.is_some() {
            cid.cid_to_gid_map_stream(c2g_ref);
        } else {
            cid.cid_to_gid_map_predefined(Name(b"Identity"));
        }

        if !widths.is_empty() {
            let mut w = cid.widths();
            write_width_runs(&mut w, &widths);
            w.finish();
        }
    }

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

    // --- FontFile2: the subsetted font when subsetting succeeded, else the whole
    // input file verbatim — both uncompressed. ---
    match &subset {
        Some((bytes, _)) => pdf.stream(font_file_ref, bytes),
        None => pdf.stream(font_file_ref, store.file_bytes(file_idx)),
    };

    // --- /CIDToGIDMap stream (only when subsetted): `2*(max_used_gid+1)` big-endian
    // u16s, `bytes[2*cid..] = remapper.get(cid).unwrap_or(0)` for every
    // `cid` from 0 up to the highest gid actually used — gaps (a CID that's
    // never shown) map to `.notdef` (0), which is harmless since no content
    // stream operand ever names them. ---
    if let Some((_, remapper)) = &subset {
        let max_cid = usage.glyphs.keys().copied().max().unwrap_or(0);
        let mut map_bytes = vec![0u8; 2 * (max_cid as usize + 1)];
        for cid in 0..=max_cid {
            let new_gid = remapper.get(cid).unwrap_or(0);
            let at = 2 * cid as usize;
            map_bytes[at..at + 2].copy_from_slice(&new_gid.to_be_bytes());
        }
        pdf.stream(c2g_ref, &map_bytes);
    }

    Ok(())
}

/// Write the Type0 font, its CIDFontType0 descendant, FontDescriptor,
/// FontFile3 and ToUnicode CMap for one physical CFF-outline font file —
/// the `CIDFontType0`/`FontFile3` sibling of `write_font`'s
/// `CIDFontType2`/`FontFile2` path.
///
/// **Real subsetting, gated by `subset`.** `CIDFontType0` has no
/// `/CIDToGIDMap`, so unlike `write_font`'s glyf path (where the CID is
/// always the original gid and `/CIDToGIDMap` absorbs any subset
/// renumbering), a *subsetted* CFF embed needs the CID space itself to
/// change, and `render_pdf_ttf_with`'s two-pass content generation has
/// already arranged for the content stream to emit that remapped CID for
/// this file's runs whenever `subset` is `Some`. So here: `/W` and the
/// ToUnicode CMap are re-keyed from the original gid (which is how
/// `usage`/glyph-metric lookups naturally work) to
/// `subset.1.get(original_gid)` — the same CID the content pass already
/// wrote — and the FontFile3 stream embeds the SUBSET sfnt
/// (`subset.0`, still a complete `OTTO`-flavoured sfnt, so `/Subtype
/// /OpenType` remains correct) instead of the whole input file.
/// `/BaseFont` gets the usual `XXXXXX+` subset tag, matching the
/// glyf path's own convention.
///
/// When `subset` is `None` (never attempted for a file with no usage, or
/// `subsetter::subset` failed — e.g. a seac composite or CFF2 face), this
/// falls back to a whole-file embed: the verbatim input file is embedded under
/// `/FontFile3 /Subtype /OpenType`, and the CID space stays the ORIGINAL
/// face gid — the same space the content pass falls back to for this file
/// (no `cid_remaps` entry) — which is correct for a non-CID-keyed CFF (PDF
/// 32000 §9.7.4.2), covering the common case (real Latin/math OTFs). A
/// CID-keyed whole-OTF CFF (rare on this track — CJK OTFs are the common
/// CID-keyed case, and every CJK face this port ships is `glyf`) is out of
/// scope for this fallback.
#[allow(clippy::too_many_arguments)]
fn write_font_cff(
    pdf: &mut Pdf,
    type0_ref: Ref,
    cid_font_ref: Ref,
    descriptor_ref: Ref,
    font_file_ref: Ref,
    to_unicode_ref: Ref,
    store: &TtfFontStore,
    file_idx: usize,
    usage: &FontUsage,
    face: &Face<'_>,
    subset: Option<&(Vec<u8>, subsetter::GlyphRemapper)>,
) -> Result<(), PdfError> {
    let units_per_em = face.units_per_em() as f64;
    let scale = |v: f64| (v * 1000.0 / units_per_em) as f32;
    // The CID actually written into `/W`/ToUnicode/content for original gid
    // `gid`: the remapped new-gid when this file was subset, else the
    // original gid unchanged — must agree with what
    // `encode_glyph_run`/the content pass's `Math` arm already emitted.
    let cid_of = |gid: u16| -> u16 { subset.and_then(|(_, r)| r.get(gid)).unwrap_or(gid) };

    let glyphs: Vec<u16> = usage.glyphs.keys().copied().collect();
    let base_name = match subset {
        Some(_) => format!("{}+{}", subset_tag(&glyphs), base_font_name(face, file_idx)),
        None => base_font_name(face, file_idx),
    };

    // --- ToUnicode CMap, keyed by the CID actually emitted in content
    // (remapped when subsetted, original gid otherwise). ---
    let mut cmap = UnicodeCmap::new(
        Name(b"Custom-UCS"),
        SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"UCS"),
            supplement: 0,
        },
    );
    for (&gid, &ch) in &usage.glyphs {
        cmap.pair(cid_of(gid), ch);
    }
    let cmap_bytes = cmap.finish();
    pdf.cmap(to_unicode_ref, &cmap_bytes);

    // --- Type0 (composite) font: identical shape to `write_font`'s. ---
    {
        let mut t0 = pdf.type0_font(type0_ref);
        t0.base_font(Name(base_name.as_bytes()));
        t0.encoding_predefined(Name(b"Identity-H"));
        t0.descendant_font(cid_font_ref);
        t0.to_unicode(to_unicode_ref);
    }

    // --- CIDFontType0 descendant: no `/CIDToGIDMap` — a non-CID-keyed CFF
    // font interprets the CID directly as the GID (PDF 32000 §9.7.4.2), and
    // a CID-keyed one (always true of `subsetter`'s output) resolves CID
    // -> GID through the embedded CFF's own charset, which `subsetter`
    // writes as identity-with-new-gid — either way the CID this dict/array
    // uses is exactly `cid_of(gid)`. Glyph METRICS are still looked up via
    // the ORIGINAL gid (`face` is the whole, un-subset font; renumbering
    // doesn't change a glyph's own advance width). ---
    let widths: BTreeMap<u16, f32> = usage
        .glyphs
        .keys()
        .map(|&gid| {
            let advance = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f64;
            (cid_of(gid), scale(advance))
        })
        .collect();
    let default_width = if widths.is_empty() {
        1000.0
    } else {
        widths.values().sum::<f32>() / widths.len() as f32
    };
    {
        let mut cid = pdf.cid_font(cid_font_ref);
        cid.subtype(CidFontType::Type0);
        cid.base_font(Name(base_name.as_bytes()));
        cid.system_info(SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        });
        cid.font_descriptor(descriptor_ref);
        cid.default_width(default_width);
        if !widths.is_empty() {
            let mut w = cid.widths();
            write_width_runs(&mut w, &widths);
            w.finish();
        }
    }

    // --- FontDescriptor: same metrics computation as `write_font`'s;
    // `font_file3` instead of `font_file2`. ---
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
        fd.stem_v(if face.is_bold() { 120.0 } else { 80.0 });
        fd.font_file3(font_file_ref);
    }

    // --- FontFile3: the SUBSET sfnt when subsetting succeeded, else
    // the whole input OTF verbatim — both carry stream
    // `/Subtype /OpenType` (subsetter's CFF output is itself a complete
    // `OTTO`-flavoured sfnt). ---
    match subset {
        Some((bytes, _)) => {
            pdf.stream(font_file_ref, bytes).pair(Name(b"Subtype"), Name(b"OpenType"));
        }
        None => {
            pdf.stream(font_file_ref, store.file_bytes(file_idx))
                .pair(Name(b"Subtype"), Name(b"OpenType"));
        }
    }

    Ok(())
}

/// A deterministic 6-uppercase-letter subset tag for `/BaseFont` (PDF 32000
/// §9.6.4: `XXXXXX+FontName`), folded from the used-gid set so identical
/// input produces an identical tag across reruns (reproducible builds) —
/// `glyphs` is already sorted (a `BTreeMap`'s key iteration order,
/// `FontUsage::glyphs`), so hashing it is deterministic regardless of the
/// order glyphs were first encountered while walking the document.
fn subset_tag(glyphs: &[u16]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    glyphs.hash(&mut hasher);
    // NOTE: `hasher.finish()` would silently resolve to `pdf_writer::Finish`
    // (a blanket `impl<T> Finish for T { fn finish(self) {} }` this module
    // already imports for the builder-pattern types) instead of
    // `std::hash::Hasher::finish`, since a by-value match wins over the
    // autoref `&self` the real `Hasher::finish` needs — fully qualify.
    let mut h = Hasher::finish(&hasher);
    let mut tag = String::with_capacity(6);
    for _ in 0..6 {
        tag.push((b'A' + (h % 26) as u8) as char);
        h /= 26;
    }
    tag
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
