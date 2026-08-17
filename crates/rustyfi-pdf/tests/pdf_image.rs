//! `load-pdf-image` PDF writer coverage (docs/plans/design-load-pdf-image.md
//! §3): a page whose only content is a `PureHorzBox::Image` box carrying a
//! `pdf: Some(PdfPageResource { .. })` payload must render as a `/Subtype
//! /Form` XObject (not the raster `/Subtype /Image` path — that stays
//! covered, byte-for-byte, by `image.rs`), remapping its imported
//! `/Resources` object graph and scaling it onto the box with the §3.3 CTM
//! formula. Mirrors `graphics.rs`/`frame_box.rs`'s hand-built-`Page` style
//! (no `rustyfi-lang` parse/eval needed to exercise the writer in
//! isolation); `crates/rustyfi-lang/tests/pdf_images.rs` covers the
//! `load-pdf-image` PRIMITIVE (parsing, MediaBox extraction, error paths)
//! separately.
//!
//! Numeric assertions (the placement `cm` scale/translate, `/BBox`,
//! `/Matrix`, the copied `/ExtGState` object) are checked by re-parsing the
//! rendered PDF bytes with `lopdf` (this crate's own PDF-reader dependency)
//! rather than by matching formatted-float substrings — robust against
//! `pdf-writer`'s own number-formatting choices, and a stronger check
//! besides (structural, not textual).

use rustyfi_backend::{
    ImageId, ImageResource, ImportedObjects, Length, ObjRepr, Page, PageGeometry, PdfPageResource,
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

/// A `100pt x 50pt` MediaBox source page whose content references an
/// indirect `/ExtGState` object (`GS1`, local id `7`) via local id `0`'s
/// `/Resources` dict — the same shape `prim_load_pdf_image`'s real `lopdf`
/// walk (`convert_pdf_object`, `rustyfi-lang/src/primitives.rs`) would
/// produce for a source PDF with one indirect resource, built here by hand
/// so this test exercises `write_form_xobjects`/`place_form` in isolation
/// from the parser.
fn pdf_page_resource() -> PdfPageResource {
    let ext_gstate = ObjRepr::Dict(vec![
        (b"Type".to_vec(), ObjRepr::Name(b"ExtGState".to_vec())),
        (b"CA".to_vec(), ObjRepr::Real(1.0)),
    ]);
    let resources_dict = ObjRepr::Dict(vec![(
        b"ExtGState".to_vec(),
        ObjRepr::Dict(vec![(b"GS1".to_vec(), ObjRepr::Ref(7))]),
    )]);
    PdfPageResource {
        media_box: (0.0, 0.0, 100.0, 50.0),
        content: b"/GS1 gs 1 0 0 RG 5 5 90 40 re S".to_vec(),
        resources: ImportedObjects(vec![(0, resources_dict), (7, ext_gstate)]),
    }
}

fn page_with_pdf_image_box() -> (Page, Vec<ImageResource>) {
    let images = vec![ImageResource {
        samples: Vec::new(),
        px_w: 0,
        px_h: 0,
        jpeg_dct: None,
        pdf: Some(pdf_page_resource()),
    }];
    let ibox = PureHorzBox::Image {
        // 100x50 MediaBox scaled to 40x20 -> sx = sy = 0.4 (§3.3).
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        image: ImageId(0),
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, ibox)],
        }],
    };
    (page, images)
}

/// Find the (sole) `/Subtype /Form` XObject stream in `doc`.
fn find_form_stream(doc: &lopdf::Document) -> &lopdf::Stream {
    doc.objects
        .values()
        .find_map(|obj| {
            let stream = obj.as_stream().ok()?;
            if stream.dict.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Form".as_slice())
            {
                Some(stream)
            } else {
                None
            }
        })
        .expect("expected exactly one /Subtype /Form XObject stream")
}

fn as_f64_array(obj: &lopdf::Object) -> Vec<f64> {
    obj.as_array()
        .expect("expected an array")
        .iter()
        .map(|o| o.as_float().expect("expected a numeric array entry") as f64)
        .collect()
}

#[test]
fn a_pdf_page_image_box_renders_as_a_form_xobject_not_a_raster_image() {
    let (page, images) = page_with_pdf_image_box();
    let geometry = geometry();
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &images)
        .expect("render_pdf must succeed with a PDF-page Image box present");
    assert!(bytes.starts_with(b"%PDF-"), "output must start with a PDF header");

    let doc = lopdf::Document::load_mem(&bytes).expect("rendered output must itself be valid PDF");

    // ---- The Form XObject itself -----------------------------------------
    let form = find_form_stream(&doc);
    assert_eq!(
        form.dict.get(b"Type").and_then(|o| o.as_name()).ok(),
        Some(b"XObject".as_slice())
    );
    assert!(
        form.dict.get(b"Filter").is_err(),
        "the imported content stream must be embedded /Filter-less (uncompressed, \
         same style as this writer's raster Image XObjects): {:?}",
        form.dict
    );

    // /BBox is the source page's raw MediaBox, unscaled (§3.1) — the scale
    // lives entirely in the placement `cm`, checked below.
    let bbox = as_f64_array(form.dict.get(b"BBox").expect("Form XObject must carry /BBox"));
    assert_eq!(bbox, vec![0.0, 0.0, 100.0, 50.0]);

    // Explicit identity /Matrix (§3.1).
    let matrix = as_f64_array(
        form.dict
            .get(b"Matrix")
            .expect("Form XObject must carry /Matrix"),
    );
    assert_eq!(matrix, vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    // The imported content stream is re-emitted byte-identical (already
    // inflated by the importer; `pdf_res.content` above is plain ASCII, so
    // no encoding round-trip risk here).
    assert_eq!(form.content, b"/GS1 gs 1 0 0 RG 5 5 90 40 re S");

    // ---- The remapped /Resources graph -------------------------------
    let resources = form
        .dict
        .get(b"Resources")
        .and_then(|o| o.as_dict())
        .expect("Form XObject must carry a /Resources dict");
    let ext_gstates = resources
        .get(b"ExtGState")
        .and_then(|o| o.as_dict())
        .expect("/Resources must carry the imported /ExtGState dict");
    let gs1_ref = ext_gstates
        .get(b"GS1")
        .and_then(|o| o.as_reference())
        .expect("/ExtGState/GS1 must be a (remapped) indirect reference");
    let gs1 = doc
        .get_dictionary(gs1_ref)
        .expect("the referenced ExtGState object must have been copied into the output PDF");
    assert_eq!(gs1.get(b"Type").and_then(|o| o.as_name()).ok(), Some(b"ExtGState".as_slice()));
    assert_eq!(gs1.get(b"CA").and_then(|o| o.as_float()).ok(), Some(1.0f32));

    // ---- Placement: the page's content stream scales+translates the Form
    // per §3.3 (`sx = w/(x1-x0)`, `sy = h/(y1-y0)`, origin translated to
    // (tx, ty)) and invokes it by name, disjoint from `ImN`.
    let (&page_num, &page_id) = doc.get_pages().iter().next().expect("one page");
    assert_eq!(page_num, 1);
    let content = doc
        .get_and_decode_page_content(page_id)
        .expect("page content stream must decode");

    let cm_op = content
        .operations
        .iter()
        .rev()
        .find(|op| op.operator == "cm")
        .expect("expected a `cm` operator placing the Form XObject");
    let cm: Vec<f64> = cm_op
        .operands
        .iter()
        .map(|o| o.as_float().expect("cm operands must be numeric") as f64)
        .collect();
    // sx = 40/100 = 0.4; sy = 20/50 = 0.4; MediaBox origin (0,0) so no
    // extra translation beyond the box's own placed (tx, ty). Paper height
    // 300pt, baseline_y 100pt -> ty = 300 - 100 = 200; tx = line.x = 50
    // (mirrors `graphics.rs`/`frame_box.rs`'s own placed-anchor math).
    let expected = [0.4, 0.0, 0.0, 0.4, 50.0, 200.0];
    for (got, want) in cm.iter().zip(expected.iter()) {
        assert!(
            (got - want).abs() < 1e-4,
            "cm operands {cm:?} do not match expected {expected:?}"
        );
    }

    let do_op = content
        .operations
        .iter()
        .rev()
        .find(|op| op.operator == "Do")
        .expect("expected a `Do` operator invoking the Form XObject");
    let name = do_op.operands[0]
        .as_name()
        .expect("Do operand must be a resource name");
    assert!(
        name.starts_with(b"Fm"),
        "Form XObjects must use the disjoint `Fm{{id}}` resource-name convention, got {:?}",
        String::from_utf8_lossy(name)
    );

    // No raster Image XObject should appear at all for this document.
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains("/Subtype /Image"),
        "a PDF-page image must not ALSO emit a raster Image XObject: {text}"
    );
}

#[test]
fn text_only_document_emits_no_form_xobject() {
    // Regression guard mirroring `image.rs`'s own text-only invariant: a
    // page with no Image box at all must emit neither Image nor Form
    // XObjects.
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![],
        }],
    };
    let geometry = geometry();
    let bytes = rustyfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[]).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/Subtype /Form"));
    assert!(!text.contains("/XObject"));
}
