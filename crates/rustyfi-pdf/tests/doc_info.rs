//! `register-document-information`'s PDF `/Info` dictionary
//! emission (`write_document_info`, `rustyfi-pdf/src/lib.rs`/`cid.rs`),
//! driven through the public `render_pdf_with` entry point with a
//! hand-built `DocExtras` — the same shape as `tests/annots.rs`'s
//! coverage (no lang layer involved;
//! `crates/rustyfi/tests/fixtures/v01-strings.saty` is the end-to-end
//! version through the real `register-document-information` primitive).

use rustyfi_backend::{DocExtras, DocInfo, Length, Page, PageGeometry};

fn geometry() -> PageGeometry {
    PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    }
}

#[test]
fn render_pdf_with_emits_info_dict_when_registered() {
    let geometry = geometry();
    let pages = vec![Page::default()];
    let extras = DocExtras {
        doc_info: Some(DocInfo {
            title: Some("A Title".to_string()),
            subject: None,
            author: Some("An Author".to_string()),
            keywords: vec!["alpha".to_string(), "beta".to_string()],
        }),
        ..Default::default()
    };
    let bytes =
        rustyfi_pdf::render_pdf_with(&geometry, &pages, &[], &extras).expect("render must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);
    for needle in [
        "/Title",
        "/Author",
        "/Keywords",
        "/Creator (SATySFi)",
        "/Producer (SATySFi)",
    ] {
        assert!(hay.contains(needle), "content missing {needle:?}:\n{hay}");
    }
    assert!(
        !hay.contains("/Subject"),
        "unregistered /Subject leaked into the dict:\n{hay}"
    );
    // Keywords are space-joined, as one single PDF text string (not two
    // separate `(alpha)(beta)` literals).
    assert!(
        hay.contains("alpha beta") || hay.contains("alpha\\040beta"),
        "keywords not space-joined:\n{hay}"
    );
}

/// A document that never calls `register-document-information`
/// (`extras.doc_info == None`, `DocExtras::default()`) must
/// render byte-identical to `render_pdf` (the `&DocExtras::default()`
/// wrapper) — no `/Info` object at all, keeping every 0.0.6 fixture's PDF
/// unchanged.
#[test]
fn render_pdf_with_omits_info_dict_when_unregistered() {
    let geometry = geometry();
    let pages = vec![Page::default()];

    let via_default = rustyfi_pdf::render_pdf(&geometry, &pages, &[]).expect("render must succeed");
    let via_with = rustyfi_pdf::render_pdf_with(&geometry, &pages, &[], &DocExtras::default())
        .expect("render must succeed");
    assert_eq!(
        via_default, via_with,
        "render_pdf must stay byte-identical to render_pdf_with(default)"
    );

    let hay = String::from_utf8_lossy(&via_with);
    assert!(
        !hay.contains("/Creator"),
        "no /Info object should exist when unregistered:\n{hay}"
    );
    assert!(
        !hay.contains("/Producer"),
        "no /Info object should exist when unregistered:\n{hay}"
    );
}
