//! `write_annotations`/`write_named_dests`/`write_outline`, driven through
//! `render_pdf_with` with a hand-built `DocExtras` (no lang layer) — see
//! `crates/rustyfi/tests/fixtures/annot-hook.saty` for the end-to-end version.

use rustyfi_backend::{
    Annot, AnnotAction, Color, DocExtras, Length, NamedDest, OutlineEntry, Page, PageGeometry,
};

fn geometry() -> PageGeometry {
    PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    }
}

/// A 2-level, 3-item outline tree — malformed Prev/Next chains are the
/// classic bug this shape is meant to catch.
fn extras() -> DocExtras {
    DocExtras {
        annotations: vec![
            Annot {
                page: 0,
                rect: (
                    Length::pt(10.0),
                    Length::pt(10.0),
                    Length::pt(50.0),
                    Length::pt(30.0),
                ),
                action: AnnotAction::Uri("https://example.com".to_string()),
                border: Some((Length::pt(1.0), Color::Gray(0.0))),
            },
            Annot {
                page: 0,
                rect: (
                    Length::pt(10.0),
                    Length::pt(40.0),
                    Length::pt(50.0),
                    Length::pt(60.0),
                ),
                action: AnnotAction::GotoName("nameddest0".to_string()),
                border: None,
            },
        ],
        destinations: vec![NamedDest {
            page: 0,
            name: "nameddest0".to_string(),
            x: Length::pt(0.0),
            y: Length::pt(300.0),
        }],
        outline: vec![
            OutlineEntry {
                level: 0,
                text: "A".to_string(),
                dest_name: "nameddest0".to_string(),
                is_open: true,
            },
            OutlineEntry {
                level: 1,
                text: "A.1".to_string(),
                dest_name: "nameddest0".to_string(),
                is_open: true,
            },
            OutlineEntry {
                level: 0,
                text: "B".to_string(),
                dest_name: "nameddest1".to_string(),
                is_open: false,
            },
        ],
        page_graphics: Vec::new(),
        doc_info: None,
    }
}

#[test]
fn render_pdf_with_emits_annots_dests_and_outlines() {
    let geometry = geometry();
    let pages = vec![Page::default()];
    let extras = extras();
    let bytes =
        rustyfi_pdf::render_pdf_with(&geometry, &pages, &[], &extras).expect("render must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);
    for needle in [
        "/Annots",
        "/Subtype /Link",
        "/URI (https://example.com)",
        "/S /GoTo",
        "/Dests",
        "/XYZ",
        "/Outlines",
        "/Title",
        "/First",
        "/Count 2",
    ] {
        assert!(hay.contains(needle), "content missing {needle:?}:\n{hay}");
    }
    // "A" has one open child ("A.1") -> /Count 1 (not printed as "-1" since
    // it's open); "B" is a closed LEAF (no children at all) -> no /Count
    // entry whatsoever (closed-ness only matters once there ARE
    // descendants to negate-or-not).
    assert!(hay.contains("/Count 1"), "A's own child count:\n{hay}");
    assert!(
        !hay.contains("/Count -0"),
        "a closed leaf must carry no /Count entry, not a degenerate -0:\n{hay}"
    );

    assert!(
        hay.contains("nameddest0"),
        "the GotoName action and /Dests entry must share the same name:\n{hay}"
    );
}

#[test]
fn render_pdf_wrapper_is_byte_identical_to_render_pdf_with_default_extras() {
    let geometry = geometry();
    let pages = vec![Page::default()];
    let a = rustyfi_pdf::render_pdf(&geometry, &pages, &[]).expect("render_pdf");
    let b = rustyfi_pdf::render_pdf_with(&geometry, &pages, &[], &DocExtras::default())
        .expect("render_pdf_with default extras");
    assert_eq!(
        a, b,
        "render_pdf (the 3-arg wrapper) must stay byte-identical to \
         render_pdf_with(..., &DocExtras::default())"
    );
}

#[test]
fn empty_extras_emit_no_annots_dests_or_outlines_catalog_keys() {
    let geometry = geometry();
    let pages = vec![Page::default()];
    let bytes = rustyfi_pdf::render_pdf_with(&geometry, &pages, &[], &DocExtras::default())
        .expect("render");
    let hay = String::from_utf8_lossy(&bytes);
    for absent in ["/Annots", "/Dests", "/Outlines"] {
        assert!(
            !hay.contains(absent),
            "an extras-free document must not gain {absent:?}:\n{hay}"
        );
    }
}
