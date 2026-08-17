//! §D underlay ordering (docs/plans/hooks-annotations-crossref.md): a
//! page's `DocExtras::page_graphics` overlay must draw BEFORE the page's own
//! text — `page_content`'s prologue, `satysfi-pdf/src/lib.rs` — so a frame's
//! background fill/border sits behind the text it decorates, exactly what
//! upstream's per-page op ordering (deco ops emitted ahead of the page's
//! text ops in `handlePdf.ml`) produces.

use satysfi_backend::{
    Closing, Color, DocExtras, FontKey, GraphicsElem, HorzStringInfo, Length, Page, PageGeometry,
    Path, PathSeg, PlacedLine, PureHorzBox, Subpath,
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

fn triangle_path() -> Path {
    Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(10.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(10.0), Length::pt(10.0))),
            ],
            closing: Closing::Line,
        }],
    }
}

fn page_with_text() -> Page {
    Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(
                Length::ZERO,
                PureHorzBox::InnerString {
                    info: HorzStringInfo { font: FontKey(0), size: Length::pt(12.0), rising: Length::ZERO, color: Color::Gray(0.0) },
                    text: "hi".to_string(),
                    width: Length::pt(12.0),
                    height: Length::pt(9.0),
                    depth: Length::pt(3.0),
                },
            )],
        }],
    }
}

#[test]
fn a_frames_deco_underlay_draws_before_the_pages_first_bt() {
    let geometry = geometry();
    let page = page_with_text();
    let extras = DocExtras {
        page_graphics: vec![vec![GraphicsElem::Fill(Color::Gray(0.5), triangle_path())]],
        ..Default::default()
    };
    let bytes = satysfi_pdf::render_pdf_with(&geometry, std::slice::from_ref(&page), &[], &extras)
        .expect("render must succeed");
    let hay = String::from_utf8_lossy(&bytes);

    let fill_pos = hay.find("f*").expect("missing the fill operator 'f*'");
    let bt_pos = hay.find("BT").expect("missing the page's text 'BT' operator");
    assert!(
        fill_pos < bt_pos,
        "the deco underlay's fill must draw BEFORE the page's first BT \
         (found f* at {fill_pos}, BT at {bt_pos}):\n{hay}"
    );
}

/// The `place_graphics` prologue emits its `q`/`cm`/`Q` wrapper
/// UNCONDITIONALLY, even for an empty slice — an empty-overlay page must NOT
/// emit that wrapper at all, or the byte-identity floor (§A9: extras-free
/// documents stay byte-for-byte identical) breaks.
#[test]
fn an_empty_overlay_emits_no_graphics_state_wrapper_at_all() {
    let geometry = geometry();
    let page = page_with_text();
    let with_default = satysfi_pdf::render_pdf_with(
        &geometry,
        std::slice::from_ref(&page),
        &[],
        &DocExtras::default(),
    )
    .expect("render");
    let without_extras =
        satysfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[]).expect("render");
    assert_eq!(with_default, without_extras);

    let hay = String::from_utf8_lossy(&with_default);
    // No stray `q ... cm ... Q` pair ahead of the text: the ONLY `cm` in this
    // text-only page's content stream would come from a non-empty overlay,
    // which this test has none of.
    assert!(
        !hay.contains(" cm\n"),
        "an empty overlay must not emit place_graphics's q/cm/Q prologue at all:\n{hay}"
    );
}
