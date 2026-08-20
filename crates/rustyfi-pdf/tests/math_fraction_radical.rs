//! (fractions + radicals via a graphics- rules channel): proof that
//! `Math::Fraction`/`Math::Radical` produce a REAL bar/radical-sign `Fill`
//! at the correct box-local (y-**up**) position — replacing the old ASCII
//! `"num / den"` / U+221A stand-ins — under BOTH a real OpenType MATH font
//! (`MathC`'s real-formula branch) and `Base14Metrics` (`MathC`'s
//! no-MATH-table fallback branch, the zero-regression floor: no bundled
//! fixture reaches these arms, so this is the ONLY place base-14
//! fraction/radical layout is exercised at all).
//!
//! Font discovery mirrors `tests/math_table.rs`/`tests/math_font.rs`:
//! fontconfig first, then a handful of common distro/nix paths, then a
//! graceful skip for the MATH-font-only tests.

use std::path::{Path as FsPath, PathBuf};
use std::process::Command;

use rustyfi_backend::{
    graphics_bbox, Color, FontKey, FontMetrics, GraphicsElem, HorzBox, Length, Page, PageGeometry,
    PlacedLine, PureHorzBox,
};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use rustyfi_pdf::{render_pdf, render_pdf_ttf, Base14Metrics, TtfFontStore};

// ----------------------------------------------------------------------
// Font discovery (copied, not shared — matches this repo's existing
// per-file convention for these small fixture-locator helpers).
// ----------------------------------------------------------------------

fn find_math_font() -> Option<PathBuf> {
    // Slice B, re-baselined for the upstream-correct default (see
    // `scripts/download-fonts.sh`'s header comment): the repo now bundles
    // the REAL Latin Modern Math at
    // `lib-rustyfi/dist/fonts/latinmodern-math.otf` (fetched by
    // `scripts/download-fonts.sh`, same as ipaexm/Junicode) and wires it as
    // `default-font.satysfi-hash`'s `"math"` default. Check it FIRST so this
    // test no longer depends on a host-wide font install once that script
    // has been run. Every assertion in this file recomputes its expected
    // value from whatever font is actually loaded (`store.math_constants`
    // etc.), so it holds for LM Math exactly as it did for DejaVu Math TeX
    // Gyre. Fall back to the previously-bundled DejaVu Math TeX Gyre (still
    // fetched as a secondary abbrev) only if LM Math isn't present, then
    // fontconfig/distro paths.
    let bundled_lmmath = FsPath::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    if bundled_lmmath.is_file() {
        return Some(bundled_lmmath);
    }
    let bundled_dejavu = FsPath::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/DejaVuMathTeXGyre.ttf");
    if bundled_dejavu.is_file() {
        return Some(bundled_dejavu);
    }

    for family in ["DejaVu Math TeX Gyre", "Noto Sans Math"] {
        if let Ok(output) = Command::new("fc-match")
            .args(["--format=%{file}", family])
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty()
                    && FsPath::new(&path).is_file()
                    && (path.contains("Math") || path.contains("math"))
                {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }

    for candidate in [
        "/usr/share/texmf/fonts/opentype/public/dejavu-otf/DejaVuMathTeXGyre.ttf",
        "/usr/share/fonts/opentype/dejavu-math-tex-gyre/DejaVuMathTeXGyre.ttf",
        "/usr/share/fonts/truetype/tex-gyre/texgyredejavu-math.otf",
        "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/OTF/NotoSansMath-Regular.otf",
        "/run/current-system/sw/share/fonts/truetype/NotoSansMath-Regular.ttf",
        "/run/current-system/sw/share/X11/fonts/NotoSansMath-Regular.ttf",
    ] {
        if FsPath::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }

    None
}

macro_rules! need_math_font {
    () => {
        match find_math_font() {
            Some(path) => path,
            None => {
                eprintln!(
                    "skipping: no math-capable OpenType font found on this system \
                     (tried `fc-match` for \"DejaVu Math TeX Gyre\"/\"Noto Sans Math\" \
                     and common nix/distro paths)"
                );
                return;
            }
        }
    };
}

// ----------------------------------------------------------------------
// Pipeline helpers (mirrors `math_table.rs`'s `run_math`/`with_ctx`).
// ----------------------------------------------------------------------

fn run_math(src: &str, metrics: &dyn FontMetrics) -> Result<Value, CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mut interp = eval::Interp::new(metrics);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))?)
}

fn with_ctx(body: &str) -> String {
    format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    )
}

/// Unwrap `embed-math ctx (...)`'s single `PureHorzBox::Math`.
fn math_box(v: Value) -> PureHorzBox {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(m @ PureHorzBox::Math { .. }) => m,
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

fn as_math_parts(bx: PureHorzBox) -> (Length, Length, Length, Vec<rustyfi_backend::MathGlyph>, Vec<GraphicsElem>) {
    match bx {
        PureHorzBox::Math {
            width,
            height,
            depth,
            glyphs,
            rules,
        } => (width, height, depth, glyphs, rules),
        other => panic!("expected PureHorzBox::Math, got {other:?}"),
    }
}

/// Every `Fill` in `rules` whose bounding box (`graphics_bbox`) is
/// non-degenerate (more than one distinct point) — i.e. skips the height-
/// only "extent marker" `Fill`s (§B2's `l_extra`-reporting technique in the
/// `Radical` arm), leaving only the actually-drawn shapes (bar/overbar,
/// radical sign).
fn drawn_fills(rules: &[GraphicsElem]) -> Vec<(Color, (Length, Length), (Length, Length))> {
    rules
        .iter()
        .filter_map(|r| match r {
            GraphicsElem::Fill(color, _) => {
                let (lo, hi) = graphics_bbox(r).expect("nonempty graphics");
                if lo == hi {
                    None // a single-point "extent marker", not real ink
                } else {
                    Some((*color, lo, hi))
                }
            }
            _ => None,
        })
        .collect()
}

fn approx(a: Length, b: Length, tol: f64) -> bool {
    (a.0 - b.0).abs() < tol
}

// ============================================================================
// Fraction — real MATH font.
// ============================================================================

#[test]
fn fraction_bar_fill_at_axis_with_math_font() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let size = Length::pt(12.0);

    let src = with_ctx(
        "embed-math ctx (math-frac (math-char MathOrd `1`) (math-char MathOrd `2`))",
    );
    let v = run_math(&src, &store).expect("math-frac 1 2 should compile and evaluate");
    let (width, _height, _depth, glyphs, rules) = as_math_parts(math_box(v));

    // No more ASCII '/' stand-in: exactly the two digit glyphs.
    assert_eq!(glyphs.len(), 2, "expected 2 glyphs (num '1', den '2'), got {glyphs:?}");
    assert_eq!(glyphs[0].text, "1");
    assert_eq!(glyphs[1].text, "2");
    assert!(
        glyphs[0].dy.0 > 0.0,
        "numerator should be raised above the axis, got dy={:?}",
        glyphs[0].dy
    );
    assert!(
        glyphs[1].dy.0 < 0.0,
        "denominator should be lowered below the axis, got dy={:?}",
        glyphs[1].dy
    );

    let mc = store
        .math_constants(FontKey(0))
        .expect("MATH font should expose MathConstants");
    let axis = size * mc.axis_height;
    let rule = size * mc.fraction_rule_thickness;

    let fills = drawn_fills(&rules);
    assert_eq!(fills.len(), 1, "expected exactly one drawn Fill (the bar), got {fills:?}");
    let (color, lo, hi) = fills[0];
    assert_eq!(color, Color::Gray(0.0), "expected ctx.text_color (default black)");
    assert!(
        approx(lo.1, axis, 1e-6) && approx(hi.1, axis + rule, 1e-6),
        "bar y-range should be [axis, axis+rule] = [{axis:?}, {:?}], got [{:?}, {:?}]",
        axis + rule,
        lo.1,
        hi.1
    );
    assert!(
        approx(lo.0, Length::ZERO, 1e-6) && approx(hi.0, width, 1e-6),
        "bar x-range should span [0, width] = [0, {width:?}], got [{:?}, {:?}]",
        lo.0,
        hi.0
    );
}

#[test]
fn fraction_bar_fallback_with_base14_is_real_and_deterministic() {
    let size = Length::pt(12.0);
    let src = with_ctx(
        "embed-math ctx (math-frac (math-char MathOrd `1`) (math-char MathOrd `2`))",
    );

    let v1 = run_math(&src, &Base14Metrics).expect("math-frac should compile under base-14");
    let (_, _, _, glyphs1, rules1) = as_math_parts(math_box(v1));
    assert_eq!(glyphs1.len(), 2);
    assert_eq!(glyphs1[0].text, "1");
    assert_eq!(glyphs1[1].text, "2");

    let fills = drawn_fills(&rules1);
    assert_eq!(fills.len(), 1, "base-14 fallback should still draw one bar Fill, got {fills:?}");
    let (_, lo, hi) = fills[0];
    // MathC's documented no-MATH-table fallback ratios (§B1's axis, §B2's
    // frac_rule): axis = 0.25 * size, rule = 0.04 * size.
    let axis = size * 0.25;
    let rule = size * 0.04;
    assert!(
        approx(lo.1, axis, 1e-6) && approx(hi.1, axis + rule, 1e-6),
        "fallback bar y-range should be [0.25*size, 0.25*size+0.04*size], got [{:?}, {:?}]",
        lo.1,
        hi.1
    );

    // Byte-stable: an independent second run produces the identical rules
    // (no nondeterminism sneaks into the fallback path).
    let v2 = run_math(&src, &Base14Metrics).expect("math-frac should compile under base-14");
    let (_, _, _, _, rules2) = as_math_parts(math_box(v2));
    assert_eq!(rules1, rules2, "base-14 fallback rules must be deterministic across runs");
}

// ============================================================================
// Radical — real MATH font.
// ============================================================================

#[test]
fn radical_overbar_and_sign_fills_with_math_font() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let size = Length::pt(12.0);

    let src = with_ctx("embed-math ctx (math-radical None (math-char MathOrd `2`))");
    let v = run_math(&src, &store).expect("math-radical None 2 should compile and evaluate");
    let (_width, height, depth, glyphs, rules) = as_math_parts(math_box(v));

    // No more U+221A stand-in: exactly the one radicand glyph.
    assert_eq!(glyphs.len(), 1, "expected 1 glyph (radicand '2'), got {glyphs:?}");
    assert_eq!(glyphs[0].text, "2");
    assert_eq!(glyphs[0].dy, Length::ZERO, "radicand should stay at its own baseline (dy=0)");

    let mc = store
        .math_constants(FontKey(0))
        .expect("MATH font should expose MathConstants");
    // The radicand is a single char at dy=0, so its own ascender/descender
    // ARE `h_cont`/`d_cont` (`glyphs_extent`'s definition).
    let h_cont = store.ascender(FontKey(0), size);
    let d_cont = store.descender(FontKey(0), size);
    let h_bar = h_cont + size * mc.radical_vertical_gap;
    let t_bar = size * mc.radical_rule_thickness;
    let l_extra = size * mc.radical_extra_ascender;

    let fills = drawn_fills(&rules);
    assert_eq!(
        fills.len(),
        2,
        "expected two drawn Fills (radical sign + overbar), got {fills:?}"
    );
    // The overbar: the Fill whose bbox y-range is exactly [h_bar, h_bar+t_bar].
    let overbar = fills
        .iter()
        .find(|(_, lo, hi)| approx(lo.1, h_bar, 1e-6) && approx(hi.1, h_bar + t_bar, 1e-6))
        .unwrap_or_else(|| panic!("expected an overbar Fill at y=[{h_bar:?},{:?}], got {fills:?}", h_bar + t_bar));
    assert_eq!(overbar.0, Color::Gray(0.0));

    // The radical sign: the OTHER Fill (not the overbar) — it sits entirely
    // to the LEFT of the overbar (the sign is placed before the overbar/
    // radicand, `radical_sign_geometry`'s own `wid` being the overbar's
    // `sign_w` x-offset), and has more than the overbar's 4 corners (an
    // 9-point checkmark polygon vs. a plain rectangle).
    let sign = fills
        .iter()
        .find(|f| *f != overbar)
        .expect("expected a second (radical-sign) Fill distinct from the overbar");
    assert_eq!(sign.0, Color::Gray(0.0));
    assert!(
        sign.2 .0 .0 <= overbar.1 .0 .0 + 1e-6,
        "radical sign's right edge should not extend past the overbar's left edge \
         (sign={sign:?}, overbar={overbar:?})"
    );

    // Overall height (math.ml:882's `h_whole = h_rad +% l_extra`).
    assert!(
        approx(height, h_bar + t_bar + l_extra, 1e-6),
        "expected height = h_bar+t_bar+l_extra = {:?}, got {height:?}",
        h_bar + t_bar + l_extra
    );
    // Depth: NOT upstream's own `d_whole = d_cont` simplification (that
    // formula's upstream comment literally flags it "temporary; should
    // consider the depth of the radical sign") — this port's rules-aware
    // `graphics_bbox` fold in `layout_math_value` (§B2's correctness fix)
    // picks up the SIGN's own deeper ink instead: `default_radical`'s
    // checkmark intentionally reaches `nonnegdpt = d_cont + size*0.1` below
    // the baseline (padding for the downward stroke), which is what a
    // correct depth report needs to avoid colliding with the next line.
    let nonnegdpt = d_cont + size * 0.1;
    assert!(
        approx(depth, nonnegdpt, 1e-6),
        "expected depth = nonnegdpt = d_cont+0.1*size = {nonnegdpt:?}, got {depth:?}"
    );
}

#[test]
fn radical_fallback_with_base14_is_real_and_deterministic() {
    let src = with_ctx("embed-math ctx (math-radical None (math-char MathOrd `2`))");

    let v1 = run_math(&src, &Base14Metrics).expect("math-radical should compile under base-14");
    let (_, _, _, glyphs1, rules1) = as_math_parts(math_box(v1));
    assert_eq!(glyphs1.len(), 1);
    assert_eq!(glyphs1[0].text, "2");

    let fills = drawn_fills(&rules1);
    assert_eq!(
        fills.len(),
        2,
        "base-14 fallback should still draw sign + overbar Fills, got {fills:?}"
    );

    let v2 = run_math(&src, &Base14Metrics).expect("math-radical should compile under base-14");
    let (_, _, _, _, rules2) = as_math_parts(math_box(v2));
    assert_eq!(rules1, rules2, "base-14 fallback rules must be deterministic across runs");
}

// ============================================================================
// E2E: a real rendered PDF shows fill ops, not the old ASCII stand-ins.
// ============================================================================

fn page_for(bx: PureHorzBox, geometry: &PageGeometry) -> Page {
    Page {
            body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: geometry.text_origin.0,
            baseline_y: geometry.text_origin.1 + Length::pt(40.0),
            contents: vec![(Length::ZERO, bx)],
        }],
    }
}

#[test]
fn fraction_and_radical_render_fill_ops_not_ascii_under_base14() {
    let geometry = PageGeometry::default();

    let frac_src = with_ctx(
        "embed-math ctx (math-frac (math-char MathOrd `1`) (math-char MathOrd `2`))",
    );
    let frac_box = math_box(run_math(&frac_src, &Base14Metrics).expect("frac compiles"));
    let frac_page = page_for(frac_box, &geometry);
    let frac_bytes = render_pdf(&geometry, std::slice::from_ref(&frac_page), &[])
        .expect("fraction PDF render");
    assert!(frac_bytes.starts_with(b"%PDF-"));
    let frac_hay = String::from_utf8_lossy(&frac_bytes);
    assert!(
        !frac_hay.contains("(/)"),
        "fraction must no longer emit the ASCII '/' stand-in:\n{frac_hay}"
    );
    assert!(
        frac_hay.contains("f*"),
        "fraction must emit a fill (f*) operator for its bar:\n{frac_hay}"
    );

    let rad_src = with_ctx("embed-math ctx (math-radical None (math-char MathOrd `2`))");
    let rad_box = math_box(run_math(&rad_src, &Base14Metrics).expect("radical compiles"));
    let rad_page = page_for(rad_box, &geometry);
    let rad_bytes =
        render_pdf(&geometry, std::slice::from_ref(&rad_page), &[]).expect("radical PDF render");
    assert!(rad_bytes.starts_with(b"%PDF-"));
    let rad_hay = String::from_utf8_lossy(&rad_bytes);
    // The old stand-in wrote a literal U+221A `Tj`, which base-14/WinAnsi
    // can't even encode — `winansi` would have errored; the fact this
    // render SUCCEEDS at all is already part of the proof, alongside the
    // fill op below.
    assert!(
        rad_hay.contains("f*"),
        "radical must emit fill (f*) operators for its sign+overbar:\n{rad_hay}"
    );
    // Exactly one Tj (the radicand '2') — no second glyph for a bygone
    // ASCII sqrt stand-in.
    assert_eq!(
        rad_hay.matches(" Tj").count(),
        1,
        "expected exactly one glyph Tj (the radicand), got:\n{rad_hay}"
    );
}

#[test]
fn fraction_renders_fill_ops_through_cid_pipeline_under_math_font() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let geometry = PageGeometry::default();

    let src = with_ctx(
        "embed-math ctx (math-frac (math-char MathOrd `1`) (math-char MathOrd `2`))",
    );
    let bx = math_box(run_math(&src, &store).expect("frac compiles under MATH font"));
    let page = page_for(bx, &geometry);
    let bytes =
        render_pdf_ttf(&geometry, &[page], &store, &[]).expect("fraction CID PDF render");
    assert!(bytes.starts_with(b"%PDF-"));
    let hay = String::from_utf8_lossy(&bytes);
    assert!(
        hay.contains("f*"),
        "CID-pipeline fraction must emit a fill (f*) operator for its bar:\n{hay}"
    );
}
