//! PDF output backend: base-14 Type1 fonts, uncompressed content streams
//! (the milestone-1 replacement for handlePdf.ml on top of `pdf-writer`),
//! plus (phase 5) ttf-parser-backed metrics and CID-keyed TrueType embedding.

pub mod base14;
pub mod cid;
pub mod ttf;

pub use base14::Base14Metrics;
pub use cid::render_pdf_ttf;
pub use ttf::{FontError, TtfFontStore};

use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use satysfi_backend::{Closing, Color, GraphicsElem, Page, PageGeometry, Path, PathSeg, PureHorzBox};

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

/// Serialize typeset pages into a complete PDF document.
pub fn render_pdf(geometry: &PageGeometry, pages: &[Page]) -> Result<Vec<u8>, PdfError> {
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
        resources.finish();
        p.finish();
    }

    Ok(pdf.finish())
}

/// Build one page's content stream: `BT … Tf … Td … Tj … ET` runs, with the
/// y axis flipped from page coordinates (downward) to PDF (upward).
fn page_content(page: &Page, paper_h: f32) -> Result<Vec<u8>, PdfError> {
    let mut content = Content::new();
    for line in &page.lines {
        let y = paper_h - line.baseline_y.0 as f32;
        for (dx, bx) in &line.contents {
            match bx {
                PureHorzBox::InnerString { info, text, .. } => {
                    let encoded = winansi(text)?;
                    let font_idx = (info.font.0 as usize).min(FONT_RES_NAMES.len() - 1);
                    content.begin_text();
                    content.set_font(
                        Name(FONT_RES_NAMES[font_idx].as_bytes()),
                        info.size.0 as f32,
                    );
                    content.next_line((line.x + *dx).0 as f32, y);
                    content.show(Str(&encoded));
                    content.end_text();
                }
                PureHorzBox::Graphics { elems, .. } => {
                    place_graphics(&mut content, elems, (line.x + *dx).0 as f32, y);
                }
                _ => {}
            }
        }
    }
    Ok(content.finish().into_vec())
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
