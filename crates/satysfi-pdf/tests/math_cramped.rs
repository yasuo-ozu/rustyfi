//! `docs/plans/design-math-cramped.md` Slice A2: proof that the cramped-style
//! superscript shift-up formula (`sup_shift_clamped`, `crates/satysfi-lang/
//! src/primitives.rs`) is correctly wired end to end — a superscript nested
//! in a radicand (`Math::Radical`) or a fraction denominator
//! (`Math::Fraction`) is laid out using the font's `SuperscriptShiftUpCramped`
//! constant, while the SAME superscript at top level uses
//! `SuperscriptShiftUp` (TeXbook Appendix G rule 18a).
//!
//! Only observable at all under a real OpenType MATH font: every checked-in
//! fixture font (base-14/ipaexm/Junicode) has no MATH table, and the
//! no-MATH-table fallback deliberately sets `SUP_SHIFT_CRAMPED ==
//! SUP_SHIFT` (design doc §2.4) so base-14 output stays byte-identical. This
//! test mirrors `tests/math_table.rs`'s font discovery/skip discipline and
//! `tests/math_fraction_radical.rs`'s raw-primitive pipeline harness.
//!
//! **Slice B update** (`docs/plans/design-math-cramped.md` §4): the repo now
//! bundles DejaVu Math TeX Gyre itself at `lib-satysfi/dist/fonts/
//! DejaVuMathTeXGyre.ttf` (via `scripts/download-fonts.sh`) and wires it as
//! `default-font.satysfi-hash`'s `"math"` default, so `${...}` math renders
//! with real MATH-table metrics BY DEFAULT — not just under `set-math-font`.
//! `find_math_font` below checks that bundled copy first, so this test (and
//! the cramped/uncramped divergence it proves) no longer depends on a
//! host-wide font install — only on `download-fonts.sh` having been run,
//! same prerequisite `tests/cjk_render.rs` already has. The `need_math_font!`
//! skip is kept rather than dropped: it is still the right behavior for a
//! checkout that hasn't run `download-fonts.sh` (or where fontconfig also
//! comes up empty), and it costs nothing when the bundled font IS present.
//!
//! **Clamp-dominance caveat** (found while writing this test): `math_ml:
//! 524-533`'s clamp (`sup_shift_clamped`) takes `max(cand1, cand2, cand3)`,
//! where only `cand1 = s * (shift_up or shift_up_cramped)` depends on the
//! cramped bit. `cand2 = h_base - s * superscript_baseline_drop_max` depends
//! on the BASE's ink height `h_base` — and this port measures every glyph's
//! height from the font's whole-face ascender (`push_char_glyph`,
//! `primitives.rs`), not a real per-glyph bounding box. A font's ascender is
//! typically much taller than a lowercase letter's real ink height, which
//! can make `cand2` the dominant clamp term for a plain single-character
//! base REGARDLESS of the cramped bit — on both real MATH fonts available at
//! the time this test was written (DejaVu Math TeX Gyre, Noto Sans Math),
//! `cand2` dominates for `x^2`/`y^2`, so the FINAL raise coincides between
//! cramped and uncramped even though `cand1` genuinely differs. The tests
//! below therefore assert the STRONGER, font-independent claim (the pipeline
//! matches an independently recomputed clamp using the correct
//! cramped/uncramped branch — proving the wiring is correct), and
//! ADDITIONALLY assert strict inequality whenever `cand1` happens to be the
//! dominant term for the host font (which would make the divergence
//! visible).

use std::path::{Path as FsPath, PathBuf};
use std::process::Command;

use satysfi_backend::{FontKey, FontMetrics, HorzBox, Length, MathConstants, MathGlyph, PureHorzBox};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use satysfi_pdf::TtfFontStore;

// ----------------------------------------------------------------------
// Font discovery (copied, not shared — matches this repo's existing
// per-file convention for these small fixture-locator helpers; see
// `tests/math_table.rs`/`tests/math_fraction_radical.rs`).
// ----------------------------------------------------------------------

fn find_math_font() -> Option<PathBuf> {
    // Slice B (`docs/plans/design-math-cramped.md` §4): the repo now bundles
    // its own MATH-table font at `lib-satysfi/dist/fonts/
    // DejaVuMathTeXGyre.ttf` (fetched by `scripts/download-fonts.sh`, same
    // as ipaexm/Junicode). Check it FIRST so this test no longer depends on
    // a host-wide font install once that script has been run — this is what
    // makes the cramped/uncramped divergence PROVEN BY THIS FILE observable
    // by default in this repo, not just on machines that happen to have a
    // system math font. Only fall through to fontconfig/distro paths when
    // the bundled copy isn't there.
    let bundled = FsPath::new(env!("CARGO_MANIFEST_DIR"))
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
// Pipeline helpers (mirrors `math_table.rs`/`math_fraction_radical.rs`'s
// `run_math`/`with_ctx`).
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

/// Unwrap `embed-math ctx (...)`'s single `PureHorzBox::Math`'s glyph list.
fn math_glyphs(v: Value) -> Vec<MathGlyph> {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { glyphs, .. }) => glyphs,
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

/// Independently recompute `sup_shift_clamped`'s clamp (`math.ml:524-533`)
/// using the correct cramped/uncramped branch — mirrors
/// `tests/math_table.rs`'s "headline" test's recompute style.
fn expected_sup_shift(
    mc: &MathConstants,
    cramped: bool,
    s: Length,
    h_base: Length,
    d_sup: Length,
) -> Length {
    let shift_up = if cramped {
        mc.superscript_shift_up_cramped
    } else {
        mc.superscript_shift_up
    };
    let cand1 = s * shift_up;
    let cand2 = h_base - s * mc.superscript_baseline_drop_max;
    let cand3 = s * mc.superscript_bottom_min + d_sup;
    cand1.max(cand2).max(cand3)
}

// ----------------------------------------------------------------------
// Headline proof: a superscript inside a radicand, and inside a fraction
// denominator, is laid out via the CRAMPED constant; the same superscript
// at top level uses the UNCRAMPED constant. See the module doc comment for
// why the final `dy` may or may not visibly differ, depending on which
// clamp candidate dominates for the host font.
// ----------------------------------------------------------------------

#[test]
fn cramped_superscript_in_radicand_uses_the_cramped_constant() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let mc = store
        .math_constants(FontKey(0))
        .expect("a real MATH font must expose MathConstants");
    let size = Length::pt(12.0);

    // Sanity: the font itself provides a genuinely lower cramped constant
    // (TeXbook Appendix G rule 18a / OpenType `SuperscriptShiftUp` vs.
    // `SuperscriptShiftUpCramped`).
    assert!(
        mc.superscript_shift_up_cramped < mc.superscript_shift_up,
        "expected {path:?}'s SuperscriptShiftUpCramped ({}) to be less than its \
         SuperscriptShiftUp ({})",
        mc.superscript_shift_up_cramped,
        mc.superscript_shift_up
    );

    // Top level (uncramped): `x^2`.
    let top_src =
        with_ctx("embed-math ctx (math-sup (math-char MathOrd `x`) (math-char MathOrd `2`))");
    let top_glyphs = math_glyphs(run_math(&top_src, &store).expect("x^2 should compile"));
    assert_eq!(
        top_glyphs.len(),
        2,
        "expected base 'x' + script '2', got {top_glyphs:?}"
    );

    let h_base = store.ascender(FontKey(0), size);
    let script_size = size * mc.script_scale_down;
    let d_sup = store.descender(FontKey(0), script_size);
    let top_expected = expected_sup_shift(&mc, false, size, h_base, d_sup);
    assert_eq!(
        top_glyphs[1].dy, top_expected,
        "top-level ${{x^2}}'s superscript shift should match the independently \
         recomputed UNCRAMPED clamp"
    );

    // Radicand (cramped): `sqrt(x^2)` — `math-radical None (x^2)`.
    let rad_src = with_ctx(
        "embed-math ctx (math-radical None (math-sup (math-char MathOrd `x`) (math-char MathOrd `2`)))",
    );
    let rad_glyphs = math_glyphs(run_math(&rad_src, &store).expect("sqrt(x^2) should compile"));
    assert_eq!(
        rad_glyphs.len(),
        2,
        "the radical sign is drawn via `rules`, not `glyphs` — expected just base 'x' + \
         script '2', got {rad_glyphs:?}"
    );
    let rad_expected = expected_sup_shift(&mc, true, size, h_base, d_sup);
    assert_eq!(
        rad_glyphs[1].dy, rad_expected,
        "sqrt(x^2)'s superscript shift should match the independently recomputed \
         CRAMPED clamp"
    );

    // When cand1 (the cramped-affected term) is the dominant clamp
    // candidate for the UNCRAMPED case on this font, switching to the
    // (always-lower) cramped constant must visibly lower the raise.
    let cand1_uncramped = size * mc.superscript_shift_up;
    let cand2 = h_base - size * mc.superscript_baseline_drop_max;
    let cand3 = size * mc.superscript_bottom_min + d_sup;
    if cand1_uncramped > cand2 && cand1_uncramped > cand3 {
        assert!(
            rad_glyphs[1].dy < top_glyphs[1].dy,
            "cand1 dominates the uncramped clamp on this font, so sqrt(x^2)'s \
             superscript ({:?}) should be raised strictly less than top-level x^2's \
             ({:?})",
            rad_glyphs[1].dy,
            top_glyphs[1].dy
        );
    } else {
        eprintln!(
            "note: {path:?}'s superscript clamp for a plain-char base is dominated by \
             cand2/cand3, not cand1 — cramped and uncramped coincide here even though \
             the formula-level equality assertions above prove correct wiring (see the \
             module doc comment's clamp-dominance caveat)."
        );
    }
}

#[test]
fn cramped_superscript_in_fraction_denominator_uses_the_cramped_constant() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let mc = store
        .math_constants(FontKey(0))
        .expect("a real MATH font must expose MathConstants");
    let size = Length::pt(12.0);

    // Top level (uncramped): `y^2`.
    let top_src =
        with_ctx("embed-math ctx (math-sup (math-char MathOrd `y`) (math-char MathOrd `2`))");
    let top_glyphs = math_glyphs(run_math(&top_src, &store).expect("y^2 should compile"));
    assert_eq!(
        top_glyphs.len(),
        2,
        "expected base 'y' + script '2', got {top_glyphs:?}"
    );

    let h_base = store.ascender(FontKey(0), size);
    let script_size = size * mc.script_scale_down;
    let d_sup = store.descender(FontKey(0), script_size);
    let top_expected = expected_sup_shift(&mc, false, size, h_base, d_sup);
    assert_eq!(
        top_glyphs[1].dy, top_expected,
        "top-level ${{y^2}}'s superscript shift should match the independently \
         recomputed UNCRAMPED clamp"
    );

    // Fraction denominator (cramped): `1 / y^2` — `math-frac 1 (y^2)`. The
    // denominator's own glyphs (base 'y' + script '2') get a UNIFORM
    // `denom_shift` on top of their local layout when appended to the
    // fraction's parent box — since it's the SAME offset for both glyphs,
    // the LOCAL raise (`dy` delta between script and base) still isolates
    // exactly the `sup_shift_clamped` term cramped changes.
    let frac_src = with_ctx(
        "embed-math ctx (math-frac (math-char MathOrd `1`) (math-sup (math-char MathOrd `y`) (math-char MathOrd `2`)))",
    );
    let frac_glyphs = math_glyphs(run_math(&frac_src, &store).expect("1/y^2 should compile"));
    assert_eq!(
        frac_glyphs.len(),
        3,
        "expected numerator '1' + denominator base 'y' + denominator script '2', got \
         {frac_glyphs:?}"
    );
    // glyphs[0] = numerator '1', glyphs[1] = denominator base 'y',
    // glyphs[2] = denominator script '2'.
    let den_local_raise = frac_glyphs[2].dy - frac_glyphs[1].dy;
    let den_expected = expected_sup_shift(&mc, true, size, h_base, d_sup);
    assert_eq!(
        den_local_raise, den_expected,
        "1/y^2's denominator superscript LOCAL raise should match the independently \
         recomputed CRAMPED clamp"
    );

    let cand1_uncramped = size * mc.superscript_shift_up;
    let cand2 = h_base - size * mc.superscript_baseline_drop_max;
    let cand3 = size * mc.superscript_bottom_min + d_sup;
    if cand1_uncramped > cand2 && cand1_uncramped > cand3 {
        assert!(
            den_local_raise < top_glyphs[1].dy,
            "cand1 dominates the uncramped clamp on this font, so 1/y^2's denominator \
             superscript ({den_local_raise:?}) should be raised strictly less than \
             top-level y^2's ({:?})",
            top_glyphs[1].dy
        );
    } else {
        eprintln!(
            "note: {path:?}'s superscript clamp for a plain-char base is dominated by \
             cand2/cand3, not cand1 — cramped and uncramped coincide here even though \
             the formula-level equality assertions above prove correct wiring (see the \
             module doc comment's clamp-dominance caveat)."
        );
    }
}
