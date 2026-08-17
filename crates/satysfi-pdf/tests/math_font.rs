//! Empirical proof that a dedicated math font (`set-math-font` /
//! `Context::math_font`, see `docs/plans/...` and `primitives.rs`'s
//! `math_glyph_font`/`math_char_available`) makes styled Mathematical-
//! Alphanumeric glyphs (gap 5's `resolve_variant_char` remap targets) render
//! end-to-end through the REAL CID pipeline — not just under the permissive
//! `Mono` stub other math tests use.
//!
//! Needs a real math-capable OpenType font on disk (one that actually maps
//! codepoints like U+1D44E MATHEMATICAL ITALIC SMALL A, U+2212 MINUS SIGN,
//! U+211D DOUBLE-STRUCK CAPITAL R, U+1D53B MATHEMATICAL DOUBLE-STRUCK
//! CAPITAL D) — located via fontconfig (`fc-match`), falling back to a few
//! common distro/nix paths, and SKIPPED gracefully (mirroring `tests/ttf.rs`)
//! when none is found.

use std::path::{Path, PathBuf};
use std::process::Command;

use satysfi_backend::{
    FontKey, FontMetrics, HorzBox, Length, Page, PageGeometry, PlacedLine, PureHorzBox,
};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use satysfi_pdf::{render_pdf_ttf, TtfFontStore};

/// Mathematical Italic Small A (`resolve_variant_char`'s default remap of
/// plain `a` under `MathCharClass::Italic`, the default restyling).
const A_ITALIC: char = '\u{1D44E}';
/// Minus sign (`default_math_class_map`'s remap of the `-` token).
const MINUS: char = '\u{2212}';
/// Double-Struck Capital R (`default_math_variant_char`'s special-cased
/// `DoubleStruck` entry for `'R'`).
const DBL_R: char = '\u{211D}';
/// Mathematical Double-Struck Capital D (`default_math_variant_char`'s
/// `DoubleStruck` fallback arm: `cp(0x1D538, 3)` since `'D'` is cap-index 3
/// and not one of the special-cased letters C/H/N/P/Q/R/Z).
const DBL_D: char = '\u{1D53B}';

/// Locate a real math-capable OpenType font: prefer fontconfig's idea of
/// "Noto Sans Math" / "DejaVu Math TeX Gyre" (guarding the resolved path
/// actually contains "Math"/"math", since `fc-match` silently substitutes a
/// generic font when the family isn't installed), then fall back to a few
/// common distro/nix paths, then skip gracefully.
fn find_math_font() -> Option<PathBuf> {
    for family in ["Noto Sans Math", "DejaVu Math TeX Gyre"] {
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
        "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/OTF/NotoSansMath-Regular.otf",
        "/usr/share/fonts/noto-fonts/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/texgyre/texgyredejavu-math.otf",
        "/usr/share/fonts/truetype/tex-gyre/texgyredejavu-math.otf",
        "/usr/share/texmf/fonts/opentype/public/dejavu-otf/DejaVuMathTeXGyre.ttf",
        "/usr/share/fonts/opentype/dejavu-math-tex-gyre/DejaVuMathTeXGyre.ttf",
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
                     (tried `fc-match` for \"Noto Sans Math\"/\"DejaVu Math TeX Gyre\" \
                     and common nix/distro paths)"
                );
                return;
            }
        }
    };
}

#[test]
fn math_font_measures_styled_codepoints() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let size = Length::pt(12.0);

    for c in [A_ITALIC, MINUS, DBL_R, DBL_D] {
        assert!(
            store.advance(FontKey(0), c, size).is_some(),
            "expected {path:?} to have an advance for U+{:04X} ({c:?})",
            c as u32
        );
    }
}

#[test]
fn math_font_has_cmap_glyphs() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");

    for c in [A_ITALIC, MINUS, DBL_R, DBL_D] {
        assert!(
            face.glyph_index(c).is_some(),
            "expected {path:?}'s cmap to cover U+{:04X} ({c:?})",
            c as u32
        );
    }
}

// ----------------------------------------------------------------------
// (c) embed-math ${...} emits the styled codepoints through the REAL
// eval pipeline (parse -> elaborate -> typecheck -> eval), under the real
// `TtfFontStore` as the `FontMetrics` impl — mirrors `math_variant_class.rs`'s
// `run`/`with_ctx`/`math_box` harness, but against a real font instead of
// the permissive `Mono` stub.
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

/// Same shape as `math_variant_class.rs`'s `with_ctx`: a `context` with no
/// package loaded, a local no-op `\dummy` `[math] inline-cmd` installed via
/// `get-initial-context`'s second argument (never invoked — every test
/// calls `embed-math` directly).
fn with_ctx(body: &str) -> String {
    format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    )
}

fn math_box(v: Value) -> (Length, FontKey, Vec<satysfi_backend::MathGlyph>) {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { width, glyphs, .. }) => {
                    let font = glyphs
                        .first()
                        .map(|g| g.info.font)
                        .unwrap_or(FontKey(0));
                    (width, font, glyphs)
                }
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

#[test]
fn embed_math_emits_styled_codepoints_under_math_font() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");

    // `${a}` -> one glyph, remapped by `resolve_variant_char` (default
    // `MathCharClass::Italic`) to U+1D44E, measured/emitted under
    // `FontKey(0)` (the default `math_font == font`, per the risk-1
    // zero-regression contract).
    let src = with_ctx("embed-math ctx ${a}");
    let v = run_math(&src, &store).expect("${a} should compile and evaluate under a real font");
    let (_, font, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1, "expected 1 glyph, got {glyphs:?}");
    assert_eq!(
        glyphs[0].text, A_ITALIC.to_string(),
        "expected the Mathematical Italic Small A remap"
    );
    assert_eq!(font, FontKey(0), "expected the default math_font (FontKey(0))");

    // `${a-b}` -> the middle glyph is the `-` token, reclassified `Bin` and
    // remapped to U+2212 MINUS SIGN by `default_math_class_map`.
    let src = with_ctx("embed-math ctx ${a-b}");
    let v = run_math(&src, &store).expect("${a-b} should compile and evaluate under a real font");
    let (_, _, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 3, "expected 3 glyphs (a, -, b), got {glyphs:?}");
    assert_eq!(
        glyphs[1].text,
        MINUS.to_string(),
        "expected the middle glyph to be U+2212 MINUS SIGN"
    );

    // `math-char-class MathDoubleStruck ${D}` -> `default_math_variant_char`
    // (DoubleStruck, 'D') = cp(0x1D538, 3) = U+1D53B.
    let src = with_ctx("embed-math ctx (math-char-class MathDoubleStruck ${D})");
    let v = run_math(&src, &store)
        .expect("math-char-class MathDoubleStruck ${D} should compile and evaluate");
    let (_, _, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1, "expected 1 glyph, got {glyphs:?}");
    assert_eq!(
        glyphs[0].text,
        DBL_D.to_string(),
        "expected the Mathematical Double-Struck Capital D remap"
    );
}

// ----------------------------------------------------------------------
// (d, bonus) styled math actually gets embedded through `render_pdf_ttf`
// (the CID-keyed PDF writer), proving the whole pipeline — not just
// in-memory glyph selection — end to end.
// ----------------------------------------------------------------------

#[test]
fn styled_math_renders_through_cid_pipeline() {
    let path = need_math_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let geometry = PageGeometry::default();
    let size = Length::pt(18.0);

    let font = FontKey(0);
    let advance = store
        .advance(font, A_ITALIC, size)
        .expect("math font should measure U+1D44E");
    let ascender = store.ascender(font, size);
    let descender = store.descender(font, size);

    let glyph = satysfi_backend::MathGlyph {
        info: satysfi_backend::HorzStringInfo { font, size },
        text: A_ITALIC.to_string(),
        gid: None,
        dx: Length::ZERO,
        dy: Length::ZERO,
        width: advance,
        height: ascender,
        depth: descender,
    };

    let line = PlacedLine {
        x: geometry.text_origin.0,
        baseline_y: geometry.text_origin.1 + ascender,
        contents: vec![(
            Length::ZERO,
            PureHorzBox::Math {
                width: advance,
                height: ascender,
                depth: descender,
                glyphs: vec![glyph],
                rules: vec![],
            },
        )],
    };
    let page = Page { lines: vec![line] };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");

    assert!(
        pdf_bytes.starts_with(b"%PDF-"),
        "output should start with a PDF header"
    );

    // Full-file embedding heuristic (mirrors `tests/ttf.rs`): the raw font
    // bytes appear verbatim in the output (no subsetting), so the PDF must
    // be at least as big as the source font file.
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    assert!(
        pdf_bytes.len() > font_len,
        "expected the PDF ({} bytes) to be larger than the embedded font file ({} bytes)",
        pdf_bytes.len(),
        font_len
    );
}
