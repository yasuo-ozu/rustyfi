//! Raster-images end-to-end test: compile a real `.saty`-shaped
//! document that `load-image`s the checked-in `dot.png` fixture and
//! `use-image-by-width`s it into a paragraph, then render the result with this
//! crate's own `render_pdf` and inspect the PDF bytes. Mirrors
//! `rustyfi-lang/tests/eval.rs`'s `compile_document_with_stdlib` helper (the
//! multi-file loader isn't pulled in for a single-file test); `document`/`+p`
//! are ordinary `stdja-mini` stdlib bindings, not primitives, so that package's
//! prelude is concatenated ahead of the fixture source the same way `rustyfi`'s
//! `merge_program` does.

use std::path::Path;
use std::rc::Rc;

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::DocumentValue;

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

/// The checked-in fixture: an 8x4 (2:1 aspect ratio) RGB8 PNG (same bytes as
/// `rustyfi-lang/tests/fixtures/dot.png`; duplicated because `load-image`
/// resolves against the process's CWD, so each crate's tests carry their own
/// copy resolved via their own `CARGO_MANIFEST_DIR`, matching how other
/// fixtures in this workspace are organized per-crate).
fn fixture_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dot.png")
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .to_string()
}

/// The checked-in JPEG fixture (JPEG DCTDecode passthrough slice): a tiny
/// 8x4 baseline (SOF0, 3-component YCbCr/RGB) JPEG, generated with Pillow —
/// same pixel dimensions as `dot.png` above so the two tests are directly
/// comparable, deliberately NOT square for the same reason `dot.png` isn't
/// (see `rustyfi-lang/tests/images.rs`'s `fixture_path` doc comment).
fn jpeg_fixture_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dot.jpg")
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .to_string()
}

/// `document`/`+p`/`\emph` are ordinary bindings in the real `stdja-mini`
/// stdlib package (`lib-rustyfi/dist/packages/stdja-mini.satyh`), not Rust
/// natives. Compile `src` the same way the multi-file loader's
/// `merge_program` does — concatenate the package's prelude ahead of
/// `src`'s own.
fn compile_document_with_stdlib(src: &str) -> Rc<DocumentValue> {
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/packages/stdja-mini.satyh");
    let lib_src = std::fs::read_to_string(&lib_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lib_path.display()));
    let lib_file = rustyfi_syntax::parse_file(&lib_src).expect("stdlib must parse");
    let doc_file = rustyfi_syntax::parse_file(src).expect("fixture source must parse");

    let mut prelude = lib_file.prelude;
    prelude.extend(doc_file.prelude);
    let merged = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: doc_file.in_kw,
        body: doc_file.body,
        eoi: doc_file.eoi,
    };
    rustyfi_lang::compile_document_cst(&merged, &Mono).expect("document must compile")
}

#[test]
fn image_in_a_paragraph_renders_as_a_pdf_image_xobject() {
    let src = "let-inline ctx \\fig it = use-image-by-width (load-image `__FIXTURE__`) 40pt
         in
         document (||) '< +p { here: \\fig{ignored} done } >"
        .replace("__FIXTURE__", &fixture_path());

    let doc = compile_document_with_stdlib(&src);
    assert_eq!(doc.pages.len(), 1, "expected a single page");
    assert_eq!(
        doc.images.len(),
        1,
        "load-image should have decoded exactly one image into DocumentValue::images"
    );

    // The placed line must actually carry the Image box (not just the
    // surrounding text), so the writer below has something to render.
    let has_image_box = doc.pages[0].lines.iter().any(|line| {
        line.contents
            .iter()
            .any(|(_, bx)| matches!(bx, rustyfi_backend::PureHorzBox::Image { .. }))
    });
    assert!(has_image_box, "expected an Image box on the placed line");

    let bytes = rustyfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images)
        .expect("render_pdf must succeed with an Image box present");

    assert!(
        bytes.starts_with(b"%PDF-"),
        "output must start with a PDF header"
    );

    // Content streams in this writer are uncompressed (see `lib.rs`'s module
    // doc comment), so both the resource dictionary and the operator stream
    // are directly visible as bytes.
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("/Subtype /Image"),
        "expected an Image XObject (/Subtype /Image): {text}"
    );
    assert!(
        text.contains("/XObject"),
        "expected an /XObject resource entry: {text}"
    );
    assert!(
        text.contains(" Do"),
        "expected a content-stream `Do` (x_object) operator: {text}"
    );
    // `DeviceRGB`/`BitsPerComponent 8`, per this test's flat-RGB8 XObject
    // encoding (see rustyfi-backend's `ImageResource` doc comment).
    assert!(text.contains("/DeviceRGB"), "expected a DeviceRGB color space: {text}");
    assert!(
        text.contains("/BitsPerComponent 8"),
        "expected 8-bit samples: {text}"
    );
}

#[test]
fn text_only_document_has_no_xobject_and_is_unaffected_by_the_images_parameter() {
    // A regression guard for the "text-only documents render
    // byte-identically to now" invariant: a document that never touches
    // `load-image` produces an empty `DocumentValue::images`, and the
    // writer must emit no `/XObject` resource or Image XObject at all.
    let doc = compile_document_with_stdlib("document (||) '< +p { hello world } >");
    assert!(doc.images.is_empty());

    let bytes = rustyfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/XObject"), "no image was ever placed: {text}");
    assert!(!text.contains("/Subtype /Image"));
}

// ============================================================================
// JPEG DCTDecode passthrough
// ============================================================================

#[test]
fn jpeg_image_embeds_via_dctdecode_passthrough_not_a_flate_reencode() {
    let src = "let-inline ctx \\fig it = use-image-by-width (load-image `__FIXTURE__`) 40pt
         in
         document (||) '< +p { here: \\fig{ignored} done } >"
        .replace("__FIXTURE__", &jpeg_fixture_path());

    let doc = compile_document_with_stdlib(&src);
    assert_eq!(doc.images.len(), 1);
    // The eager RGB8 decode (still needed for `use-image-by-width`'s
    // aspect-ratio math and the HTML backend) sees the same 8x4 pixel grid
    // `dot.png` does.
    assert_eq!(doc.images[0].px_w, 8);
    assert_eq!(doc.images[0].px_h, 4);
    // The JPEG-specific passthrough metadata (`ImageResource::jpeg_dct`)
    // must ALSO be present: `dot.jpg` is a baseline (SOF0), 8-bit,
    // 3-component YCbCr/RGB JPEG, exactly what `sniff_baseline_jpeg_dct`
    // accepts.
    let dct = doc.images[0]
        .jpeg_dct
        .as_ref()
        .expect("a baseline JPEG source must record jpeg_dct");
    assert_eq!(dct.components, 3, "dot.jpg is a 3-component YCbCr/RGB JPEG");

    let bytes = rustyfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images)
        .expect("render_pdf must succeed with a JPEG Image box present");
    assert!(bytes.starts_with(b"%PDF-"));

    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Subtype /Image"));
    assert!(text.contains("/XObject"));
    assert!(text.contains(" Do"));
    assert!(
        text.contains("/Filter /DCTDecode"),
        "expected the JPEG to be embedded via a DCTDecode passthrough: {text}"
    );
    assert!(text.contains("/DeviceRGB"), "3-component JPEG maps to DeviceRGB: {text}");
    assert!(text.contains("/BitsPerComponent 8"));
    assert!(
        !text.contains("/FlateDecode"),
        "a DCTDecode passthrough image must not ALSO be FlateDecode re-encoded: {text}"
    );

    // The embedded XObject stream must be the ORIGINAL JPEG file's bytes,
    // verbatim — not a decode-then-recompress. This writer emits streams
    // uncompressed at the PDF-container level (no filter chaining on top of
    // `/Filter /DCTDecode`), so the fixture's exact byte sequence must
    // appear as a contiguous run inside the PDF output.
    let original = std::fs::read(jpeg_fixture_path()).expect("fixture must be readable");
    let embedded_verbatim = bytes.windows(original.len()).any(|w| w == original.as_slice());
    assert!(
        embedded_verbatim,
        "expected the original {}-byte JPEG embedded verbatim in the PDF, not a re-encoded copy",
        original.len()
    );

    // And to rule out a coincidental match against the flattened,
    // decode-then-reencode form: 8x4 RGB8 with no padding is only 96 bytes,
    // far shorter than (and byte-for-byte different from) the original
    // JPEG file — so this is genuinely asserting "original JPEG size", not
    // "decoded RGB size", per this test's whole point.
    let decoded_rgb_len = 8 * 4 * 3;
    assert_ne!(
        original.len(),
        decoded_rgb_len,
        "sanity: fixture's JPEG size must differ from its flattened RGB8 size"
    );
}
