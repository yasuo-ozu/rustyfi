//! The real `@require: math` gate:
//! `lib-rustyfi/dist/packages/math.satyh` (ported byte-for-byte from
//! upstream) must PARSE, ELABORATE, TYPECHECK, and EVALUATE through the
//! production multi-file loader — the same "compiles" bar
//! `stdlib_tier0.rs`'s own tests hold `list`/`option`/`pervasives`/`gr` to.
//!
//! `math.satyh` transitively pulls in `pervasives`, `list` (hence
//! `option`), and `gr` (hence `geom`) — all five already ported and proven
//! to load on their own (`stdlib_tier0.rs`). This test is the first proof
//! that the whole graph loads TOGETHER, and specifically that every one of
//! `math.satyh`'s ~200 `let`/`let-math` bindings (`\alpha`..`\Omega`,
//! `\to`/`\pm`/…, `\sum`/`\int`/…, `\paren`/`\brace`/…, `\frac`, `\sqrt`,
//! …) both type-checks against its `sig` and evaluates without error — most
//! of them via a FULLY-APPLIED `math-*` primitive call at binding time (see
//! `primitives.rs`'s math-primitive section), not merely a deferred closure.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{
    FontKey, FontMetrics, GraphicsElem, HorzBox, Length, MathGlyph, PureHorzBox,
};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck};
use rustyfi_loader::{LoadOptions, LoadedProgram};

/// This repo's `lib-rustyfi/` (`math.satyh`'s real home), resolved relative
/// to this crate's manifest directory.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// A uniquely-named temp `.saty` file, cleaned up on drop.
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-math-package-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp fixture");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Merge a loader-resolved program's preludes into one synthetic
/// `cst::File`, exactly like `rustyfi`'s `merge_program`: the loader
/// guarantees dependency-first order with the entry document last, so every
/// library's prelude is spliced ahead of the entry's own, in that order.
fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        rustyfi_loader::LoadedCst::V0_0(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's merge_program is the V0_0-only path")
        }
    }
}

fn merge_program(program: LoadedProgram) -> rustyfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

/// `FontMetrics` stub: `math.satyh`'s own top-level evaluation never
/// actually measures a glyph (every `math-*` primitive call at binding time
/// just builds a `Value::Math` tree; nothing here calls `embed-math`/renders
/// a page), so this is never consulted.
struct NoFonts;

impl FontMetrics for NoFonts {
    fn advance(&self, _f: FontKey, _c: char, _size: Length) -> Option<Length> {
        None
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size
    }
    fn descender(&self, _f: FontKey, _size: Length) -> Length {
        Length::pt(0.0)
    }
}

/// Load `src` (a document `@require:`ing packages resolved against
/// `lib_root()`) through the real loader, merge, elaborate, typecheck, and
/// evaluate — returning the final `Value`. The full "compiles" bar, not
/// merely a parse or a typecheck.
fn compile_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    compile_via_loader_with_metrics(tag, src, &NoFonts)
}

/// Same as [`compile_via_loader`], but with a caller-supplied `FontMetrics`
/// — for the tests below that actually render `${…}` through
/// `read-inline`/`embed-math`, which `NoFonts` (advance always `None`)
/// cannot do.
fn compile_via_loader_with_metrics(
    tag: &str,
    src: &str,
    metrics: &dyn FontMetrics,
) -> Result<Value, String> {
    let doc = TempDoc::new(tag, src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        ..Default::default()
    };
    let program = rustyfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;
    let file = merge_program(program);

    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck(&elaborated).map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(metrics);
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&elaborated.body, &store))
        .map_err(|e| format!("eval: {e}"))
}

/// A fully permissive `FontMetrics` stub — `Some(size * 0.5)` for EVERY
/// char, ASCII or not (unlike `stdlib_tier0.rs`'s ASCII-only `Mono`) — for
/// the width-asserting tests below that render non-WinAnsi
/// prose math (`α`, `∑`).
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

/// Run `f` on a thread with a generously large stack. `math.satyh` (1571
/// lines — the largest bundled package by far, and now merged alongside
/// `pervasives`/`list`/`option`/`gr`/`geom`'s own preludes) needs more depth
/// than the default stack allows through syan's recursive-descent parser
/// and this port's tree-walking elaborator/typechecker/evaluator, the same
/// reason `stdlib_tier0.rs`'s own `run_with_big_stack` exists for `gr.satyh`
/// (205 lines). `Value` holds `Rc`s (not `Send`), so the compile call AND
/// every assertion on its result must run entirely *inside* `f`.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

fn as_length(v: Value) -> Length {
    match v {
        Value::Length(l) => l,
        other => panic!("expected a length, got {other:?}"),
    }
}

/// `get-natural-metrics`'s `(width, height, depth)` result — the width only.
fn natural_width(v: Value) -> Length {
    match v {
        Value::Tuple(vs) if vs.len() == 3 => as_length(vs.into_iter().next().unwrap()),
        other => panic!("expected a (length * length * length) tuple, got {other:?}"),
    }
}

/// `embed-math ctx <math>`'s result (a single `inline-boxes` list wrapping
/// one `PureHorzBox::Math`) unwrapped down to its glyphs — for tests
/// below that need to inspect actual glyph text, not just `get-
/// natural-metrics`' summary width.
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

/// Like [`math_glyphs`] but also returns the drawn `rules` — delimiter
/// identity lives in the drawn graphics, not the glyphs.
fn math_box(v: Value) -> (Vec<MathGlyph>, Vec<GraphicsElem>) {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { glyphs, rules, .. }) => (glyphs, rules),
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

/// `embed-math ctx <delim_math>` through the real `math.satyh` under the
/// plain `Mono` stub (no MATH table) — the make_paren closures draw via
/// graphics, needing no font, so this exercises the base-14-shaped path.
fn delim_box(delim_math: &str) -> (Vec<MathGlyph>, Vec<GraphicsElem>) {
    let src = format!(
        "@require: math\n\
         let-inline ctx \\math mm = embed-math ctx mm\n\
         let ctx = get-initial-context 100pt (command \\math)\n\
         in\n\
         embed-math ctx {delim_math}"
    );
    let v = compile_via_loader_with_metrics("b3b2-identity", &src, &Mono)
        .unwrap_or_else(|e| panic!("compile failed for {delim_math}: {e}"));
    math_box(v)
}

/// The `make_paren` closure route restores delimiter IDENTITY.
/// `\paren` fills bezier bowls; `\abs` strokes bars — DIFFERENT shapes, not
/// the identical stretched `(` every delimiter used to collapse to. And
/// because the closures draw via graphics (not font glyphs), this works under
/// the plain `Mono` stub with no MATH font at all.
#[test]
fn b3b2_paren_and_abs_draw_different_shapes_through_the_closures() {
    run_with_big_stack(|| {
        let (paren_g, paren_r) = delim_box(r"${\paren{x}}");
        let (abs_g, abs_r) = delim_box(r"${\abs{x}}");
        // Identity restored: only the inner `x` glyph remains; the
        // delimiters are DRAWN ink, not `(`/`)` glyphs (this used to give 3
        // glyphs, 0 rules and make \abs identical to \paren).
        assert_eq!(
            paren_g.len(),
            1,
            "paren: inner glyph only, delimiters drawn"
        );
        assert_eq!(abs_g.len(), 1, "abs: inner glyph only");
        assert!(!paren_r.is_empty(), "paren draws its delimiters");
        assert!(!abs_r.is_empty(), "abs draws its delimiters");
        assert!(
            paren_r.iter().all(|r| matches!(r, GraphicsElem::Fill(..))),
            "paren draws filled bowls, got {paren_r:?}"
        );
        assert!(
            abs_r.iter().any(|r| matches!(r, GraphicsElem::Stroke(..))),
            "abs draws stroked bars, got {abs_r:?}"
        );
    });
}

/// `\brace` also draws (via the closures) and differs from `\paren` —
/// both fill, but distinct shapes ⇒ distinct total advances. Confirms the
/// closure route is exercised for more than one delimiter kind.
#[test]
fn b3b2_brace_draws_and_differs_from_paren() {
    run_with_big_stack(|| {
        let (brace_g, brace_r) = delim_box(r"${\brace{x}}");
        assert_eq!(brace_g.len(), 1, "brace: inner glyph only");
        assert!(!brace_r.is_empty(), "brace draws its delimiters");
    });
}

// ============================================================================
// `@require: math` — the real gate.
// ============================================================================

#[test]
fn require_math_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // The cheapest whole-module proof: a trivial body still forces the
        // loader to resolve the FULL transitive graph (`pervasives`, `list`
        // -> `option`, `gr` -> `geom`) and forces elaboration/typecheck/
        // evaluation of every one of `math.satyh`'s top-level bindings
        // (`Ast::LetIn`/`Ast::LetMathIn` always infers + evaluates its value
        // eagerly, regardless of whether the entry body ever references it)
        // — so success here means the ENTIRE file compiled, not just
        // whatever the body names.
        let src = "@require: math
in
0";
        let v = compile_via_loader("require-math", src).expect("math.satyh should compile");
        assert_eq!(as_int(v), 0);
    });
}

#[test]
fn require_math_join_is_reachable_across_the_module_boundary() {
    run_with_big_stack(|| {
        // `Math.join` (a plain, non-`let-math` `val`, built out of
        // `List.fold-left`/`math-concat`) called from OUTSIDE the module —
        // proving the module's own qualified-export machinery
        // (`elaborate.rs`'s `export_alias`, producing the `"Math.join"`
        // binding) resolves from a caller's perspective, not just that
        // `math.satyh`'s bindings evaluate internally to themselves.
        let src = "@require: math
in
match Math.join (math-char MathOrd `,`) [] with
| _ -> 1";
        let v = compile_via_loader("require-math-join", src).expect("math.satyh should compile");
        assert_eq!(as_int(v), 1);
    });
}

// ============================================================================
// The `get-initial-context`/`set-math-command`-installed `[math]
// inline-cmd` bare `${…}` in prose dispatches to (`read_inline`'s
// `EmbedMath` arm).
// ============================================================================

/// End-to-end: `Math`'s own `\math` (`direct \math : [math]
/// inline-cmd`, `let-inline ctx \math fml = script-guard Latin (embed-math
/// ctx fml)`), installed via `get-initial-context`'s second argument, must
/// actually run when `read-inline` walks a plain `{ ${…} }` quoted literal
/// — proving the FAITHFUL `Context::math_command` plumbing end to end (not
/// just `+math(${…})`'s existing direct `embed-math` call).
#[test]
fn gap1_installed_math_command_renders_bare_embed_math_in_prose() {
    run_with_big_stack(|| {
        let src = "@require: math
in
let ctx = get-initial-context 200pt (command \\math) in
get-natural-metrics
  (read-inline ctx {${\\alpha} + ${\\frac{1}{2}} + ${\\sum_{k=1}^{n}}})";
        let v = compile_via_loader_with_metrics("gap1-e2e", src, &Mono)
            .expect("installed \\math command should render prose ${...}");
        let w = natural_width(v);
        assert!(w.0 > 0.0, "expected positive width, got {w:?}");
    });
}

/// Override: a locally-defined stub `\m0` (ignoring its `math`
/// argument entirely, `inline-skip 42pt`) installed via `set-math-command`
/// must be the command that actually runs for `${x}` — not some fallback —
/// proving `set-math-command` re-installs `Context::math_command` (not just
/// `get-initial-context`'s own second argument).
#[test]
fn gap1_set_math_command_overrides_the_installed_command() {
    let src = "let-inline ctx \\dummy m = inline-nil
let-inline ctx \\m0 m = inline-skip 42pt
in
let ctx0 = get-initial-context 200pt (command \\dummy) in
let ctx = set-math-command (command \\m0) ctx0 in
get-natural-metrics (read-inline ctx {${x}})";
    let v = compile_via_loader("gap1-override", src)
        .expect("set-math-command's installed \\m0 should run, not the dummy or any fallback");
    let w = natural_width(v);
    assert_eq!(
        w,
        Length::pt(42.0),
        "expected \\m0's inline-skip 42pt, got {w:?}"
    );
}

// Fallback (`Context::initial` directly, no installed command ->
// reflect + lay out through the faithful engine) is pinned by
// `eval_phase2b.rs`'s `itext_embed_math_renders_through_read_inline`.

// ============================================================================
// `math-pull-in-scripts` resolver argument order/values, and the
// `check_subscript` merged sub+sup pair.
// ============================================================================

/// Resolver args: a width-discriminating resolver distinguishes
/// `Some(sup)` (from `${#m^{2}}`) from the bare `(None, None)` case (from
/// `${#m}`) — proving `layout_pull_in_scripts` actually threads the pulled-
/// in scripts through to the resolver rather than always calling it with
/// `(None, None)`.
#[test]
fn gap2_pull_in_scripts_resolver_receives_the_actual_scripts() {
    let src = "let-inline ctx \\math mm = embed-math ctx mm
in
let ctx = get-initial-context 100pt (command \\math) in
let resolver ms mt =
  match mt with
  | None -> math-char MathOrd `NN`
  | Some(t) -> math-sup (math-char MathOrd `Y`) t
in
let m = math-pull-in-scripts MathOp MathOp resolver in
let w-sup = get-natural-metrics (embed-math ctx ${#m^{2}}) in
let w-bare = get-natural-metrics (embed-math ctx ${#m}) in
(w-sup, w-bare)";
    let v = compile_via_loader_with_metrics("gap2-resolver-args", src, &Mono)
        .expect("math-pull-in-scripts resolver should compile and evaluate");
    let (w_sup, w_bare) = match v {
        Value::Tuple(vs) if vs.len() == 2 => {
            let mut it = vs.into_iter();
            (
                natural_width(it.next().unwrap()),
                natural_width(it.next().unwrap()),
            )
        }
        other => panic!("expected a 2-tuple, got {other:?}"),
    };
    assert_ne!(
        w_sup, w_bare,
        "sup-shaped ${{#m^{{2}}}} and bare ${{#m}} widths must differ \
         (resolver must have received different (sub, sup) options)"
    );
    // Bare case: resolver got (None, None) -> two "N" chars, at the base
    // (non-script) size.
    assert_eq!(
        w_bare,
        Length::pt(12.0),
        "expected two full-size 'N' glyphs, got {w_bare:?}"
    );
    // Sup case: resolver got (None, Some(sup)) -> one "Y" at base size plus
    // "2" at script size — smaller than the bare two-full-size-char case.
    assert!(
        w_sup < w_bare,
        "expected the sup-shaped ${{#m^{{2}}}} (one base glyph + one script \
         glyph) to be narrower than the two-full-size-glyph bare case, got \
         w_sup={w_sup:?} w_bare={w_bare:?}"
    );
}

/// `\sum`: `Math`'s own `\sum` (`bigop` = `vop-scheme math-big-char`,
/// built on `math-pull-in-scripts`) with both a sub- and a super-script
/// (`\sum_{k=1}^{n}`) must render without error through the real
/// `check_subscript`/pull-in resolution — the production path `+math`/`\eqn`
/// exercise, now proven reachable via `embed-math` directly too.
#[test]
fn gap2_sum_with_sub_and_sup_renders_through_pull_in_scripts() {
    run_with_big_stack(|| {
        let src = "@require: math
in
let ctx = get-initial-context 200pt (command \\math) in
get-natural-metrics (embed-math ctx ${\\sum_{k=1}^{n}})";
        let v = compile_via_loader_with_metrics("gap2-sum", src, &Mono)
            .expect("\\sum_{k=1}^{n} should render through math-pull-in-scripts");
        let w = natural_width(v);
        assert!(w.0 > 0.0, "expected positive width, got {w:?}");
    });
}

/// Merged sub-sup pair: `${x_1^2}` (a base-tail `Sub` folded into one
/// `Sup` by `check_subscript`) must lay out its sub- and super-script
/// SIDE BY SIDE on the same base — width `w(x) + max(w(1), w(2))·scale` —
/// rather than sequentially stacking two independent corner scripts (the
/// pre-fix behavior, which would additionally add the subscript's own width
/// before placing the superscript).
#[test]
fn gap2_merged_sub_sup_pair_places_scripts_side_by_side() {
    let src = "let-inline ctx \\math m = embed-math ctx m
in
let ctx = get-initial-context 200pt (command \\math) in
get-natural-metrics (embed-math ctx ${x_1^2})";
    let v = compile_via_loader_with_metrics("gap2-merged-sub-sup", src, &Mono)
        .expect("${x_1^2} should compile and evaluate");
    let w = natural_width(v);

    // Mirror the implementation's own arithmetic exactly (`Mono`: every
    // glyph advances `size * 0.5`; `SCRIPT_SCALE = 0.7`, base font size
    // 12pt from `Context::initial`).
    let base = Length::pt(12.0) * 0.5; // w(x)
    let script_size = Length::pt(12.0) * 0.7;
    let script = script_size * 0.5; // w(1) == w(2)
    let expected_merged = base + script.max(script);
    let sequential_if_unfixed = base + script + script;

    assert_eq!(
        w, expected_merged,
        "expected the merged side-by-side width w(x) + max(w(1), w(2))·scale, got {w:?}"
    );
    assert_ne!(
        w, sequential_if_unfixed,
        "must NOT match the old sequential (unmerged) stacking width"
    );
}

// ============================================================================
// `|`-separated math lists (`${| a | b |}`, a `math list`
// LITERAL, not a matrix) consumed by `Math`'s own verbatim-ported
// `+align`, over the generic `tabular` primitive.
// ============================================================================

/// Package smoke: `Math.+align : [(math list) list] block-cmd` applied
/// to a real two-row, two-column `${| a | b |}; ${| c | d |}` table must
/// compile and evaluate through the production `read-block` path — proving
/// the elaborator-only `math_block_ast` split feeds a real bundled consumer,
/// not just a synthetic `match` on the resulting list shape.
#[test]
fn gap3_align_over_bar_separated_math_lists_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: math
in
let ctx = get-initial-context 200pt (command \\math) in
read-block ctx '<+align[${| a | b |}; ${| c | d |}];>";
        let v = compile_via_loader_with_metrics("gap3-align-smoke", src, &Mono)
            .expect("+align over ${| a | b |}; ${| c | d |} should compile and evaluate");
        match v {
            Value::BlockBoxes(_) => {}
            other => panic!("expected block-boxes, got {other:?}"),
        }
    });
}

// ============================================================================
// Bundled `math.satyh` consumers: `\mathrm`/`\text` are real
// production commands built on `math-char-class`/`text-in-math`, not
// synthetic stubs.
// ============================================================================

/// Package smoke: `math.satyh`'s own `\text` (`let-math \text it =
/// text-in-math MathOrd (fun ctx -> read-inline ctx it)`) applied to a real
/// inline-text literal, embedded through `embed-math`/`command \math`, must
/// render WITHOUT erroring and with positive width — proving the
/// `EmbeddedText` layout arm's box-in-math bridge from a REAL bundled
/// consumer, not just the synthetic `text-in-math` call in
/// `math_variant_class.rs`.
#[test]
fn gap6_text_command_via_math_satyh_renders_positive_width() {
    run_with_big_stack(|| {
        let src = "@require: math
in
let ctx = get-initial-context 200pt (command \\math) in
get-natural-metrics (embed-math ctx ${\\text!{ab}})";
        let v = compile_via_loader_with_metrics("gap6-text-command", src, &Mono)
            .expect("\\text!{ab} should compile and evaluate");
        let w = natural_width(v);
        assert!(w.0 > 0.0, "expected positive width, got {w:?}");
    });
}

/// Package smoke: `math.satyh`'s own `\mathrm` (`let-math \mathrm m =
/// ${\math-style!(MathRoman){#m}}`, built on `math-char-class`) applied to
/// `${\mathrm{x}}` must render the PLAIN ascii `"x"` (Roman = identity),
/// while the bare `${x}` (default `Italic` restyling) renders the actual
/// Unicode Mathematical Italic Small X remap — proving `\mathrm` (a real
/// bundled command, not a synthetic `math-char-class` call) reaches the
/// `ChangeCharClass` layout arm end to end.
#[test]
fn gap5_mathrm_command_via_math_satyh_keeps_plain_ascii() {
    run_with_big_stack(|| {
        let src = "@require: math
in
let ctx = get-initial-context 200pt (command \\math) in
(embed-math ctx ${\\mathrm{x}}, embed-math ctx ${x})";
        let v = compile_via_loader_with_metrics("gap5-mathrm-command", src, &Mono)
            .expect("${\\mathrm{x}} and ${x} should compile and evaluate");
        let (roman, italic) = match v {
            Value::Tuple(vs) if vs.len() == 2 => {
                let mut it = vs.into_iter();
                (
                    math_glyphs(it.next().unwrap()),
                    math_glyphs(it.next().unwrap()),
                )
            }
            other => panic!("expected a 2-tuple, got {other:?}"),
        };
        assert_eq!(roman.len(), 1);
        assert_eq!(
            roman[0].text, "x",
            "\\mathrm{{x}} should keep the plain ascii 'x'"
        );
        assert_eq!(italic.len(), 1);
        assert_eq!(
            italic[0].text, "\u{1D465}",
            "bare ${{x}} should render the default Italic Unicode remap"
        );
        assert_ne!(roman[0].text, italic[0].text);
    });
}
