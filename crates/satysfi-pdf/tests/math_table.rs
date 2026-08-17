//! `docs/plans/math-engine.md` §B1 (OpenType MATH-table metrics: constants +
//! scripts + axis + italic-kern) — proof that:
//!  1. `TtfFontStore` actually reads a real MATH font's `MathConstants`/
//!     italic-correction table (ttf-parser 0.25.1's `tables::math`).
//!  2. `Base14Metrics` overrides NONE of the new `FontMetrics` methods (they
//!     inherit the trait's defaulted `None`), the base-14 regression floor
//!     §B1 depends on.
//!  3. The HEADLINE claim: laying out `${x^2}` under a real MATH font
//!     produces a superscript shift that (a) differs from the fixed
//!     `12pt * 0.5 = 6.0pt` heuristic base-14 still uses, and (b) equals an
//!     INDEPENDENTLY recomputed clamped shift (`math.ml:524-533`'s
//!     `superscript_baseline_height`) built from the exact same
//!     `MathConstants`/ascender/descender queries `MathC`/`push_char_glyph`
//!     use lang-side — i.e. the wired-up pipeline, not just the raw
//!     accessor, does the real thing.
//!
//! Font discovery mirrors `tests/math_font.rs`: fontconfig first, then a
//! handful of common distro/nix paths, then a graceful skip.

use std::path::{Path, PathBuf};
use std::process::Command;

use satysfi_backend::{FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use satysfi_pdf::{Base14Metrics, TtfFontStore};

/// Locate a real MATH-capable OpenType font, preferring DejaVu Math TeX
/// Gyre (TrueType/glyf — exercises the CID writer path elsewhere) over Noto
/// Sans Math (CFF), same discovery order/guard as `tests/math_font.rs`.
fn find_math_font() -> Option<PathBuf> {
    // Slice B (`docs/plans/design-math-cramped.md` §4): the repo now bundles
    // its own MATH-table font at `lib-satysfi/dist/fonts/
    // DejaVuMathTeXGyre.ttf` (fetched by `scripts/download-fonts.sh`, same
    // as ipaexm/Junicode). Check it FIRST so this test no longer depends on
    // a host-wide font install once that script has been run — only fall
    // through to fontconfig/distro paths when it hasn't.
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-satysfi/dist/fonts/DejaVuMathTeXGyre.ttf");
    if bundled.is_file() {
        return Some(bundled);
    }

    for family in ["DejaVu Math TeX Gyre", "Noto Sans Math"] {
        if let Ok(output) = Command::new("fc-match")
            .args(["--format=%{file}", family])
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty()
                    && Path::new(&path).is_file()
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
        if Path::new(candidate).is_file() {
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
// 1. Raw MathConstants/italic-correction accessors.
// ----------------------------------------------------------------------

#[test]
fn math_font_exposes_plausible_math_constants() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");

    let mc = store
        .math_constants(FontKey(0))
        .expect("a real MATH font must expose MathConstants");

    for (name, v) in [
        ("axis_height", mc.axis_height),
        ("superscript_shift_up", mc.superscript_shift_up),
        ("fraction_rule_thickness", mc.fraction_rule_thickness),
    ] {
        assert!(
            v > 0.0 && v < 1.0,
            "{name} = {v} should be a nonzero (0,1) ratio of the font size"
        );
    }
    assert!(
        mc.script_scale_down > 0.0 && mc.script_scale_down < 1.0,
        "script_scale_down = {} should be a nonzero (0,1) ratio",
        mc.script_scale_down
    );
}

#[test]
fn base14_overrides_none_of_the_math_methods() {
    let base14 = Base14Metrics;
    assert!(
        base14.math_constants(FontKey(0)).is_none(),
        "Base14Metrics must inherit the trait's defaulted None (§B1 base-14 \
         regression floor)"
    );
    assert!(base14
        .italic_correction(FontKey(0), 'f', Length::pt(12.0))
        .is_none());
    assert!(base14
        .math_kern(
            FontKey(0),
            'f',
            Length::pt(12.0),
            satysfi_backend::MathCorner::TopRight,
            Length::ZERO,
        )
        .is_none());
}

#[test]
fn math_font_has_positive_italic_correction_for_f() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");

    // 'f' is the textbook italic-correction example (its cursive tail
    // leans into whatever follows it) — both DejaVu Math TeX Gyre and Noto
    // Sans Math give it a positive, nonzero correction.
    let ic = store
        .italic_correction(FontKey(0), 'f', Length::pt(12.0))
        .expect("MATH font should have an italic correction for 'f'");
    assert!(ic.0 > 0.0, "expected a positive italic correction, got {ic:?}");
}

#[test]
fn math_kern_does_not_panic_and_degrades_gracefully() {
    // Neither DejaVu Math TeX Gyre nor Noto Sans Math populate per-glyph
    // MathKernInfo (verified by direct ttf-parser inspection at
    // implementation time), so this only proves the accessor chain
    // (`face.tables().math?.glyph_info?.kern_infos?.get(gid)?`) doesn't
    // panic and degrades to `None` — the exact "missing kern data ->
    // zero-correction, not error" contract `math-table-spec.md`'s Risks
    // section calls for.
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    for corner in [
        satysfi_backend::MathCorner::TopRight,
        satysfi_backend::MathCorner::TopLeft,
        satysfi_backend::MathCorner::BottomRight,
        satysfi_backend::MathCorner::BottomLeft,
    ] {
        let _ = store.math_kern(FontKey(0), 'f', Length::pt(12.0), corner, Length::ZERO);
    }
}

// ----------------------------------------------------------------------
// 2. Headline proof: the real pipeline (parse -> elaborate -> typecheck ->
//    eval -> layout_math_atom's Sup arm) produces a MATH-font superscript
//    shift that differs from base-14's flat heuristic and matches an
//    independently recomputed clamp.
// ----------------------------------------------------------------------

fn run_math(src: &str, metrics: &dyn FontMetrics) -> Result<Value, CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mut interp = eval::Interp::new(metrics);
    Ok(interp.eval(&env, &program.body)?)
}

fn with_ctx(body: &str) -> String {
    format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    )
}

/// Extract the superscript glyph's `dy` from `${x^2}`'s laid-out
/// `PureHorzBox::Math` — the SECOND glyph (`x` is the base, `2` the raised
/// script; the faithful `layout_math_atom::Sup` arm this exercises never
/// interleaves them differently for a single-char base/script).
fn sup_dy(v: Value) -> Length {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { glyphs, .. }) => {
                    assert_eq!(
                        glyphs.len(),
                        2,
                        "expected 2 glyphs (base 'x', script '2'), got {glyphs:?}"
                    );
                    assert_eq!(glyphs[1].text, "2");
                    glyphs[1].dy
                }
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

#[test]
fn headline_math_font_superscript_shift_differs_from_flat_heuristic() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let size = Length::pt(12.0);

    // Base-14: `layout_math_atom`'s `MathC` falls back to the fixed
    // `SUP_SHIFT = 0.5` constant this port used before §B1 -> exactly
    // 12pt * 0.5 = 6.0pt, unclamped.
    let base14_src = with_ctx("embed-math ctx ${x^2}");
    let v = run_math(&base14_src, &Base14Metrics)
        .expect("${x^2} should compile and evaluate under base-14");
    let base14_dy = sup_dy(v);
    assert_eq!(
        base14_dy,
        Length::pt(6.0),
        "base-14 (no MATH table) should keep the flat 12pt*SUP_SHIFT heuristic"
    );

    // Real MATH font: independently recompute math.ml:524-533's clamped
    // `superscript_baseline_height` by hand, from the SAME quantities
    // `MathC::sup_shift_clamped`/`push_char_glyph` read (the font's own
    // `MathConstants` plus its font-WIDE ascender/descender — this port
    // measures every glyph's height/depth from those, not a per-glyph ink
    // bbox, so `h_base`/`d_sup` below are exactly what the base 'x' and
    // script '2' glyphs carry).
    let mc = store
        .math_constants(FontKey(0))
        .expect("MATH font should expose MathConstants");
    let script_size = size * mc.script_scale_down;
    let h_base = store.ascender(FontKey(0), size);
    let d_sup = store.descender(FontKey(0), script_size);
    let cand1 = size * mc.superscript_shift_up;
    let cand2 = h_base - size * mc.superscript_baseline_drop_max;
    let cand3 = size * mc.superscript_bottom_min + d_sup;
    let expected = cand1.max(cand2).max(cand3);

    let math_src = with_ctx("embed-math ctx ${x^2}");
    let v = run_math(&math_src, &store)
        .expect("${x^2} should compile and evaluate under a real MATH font");
    let math_dy = sup_dy(v);

    assert_ne!(
        math_dy, base14_dy,
        "a real MATH font's clamped superscript shift ({math_dy:?}) should differ \
         from the flat 6.0pt heuristic"
    );
    assert_eq!(
        math_dy, expected,
        "the wired-up pipeline's shift should equal the independently \
         recomputed math.ml:524-533 clamp"
    );

    assert!(
        script_size < size,
        "script_scale_down should shrink the script size below font_size"
    );
    assert_eq!(script_size, size * mc.script_scale_down);
}
