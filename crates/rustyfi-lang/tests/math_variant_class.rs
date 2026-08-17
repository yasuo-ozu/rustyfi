//! Gap 5 (char-class value model) / Gap 6 (text-in-math degrade) / Gap 7
//! (`set-math-variant-char`/`get-left-math-class`/`get-right-math-class`)
//! acceptance coverage (`docs/plans/math-mode-language-gaps.md`). Pure-
//! pipeline `run`/helper shapes copied from `math_optional_args.rs` (per
//! that file's own copy-not-share convention for standalone test harnesses)
//! — no `@require:`, no loader, `rustyfi_syntax::parse_file` straight
//! through elaborate/typecheck/eval.

use rustyfi_backend::{FontKey, FontMetrics, HorzBox, Length, MathGlyph, PureHorzBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

/// A fully permissive `FontMetrics` stub — `Some(size * 0.5)` for EVERY
/// char, including the non-ASCII Mathematical-Alphanumeric remap targets
/// gap 5 introduces — so the metrics-probe fallback in
/// `resolve_variant_char` always succeeds and the full remap is
/// observable/unit-testable. Mirrors `math_package.rs`'s own `Mono`.
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

/// An ASCII-only `FontMetrics` stub (mirrors `Base14Metrics`'s WinAnsi-only
/// shape, and `eval_phase2b.rs`'s own `Mono`) — the metrics probe fails for
/// every remapped Mathematical-Alphanumeric codepoint (all outside ASCII),
/// so gap 5's fallback policy must keep the SOURCE char.
struct AsciiMono;

impl FontMetrics for AsciiMono {
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

fn run_with(src: &str, metrics: &dyn FontMetrics) -> Result<Value, CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mut interp = eval::Interp::new(metrics);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))?)
}

fn run(src: &str) -> Result<Value, CompileError> {
    run_with(src, &Mono)
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

/// Every test's math value is built via `embed-math ctx <math>` directly
/// (not wrapped in `get-natural-metrics`, which discards the glyphs) —
/// unwraps the resulting single `PureHorzBox::Math` box into its
/// `(width, glyphs)`.
fn math_box(v: Value) -> (Length, Vec<MathGlyph>) {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { width, glyphs, .. }) => (width, glyphs),
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

/// A `context` with no package loaded, built the same way
/// `math_package.rs`'s Gap 1 override test does (a local `\dummy`
/// `[math] inline-cmd`, installed via `get-initial-context`'s second
/// argument, never actually invoked by these tests — they all call
/// `embed-math` directly).
fn with_ctx(body: &str) -> String {
    format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    )
}

// ============================================================================
// Gap 5 — whole-token class-map reclassification + spacing
// (`docs/plans/math-mode-language-gaps.md` row 5).
// ============================================================================

/// `${a-b}`: `-` is its own MATHCHAR token, reclassified `Bin` (and
/// remapped to U+2212 MINUS SIGN) by `default_math_class_map` — width gains
/// `Bin` spacing on both sides (`space_before`'s `font_size * 0.22`, the
/// REAL constant read from `primitives.rs`, not hardcoded).
#[test]
fn gap5_minus_reclassified_as_bin_with_minus_sign_glyph() {
    let src = with_ctx("embed-math ctx ${a-b}");
    let v = run(&src).expect("${a-b} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(
        glyphs.len(),
        3,
        "expected 3 glyphs (a, -, b), got {glyphs:?}"
    );
    assert_eq!(
        glyphs[1].text, "\u{2212}",
        "middle glyph should be U+2212 MINUS SIGN"
    );

    let glyph_w = Length::pt(12.0) * 0.5;
    let bin_space = Length::pt(12.0) * 0.22;
    let expected = glyph_w + bin_space + glyph_w + bin_space + glyph_w;
    assert_eq!(
        width, expected,
        "expected 3 glyphs + 2 Bin spaces, got {width:?}"
    );
}

/// `${a->b}`: `-` and `>` are consecutive symbol chars, lexed as ONE
/// MATHCHAR token `"->"` — not in `default_math_class_map`, so the whole
/// token is ONE `Ord` atom (no Bin/Rel spacing anywhere), unlike the
/// pre-gap-5 per-char split which would have added `Bin` spacing around the
/// `-`.
#[test]
fn gap5_multi_char_symbol_run_is_one_ord_atom_no_spacing() {
    let src = with_ctx("embed-math ctx ${a->b}");
    let v = run(&src).expect("${a->b} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(
        glyphs.len(),
        4,
        "expected 4 glyphs (a, -, >, b), got {glyphs:?}"
    );
    assert_eq!(
        width,
        Length::pt(24.0),
        "expected 4 * 6pt with no inter-atom spacing"
    );
}

/// `${a:b}`: `:` is reclassified `Rel` (was `Punct` under the old
/// `ascii_math_kind` stand-in) — width gains `Rel` spacing (`font_size *
/// 0.28`) on both sides, strictly more than the old 18pt.
#[test]
fn gap5_colon_reclassified_as_rel() {
    let src = with_ctx("embed-math ctx ${a:b}");
    let v = run(&src).expect("${a:b} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 3);

    let glyph_w = Length::pt(12.0) * 0.5;
    let rel_space = Length::pt(12.0) * 0.28;
    let expected = glyph_w + rel_space + glyph_w + rel_space + glyph_w;
    assert_eq!(width, expected, "expected Rel spacing on both sides of ':'");
    assert!(
        width > Length::pt(18.0),
        "must be strictly wider than the old Punct-spacing width"
    );
}

/// `${x}`'s default restyling is `MathCharClass::Italic`
/// (`Context::initial`) — under the permissive `Mono` stub the metrics
/// probe always succeeds, so the glyph is the actual Unicode Mathematical
/// Italic Small X (U+1D465).
#[test]
fn gap5_default_char_class_is_italic() {
    let src = with_ctx("embed-math ctx ${x}");
    let v = run(&src).expect("${x} should compile and evaluate");
    let (_, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].text, "\u{1D465}");
}

/// `math-char-class MathRoman ${x}` sets `Context::math_char_class` to
/// `Roman` while laying out its inner list — `default_math_variant_char`'s
/// `Roman` arm is the identity (keep the plain ASCII letter), so the glyph
/// stays `"x"` even though the probe would succeed for anything under
/// `Mono`.
#[test]
fn gap5_change_char_class_to_roman_keeps_plain_ascii() {
    let src = with_ctx("embed-math ctx (math-char-class MathRoman ${x})");
    let v = run(&src).expect("math-char-class MathRoman ${x} should compile and evaluate");
    let (_, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].text, "x");
}

/// `math-char-class MathBoldItalic ${x}` remaps to U+1D499 MATHEMATICAL
/// BOLD ITALIC SMALL X — proving `Math::ChangeCharClass`'s layout arm
/// actually threads the requested style through to
/// `VariantCharPending`/`resolve_variant_char`, not a layout no-op anymore.
#[test]
fn gap5_change_char_class_to_bold_italic() {
    let src = with_ctx("embed-math ctx (math-char-class MathBoldItalic ${x})");
    let v = run(&src).expect("math-char-class MathBoldItalic ${x} should compile and evaluate");
    let (_, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].text, "\u{1D499}");
}

/// Under an ASCII-only font (mirrors `Base14Metrics`'s WinAnsi-only shape),
/// the metrics probe for U+1D465 fails, so `resolve_variant_char` falls
/// back to the SOURCE char `x` — the "zero regression under base-14" policy
/// gap 5's whole design hinges on.
#[test]
fn gap5_ascii_only_font_falls_back_to_source_char() {
    let src = with_ctx("embed-math ctx ${x}");
    let v = run_with(&src, &AsciiMono).expect("${x} should compile and evaluate under AsciiMono");
    let (_, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(
        glyphs[0].text, "x",
        "must fall back to the source char under an ASCII-only font"
    );
}

// ============================================================================
// Gap 7 — `set-math-variant-char` / `get-left-math-class` /
// `get-right-math-class` (`docs/plans/math-mode-language-gaps.md` row 7).
// ============================================================================

/// `set-math-variant-char MathItalic 0x78 0x79 ctx` installs a runtime
/// override (`'x' -> 'y'` under `Italic`), consulted BEFORE
/// `default_math_variant_char` — `${x}` under the resulting context must
/// render `'y'`, not the built-in U+1D465 remap.
#[test]
fn gap7_set_math_variant_char_overrides_default_remap() {
    let src = with_ctx(
        "let ctx2 = set-math-variant-char MathItalic 0x78 0x79 ctx in\n\
         embed-math ctx2 ${x}",
    );
    let v = run(&src).expect("set-math-variant-char should compile and evaluate");
    let (_, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].text, "y");
}

/// `get-left-math-class ctx ${-x}`: the FIRST atom is the `-` token,
/// reclassified `Bin` by `default_math_class_map` — must report
/// `Some(MathBin)`.
#[test]
fn gap7_get_left_math_class_on_leading_minus() {
    let src = with_ctx(
        "match get-left-math-class ctx ${-x} with\n\
         | Some(MathBin) -> 1\n\
         | _ -> 0",
    );
    let v = run(&src).expect("get-left-math-class should compile and evaluate");
    assert_eq!(as_int(v), 1);
}

/// `get-right-math-class ctx ${x-}`: the LAST atom is the trailing `-`
/// token — must also report `Some(MathBin)`.
#[test]
fn gap7_get_right_math_class_on_trailing_minus() {
    let src = with_ctx(
        "match get-right-math-class ctx ${x-} with\n\
         | Some(MathBin) -> 1\n\
         | _ -> 0",
    );
    let v = run(&src).expect("get-right-math-class should compile and evaluate");
    assert_eq!(as_int(v), 1);
}

/// `get-left-math-class ctx ${}`: an empty `math` list has no boundary
/// atom at all — the synthetic `MathKind::End` sentinel, which
/// `make_math_class_option_value` maps to `None`.
#[test]
fn gap7_get_left_math_class_on_empty_math_is_none() {
    let src = with_ctx(
        "match get-left-math-class ctx ${} with\n\
         | None -> 1\n\
         | _ -> 0",
    );
    let v = run(&src).expect("get-left-math-class on ${} should compile and evaluate");
    assert_eq!(as_int(v), 1);
}

// ============================================================================
// Gap 6 — text-in-math degrade (`docs/plans/math-mode-language-gaps.md`
// row 6).
// ============================================================================

/// `text-in-math MathOrd (fun c -> read-inline c {ab})`, embedded via
/// `embed-math`, must render WITHOUT erroring (the old `layout_math_atom`
/// arm was a hard `eval_error`), with a width equal to the embedded
/// `inline-boxes`' own natural width (one `"ab"` word box under `Mono`:
/// `2 * (12pt * 0.5) = 12pt`) and a glyph whose text contains `"ab"`.
#[test]
fn gap6_text_in_math_renders_through_read_inline() {
    let src = with_ctx("embed-math ctx (text-in-math MathOrd (fun c -> read-inline c {ab}))");
    let v = run(&src).expect("text-in-math should no longer error");
    let (width, glyphs) = math_box(v);
    assert_eq!(
        width,
        Length::pt(12.0),
        "expected the embedded \"ab\" word's own 12pt width"
    );
    let joined: String = glyphs.iter().map(|g| g.text.as_str()).collect();
    assert!(
        joined.contains("ab"),
        "expected a glyph whose text contains \"ab\", got {glyphs:?}"
    );
}

// ============================================================================
// `convert-string-for-math` — the whole-string Mathematical-Alphanumeric
// remap (`vminstdef.yaml` `PrimitiveConvertStringForMath`;
// `types.cppo.ml:1602` `convert_math_variant_char`). Reuses gap 5's
// `default_math_variant_char` + `default_math_class_map` over a whole string
// under a passed `math-char-class`.
// ============================================================================

fn as_string(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected a string, got {other:?}"),
    }
}

/// `convert-string-for-math ctx MathItalic `abc``: each ASCII small letter
/// maps to its normal-italic Mathematical-Alphanumeric codepoint (the
/// U+1D44E block — `a`→U+1D44E, `b`→U+1D44F, `c`→U+1D450). NOT gated on font
/// availability (this is a string primitive, unlike the rendering-path
/// `resolve_variant_char`), so `Mono` is irrelevant here.
#[test]
fn convert_string_for_math_italic_maps_abc_to_1d44e_block() {
    let src = with_ctx("convert-string-for-math ctx MathItalic `abc`");
    let v = run(&src).expect("convert-string-for-math should compile and evaluate");
    assert_eq!(as_string(v), "\u{1D44E}\u{1D44F}\u{1D450}");
}

/// Special-cased letter (`h` under Italic → U+210E PLANCK CONSTANT, an
/// upstream exception, not U+1D455) and pass-through for chars with no
/// variant mapping (a space and a digit stay unchanged).
#[test]
fn convert_string_for_math_italic_passthrough_and_h_exception() {
    let src = with_ctx("convert-string-for-math ctx MathItalic `h 7`");
    let v = run(&src).expect("convert-string-for-math should compile and evaluate");
    // 'h' -> U+210E, ' ' and '7' have no Italic variant -> unchanged.
    assert_eq!(as_string(v), "\u{210E} 7");
}

/// A whole-token `math_class_map` hit (`-` → U+2212 MINUS SIGN) short-circuits
/// the per-char path, exactly as upstream's `MathClassMap.find_opt s` does
/// before the per-char loop.
#[test]
fn convert_string_for_math_whole_token_class_map_minus() {
    let src = with_ctx("convert-string-for-math ctx MathItalic `-`");
    let v = run(&src).expect("convert-string-for-math should compile and evaluate");
    assert_eq!(as_string(v), "\u{2212}");
}

/// The passed `math-char-class` (not the context's default `MathItalic`)
/// drives the remap: under `MathBoldRoman`, `A` → U+1D400.
#[test]
fn convert_string_for_math_uses_passed_class() {
    let src = with_ctx("convert-string-for-math ctx MathBoldRoman `A`");
    let v = run(&src).expect("convert-string-for-math should compile and evaluate");
    assert_eq!(as_string(v), "\u{1D400}");
}
