//! `used_images` must find an `Image` wherever the box tree can put one.
//!
//! It decides which Image XObjects a page's `/Resources` declares, while
//! `emit_box`/`place_graphics` decide which `/ImN Do` operators the content
//! stream issues. When the two disagree the PDF carries a dangling reference:
//! structurally valid, silently blank in a viewer, and invisible to every
//! other test — exactly the class of bug the generated traversal in
//! `rustyfi_backend::visit` exists to rule out.
//!
//! The case below is the one the previous hand-written scan missed. Its
//! `PureHorzBox::Graphics` arm inlined a copy of the `GraphicsElem::Text`
//! case instead of delegating to the sibling function that handles
//! `Group`/`Clip`, so a `draw-text` run reached through a `unite-graphics`
//! group — a `graphics` node 0.1 documents really do build, and the shape the
//! cross-version `deco` bridge emits — declared no XObject while
//! `place_graphics` happily emitted its `Do`.
//!
//! Only the base-14 writer is driven here; `cid.rs` calls the very same
//! `used_images`, and standing a `TtfFontStore` up would make this test depend
//! on `download-fonts.sh` for no extra coverage of the scan.

use rustyfi_backend::{
    GraphicsElem, ImageId, ImageResource, Length, Page, PageGeometry, Path, PlacedLine, PureHorzBox,
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

fn one_image() -> Vec<ImageResource> {
    vec![ImageResource {
        samples: vec![0u8; 3 * 2 * 2],
        px_w: 2,
        px_h: 2,
        jpeg_dct: None,
        pdf: None,
    }]
}

fn image_box() -> PureHorzBox {
    PureHorzBox::Image {
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        image: ImageId(0),
    }
}

/// A `draw-text` run holding just the image.
fn draw_text_with_image() -> GraphicsElem {
    GraphicsElem::Text {
        pt: (Length::ZERO, Length::ZERO),
        contents: vec![(Length::ZERO, image_box())],
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        transform: None,
    }
}

fn page_with(elems: Vec<GraphicsElem>) -> Page {
    Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(10.0),
            baseline_y: Length::pt(50.0),
            contents: vec![(
                Length::ZERO,
                PureHorzBox::Graphics {
                    width: Length::pt(10.0),
                    height: Length::pt(10.0),
                    depth: Length::ZERO,
                    elems,
                    origin_independent: false,
                },
            )],
        }],
    }
}

/// The content stream must never name an XObject the page's `/Resources` does
/// not declare.
fn assert_no_dangling_image(bytes: &[u8], what: &str) {
    let hay = String::from_utf8_lossy(bytes);
    assert!(
        hay.contains("/Im0 Do"),
        "the writer should have drawn the image for {what}; the fixture is not \
         exercising the path it claims to:\n{hay}"
    );
    assert!(
        hay.contains("/Subtype /Image"),
        "`/Im0 Do` is emitted for {what} but no Image XObject was written — a \
         dangling reference:\n{hay}"
    );
    assert!(
        hay.contains("/XObject"),
        "expected an /XObject resource entry for {what}:\n{hay}"
    );
}

#[test]
fn an_image_inside_a_united_graphics_group_gets_an_xobject() {
    let page = page_with(vec![GraphicsElem::Group(vec![draw_text_with_image()])]);
    let bytes = rustyfi_pdf::render_pdf(&geometry(), std::slice::from_ref(&page), &one_image())
        .expect("render must succeed");
    assert_no_dangling_image(&bytes, "a `Group`-nested `draw-text`");
}

#[test]
fn an_image_inside_a_clipped_graphics_group_gets_an_xobject() {
    let page = page_with(vec![GraphicsElem::Clip(
        Path {
            subpaths: Vec::new(),
        },
        vec![draw_text_with_image()],
    )]);
    let bytes = rustyfi_pdf::render_pdf(&geometry(), std::slice::from_ref(&page), &one_image())
        .expect("render must succeed");
    assert_no_dangling_image(&bytes, "a `Clip`-nested `draw-text`");
}
