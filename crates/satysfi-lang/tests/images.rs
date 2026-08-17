//! Slice 1 (raster images; `docs/plans/math-images.md`) runtime round trip:
//! `load-image` decodes a real, tiny checked-in PNG fixture and
//! `use-image-by-width` scales it into a `PureHorzBox::Image`, driven
//! through `eval::Interp` the same way `prims_phase4.rs` exercises other
//! primitives (`Ast` apply chains built by hand, no parser involved). A
//! typecheck-only (no file I/O) round trip lives in `tests/typecheck.rs`.

use satysfi_backend::{FontKey, FontMetrics, HorzBox, ImageResource, Length, PureHorzBox};
use satysfi_lang::ast::Ast;
use satysfi_lang::eval;
use satysfi_lang::primitives;
use satysfi_lang::prim_types;
use satysfi_lang::value::Value;
use satysfi_syntax::Span;
use std::path::Path;

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

// ---- small Ast-builder helpers (mirrors prims_phase4.rs) -------------------

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

/// The checked-in fixture: an 8x4 (2:1 aspect ratio) RGB8 PNG. Deliberately
/// non-square, so a test asserting `use-image-by-width`'s height computation
/// actually exercises the aspect-ratio math rather than a width == height
/// coincidence. `load-image` resolves its path against the process's
/// current working directory (see `primitives.rs`'s `prim_load_image` doc
/// comment), so tests must hand it an absolute path rather than relying on
/// `cargo test`'s (unspecified, and in practice workspace-root) CWD.
fn fixture_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dot.png")
        .to_str()
        .expect("fixture path must be valid UTF-8")
        .to_string()
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
// load-image
// ============================================================================

#[test]
fn load_image_decodes_the_fixture_and_records_its_pixel_dimensions() {
    let ast = app1(var("load-image"), str_lit(&fixture_path()));
    let out = run(&ast);
    assert!(
        matches!(out.value, Value::Image(_)),
        "expected an image value, got {:?}",
        out.value
    );
    assert_eq!(out.images.len(), 1, "load-image should push exactly one resource");
    assert_eq!(out.images[0].px_w, 8);
    assert_eq!(out.images[0].px_h, 4);
    // 8x4 RGB8, no padding: 4 rows * 8 px * 3 bytes/px.
    assert_eq!(out.images[0].samples.len(), 4 * 8 * 3);
}

#[test]
fn load_image_reports_a_clean_error_for_a_missing_file() {
    let ast = app1(
        var("load-image"),
        str_lit("/nonexistent/path/does-not-exist-satysfi-rust.png"),
    );
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    let err = interp
        .eval(&env, &ast)
        .expect_err("a missing file must be a clean EvalError, not a panic");
    assert!(
        err.to_string().contains("load-image"),
        "error should name the primitive: {err}"
    );
}

// ============================================================================
// use-image-by-width
// ============================================================================

#[test]
fn use_image_by_width_scales_height_by_the_source_aspect_ratio() {
    // use-image-by-width (load-image <fixture>) 40pt
    let ast = app2(
        "use-image-by-width",
        app1(var("load-image"), str_lit(&fixture_path())),
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
                    // 8x4 source: height = width * (4 / 8) = width * 0.5.
                    assert_eq!(*height, Length::pt(20.0));
                    assert_eq!(image.0, 0);
                }
                other => panic!("expected an Image box, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

#[test]
fn use_image_by_width_is_not_a_line_break_point_once_placed() {
    let ast = app2(
        "use-image-by-width",
        app1(var("load-image"), str_lit(&fixture_path())),
        len(40.0),
    );
    let out = run(&ast);
    let Value::InlineBoxes(boxes) = out.value else {
        panic!("expected inline-boxes")
    };
    let HorzBox::Pure(pure) = &boxes[0];
    assert!(!pure.is_glue());
    assert_eq!(pure.natural_width(), Length::pt(40.0));
}

// ============================================================================
// Registration coverage: both new names resolve in base_env AND typecheck
// (mirrors prims_phase4.rs's own coverage section).
// ============================================================================

const NEW_NAMES: &[&str] = &["load-image", "use-image-by-width"];

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
