//! `load-pdf-image` runtime round trip, mirroring `tests/images.rs`'s style
//! exactly (Ast apply chains driven through `eval::Interp`, no parser
//! involved): a real, tiny PDF is generated on the fly with `lopdf` (this
//! crate's own PDF-reader dependency, also perfectly usable as a writer for
//! a test fixture) and `load-pdf-image`d, then `use-image-by-width` is
//! checked against the source page's `/MediaBox` aspect ratio — the
//! PDF-page analogue of `images.rs`'s raster-image tests. Error-path
//! coverage (missing file, bad page number) lives here too (design doc §4's
//! error table).

use rustyfi_backend::{FontKey, FontMetrics, HorzBox, ImageResource, Length, PureHorzBox};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval;
use rustyfi_lang::prim_types;
use rustyfi_lang::primitives;
use rustyfi_lang::value::Value;
use rustyfi_syntax::Span;

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

// ---- small Ast-builder helpers (mirrors images.rs/prims_phase4.rs) --------

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn len(pt: f64) -> Ast {
    Ast::Length(Length::pt(pt))
}

fn str_lit(s: &str) -> Ast {
    Ast::Str(s.to_string())
}

fn int_lit(n: i64) -> Ast {
    Ast::Int(n)
}

struct Run {
    value: Value,
    images: Vec<ImageResource>,
}

fn run(ast: &Ast) -> Run {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let value = interp.eval(&env, ast).expect("evaluation should succeed");
    Run {
        value,
        images: interp.images,
    }
}

// ============================================================================
// A tiny, self-built one-page PDF fixture, generated with `lopdf`'s own
// writer side (the crate this port added specifically to *read* foreign
// PDFs, §S0) rather than checked in as an opaque binary blob — deterministic
// and reviewable as plain Rust. `/MediaBox` and `/Resources` are placed on
// the shared `Pages` node rather than the leaf `Page` (deliberately, not an
// oversight): `lopdf` does not auto-resolve page-tree inheritance
// (design doc §1 Risk 3 / §4's `resolve_pdf_media_box` /
// `resolve_pdf_resources_object`), so this fixture directly exercises that
// inheritance-walk code path, not just the never-inherited case.
// ============================================================================

/// `100pt x 50pt` MediaBox, one page whose content stream references an
/// indirect `/ExtGState` object (`GS1`) via `/Resources` — so a test can
/// confirm the importer's transitive object walk actually copied that
/// referenced object into the output PDF, not just the page's own content.
fn build_fixture_pdf() -> std::path::PathBuf {
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let gs_id = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "CA" => 1.0,
    });
    let resources_id = doc.add_object(dictionary! {
        "ExtGState" => dictionary! {
            "GS1" => Object::Reference(gs_id),
        },
    });
    let content = b"/GS1 gs 1 0 0 RG 5 5 90 40 re S".to_vec();
    let content_id = doc.add_object(Stream::new(dictionary! {}, content));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages_id),
        "Contents" => Object::Reference(content_id),
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
            "Resources" => Object::Reference(resources_id),
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 50.into()],
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let path = std::env::temp_dir().join(format!(
        "rustyfi_rust_test_pdf_image_fixture_{}_{:?}.pdf",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    doc.save(&path)
        .expect("saving the fixture PDF must succeed");
    path
}

fn fixture_path() -> String {
    build_fixture_pdf()
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .to_string()
}

// ============================================================================
// load-pdf-image
// ============================================================================

#[test]
fn load_pdf_image_reads_the_media_box_into_intrinsic_dims() {
    let ast = app2("load-pdf-image", str_lit(&fixture_path()), int_lit(1));
    let out = run(&ast);
    assert!(
        matches!(out.value, Value::Image(_)),
        "expected an image value, got {:?}",
        out.value
    );
    assert_eq!(
        out.images.len(),
        1,
        "load-pdf-image should push exactly one resource"
    );
    let res = &out.images[0];
    let pdf = res.pdf.as_ref().expect("resource must carry a pdf payload");
    assert_eq!(pdf.media_box, (0.0, 0.0, 100.0, 50.0));
    assert_eq!(res.intrinsic_dims_pt(), (100.0, 50.0));
    // Raster fields are left at their inert defaults for a PDF-kind resource.
    assert!(res.samples.is_empty());
    assert_eq!(res.px_w, 0);
    assert_eq!(res.px_h, 0);
    assert!(res.jpeg_dct.is_none());
}

#[test]
fn load_pdf_image_reports_a_clean_error_for_a_missing_file() {
    let ast = app2(
        "load-pdf-image",
        str_lit("/nonexistent/path/does-not-exist-rustyfi.pdf"),
        int_lit(1),
    );
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let err = interp
        .eval(&env, &ast)
        .expect_err("a missing file must be a clean EvalError, not a panic");
    assert!(
        err.to_string().contains("load-pdf-image") && err.to_string().contains("cannot open"),
        "error should name the primitive and the failure kind: {err}"
    );
}

#[test]
fn load_pdf_image_rejects_page_number_zero() {
    let ast = app2("load-pdf-image", str_lit(&fixture_path()), int_lit(0));
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let err = interp
        .eval(&env, &ast)
        .expect_err("page 0 must be a clean EvalError, not a panic");
    assert!(
        err.to_string().contains("page number must be >= 1"),
        "error should match design doc §4's message: {err}"
    );
}

#[test]
fn load_pdf_image_rejects_an_out_of_range_page() {
    let ast = app2("load-pdf-image", str_lit(&fixture_path()), int_lit(2));
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let err = interp
        .eval(&env, &ast)
        .expect_err("a one-page PDF has no page 2 — must be a clean EvalError");
    assert!(
        err.to_string().contains("has no page 2"),
        "error should name the requested page: {err}"
    );
}

// ============================================================================
// use-image-by-width
// ============================================================================

#[test]
fn use_image_by_width_scales_height_by_the_media_box_aspect_ratio() {
    // use-image-by-width (load-pdf-image <fixture> 1) 40pt
    let ast = app2(
        "use-image-by-width",
        app2("load-pdf-image", str_lit(&fixture_path()), int_lit(1)),
        len(40.0),
    );
    let out = run(&ast);
    match out.value {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1);
            match &boxes[0] {
                HorzBox::Pure(PureHorzBox::Image {
                    width,
                    height,
                    image,
                }) => {
                    assert_eq!(*width, Length::pt(40.0));
                    // 100x50 MediaBox: height = width * (50 / 100) = width * 0.5.
                    assert_eq!(*height, Length::pt(20.0));
                    assert_eq!(image.0, 0);
                }
                other => panic!("expected an Image box, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// Registration coverage: mirrors images.rs's own coverage section.
// ============================================================================

const NEW_NAMES: &[&str] = &["load-pdf-image"];

#[test]
fn every_new_primitive_resolves_in_base_env() {
    let env = primitives::base_env();
    for name in NEW_NAMES {
        assert!(
            env.lookup(name).is_some(),
            "primitive `{name}` is not bound in base_env()"
        );
    }
}

#[test]
fn every_new_primitive_has_a_registered_type() {
    for name in NEW_NAMES {
        assert!(
            prim_types::primitive_type(name).is_some(),
            "primitive `{name}` has no registered type"
        );
    }
}
