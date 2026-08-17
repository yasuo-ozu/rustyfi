//! Vendoring Wave 2 (`…/tmp/vendoring-scout.md` §4 Wave 1, gated on L5b/
//! G3): the 9 graphics-tier upstream 0.1 `stdlib`/`tabular`/
//! `footnote-scheme` packages — `lib-rustyfi/dist-v01/packages/{graphics,
//! deco,hdecoset,vdecoset,inline,block,logo,tabular,footnote-scheme}.
//! satyh` — transliterated from the real upstream source
//! (`saphe-split@b836d512`) per the scout's T1-T8 dialect, extended to
//! cover module-body `use open` erasure via per-binding `let open M in`
//! (no `Bind::Open` exists in this port's v1 grammar — see `logo.satyh`'s
//! own banner) and the T4/G1 operator-section lambda fallback (`( ++ )`/
//! `( +++ )`, `Atomic::OpRef` still unbuilt).
//!
//! **G8 (discovered by this wave, see `graphics.satyh`'s banner for the
//! full writeup): every package here is tested ONLY via the "value bar"**
//! (`compile_v01_via_loader`, reproduced from `v01_stdlib.rs` — real
//! loader -> `lower_file_v1` prelude concatenation -> `elaborate_program`
//! -> plain `typecheck_with_version` -> `eval::Interp::eval`), never via
//! the sealing-checked `compile_document_v1` bar `v01_stdlib.rs` reserves
//! for `color`'s capstone. Reason: this port's v1 module-sealing width
//! check (`v1::module_check::check_program`, what `compile_document_v1`
//! runs) resolves bare sig type names through `typecheck.rs`'s
//! `name_to_mono`, which has no case for `"path"`/`"pre-path"`/
//! `"graphics"`/`"deco"`/`"deco-set"`/`"image"` — every one of these 9
//! packages' SEALED (`:>`) signatures mentions at least one of them
//! (directly, or transitively through `@require: inline`, whose own
//! sig does), so `compile_document_v1` on ANY document requiring ANY of
//! them fails with a spurious "does not match its signature" mismatch
//! (confirmed by an un-committed probe against `path.satyh`, which
//! already carried this exact latent gap since Wave 0 — Wave 0 simply
//! never exercised it, since only `color`'s capstone used
//! `compile_document_v1` and `color`'s sig only mentions `color`, a
//! registered builtin variant, which — unlike `path`/`graphics`/`deco`/
//! `deco-set` — happens to already resolve correctly). `footnote-
//! scheme.satyh`'s OWN sig is clean (no `path`/`graphics`/`deco`
//! mentions) but it `@require:`s `inline`, which is not, so it is
//! equally affected transitively. This is a PRE-EXISTING port gap, not
//! introduced by this vendoring pass, and not fixable without editing
//! `typecheck.rs`/`v1/module_check.rs` (out of scope for this pass: no
//! `.rs` source edits). The value-bar tests below still prove real
//! end-to-end loading, lowering, elaboration, (unsealed) typechecking,
//! and evaluation through the production loader — exactly the bar 16 of
//! Wave 0's 17 packages already relied on.
//!
//! Three more PRE-EXISTING, previously-latent gaps this wave's transliteration
//! surfaced — a later language-completeness pass resolved all three (G9
//! by a real one-line fix; G10/G11 were already subsumed by intervening
//! work, see each note below):
//!  - **G9 FIXED** (`inline.satyh`'s banner): `typecheck.rs`'s
//!    `PRIMITIVE_NAMES` list (consulted only by the plain/unsealed
//!    typecheck path) omitted `"inline-frame-inner"`, even though the
//!    primitive itself and `prim_types.rs` both registered it correctly —
//!    `frame-inner` was dropped from the vendored `Inline` module (T6) as
//!    a result. Fixed by adding the one missing name; `Inline.frame-inner`
//!    is restored. See `crates/rustyfi-lang/tests/v01_lang_completeness.rs`'s
//!    `inline_frame_inner_typechecks_and_evaluates`.
//!  - **G10 ALREADY FIXED** (`hdecoset.satyh`'s banner): an
//!    EXPRESSION-LEVEL named `let NAME param* = … in …`
//!    (`cst_v1::ast::Expr::LetIn`) was reported to only accept plain
//!    variable names as params — no wildcards, no compound patterns —
//!    unlike top-level `val` binds. By the time this was investigated, the
//!    optional-arg-rows increments (2/3a) had already generalized
//!    `cst_v1::ast::Param`/`ParamBody` (shared by `Expr::Fun`,
//!    `RecClauseV1`, AND `Expr::LetIn` alike) to carry a full `PatBot`, so
//!    `let decoS (x, y) w h d = …` now parses and lowers exactly like
//!    `hdecoset.satyh`/`vdecoset.satyh`'s ORIGINAL (pre-transliteration)
//!    source without the `fun (x, y) w h d -> …` spelling workaround. See
//!    `v01_lang_completeness.rs`'s `let_binding_wildcard_parameter_
//!    ignores_its_argument`/`let_binding_tuple_destructuring_parameter`.
//!  - **G11 NOT A BUG** (this file, `footnote-scheme.satyh`'s test
//!    section): a flat program containing BOTH a `command \math`-shaped
//!    value AND a `+++` (`block-boxes` concat) application elsewhere was
//!    reported to spuriously fail the `+++` site. Root-caused to `let open
//!    V01Mini in` shadowing the global `+++` with `v01-mini.satyh`'s own
//!    test-only `val (+++) a b = a + b * 2` (ordinary `open` shadowing,
//!    not an inference defect) — see that test section's comment for the
//!    full writeup and `footnote_scheme_main_with_a_command_math_context_
//!    compiles_and_evaluates` for the real-world scenario now proven to
//!    work end to end.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length, VertBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile};
use rustyfi_syntax::RustyfiVersion;

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist-v01/packages")
}

struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-v01-stdlib-graphics-{tag}-{}-{}.saty",
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

/// A real (ASCII-only) `FontMetrics`, for tests that DO render real text
/// through `read-inline` (`logo.satyh`'s `\SATySFi`/`\LaTeX`/`\TeX`,
/// `footnote-scheme.satyh`'s label text) — mirrors `v01_stdlib.rs`'s own
/// `Mono`.
struct Mono;

impl FontMetrics for Mono {
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

fn as_v01(f: &LoadedFile) -> &rustyfi_syntax::cst_v1::FileV1 {
    match &f.cst {
        LoadedCst::V0_1(cst) => cst,
        LoadedCst::V0_0_6(_) => unreachable!("this test's helper is V0_1-only"),
    }
}

/// Reproduced from `v01_stdlib.rs` (this crate's established per-file-
/// helper convention — no shared test-support library target exists).
fn compile_v01_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    compile_v01_via_loader_with_metrics(tag, src, &NoFonts)
}

fn compile_v01_via_loader_with_metrics(
    tag: &str,
    src: &str,
    metrics: &dyn FontMetrics,
) -> Result<Value, String> {
    let doc = TempDoc::new(tag, src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        version: RustyfiVersion::V0_1,
        ..Default::default()
    };
    let program = rustyfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;

    let (entry, deps) = program
        .files
        .split_last()
        .expect("loader always yields at least the entry file");
    let mut prelude = Vec::new();
    for dep in deps {
        prelude.extend(lower::lower_file_v1(as_v01(dep)).map_err(|e| format!("lower dep: {e}"))?);
    }
    let entry_cst = as_v01(entry);
    let body = lower::lower_document_v1(entry_cst).map_err(|e| format!("lower entry: {e}"))?;
    let eoi = match entry_cst {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("entry must parse as a V0_1 document".to_string()),
    };
    let file = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(rustyfi_syntax::leaf::KwIn(rustyfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };

    let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, RustyfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(metrics);
    // `coerce_graphics_result` (H1-H6/R2, `v01_graphics_collection.rs`'s own
    // harness) forks on `interp.version` at EVAL time (unlike the primitive
    // *selection*, which `base_env_with_version` already resolved) — a
    // graphics-tier package's `graphics`-callback primitives
    // (`inline-graphics`/`inline-graphics-outer`/`tabular`/the deco family)
    // need this set, or they eval under the default `V0_0_6` behavior
    // (expects the callback to return a LIST) even though everything above
    // was compiled as V0_1.
    interp.version = RustyfiVersion::V0_1;
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&elaborated.body, &store))
        .map_err(|e| format!("eval: {e}"))
}

fn assert_bare_access_unbound(tag: &str, require: &str, bare_expr: &str) {
    let src = format!("@require: {require}\n{bare_expr}");
    let err = compile_v01_via_loader(tag, &src)
        .err()
        .unwrap_or_else(|| panic!("[{tag}] expected bare `{bare_expr}` to fail, it compiled"));
    assert!(
        err.contains("unbound variable"),
        "[{tag}] expected an unbound-variable error, got: {err}"
    );
}

fn as_length(v: Value) -> Length {
    match v {
        Value::Length(l) => l,
        other => panic!("expected a length, got {other:?}"),
    }
}

fn as_tuple(v: Value) -> Vec<Value> {
    match v {
        Value::Tuple(vs) => vs,
        other => panic!("expected a tuple, got {other:?}"),
    }
}

fn as_inline_boxes(v: Value) -> Vec<rustyfi_backend::HorzBox> {
    match v {
        Value::InlineBoxes(bs) => bs,
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

fn as_block_boxes(v: Value) -> Vec<VertBox> {
    match v {
        Value::BlockBoxes(bs) => bs,
        other => panic!("expected block-boxes, got {other:?}"),
    }
}

fn as_bbox_option(v: Value) -> Option<((f64, f64), (f64, f64))> {
    fn as_point(v: Value) -> (f64, f64) {
        let vs = as_tuple(v);
        (as_length(vs[0].clone()).0, as_length(vs[1].clone()).0)
    }
    match v {
        Value::Ctor(name, payload) => match (name.as_str(), payload.map(|b| *b)) {
            ("None", None) => None,
            ("Some", Some(Value::Tuple(vs))) if vs.len() == 2 => {
                let mut it = vs.into_iter();
                Some((as_point(it.next().unwrap()), as_point(it.next().unwrap())))
            }
            (other, _) => panic!("expected a bbox option, got variant '{other}'"),
        },
        other => panic!("expected an option, got {other:?}"),
    }
}

fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

// ============================================================================
// `graphics.satyh` — sealed; `Basic.point`/`unite-graphics`/`shift-
// graphics`/`get-graphics-bbox`'s `Option` fork (R3).
// ============================================================================

#[test]
fn graphics_bare_empty_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("graphics-bare", "graphics", "empty");
    });
}

#[test]
fn graphics_empty_has_no_bbox() {
    run_with_big_stack(|| {
        let src = "@require: graphics
get-graphics-bbox Graphics.empty";
        let v = compile_v01_via_loader("graphics-empty-bbox", src)
            .expect("graphics.satyh should compile");
        assert_eq!(as_bbox_option(v), None);
    });
}

#[test]
fn graphics_shift_translates_a_filled_rectangle_bbox() {
    run_with_big_stack(|| {
        let src = "@require: color
@require: graphics
@require: path
let p = Path.rectangle (0pt, 0pt) (10pt, 10pt) in
let gr = fill Color.black p in
get-graphics-bbox (Graphics.shift (5pt, 5pt) gr)";
        let v = compile_v01_via_loader("graphics-shift-bbox", src)
            .expect("graphics.satyh should compile");
        assert_eq!(
            as_bbox_option(v),
            Some(((5.0, 5.0), (15.0, 15.0))),
            "(0,0)-(10,10) shifted by (5,5)"
        );
    });
}

// ============================================================================
// `deco.satyh` — sealed; a `deco` is a plain 4-ary curried function
// `point -> length -> length -> length -> graphics` (L5b/G3's H3-H6
// singular retype) — applied directly here, no document/frame firing
// needed to observe it.
// ============================================================================

#[test]
fn deco_bare_empty_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("deco-bare", "deco", "empty (0pt, 0pt) 1pt 1pt 1pt");
    });
}

#[test]
fn deco_simple_frame_bbox_matches_the_frame_box() {
    run_with_big_stack(|| {
        let src = "@require: deco
@require: color
get-graphics-bbox (Deco.simple-frame 1pt Color.black Color.white (10pt, 20pt) 30pt 5pt 2pt)";
        let v = compile_v01_via_loader("deco-simple-frame-bbox", src)
            .expect("deco.satyh should compile");
        // `simple-frame`'s path is `Path.rectangle (x, y -' d) (x +' w, y +' h)`:
        // (10, 20-2)-(10+30, 20+5) = (10,18)-(40,25).
        assert_eq!(as_bbox_option(v), Some(((10.0, 18.0), (40.0, 25.0))));
    });
}

// ============================================================================
// `hdecoset.satyh` — sealed; `deco-set = deco * deco * deco * deco`.
// ============================================================================

#[test]
fn hdecoset_bare_empty_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound(
            "hdecoset-bare",
            "hdecoset",
            "let (decoS, _, _, _) = empty in decoS (0pt, 0pt) 1pt 1pt 1pt",
        );
    });
}

#[test]
fn hdecoset_simple_frame_stroke_decos_bbox_matches_the_frame_box() {
    run_with_big_stack(|| {
        let src = "@require: hdecoset
@require: color
let (decoS, decoH, decoM, decoT) = HDecoSet.simple-frame-stroke 1pt Color.black in
get-graphics-bbox (decoS (0pt, 0pt) 10pt 4pt 2pt)";
        let v = compile_v01_via_loader("hdecoset-decos-bbox", src)
            .expect("hdecoset.satyh should compile");
        assert_eq!(as_bbox_option(v), Some(((0.0, -2.0), (10.0, 4.0))));
    });
}

// ============================================================================
// `vdecoset.satyh` — sealed; `Gray(0.5)`/`Color.black` flat ctor/qualified
// constant, `unite-graphics` called bare.
// ============================================================================

#[test]
fn vdecoset_bare_paper_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound(
            "vdecoset-bare",
            "vdecoset",
            "let (decoS, _, _, _) = paper in decoS (0pt, 0pt) 1pt 1pt 1pt",
        );
    });
}

#[test]
fn vdecoset_quote_round_decos_bbox_matches_the_quote_rectangle() {
    run_with_big_stack(|| {
        let src = "@require: vdecoset
@require: color
let (decoS, decoH, decoM, decoT) = VDecoSet.quote-round 6pt 1pt Color.black in
get-graphics-bbox (decoS (0pt, 0pt) 10pt 4pt 2pt)";
        let v = compile_v01_via_loader("vdecoset-quote-round-bbox", src)
            .expect("vdecoset.satyh should compile");
        // `decoS` is `rectangle-round-left r (x, y-'d) (x+'qw, y+'h)`:
        // (0, -2)-(6, 4).
        assert_eq!(as_bbox_option(v), Some(((0.0, -2.0), (6.0, 4.0))));
    });
}

#[test]
fn vdecoset_paper_decos_bbox_is_a_union_including_the_shadow() {
    run_with_big_stack(|| {
        let src = "@require: vdecoset
@require: color
let (decoS, decoH, decoM, decoT) = VDecoSet.paper in
get-graphics-bbox (decoS (0pt, 0pt) 10pt 4pt 2pt)";
        let v = compile_v01_via_loader("vdecoset-paper-bbox", src)
            .expect("vdecoset.satyh should compile");
        // Union of the shadow polygon (extends `xshift`=2pt/`yshift`=1pt past
        // the frame) and the 0.5pt-stroked frame rectangle itself — just prove
        // it is `Some` and strictly larger than the bare frame rectangle
        // (0,-2)-(10,4), rather than pinning every shadow vertex.
        let (lo, hi) = v
            .clone()
            .pipe(as_bbox_option)
            .expect("paper's decoS must have a bbox");
        assert!(
            hi.0 - lo.0 > 10.0,
            "expected the shadow to widen the bbox, got {lo:?}-{hi:?}"
        );
    });
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

// ============================================================================
// `inline.satyh` — sealed; `concat`'s `( ++ )` T4/G1 lambda fallback,
// `skip`/`kern`/`get-natural-advance`, `graphics-fixed`/`graphics-outer`
// (H1/H2 singular-`graphics` callbacks).
// ============================================================================

#[test]
fn inline_bare_nil_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("inline-bare", "inline", "get-natural-advance nil");
    });
}

#[test]
fn inline_concat_sums_skip_widths_via_the_lambda_fallback_operator_section() {
    run_with_big_stack(|| {
        let src = "@require: inline
Inline.get-natural-advance (Inline.concat [Inline.skip 3pt, Inline.skip 4pt, Inline.skip 5pt])";
        let v = compile_v01_via_loader("inline-concat-advance", src)
            .expect("inline.satyh should compile");
        assert_eq!(as_length(v), Length::pt(12.0));
    });
}

#[test]
fn inline_kern_negates_its_length() {
    run_with_big_stack(|| {
        let src = "@require: inline
Inline.get-natural-advance (Inline.kern 5pt)";
        let v = compile_v01_via_loader("inline-kern-advance", src)
            .expect("inline.satyh should compile");
        assert_eq!(as_length(v), Length::pt(-5.0));
    });
}

#[test]
fn inline_graphics_fixed_wraps_the_singular_graphics_callback() {
    run_with_big_stack(|| {
        let src = "@require: inline
@require: color
@require: path
Inline.get-natural-advance
  (Inline.graphics-fixed 10pt 10pt 0pt (fun pt -> fill Color.black (Path.rectangle pt (10pt, 10pt))))";
        let v = compile_v01_via_loader("inline-graphics-fixed", src)
            .expect("inline.satyh (H1 singular-graphics callback) should compile");
        assert_eq!(as_length(v), Length::pt(10.0));
    });
}

// ============================================================================
// `block.satyh` — sealed; every `Inline` reference already qualified
// upstream; `concat`'s `( +++ )` T4/G1 lambda fallback.
// ============================================================================

#[test]
fn block_bare_nil_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("block-bare", "block", "concat [nil]");
    });
}

#[test]
fn block_concat_preserves_each_skip_via_the_lambda_fallback_operator_section() {
    run_with_big_stack(|| {
        let src = "@require: block
Block.concat [Block.skip 3pt, Block.skip 4pt]";
        let v = compile_v01_via_loader("block-concat", src).expect("block.satyh should compile");
        let bbs = as_block_boxes(v);
        assert_eq!(bbs.len(), 2);
        assert!(matches!(bbs[0], VertBox::Skip(w) if w == Length::pt(3.0)));
        assert!(matches!(bbs[1], VertBox::Skip(w) if w == Length::pt(4.0)));
    });
}

// ============================================================================
// `logo.satyh` — sealed, command-only; `use open Inline` erasure via
// per-binding `let open Inline in` (no `Bind::Open` in this grammar).
// ============================================================================

#[test]
fn logo_rustyfi_command_reads_to_nonempty_inline_boxes() {
    run_with_big_stack(|| {
        let src = "@require: logo
@require: context
@require: v01-mini
let open V01Mini in
let ctx = Context.initial 400pt (command \\math) in
read-inline ctx {\\Logo.SATySFi;}";
        let v = compile_v01_via_loader_with_metrics("logo-rustyfi", src, &Mono)
            .expect("logo.satyh should compile");
        let ibs = as_inline_boxes(v);
        assert!(!ibs.is_empty(), "\\SATySFi should read to at least one box");
    });
}

// ============================================================================
// `tabular.satyh` — sealed, command-only; the rules callback's singular-
// `graphics` command-arg type (R2). The `\tabular` command's own two
// command-args are each themselves FUNCTION types (a callback taking two
// more callbacks, then a `cell` builder), and this port's `CmdTail::Args`
// lowering (`v1/lower.rs::lower_cmd_tail`) treats everything after the
// command name as ONE plain application chain — getting a *closure-typed*
// argument bound to the right `AppArg` position while also satisfying the
// lexer's "active area" tokenizing turned out fragile to hand-write get
// right in a `{…}` inline-text call site; rather than chase that
// syntax, this package is proven the same way `path`/`context`'s own
// non-command members are: the qualified-export probe below, plus loading
// the module standalone (the same bar `color`/`context`'s OTHER members
// use) — real loader -> lower -> elaborate -> typecheck -> eval, proving
// the sig's command-arg type (with its embedded singular-`graphics`
// callback) itself lowers/typechecks cleanly.
// ============================================================================

#[test]
fn tabular_bare_tabular_command_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("tabular-bare", "tabular", "command \\tabular");
    });
}

#[test]
fn tabular_module_loads_and_typechecks_standalone() {
    run_with_big_stack(|| {
        let src = "@require: tabular
0";
        let v = compile_v01_via_loader("tabular-load", src).expect("tabular.satyh should compile");
        assert!(matches!(v, Value::Int(0)));
    });
}

// ============================================================================
// `footnote-scheme.satyh` — sealed; its OWN sig is G8-clean (no `path`/
// `graphics`/`deco`/`deco-set` mentions), but it `@require:`s `inline`,
// which is not — still tested via the value bar for consistency with the
// rest of this wave (and because `@require: inline` alone already makes
// a sealing-checked capstone fail).
// ============================================================================

#[test]
fn footnote_scheme_bare_initialize_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("footnote-scheme-bare", "footnote-scheme", "initialize ()");
    });
}

#[test]
fn footnote_scheme_initialize_and_start_page_compile_and_evaluate() {
    run_with_big_stack(|| {
        let src = "@require: footnote-scheme
let () = FootnoteScheme.initialize () in
FootnoteScheme.start-page ()";
        let v = compile_v01_via_loader("footnote-scheme-init", src)
            .expect("footnote-scheme.satyh should compile");
        assert!(matches!(v, Value::Unit), "expected unit, got {v:?}");
    });
}

// `FootnoteScheme.main`/`main-no-number` ARE now exercised end-to-end below
// (see `footnote_scheme_main_with_a_command_math_context_compiles_and_
// evaluates`).
//
// G11 RESOLVED (was reported as: in this harness, a flat program
// containing BOTH a `command \math`-shaped value AND a `+++` application
// elsewhere spuriously fails the `+++` site with "type mismatch: expected
// `int`, found `block-boxes`"). INVESTIGATED: this is NOT a type-inference
// bug. The only source of a bare, unqualified `\math` binding available to
// this harness's tests is `let open V01Mini in` (`v01-mini.satyh`), and
// `V01Mini` ALSO defines its own test-only `val (+++) a b = a + b * 2`
// (added for Sub-slice 2b's "`val ( binop )` binds" coverage) — `let open
// V01Mini in` therefore shadows the GLOBAL block-boxes-concatenating
// `+++` with `V01Mini`'s own int-typed one for the rest of that scope,
// exactly matching the "expected `int`" symptom. This is ordinary `open`
// shadowing, not a defect: a `command \math` value built WITHOUT opening
// `V01Mini` (e.g. a fresh module that doesn't redefine `+++`) never
// triggers it (see `crates/rustyfi-lang/tests/v01_lang_completeness.rs`'s
// `command_math_value_does_not_shadow_the_global_plus_plus_plus_operator`),
// and the actual real-world scenario this blocked —
// `FootnoteScheme.main` (which itself uses `+++` internally) applied to a
// real context built via `command \math` — compiles and evaluates cleanly
// end to end, proven below.

// ============================================================================
// G8 FIXED: `typecheck.rs`'s `name_to_mono` now recognizes `path`/
// `pre-path`/`graphics`/`image`/`deco`/`deco-set` (version-gated on V0_1)
// — see that function's own doc comment. This wave's packages (`deco`,
// transitively `graphics`/`path`) can now be proven through the REAL
// sealing-checked pipeline (`rustyfi_lang::compile_document_v1`, which
// runs `v1::module_check::check_program` over every dependency), not just
// the "value bar" (`compile_v01_via_loader`) every test above uses. This
// mirrors `v01_stdlib.rs`'s `color_document_capstone_loads_and_compiles_
// via_v01_mini` — real loader -> `compile_document_v1` through
// `V01Mini.document` — except here the exercised sig lines are `deco`
// (`Deco.simple-frame`'s own `: length -> color -> color -> deco`) and,
// transitively via `deco.satyh`'s `@require: graphics`/`@require: path`,
// `graphics` and `path` (`Graphics`/`Path`'s sealed sigs) — exactly the
// three base types `graphics.satyh`'s G8 banner named as unrecognized
// before this fix. Before the fix, this test's `compile_document_v1` call
// failed with a spurious "module `Deco`/`Graphics`/`Path` does not match
// its signature: … type mismatch: expected `deco`, found `deco`" (etc.,
// `Display`-identical both sides); it now succeeds.
// ============================================================================

#[test]
fn deco_document_capstone_loads_and_compiles_via_v01_mini_through_the_sealed_pipeline() {
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: deco
@require: color

let ib = embed-string (match get-graphics-bbox (Deco.simple-frame 1pt Color.black Color.white (0pt, 0pt) 10pt 5pt 2pt) with
  | None    -> `no-bbox`
  | Some(_) -> `has-bbox`
  end) in
let open V01Mini in
document (| title = `deco` |) '<
  +p { Deco says #ib;. }
>";
        let doc = TempDoc::new("deco-capstone-sealed", src);
        let opts = LoadOptions {
            lib_root: Some(lib_root()),
            version: RustyfiVersion::V0_1,
            ..Default::default()
        };
        let program = rustyfi_loader::load(&doc.0, &opts)
            .expect("v01-mini + deco (+ its graphics/path deps) should load");

        // Every dependency the loader pulled in must be a real V0_1 parse
        // (deco.satyh, graphics.satyh, path.satyh, color.satyh, v01-mini.satyh,
        // and their own basic/float/point/length/list transitive deps).
        for f in &program.files {
            assert!(matches!(f.cst, LoadedCst::V0_1(_)));
        }

        // THE POINT OF THIS TEST: `compile_document_v1` runs
        // `v1::module_check::check_program`, the sealing width check G8
        // fixed. Before the fix this `expect` panicked with a spurious
        // signature-mismatch on `deco`/`graphics`/`path`.
        let doc_value = rustyfi_lang::compile_document_v1(&program.files, &Mono).expect(
            "deco.satyh (+ transitively graphics.satyh/path.satyh) should now seal \
             through the real compile_document_v1 pipeline (G8 fixed)",
        );
        assert_eq!(doc_value.pages.len(), 1);
        assert!(
            doc_value.pages[0].lines.len() >= 2,
            "expected the +p paragraph plus v01-mini's footer line, got {}",
            doc_value.pages[0].lines.len()
        );
    });
}

#[test]
fn footnote_scheme_main_with_a_command_math_context_compiles_and_evaluates() {
    // THE POINT OF THIS TEST (G11): builds a REAL context via
    // `command \math` (through `V01Mini`, `let open V01Mini in`) and
    // applies `FootnoteScheme.main` — whose own body uses `+++`
    // internally (`add-footnote (bb-before +++ bb)`) — to it, all in one
    // flat program. Before investigating G11 this combination was
    // suspected to spuriously fail the `+++` site; it does not (see the
    // "G11 RESOLVED" comment above this test's module section for why the
    // original report's repro fails for an unrelated, expected reason:
    // `V01Mini`'s own test-only `val (+++)` shadowing the global operator
    // under `let open`, not a type-checker defect).
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: footnote-scheme
let open V01Mini in
let ctx = get-initial-context 100pt (command \\math) in
let () = FootnoteScheme.initialize () in
FootnoteScheme.main ctx (fun n -> read-inline ctx (embed-string (arabic n))) (fun n -> block-skip 1pt)";
        let v =
            compile_v01_via_loader_with_metrics("footnote-scheme-main-command-math", src, &Mono)
                .expect(
                    "FootnoteScheme.main applied to a `command \\math`-built context \
             should typecheck and evaluate (G11: confirmed not a bug)",
                );
        assert!(
            matches!(v, Value::InlineBoxes(_)),
            "expected inline-boxes, got {v:?}"
        );
    });
}
