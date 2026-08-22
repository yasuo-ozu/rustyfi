//! Char-class value model, text-in-math degrade, and
//! `set-math-variant-char`/`get-left-math-class`/`get-right-math-class`
//! acceptance coverage. Pure pipeline — no `@require:`, no loader,
//! `rustyfi_syntax::parse_file` straight through elaborate/typecheck/eval.

use rustyfi_backend::{FontKey, FontMetrics, HorzBox, Length, MathGlyph, PureHorzBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

/// A fully permissive `FontMetrics` stub — `Some(size * 0.5)` for EVERY
/// char, including the non-ASCII Mathematical-Alphanumeric remap targets —
/// so the metrics-probe fallback in `resolve_variant_char` always succeeds
/// and the full remap is observable/unit-testable.
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
/// shape) — the metrics probe fails for every remapped Mathematical-
/// Alphanumeric codepoint (all outside ASCII), so the fallback policy
/// must keep the SOURCE char.
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

/// A `context` with no package loaded: a local `\dummy` `[math] inline-cmd`
/// installed via `get-initial-context`'s second argument, never actually
/// invoked by these tests — they all call `embed-math` directly.
fn with_ctx(body: &str) -> String {
    format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    )
}

// ============================================================================
// Whole-token class-map reclassification + spacing.
// ============================================================================

/// `${a-b}`: `-` is its own MATHCHAR token, reclassified `Bin` (and
/// remapped to U+2212 MINUS SIGN) by `default_math_class_map` — width gains
/// `Bin` spacing on both sides (`space_before`'s `SPACE_MATH_BIN`,
/// `primitives.cppo.ml:528`'s `space_math_bin` natural ratio).
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
    let bin_space = Length::pt(12.0) * 0.25;
    let expected = glyph_w + bin_space + glyph_w + bin_space + glyph_w;
    assert_eq!(
        width, expected,
        "expected 3 glyphs + 2 Bin spaces, got {width:?}"
    );
}

/// `${a->b}`: `-` and `>` are consecutive symbol chars, and each is its OWN
/// MATHCHAR token — so each hits `default_math_class_map` on its own (`-` ->
/// U+2212 `Bin`, `>` -> `Rel`), which a single `"->"` token could not (the map's
/// keys are all one character, and the run used to fall through to one `Ord`
/// atom with no spacing anywhere).
///
/// The `-` here is `Bin` RAW but sits immediately before a `Rel`, so
/// `normalize_math_kind` demotes it to `Ord` — no `Bin` space at all. What
/// remains is `Rel` spacing on both sides of the `>`, matching how the
/// reference engine sets `${a->b}` (`𝑎−` ␣ `>` ␣ `𝑏`).
#[test]
fn gap5_multi_char_symbol_run_splits_into_per_char_atoms() {
    let src = with_ctx("embed-math ctx ${a->b}");
    let v = run(&src).expect("${a->b} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(
        glyphs.len(),
        4,
        "expected 4 glyphs (a, -, >, b), got {glyphs:?}"
    );
    assert_eq!(
        glyphs[1].text, "\u{2212}",
        "the `-` gets its own class-map hit now that it is its own token"
    );
    let glyph_w = Length::pt(12.0) * 0.5;
    let rel_space = Length::pt(12.0) * 0.375;
    assert_eq!(
        width,
        glyph_w * 4.0 + rel_space * 2.0,
        "expected 4 glyphs + Rel spacing either side of `>` (the `-` is \
         normalized to Ord before a Rel), got {width:?}"
    );
}

/// Two atoms of the SAME class in a row get NO space between them: upstream's
/// `space_between_math_kinds` table has `(Rel, Ord)` and `(Ord, Rel)` but no
/// `(Rel, Rel)`, so `${a:=b}` sets `:=` tight between two thick spaces rather
/// than opening a third one in the middle.
#[test]
fn adjacent_relations_get_no_space_between_them() {
    let src = with_ctx("embed-math ctx ${a:=b}");
    let v = run(&src).expect("${a:=b} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 4, "expected 4 glyphs (a, :, =, b)");
    let glyph_w = Length::pt(12.0) * 0.5;
    let rel_space = Length::pt(12.0) * 0.375;
    assert_eq!(
        width,
        glyph_w * 4.0 + rel_space * 2.0,
        "expected 4 glyphs + exactly TWO Rel spaces, got {width:?}"
    );
}

/// A `Bin` at the START of a math list is unary, and `normalize_math_kind`
/// demotes it — `${-a}` is a minus sign tight against its operand, not a
/// binary minus with a leading thin space.
#[test]
fn leading_binary_is_normalized_to_ordinary() {
    let src = with_ctx("embed-math ctx ${-a}");
    let v = run(&src).expect("${-a} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 2, "expected 2 glyphs (-, a)");
    assert_eq!(
        width,
        Length::pt(12.0) * 0.5 * 2.0,
        "expected 2 glyphs and NO Bin space, got {width:?}"
    );
}

/// The same demotion one atom in: in `${-------}` only the first `-` could be
/// binary (the rest each follow a raw `Bin`, which `normalize_math_kind`
/// counts as unary-making), and the first is at the list start — so the whole
/// run sets tight, exactly as the reference does for latexcmds'
/// `\underbrace…{-------}`.
#[test]
fn a_run_of_minuses_sets_tight_as_all_minus_signs() {
    let src = with_ctx("embed-math ctx ${-------}");
    let v = run(&src).expect("${-------} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 7, "expected 7 glyphs, got {glyphs:?}");
    assert!(
        glyphs.iter().all(|g| g.text == "\u{2212}"),
        "every one should be U+2212 MINUS SIGN, got {glyphs:?}"
    );
    assert_eq!(
        width,
        Length::pt(12.0) * 0.5 * 7.0,
        "expected 7 glyphs and no inter-atom spacing, got {width:?}"
    );
}

/// `${a:b}`: `:` is reclassified `Rel` (was `Punct` under the old
/// `ascii_math_kind` stand-in) — width gains `Rel` spacing
/// (`SPACE_MATH_REL`, `primitives.cppo.ml:529`) on both sides, strictly more
/// than the old 18pt.
#[test]
fn gap5_colon_reclassified_as_rel() {
    let src = with_ctx("embed-math ctx ${a:b}");
    let v = run(&src).expect("${a:b} should compile and evaluate");
    let (width, glyphs) = math_box(v);
    assert_eq!(glyphs.len(), 3);

    let glyph_w = Length::pt(12.0) * 0.5;
    let rel_space = Length::pt(12.0) * 0.375;
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

/// Under an ASCII-only font the metrics probe for U+1D465 fails, so
/// `resolve_variant_char` falls back to the SOURCE char `x` — the "zero
/// regression under base-14" policy the whole design hinges on.
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
// `set-math-variant-char` / `get-left-math-class` /
// `get-right-math-class`.
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
// Text-in-math degrade.
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
// `types.cppo.ml:1602` `convert_math_variant_char`). Reuses the
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

// ============================================================================
// `text-in-math` over a `line-stack-*` box — azmath's `\overbrace` shape.
// ============================================================================

/// Same unwrap as [`math_box`], but keeping the `rules` too: a stacked
/// body's ink can be GRAPHICS (azmath draws its brace with
/// `inline-graphics`), not only glyphs.
fn math_box_full(v: Value) -> (Length, Vec<MathGlyph>, Vec<rustyfi_backend::GraphicsElem>) {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math {
                    width,
                    glyphs,
                    rules,
                    ..
                }) => (width, glyphs, rules),
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

/// azmath's `\overbrace`/`\underbrace` (`parens.satyh:533`/`:561`) stack the
/// brace over the braced formula with `line-stack-bottom`/`-top` and hand
/// the result BACK to math through `text-in-math`. That makes the
/// `text-in-math` body a single `PureHorzBox::EmbeddedBlock`, and
/// `math_boxes_of_inline_boxes` used to keep only its WIDTH — so every
/// `\overbrace{…}` in the corpus rendered as a correctly-sized hole, brace
/// and contents alike. This is the same nested-container walk bug as the
/// `\underset`/`\overset` one the `EmbeddedText` arm's comment records, one
/// container further in.
#[test]
fn text_in_math_descends_into_a_line_stacked_embedded_block() {
    let src = with_ctx(
        "let braced = embed-math ctx ${abc} in\n\
         let brace = inline-graphics 10pt 3pt 0pt (fun (x, y) ->\n\
           [fill (Gray(0.0)) (start-path (x, y) |> line-to (x +' 10pt, y)\n\
                              |> close-with-line)]) in\n\
         let stacked = line-stack-bottom [brace; braced] in\n\
         embed-math ctx (text-in-math MathOrd (fun _ -> stacked))",
    );
    let v = run(&src).expect("the `\\overbrace` shape should compile and evaluate");
    let (width, glyphs, rules) = math_box_full(v);

    assert_eq!(
        glyphs.len(),
        3,
        "the stacked `${{abc}}` line's three glyphs must survive, got {glyphs:?}"
    );
    assert!(
        !rules.is_empty(),
        "the stacked `inline-graphics` brace must survive as a rule"
    );
    assert!(width > Length::ZERO, "the box keeps its width");

    // `line-stack-bottom` anchors the LAST line, so the brace line above it
    // sits at a POSITIVE `dy` — the stack's vertical offsets have to reach
    // the glyphs, not just their horizontal ones.
    let braced_line_dy = glyphs[0].dy;
    assert_eq!(
        braced_line_dy,
        Length::ZERO,
        "the anchored (last) line sits on the math baseline"
    );
    let rule_top = rules
        .iter()
        .filter_map(rustyfi_backend::graphics_bbox)
        .map(|((_, _), (_, max_y))| max_y)
        .fold(Length::ZERO, |a, b| if b > a { b } else { a });
    assert!(
        rule_top > Length::ZERO,
        "the brace line stacks ABOVE the anchored line, got top {rule_top:?}"
    );
}

/// The same walk's vertical thread, on the GLYPH side: with the math line
/// stacked above the anchored one, its glyphs have to come out above the
/// math baseline too. Harvesting a nested `PureHorzBox::Math`'s glyphs
/// without adding the stacked line's offset collapses every line onto one
/// baseline — the brace overprinting the formula rather than sitting over
/// it, which reads as a rendering artifact instead of the walk bug it is.
#[test]
fn a_stacked_math_line_above_the_anchor_keeps_its_vertical_offset() {
    let src = with_ctx(
        "let braced = embed-math ctx ${abc} in\n\
         let anchor = inline-skip 10pt in\n\
         let stacked = line-stack-bottom [braced; anchor] in\n\
         embed-math ctx (text-in-math MathOrd (fun _ -> stacked))",
    );
    let v = run(&src).expect("a two-line stack should compile and evaluate");
    let (_, glyphs, _) = math_box_full(v);
    assert_eq!(glyphs.len(), 3, "expected 3 glyphs, got {glyphs:?}");
    assert!(
        glyphs.iter().all(|g| g.dy > Length::ZERO),
        "every glyph of the non-anchored line sits above the baseline, got {glyphs:?}"
    );
}
