//! Empirical proof that a dedicated math font (`set-math-font` /
//! `Context::math_font` and `primitives.rs`'s
//! `math_glyph_font`/`math_char_available`) makes styled Mathematical-
//! Alphanumeric glyphs (`resolve_variant_char`'s remap targets) render
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

use rustyfi_backend::{
    FontKey, FontMetrics, HorzBox, Length, Page, PageGeometry, PlacedLine, PureHorzBox,
};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use rustyfi_pdf::{render_pdf_ttf, TtfFontStore};

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
    // The canonical copy of this discovery order (the sibling
    // math tests point here): the repo bundles the real Latin Modern Math at
    // `lib-rustyfi/dist/fonts/latinmodern-math.otf` (fetched by
    // `download-fonts.sh`, same as ipaexm/Junicode) and wires it as
    // `default-font.satysfi-hash`'s `"math"` default. Check it FIRST, so
    // this test depends on that script having been run rather than on a
    // host-wide font install. LM Math covers every codepoint this test needs
    // (Mathematical Italic 'a', minus sign, Double-Struck R/D). Fall back to
    // the secondary bundled DejaVu Math TeX Gyre only if LM Math is absent,
    // then fontconfig/distro paths.
    let bundled_lmmath = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    if bundled_lmmath.is_file() {
        return Some(bundled_lmmath);
    }
    let bundled_dejavu = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/DejaVuMathTeXGyre.ttf");
    if bundled_dejavu.is_file() {
        return Some(bundled_dejavu);
    }

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
// eval pipeline (parse -> elaborate -> typecheck -> eval) — mirrors
// `math_variant_class.rs`'s `run`/`with_ctx`/`math_box` harness.
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

fn math_box(v: Value) -> (Length, FontKey, Vec<rustyfi_backend::MathGlyph>) {
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

    let glyph = rustyfi_backend::MathGlyph {
        info: rustyfi_backend::HorzStringInfo {
            font,
            size,
            rising: Length::ZERO,
            color: rustyfi_backend::Color::Gray(0.0),
        },
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
    let page = Page {
            body_lines: usize::MAX, lines: vec![line] };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");

    assert!(
        pdf_bytes.starts_with(b"%PDF-"),
        "output should start with a PDF header"
    );

    // Which key the font is embedded under follows the face's OUTLINE
    // FLAVOUR, so the assertion has to read the face rather than assume one:
    // `find_math_font` prefers the bundled CFF Latin Modern Math but falls
    // back to fontconfig and to distro paths, and a runner with no bundled
    // fonts legitimately lands on a `glyf` face such as Noto Sans Math. A CFF
    // (`OTTO`) face takes `cid.rs`'s `write_font_cff` -> `CIDFontType0` /
    // `FontFile3`; a `glyf` face takes `CIDFontType2` / `FontFile2`. Asserting
    // `FontFile3` unconditionally made this test pass on a developer machine
    // with the bundled fonts and fail in CI, which does not fetch them for the
    // unit-test job.
    let flavour = std::fs::read(&path).expect("read font file");
    let is_cff = flavour.starts_with(b"OTTO");
    let want: &[u8] = if is_cff { b"FontFile3" } else { b"FontFile2" };
    assert!(
        pdf_bytes.windows(want.len()).any(|w| w == want),
        "expected the math font to be embedded as {} (the {} face at {}) — asserting \
         embedding, not just size, so 'smaller' cannot be satisfied by dropping the font",
        String::from_utf8_lossy(want),
        if is_cff { "CFF/OTTO" } else { "glyf" },
        path.display()
    );

    // Both paths SUBSET, so the whole-file comparison holds either way: a
    // PDF carrying a subset of one face is smaller than that face's file.
    // Same direction as `tests/ttf.rs`'s
    // `cff_face_embeds_as_fontfile3_cidfonttype0`.
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    assert!(
        pdf_bytes.len() < font_len,
        "expected the subsetted PDF ({} bytes) to be smaller than the whole source math \
         font file ({} bytes, {})",
        pdf_bytes.len(),
        font_len,
        path.display()
    );
}

/// A store built from BYTES can be given a math default, and without one the
/// math font falls back to the Latin face.
///
/// This is the browser build's whole problem: `from_bytes_with_abbrevs` sets
/// `math_default: None`, so `Context::math_font` stays at its seed — the text
/// face — which has no `MATH` table. Every constant math layout reads then
/// falls back to a guess and an equation collapses: limits land on their
/// operator, fraction bars vanish, fences stop stretching. It renders, so
/// nothing errors; it is just wrong.
#[test]
fn a_byte_built_store_takes_a_math_default() {
    let path = need_math_font!();
    let math = std::fs::read(&path).expect("read math font");
    let text = match std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist/fonts/Junicode.ttf"),
    ) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!("skipping: the bundled text face is absent — run download-fonts.sh");
            return;
        }
    };

    let store = TtfFontStore::from_bytes_with_abbrevs(
        text,
        None,
        None,
        [("lmmath".to_string(), math)],
        "test fonts",
    )
    .expect("both faces must parse");

    // The control: this is what the browser build ships today.
    assert!(
        store.default_math_font().is_none(),
        "a byte-built store must not acquire a math default by itself",
    );

    let key = store.abbrev_key("lmmath").expect("the abbrev was registered");
    let store = store.with_math_default(key);
    assert_eq!(
        store.default_math_font(),
        Some(key),
        "with_math_default must point the math font at the face given",
    );
    // And it has to be a DIFFERENT face from the text one, or the fix is
    // pointing math back at the font that caused the problem.
    assert_ne!(key, FontKey(0), "the math face must not be the Latin slot");
    assert!(
        store.math_constants(key).is_some(),
        "the math default must resolve to a face with a MATH table",
    );
    assert!(
        store.math_constants(FontKey(0)).is_none(),
        "the bundled text face must have no MATH table — otherwise this test \
         proves nothing about why the math default is needed",
    );
}
