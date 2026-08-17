//! Slice 1 (raster images; `docs/plans/math-images.md`) end-to-end test:
//! compile a real `.saty`-shaped document that `load-image`s the checked-in
//! `dot.png` fixture and `use-image-by-width`s it into a paragraph, then
//! render the result with this crate's own `render_pdf` and inspect the PDF
//! bytes. Mirrors `satysfi-lang/tests/eval.rs`'s `compile_document_with_stdlib`
//! helper (the multi-file loader isn't pulled in for a single-file test);
//! `document`/`+p` are ordinary `stdja-mini` stdlib bindings, not
//! primitives, so that package's prelude is concatenated ahead of the
//! fixture source the same way `satysfi-cli`'s `merge_program` does.

use std::path::Path;
use std::rc::Rc;

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::DocumentValue;

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
/// `satysfi-lang/tests/fixtures/dot.png`; duplicated because `load-image`
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

/// `document`/`+p`/`\emph` are no longer hardcoded Rust natives (phase 4 of
/// the satysfi-lang port): they're ordinary bindings in the real
/// `stdja-mini` stdlib package (`lib-satysfi/dist/packages/stdja-mini.satyh`).
/// Compile `src` the same way the multi-file loader's `merge_program` does —
/// concatenate the package's prelude ahead of `src`'s own.
fn compile_document_with_stdlib(src: &str) -> Rc<DocumentValue> {
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-satysfi/dist/packages/stdja-mini.satyh");
    let lib_src = std::fs::read_to_string(&lib_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lib_path.display()));
    let lib_file = satysfi_syntax::parse_file(&lib_src).expect("stdlib must parse");
    let doc_file = satysfi_syntax::parse_file(src).expect("fixture source must parse");

    let mut prelude = lib_file.prelude;
    prelude.extend(doc_file.prelude);
    let merged = satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: doc_file.in_kw,
        body: doc_file.body,
        eoi: doc_file.eoi,
    };
    satysfi_lang::compile_document_cst(&merged, &Mono).expect("document must compile")
}

#[test]
fn image_in_a_paragraph_renders_as_a_pdf_image_xobject() {
    // let-inline ctx \fig it = use-image-by-width (load-image `<fixture>`) 40pt
    // in
    // document (||) '< +p { here: \fig{ignored} done } >
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
            .any(|(_, bx)| matches!(bx, satysfi_backend::PureHorzBox::Image { .. }))
    });
    assert!(has_image_box, "expected an Image box on the placed line");

    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images)
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
    // `DeviceRGB`/`BitsPerComponent 8`, per this slice's flat-RGB8 XObject
    // encoding (see satysfi-backend's `ImageResource` doc comment).
    assert!(text.contains("/DeviceRGB"), "expected a DeviceRGB color space: {text}");
    assert!(
        text.contains("/BitsPerComponent 8"),
        "expected 8-bit samples: {text}"
    );
}

#[test]
fn text_only_document_has_no_xobject_and_is_unaffected_by_the_images_parameter() {
    // A regression guard for the plan's "text-only documents render
    // byte-identically to now" invariant: a document that never touches
    // `load-image` produces an empty `DocumentValue::images`, and the
    // writer must emit no `/XObject` resource or Image XObject at all.
    let doc = compile_document_with_stdlib("document (||) '< +p { hello world } >");
    assert!(doc.images.is_empty());

    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/XObject"), "no image was ever placed: {text}");
    assert!(!text.contains("/Subtype /Image"));
}
