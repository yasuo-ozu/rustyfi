//! §D writer coverage (docs/plans/hooks-annotations-crossref.md): `emit_box`'s
//! new `PureHorzBox::Frame` recursion (contents rendered on the frame's own
//! baseline, x-shifted by each content's own offset) and `used_images`'s
//! recursive box scan finding an `Image` nested inside a `Frame`.

use rustyfi_backend::{
    Color, DecoId, FontKey, HorzStringInfo, ImageId, ImageResource, Length, Page, PageGeometry,
    PlacedLine, PureHorzBox,
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

#[test]
fn frame_content_renders_at_the_frames_placed_anchor_plus_its_own_offset() {
    let inner = PureHorzBox::InnerString {
        info: HorzStringInfo { font: FontKey(0), size: Length::pt(12.0), rising: Length::ZERO, color: Color::Gray(0.0) },
        text: "hi".to_string(),
        width: Length::pt(12.0),
        height: Length::pt(9.0),
        depth: Length::pt(3.0),
    };
    let frame = PureHorzBox::Frame {
        width: Length::pt(20.0),
        height: Length::pt(9.0),
        depth: Length::pt(3.0),
        deco: DecoId(0),
        contents: vec![(Length::pt(6.0), inner)],
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::pt(4.0), frame)],
        }],
    };
    let geometry = geometry();
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("render must succeed");
    let hay = String::from_utf8_lossy(&bytes);

    // Text anchor: tx = line.x + box_dx(4) + inner_dx(6) = 60; ty = paper_h -
    // baseline_y = 200 — the frame's own baseline, inherited unshifted by
    // its content (emit_box's Frame arm passes `ty` through as-is).
    let expected_td = "60 200 Td";
    assert!(
        hay.contains(expected_td),
        "expected the frame's inner text at its own x-offset ({expected_td:?}):\n{hay}"
    );
    assert!(hay.contains("(hi)"), "expected the frame's inner text glyph run:\n{hay}");
}

#[test]
fn used_images_recurses_into_a_frames_contents() {
    let image_box = PureHorzBox::Image {
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        image: ImageId(0),
    };
    let frame = PureHorzBox::Frame {
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        deco: DecoId(0),
        contents: vec![(Length::ZERO, image_box)],
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(10.0),
            baseline_y: Length::pt(50.0),
            contents: vec![(Length::ZERO, frame)],
        }],
    };
    let images = vec![ImageResource {
        samples: vec![0u8; 3 * 2 * 2],
        px_w: 2,
        px_h: 2,
        jpeg_dct: None,
        pdf: None,
    }];
    let geometry = geometry();
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &images)
        .expect("render must succeed");
    let hay = String::from_utf8_lossy(&bytes);

    assert!(
        hay.contains("/Subtype /Image"),
        "an Image nested inside a Frame's contents must still get an XObject \
         (used_images/scan_box_images must recurse into Frame):\n{hay}"
    );
    assert!(hay.contains("/XObject"), "expected an /XObject resource entry:\n{hay}");
    assert!(hay.contains(" Do"), "expected a content-stream Do operator:\n{hay}");
}
