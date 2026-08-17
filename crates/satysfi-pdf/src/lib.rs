//! PDF output backend: base-14 Type1 fonts, uncompressed content streams
//! (the milestone-1 replacement for handlePdf.ml on top of `pdf-writer`),
//! plus (phase 5) ttf-parser-backed metrics and CID-keyed TrueType embedding.

pub mod base14;
pub mod cid;
pub mod fonts;
pub mod ttf;

pub use base14::Base14Metrics;
pub use cid::render_pdf_ttf;
pub use fonts::{FontConfigError, FontFlags, FontRegistry, FontSource};
pub use ttf::{FontError, TtfFontStore};

use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use satysfi_backend::{Page, PageGeometry, PureHorzBox};

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
            let PureHorzBox::InnerString { info, text, .. } = bx else {
                continue;
            };
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
    }
    Ok(content.finish().into_vec())
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
