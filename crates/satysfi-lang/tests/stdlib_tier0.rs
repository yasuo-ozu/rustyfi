//! Slice 1 / Tier 0 stdlib port proof (`docs/plans/stdlib-port.md` §Slice 1):
//! `@require: list` (hence, transitively, `@require: option`) — and
//! `@require: option` alone — must PARSE, ELABORATE, TYPECHECK, and EVALUATE
//! through the real multi-file loader with this repo's `lib-satysfi/` as
//! `lib_root`. This mirrors `satysfi-cli`'s own production pipeline
//! (`main.rs`'s `cmd_compile`: `satysfi_loader::load` -> merge preludes ->
//! `compile_document_cst`) rather than a bespoke single-file harness, so it
//! genuinely exercises `@require:` resolution (including the NESTED
//! `list.satyg -> @require: option` edge) through the production loader
//! crate — not just a hand-rolled shortcut.
//!
//! `option.satyg`/`list.satyg` under `lib-satysfi/dist/packages/` are copied
//! byte-for-byte from upstream (the plan's "copy-verbatim" policy) — this
//! test is the proof the compiler now *accepts* them (the Slice-1
//! acceptance bar is "compiles", i.e. evaluates to a value, not merely
//! "parses" — see the plan's Verification table).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{
    natural_metrics, Closing, FontKey, FontMetrics, GraphicsElem, HorzBox, Length, PathSeg,
    PureHorzBox,
};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck};
use satysfi_loader::{LoadOptions, LoadedProgram};

/// This repo's `lib-satysfi/` (the real Tier-0 packages' home), resolved
/// relative to this crate's own manifest directory — the same convention
/// `compile.rs`'s private `prepare_document` test helper already uses for
/// `stdja-mini.satyh`.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

/// A uniquely-named temp `.saty` file, cleaned up on drop — scaled down from
/// `satysfi-loader/tests/loader.rs`'s `TempDir` fixture pattern to the one
/// entry file each test here needs (the packages themselves already live on
/// disk under `lib_root()`, so there is no fixture tree to build).
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-stdlib-tier0-{tag}-{}-{}.saty",
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
/// `cst::File`, exactly like `satysfi-cli`'s private `merge_program`
/// (`main.rs`): the loader guarantees dependency-first order with the entry
/// document last, so every library's prelude is spliced ahead of the
/// entry's own, in that order (the v0.0.6 analog typechecks each library
/// into a shared environment in dependency order; untyped elaboration gets
/// the same scoping by prelude concatenation).
fn merge_program(program: LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry.cst.in_kw,
        body: entry.cst.body,
        eoi: entry.cst.eoi,
    }
}

/// `FontMetrics` stub: Tier-0 packages (`option`/`list`) are pure computation
/// over int/list/option values — never text/box primitives — so this is
/// never actually consulted. It exists only because `eval::Interp::new`
/// requires a `&dyn FontMetrics`.
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

/// A real (if crude) `FontMetrics`, for the `pervasives` tests below that
/// call `read-inline` on actual text (`\SATySFi` et al.) — `NoFonts`'s
/// always-`None` `advance` would make any non-empty word a dynamic error
/// (see `primitives.rs`'s `text_to_boxes`). Mirrors the `Mono` stub already
/// used the same way by `prims_phase4.rs`/`eval_phase2.rs`/etc.
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

/// Load `src` (a document `@require:`ing packages resolved against
/// `lib_root()`) through the real loader, merge, elaborate, typecheck, and
/// evaluate — returning the final `Value`. This is the full Slice-1
/// "compiles" bar (`docs/plans/stdlib-port.md`'s Verification table:
/// `Parses` / `Typechecks` / `Compiles`), not merely a parse or a typecheck.
fn compile_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    compile_via_loader_with_metrics(tag, src, &NoFonts)
}

/// Same as [`compile_via_loader`], but with a caller-supplied
/// `FontMetrics` — for tests (see `Mono`, below) that actually render text
/// through `read-inline`, which `NoFonts` (advance always `None`) cannot do.
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
    let program = satysfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;
    let file = merge_program(program);

    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck(&elaborated).map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(metrics);
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

/// Run `f` (a self-contained compile-and-assert closure) on a thread with a
/// generously large stack. `gr.satyh` (205 lines — considerably bigger than
/// any package loaded so far) needs more depth than the default stack
/// allows through syan's recursive-descent parser, the same reason
/// `satysfi-syntax/tests/roundtrip.rs`'s deep-nesting test spawns a
/// bigger-stack thread. Unlike that test, `Value` holds `Rc`s (not `Send`),
/// so the compile call AND every assertion on its result must run entirely
/// *inside* `f` — nothing `Value`-shaped can cross back out to the caller's
/// (default-size-stack) thread.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
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

/// Extract `(x, y)` raw `f64`s from a `point` (`Value::Tuple([Length,
/// Length])`) — for asserting exact coordinates after a `shift`/`linear-
/// transform` (roadmap A/B) or a `get-graphics-bbox` corner.
fn as_point_f64(v: &Value) -> (f64, f64) {
    match v {
        Value::Tuple(vs) if vs.len() == 2 => match (&vs[0], &vs[1]) {
            (Value::Length(x), Value::Length(y)) => (x.0, y.0),
            other => panic!("expected a point (length * length), got {other:?}"),
        },
        other => panic!("expected a tuple, got {other:?}"),
    }
}

fn as_int_list(v: Value) -> Vec<i64> {
    match v {
        Value::List(items) => items.into_iter().map(as_int).collect(),
        other => panic!("expected a list, got {other:?}"),
    }
}

// ============================================================================
// `@require: list` (transitively pulls in `option`, per `list.satyg`'s own
// `@require: option` header).
// ============================================================================

#[test]
fn require_list_reverse_and_map_compiles_and_evaluates() {
    let src = "@require: list
in
List.reverse (List.map (fun x -> x + 1) [1; 2; 3])";
    let v = compile_via_loader("list-reverse-map", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![4, 3, 2]);
}

#[test]
fn require_list_length_compiles_and_evaluates() {
    let src = "@require: list
in
List.length (List.map (fun x -> x + 1) [1; 2; 3])";
    let v = compile_via_loader("list-length", src).expect("list.satyg should compile");
    assert_eq!(as_int(v), 3);
}

#[test]
fn require_list_transitively_compiles_option() {
    // `list.satyg` itself `@require:`s `option` (`List.map-with-ends` calls
    // `Option.is-none`) — proving a *nested* `@require:` resolves and the
    // resulting merged program still compiles, not just `list.satyg` in
    // isolation.
    let src = "@require: list
in
Option.from 0 (Some 5)";
    let v = compile_via_loader("list-nested-option", src).expect("should compile");
    assert_eq!(as_int(v), 5);
}

#[test]
fn require_list_uses_pipe_internally() {
    // `list.satyg`'s `reverse`/`map-adjacent`/`map-with-ends` all use `|>`
    // internally (Blocker B) — `fold-left-adjacent` is the most direct way
    // to exercise that path from outside the module.
    let src = "@require: list
in
List.map-adjacent (fun x left right -> x) [1; 2; 3]";
    let v = compile_via_loader("list-pipe-internal", src).expect("should compile");
    assert_eq!(as_int_list(v), vec![1, 2, 3]);
}

#[test]
fn require_list_mapi_adjacent_uses_a_tuple_pattern_lambda_correctly() {
    // `List.mapi-adjacent`'s OWN definition is `fun (i, acc) x leftopt
    // rightopt -> ..` — a tuple-DESTRUCTURING first parameter. This was the
    // one real grammar gap Slice 1 hit porting `list.satyg` VERBATIM
    // (`Expr::Fun`'s parameters were `Vec<VarTok>`, i.e. plain variables
    // only; `cst.rs`/`elaborate.rs` now accept a full `patbot`, reusing the
    // same pattern-currying `let-rec` already used — see `cst::ast::Expr::
    // Fun`'s doc comment). Asserting on the resulting VALUES (not just that
    // this parses) proves the destructuring actually binds `i`/`acc`
    // correctly through every fold step, not merely that parsing recovered.
    let src = "@require: list
in
List.mapi-adjacent (fun i x leftopt rightopt -> i) [10; 20; 30]";
    let v =
        compile_via_loader("list-mapi-adjacent", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![0, 1, 2]);
}

#[test]
fn require_list_map_with_ends_calls_option_is_none_across_module_boundary() {
    // `List.map-with-ends` (also `|>`-internal) calls `Option.is-none`
    // *from list.satyg's own body* — proving the nested `@require: option`
    // dependency is not just loaded but actually callable from within the
    // dependent package, not merely from the entry document.
    let src = "@require: list
in
List.map-with-ends
  (fun is-first is-last x -> if is-first then 100 else if is-last then 200 else x)
  [1; 2; 3]";
    let v = compile_via_loader("list-map-with-ends", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![100, 2, 200]);
}

// ============================================================================
// `@require: option` alone (the second, minimal Slice-1 case).
// ============================================================================

#[test]
fn require_option_alone_compiles_and_evaluates() {
    let src = "@require: option
in
Option.from 0 (Some 5)";
    let v = compile_via_loader("option-alone", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 5);
}

#[test]
fn require_option_map_and_is_none() {
    let src = "@require: option
in
if Option.is-none None
then Option.from 0 (Option.map (fun x -> x + 1) (Some 41))
else -1";
    let v = compile_via_loader("option-map", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 42);
}

#[test]
fn require_option_bind() {
    let src = "@require: option
in
Option.from 0 (Option.bind (Some 10) (fun x -> Some (x * 2)))";
    let v = compile_via_loader("option-bind", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 20);
}

// ============================================================================
// `color` built-in variant (frontend-completion.md §Slice1-B), through the
// SAME loader pipeline — no `@require:` needed (it's a built-in, seeded by
// `prim_types::builtin_variants` before any package loads), but routing it
// through `compile_via_loader` proves a document using it compiles
// end-to-end via the production load path too, not merely in isolation.
// ============================================================================

#[test]
fn a_color_value_compiles_via_the_loader() {
    // No `@require:` at all: `color` is a true built-in (seeded before any
    // package loads), so a document naming it needs no package dependency —
    // this also proves the loader/merge path tolerates a zero-dependency
    // entry document just as well as a multi-file one.
    let src = "let c = RGB (0.2, 0.4, 0.6)
in
match c with
| Gray(_)          -> 1
| RGB(r, g, b)     -> 2
| CMYK(_, _, _, _) -> 3";
    let v = compile_via_loader("color-via-loader", src).expect("should compile");
    assert_eq!(as_int(v), 2);
}

#[test]
fn a_color_value_compiles_alongside_require_list() {
    // Combined with a real `@require:` too, so the built-in variant and a
    // loaded package coexist in the same merged program.
    let src = "@require: list
in
let cs = [RGB (1.0, 0.0, 0.0); Gray 0.5; CMYK (0.0, 0.0, 0.0, 1.0)] in
List.length cs";
    let v = compile_via_loader("color-and-list", src).expect("should compile");
    assert_eq!(as_int(v), 3);
}

// ============================================================================
// `@require: color` — the FIRST bundled `.satyh` package ported verbatim
// (`lib-satysfi/dist/packages/color.satyh`, byte-for-byte from upstream). It
// wraps the built-in `Gray`/`RGB`/`CMYK` ctors in a `Color : sig … end =
// struct … end` module (`Color.rgb`/`Color.gray`/`Color.red`/…). These prove
// the package's module signature + struct typecheck against the built-in
// `color` type and evaluate through the real loader — the module system
// (already proven on `option`/`list`) applied to a `.satyh` (not `.satyg`)
// package resolved by the loader's `.satyh`-first extension search.
// ============================================================================

#[test]
fn require_color_module_constant_red_compiles() {
    // `Color.red = rgb 1. 0. 0. = RGB(1., 0., 0.)` — a module CONSTANT whose
    // body calls the module's own `rgb` helper, itself a wrapper over the
    // built-in `RGB` ctor. Matching on the result proves it evaluated to the
    // expected variant across the module boundary.
    let src = "@require: color
in
match Color.red with
| RGB(_, _, _)     -> 1
| Gray(_)          -> 2
| CMYK(_, _, _, _) -> 3";
    let v = compile_via_loader("color-red", src).expect("color.satyh should compile");
    assert_eq!(as_int(v), 1);
}

#[test]
fn require_color_module_functions_gray_and_rgb_compile() {
    // The module FUNCTIONS `Color.gray : float -> color` and
    // `Color.rgb : float -> float -> float -> color` — proving the sig's
    // arrow-typed `val`s typecheck and the struct's curried lets apply.
    let src = "@require: color
in
let g = (match Color.gray 0.5 with Gray(_) -> 1 | _ -> 0) in
let c = (match Color.rgb 0.1 0.2 0.3 with RGB(_, _, _) -> 1 | _ -> 0) in
g + c";
    let v = compile_via_loader("color-fns", src).expect("color.satyh should compile");
    assert_eq!(as_int(v), 2);
}

// ============================================================================
// `@require: pervasives` (docs/plans/stdlib-port.md) — the critical-path
// stdlib package nearly every other bundled package `@require:`s. Ported
// verbatim to `lib-satysfi/dist/packages/pervasives.satyh`. These tests
// prove it PARSES + TYPECHECKS + EVALUATES through the real loader, and
// specifically exercise the 5 primitives it needed that this port didn't
// already have (`get-natural-metrics`, `inline-frame-outer`,
// `set-manual-rising`, `script-guard`, `discretionary` — see
// primitives.rs's "pervasives.satyh unblockers" section).
// ============================================================================

#[test]
fn require_pervasives_compiles_and_evaluates_math_pi() {
    // The cheapest possible proof the WHOLE prelude typechecks: every
    // top-level `let`/`let-inline`/`type` in pervasives.satyh sits ahead of
    // `body` in the same nested-let chain, so this succeeding at all means
    // the entire file (both type synonyms, all 5 commands, every helper)
    // typechecked — not just the one binding actually evaluated below.
    let src = "@require: pervasives
in
math-pi";
    let v =
        compile_via_loader("pervasives-math-pi", src).expect("pervasives.satyh should compile");
    match v {
        Value::Float(f) => assert!(
            (f - std::f64::consts::PI).abs() < 1e-9,
            "expected math-pi ~ pi, got {f}"
        ),
        other => panic!("expected a float, got {other:?}"),
    }
}

#[test]
fn require_pervasives_no_break_and_mandatory_break_exercise_new_prims() {
    // `no-break`/`mandatory-break` are plain pervasives.satyh `let`s (not
    // commands), so they can be called directly with no text rendering
    // needed — `no-break` calls the new `inline-frame-outer`,
    // `mandatory-break` calls the new `discretionary`, and wrapping the
    // result in `get-natural-metrics` (also new) exercises all three at
    // once. `ctx`'s 400pt paragraph width makes `mandatory-break`'s
    // `inline-skip (get-text-width ctx *' 2.)` a fixed 800pt no-break box;
    // `no-break`'s zero padding leaves it unchanged, so the natural width
    // comes out exactly 800pt.
    let src = "@require: pervasives
let-inline ctx \\math m = inline-nil
in
let ctx = get-initial-context 400pt (command \\math) in
get-natural-metrics (no-break (mandatory-break ctx))";
    let v = compile_via_loader("pervasives-no-break", src)
        .expect("pervasives.satyh should compile");
    match v {
        Value::Tuple(vs) => {
            assert_eq!(vs.len(), 3);
            match &vs[0] {
                Value::Length(w) => assert_eq!(*w, Length::pt(800.0), "unexpected natural width"),
                other => panic!("expected a length, got {other:?}"),
            }
        }
        other => panic!("expected a tuple, got {other:?}"),
    }
}

#[test]
fn require_pervasives_satysfi_command_renders_via_read_inline() {
    // The `\SATySFi` logo command exercises `set-manual-rising` and
    // `script-guard` (both new), plus `no-break`/`inline-frame-outer` again
    // — through a REAL `read-inline` pass, hence the `Mono` font stub
    // (`NoFonts`'s always-`None` `advance` would reject any actual glyph).
    // The trailing `;` is real SATySFi surface syntax (`lexer.rs`'s "active
    // area"/`EndActive`): a command with no bracket arguments must be
    // explicitly terminated before non-argument text (here, the closing
    // `}`) may follow.
    let src = "@require: pervasives
let-inline ctx \\math m = inline-nil
in
read-inline (get-initial-context 400pt (command \\math)) {\\SATySFi;}";
    let v = compile_via_loader_with_metrics("pervasives-satysfi-cmd", src, &Mono)
        .expect("pervasives.satyh should compile");
    match v {
        Value::InlineBoxes(boxes) => {
            let (width, _, _) = natural_metrics(&boxes);
            assert!(
                width > Length::ZERO,
                "expected non-zero natural width, got {width:?}"
            );
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// `@require: geom` (`lib-satysfi/dist/packages/geom.satyh`, ported verbatim,
// `@require: pervasives`) — a tiny module of two `point`-synonym helpers,
// `Geom.atan2-point` and `Geom.div-perp`. Needs NO new primitives: both
// bodies are just `atan2`/`sin`/`cos`/`math-pi`/length arithmetic, all
// already present. These tests prove the nested `geom -> pervasives`
// `@require:` resolves and both functions evaluate correctly.
// ============================================================================

#[test]
fn require_geom_atan2_point_compiles_and_evaluates() {
    // (0pt,0pt) -> (0pt,1pt): dy=1pt, dx=0pt, atan2(1,0) = pi/2.
    let src = "@require: geom
in
Geom.atan2-point (0pt, 0pt) (0pt, 1pt)";
    let v = compile_via_loader("geom-atan2-point", src).expect("geom.satyh should compile");
    match v {
        Value::Float(f) => assert!(
            (f - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "expected pi/2, got {f}"
        ),
        other => panic!("expected a float, got {other:?}"),
    }
}

#[test]
fn require_geom_div_perp_compiles_and_evaluates() {
    // (0pt,0pt) -> (1pt,0pt), t=0.5, len=10pt: midpoint (0.5pt,0pt) offset
    // perpendicular (theta = atan2(0,1) + pi/2 = pi/2) by 10pt along
    // (cos, sin) = (0,1), giving (0.5pt, 10pt).
    let src = "@require: geom
in
Geom.div-perp (0pt, 0pt) (1pt, 0pt) 0.5 10pt";
    let v = compile_via_loader("geom-div-perp", src).expect("geom.satyh should compile");
    match v {
        Value::Tuple(vs) => {
            assert_eq!(vs.len(), 2);
            match (&vs[0], &vs[1]) {
                (Value::Length(x), Value::Length(y)) => {
                    assert!((x.0 - 0.5).abs() < 1e-6, "expected x ~ 0.5pt, got {x:?}");
                    assert!((y.0 - 10.0).abs() < 1e-6, "expected y ~ 10pt, got {y:?}");
                }
                other => panic!("expected two lengths, got {other:?}"),
            }
        }
        other => panic!("expected a tuple, got {other:?}"),
    }
}

// ============================================================================
// `@require: gr` (`lib-satysfi/dist/packages/gr.satyh`, ported verbatim;
// `@require: pervasives`/`geom`/`list`) — the graphics hub package, and the
// point of the whole graphics-roadmap prim additions
// (`docs/plans/graphics-subsystem.md` §Full roadmap A/B/C/D): `bezier-to`,
// `close-with-bezier`, `shift-path`, `linear-transform-path`,
// `shift-graphics`, `linear-transform-graphics`, `get-graphics-bbox`,
// `dashed-stroke` (all FAITHFUL), and `draw-text` (a documented STAND-IN,
// see `GraphicsElem::Text`'s doc comment). These tests prove the triple
// nested `@require:` resolves and exercise every new prim through REAL
// `Gr.*` module code (not bare primitive calls), except `draw-text`/
// `get-graphics-bbox`, which `gr.satyh` only ever composes inside
// `Gr.text-centering`/`-leftward` — covered separately, below, via bare
// primitive calls (the `Gr.*` functions that use them are never invoked by
// any bundled package in a no-real-text way, so a direct call is the
// faithful way to exercise them here).
// ============================================================================

#[test]
fn require_gr_rectangle_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // The cheapest whole-module proof (same rationale as `math-pi`
        // above): this forces evaluating every `let` in `Gr`'s struct body
        // into a closure, so success here means the entire file
        // typechecked, not just `rectangle`. `close-with-line` on 3
        // `line-to`s from `start-path` yields one closed subpath.
        let src = "@require: gr
in
match Gr.rectangle (0pt, 0pt) (10pt, 10pt) with
| _ -> 1";
        let v = compile_via_loader("gr-rectangle", src).expect("gr.satyh should compile");
        assert_eq!(as_int(v), 1);
    });
}

#[test]
fn require_gr_circle_exercises_bezier_to_and_close_with_bezier() {
    run_with_big_stack(|| {
        // `Gr.circle` is 3 `bezier-to`s from `start-path` then a
        // `close-with-bezier` — the ONLY bundled use of either prim. One
        // subpath, 3 `PathSeg::Bezier` segments, `Closing::Bezier`.
        let src = "@require: gr
in
Gr.circle (50pt, 50pt) 10pt";
        let v = compile_via_loader("gr-circle", src).expect("gr.satyh should compile");
        match v {
            Value::Path(p) => {
                assert_eq!(p.subpaths.len(), 1, "expected one subpath");
                let sub = &p.subpaths[0];
                assert_eq!(sub.segs.len(), 3, "expected 3 bezier-to segments");
                for seg in &sub.segs {
                    assert!(
                        matches!(seg, PathSeg::Bezier(..)),
                        "expected a Bezier segment, got {seg:?}"
                    );
                }
                assert!(
                    matches!(sub.closing, Closing::Bezier(..)),
                    "expected Closing::Bezier, got {:?}",
                    sub.closing
                );
            }
            other => panic!("expected a path, got {other:?}"),
        }
    });
}

#[test]
fn require_gr_scale_path_exercises_shift_and_linear_transform_path() {
    run_with_big_stack(|| {
        // `Gr.scale-path` = `shift-path (-center) |> linear-transform-path
        // (scalex, 0, 0, scaley) |> shift-path center` — the ONLY bundled
        // use of `shift-path`/`linear-transform-path`. With `center` =
        // the origin, both shifts are no-ops, isolating the matrix math:
        // `(x, y) |-> (2x, 3y)`.
        let src = "@require: gr
in
Gr.scale-path (0pt, 0pt) 2. 3. (Gr.line (1pt, 1pt) (2pt, 2pt))";
        let v = compile_via_loader("gr-scale-path", src).expect("gr.satyh should compile");
        match v {
            Value::Path(p) => {
                assert_eq!(p.subpaths.len(), 1);
                let sub = &p.subpaths[0];
                assert!(
                    (sub.start.0 .0 - 2.0).abs() < 1e-6 && (sub.start.1 .0 - 3.0).abs() < 1e-6,
                    "expected start (2, 3), got {:?}",
                    sub.start
                );
                assert_eq!(sub.segs.len(), 1);
                match sub.segs[0] {
                    PathSeg::Line(pt) => {
                        assert!(
                            (pt.0 .0 - 4.0).abs() < 1e-6 && (pt.1 .0 - 6.0).abs() < 1e-6,
                            "expected end (4, 6), got {pt:?}"
                        );
                    }
                    other => panic!("expected a Line segment, got {other:?}"),
                }
                assert!(matches!(sub.closing, Closing::Open));
            }
            other => panic!("expected a path, got {other:?}"),
        }
    });
}

#[test]
fn require_gr_dashed_arrow_exercises_dashed_stroke() {
    run_with_big_stack(|| {
        // `Gr.dashed-arrow thkns dash = arrow-scheme (dashed-stroke thkns
        // dash)` — the ONLY bundled use of `dashed-stroke`. Returns
        // `[stroke-shaft; fill-head]`; the first element is the
        // dashed-stroked shaft.
        let src = "@require: gr
in
Gr.dashed-arrow 1pt (2pt, 2pt, 0pt) (Gray(0.)) 5pt 3pt 2pt (0pt, 0pt) (10pt, 0pt)";
        let v = compile_via_loader("gr-dashed-arrow", src).expect("gr.satyh should compile");
        match v {
            Value::List(items) => {
                assert_eq!(items.len(), 2, "expected [stroke; fill]");
                match &items[0] {
                    Value::Graphics(GraphicsElem::DashedStroke(w, dash, _, _)) => {
                        assert!((w.0 - 1.0).abs() < 1e-9, "width, got {w:?}");
                        assert!(
                            (dash.0 .0 - 2.0).abs() < 1e-9
                                && (dash.1 .0 - 2.0).abs() < 1e-9
                                && (dash.2 .0 - 0.0).abs() < 1e-9,
                            "dash pattern, got {dash:?}"
                        );
                    }
                    other => panic!("expected a DashedStroke graphics, got {other:?}"),
                }
                match &items[1] {
                    Value::Graphics(GraphicsElem::Fill(..)) => {}
                    other => panic!("expected a Fill graphics, got {other:?}"),
                }
            }
            other => panic!("expected a graphics list, got {other:?}"),
        }
    });
}

#[test]
fn require_get_graphics_bbox_is_faithful_on_a_fill() {
    run_with_big_stack(|| {
        // `get-graphics-bbox`/`draw-text` are only ever composed inside
        // `Gr.text-centering`/`-leftward` (both built on the `draw-text`
        // STAND-IN) — exercise the FAITHFUL `Fill`/`Stroke` bbox path
        // directly instead: a rectangle's bbox is exactly its own corners.
        let src = "@require: gr
in
get-graphics-bbox (fill (Gray(0.)) (Gr.rectangle (0pt, 0pt) (10pt, 20pt)))";
        let v = compile_via_loader("gr-bbox-fill", src).expect("gr.satyh should compile");
        match v {
            Value::Tuple(vs) if vs.len() == 2 => {
                let (x0, y0) = as_point_f64(&vs[0]);
                let (x1, y1) = as_point_f64(&vs[1]);
                assert!(
                    (x0 - 0.0).abs() < 1e-6
                        && (y0 - 0.0).abs() < 1e-6
                        && (x1 - 10.0).abs() < 1e-6
                        && (y1 - 20.0).abs() < 1e-6,
                    "expected bbox (0,0)-(10,20), got ({x0},{y0})-({x1},{y1})"
                );
            }
            other => panic!("expected a (point*point) tuple, got {other:?}"),
        }
    });
}

#[test]
fn require_draw_text_stand_in_composes_with_shift_and_bbox() {
    run_with_big_stack(|| {
        // `draw-text` is a STAND-IN (`GraphicsElem::Text`, anchor point
        // only) — prove it still composes correctly with the FAITHFUL
        // `shift-graphics`/`get-graphics-bbox` (exactly how `Gr.text-
        // centering`/`-leftward` use them), with no real font/line-break
        // pass needed: shifting a `Text(1, 2)` by `(5, 5)` then taking its
        // bbox gives the single point `(6, 7)`.
        let src = "@require: gr
in
get-graphics-bbox (shift-graphics (5pt, 5pt) (draw-text (1pt, 2pt) inline-nil))";
        let v =
            compile_via_loader("gr-draw-text-standin", src).expect("gr.satyh should compile");
        match v {
            Value::Tuple(vs) if vs.len() == 2 => {
                let (x0, y0) = as_point_f64(&vs[0]);
                let (x1, y1) = as_point_f64(&vs[1]);
                assert!(
                    (x0 - 6.0).abs() < 1e-6
                        && (y0 - 7.0).abs() < 1e-6
                        && (x1 - 6.0).abs() < 1e-6
                        && (y1 - 7.0).abs() < 1e-6,
                    "expected bbox (6,7)-(6,7), got ({x0},{y0})-({x1},{y1})"
                );
            }
            other => panic!("expected a (point*point) tuple, got {other:?}"),
        }
    });
}

// ============================================================================
// Tier-2 decoration/graphics packages (docs/plans/stdlib-port.md):
// `deco`/`hdecoset`/`vdecoset`/`picture` — all copied verbatim except
// `picture.satyh`'s one unparseable sig line (bracket command-type syntax;
// see that file's own comment). `cd.satyh` is NOT ported (needs optional
// function parameters — `elaborate.rs`'s `app_arg_to_ast` still rejects
// `?:`/`?*` as "not supported yet (phase 3)").
// ============================================================================

#[test]
fn require_deco_simple_frame_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `Deco.simple-frame` is `deco.satyh`'s only non-trivial export:
        // `[fill fcolor path; stroke t scolor path]` once applied to a
        // placement — forces the whole (tiny) module to evaluate.
        let src = "@require: deco
in
Deco.simple-frame 1pt (Gray(0.)) (Gray(1.)) (0pt, 0pt) 10pt 5pt 2pt";
        let v = compile_via_loader("deco-simple-frame", src).expect("deco.satyh should compile");
        match v {
            Value::List(items) => {
                assert_eq!(items.len(), 2, "expected [fill; stroke]");
                assert!(matches!(&items[0], Value::Graphics(GraphicsElem::Fill(..))));
                assert!(matches!(&items[1], Value::Graphics(GraphicsElem::Stroke(..))));
            }
            other => panic!("expected a graphics list, got {other:?}"),
        }
    });
}

#[test]
fn require_hdecoset_simple_frame_stroke_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `HDecoSet.simple-frame-stroke` returns a `deco-set` (a plain
        // 4-tuple of `deco`s, `(decoS, decoH, decoM, decoT)`) — destructure
        // it and apply the first component to prove the whole module
        // (including its `Gr.poly-line`/`Gr.line` calls) evaluates.
        let src = "@require: hdecoset
in
let (decoS, _, _, _) = HDecoSet.simple-frame-stroke 1pt (Gray(0.)) in
decoS (0pt, 0pt) 10pt 5pt 2pt";
        let v = compile_via_loader("hdecoset-simple-frame-stroke", src)
            .expect("hdecoset.satyh should compile");
        match v {
            Value::List(items) => {
                assert_eq!(items.len(), 1, "expected [stroke]");
                assert!(matches!(&items[0], Value::Graphics(GraphicsElem::Stroke(..))));
            }
            other => panic!("expected a graphics list, got {other:?}"),
        }
    });
}

#[test]
fn require_vdecoset_simple_frame_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `VDecoSet.simple-frame` (fill background + stroke border) is the
        // richest of `vdecoset.satyh`'s deco-sets short of `paper`/
        // `quote-round`; its `decoS` component yields `[fill; stroke]`.
        let src = "@require: vdecoset
in
let (decoS, _, _, _) = VDecoSet.simple-frame 1pt (Gray(0.)) (Gray(1.)) in
decoS (0pt, 0pt) 10pt 5pt 2pt";
        let v = compile_via_loader("vdecoset-simple-frame", src)
            .expect("vdecoset.satyh should compile");
        match v {
            Value::List(items) => {
                assert_eq!(items.len(), 2, "expected [fill-back; stroke-border]");
                assert!(matches!(&items[0], Value::Graphics(GraphicsElem::Fill(..))));
                assert!(matches!(&items[1], Value::Graphics(GraphicsElem::Stroke(..))));
            }
            other => panic!("expected a graphics list, got {other:?}"),
        }
    });
}

#[test]
fn require_picture_draw_line_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `Picture.draw-line` runs `draw-line-scheme`'s full bbox-vs-slope
        // geometry (the meatiest part of `picture.satyh`) between two
        // synthetic `node`s (`point * graphics`, built directly with the
        // `draw-text` STAND-IN so no real font metrics are needed — same
        // trick `require_draw_text_stand_in_...` above uses) and renders
        // one stroked, open, single-segment connecting line.
        let src = "@require: picture
in
Picture.draw-line 1pt (Gray(0.))
  ((0pt, 0pt), draw-text (0pt, 0pt) inline-nil)
  ((10pt, 10pt), draw-text (10pt, 10pt) inline-nil)";
        let v =
            compile_via_loader("picture-draw-line", src).expect("picture.satyh should compile");
        match v {
            Value::List(items) => {
                assert_eq!(items.len(), 1, "expected [stroke]");
                match &items[0] {
                    Value::Graphics(GraphicsElem::Stroke(w, _color, path)) => {
                        assert!((w.0 - 1.0).abs() < 1e-9, "thickness, got {w:?}");
                        assert_eq!(path.subpaths.len(), 1);
                        let sub = &path.subpaths[0];
                        assert_eq!(sub.segs.len(), 1, "expected one line-to segment");
                        assert!(matches!(sub.segs[0], PathSeg::Line(..)));
                        assert!(matches!(sub.closing, Closing::Open));
                    }
                    other => panic!("expected a Stroke graphics, got {other:?}"),
                }
            }
            other => panic!("expected a graphics list, got {other:?}"),
        }
    });
}

// ============================================================================
// `tabular` (docs/plans/table-subsystem.md) — newly unblocked by the
// `tabular` primitive/`cell` type landing: `tabular.satyh`'s own module
// wrapper is entirely commented out upstream (dead code, `%`-prefixed), so
// it just defines a bare `\tabular` inline command directly, positionally
// (`lstf cellf multif empty`, then `rulef`) — no record/optional-arg
// surface syntax at all. Renders real cell text, hence `Mono` (not
// `NoFonts`).
//
// `tabularx.satyh`/`table.satyh` were previously blocked by the same
// record-types-in-`TypeExpr` gap (see prior revision of this comment); now
// that `cst.rs`'s `TypeAtom::Record` accepts `(| l : ty; … |)` in ordinary
// type position (`record_types.rs`), both port verbatim below.
// ============================================================================

#[test]
fn require_tabularx_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `Tabularx.\tabular`'s `lstf` callback receives `cellf`/`multif`/
        // `empty`, where `cellf : cell-record option -> inline-text -> cell`
        // — a `None` alignment override exercises `make-alignments`'s
        // `Option.from` default-record path. The 2nd bracket arg (`rulef`)
        // completes the underlying `tabular` primitive's partial
        // application (the struct's `\tabular` body binds only `lstf`, one
        // fewer param than the sig's 2-element cmd type — sig checking is a
        // no-op, and `tabular (lstf cellf multif empty)` is itself a
        // `Value::Prim` left partially applied, completed by the 2nd
        // bracket arg at the call site).
        let src = "@require: tabularx
let-inline ctx \\math m = inline-nil
in
let rule xs ys = [] in
read-inline (get-initial-context 400pt (command \\math))
  {\\tabular(fun cellf multif empty -> [[cellf None {A}; cellf None {B}]; [cellf None {C}; cellf None {D}]])(rule);}";
        let v = compile_via_loader_with_metrics("tabularx-basic", src, &Mono)
            .expect("tabularx.satyh should compile");
        match v {
            Value::InlineBoxes(boxes) => {
                assert_eq!(boxes.len(), 1);
                assert!(
                    matches!(&boxes[0], HorzBox::Pure(PureHorzBox::Tabular(_))),
                    "expected a Tabular pure box, got {:?}",
                    boxes[0]
                );
            }
            other => panic!("expected inline-boxes, got {other:?}"),
        }
    });
}

// ============================================================================
// `progsynt.satyh` is NOT ported: it defines a custom infix operator as an
// ordinary name, `let (-->) t1 t2 = ..` (sig: `val (-->) : t -> t -> t`) —
// confirmed via a minimal isolated repro (a bare `module .. : sig val (-->)
// .. end = struct let (-->) .. end` fails to parse at the `module` keyword
// itself; the identical module WITHOUT the `(-->)` binding parses fine).
// `cst.rs`'s `TopLet`/`SigItem::Val` both take a plain `VarTok` name — there
// is no grammar production accepting a parenthesized operator symbol
// (`(-->)`,`(+++>)`, `(++)`, ...) as a bindable name, only bare identifiers.
// This is a lexer/grammar gap (`cst.rs`/`leaf.rs`/`lexer.rs`), out of this
// wave's file boundary (no `.rs` edits permitted) — every other primitive
// `progsynt.satyh` needs (`math-color`, `math-char-class`, `text-in-math`,
// `math-group`, `get-font-size`, abstract `type t` in a sig, optional args)
// already exists/parses (verified in isolation before narrowing to this).
// ============================================================================

#[test]
fn require_table_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `Table.\tabular`'s `cellssf` callback receives a *record VALUE*
        // (fields `l`/`r`/`c`/`m`/`e`) whose sig type is a record `(| l :
        // ..; r : ..; c : ..; m : ..; e : .. |)` sitting inside the arrow
        // chain of the sig's first cmd-type element — the other half of the
        // same record-types-in-`TypeExpr` gap `tabularx` exercises (there as
        // a `type` synonym body, here as a bare inline record type).
        let src = "@require: table
let-inline ctx \\math m = inline-nil
in
let rule xs ys = [] in
read-inline (get-initial-context 400pt (command \\math))
  {\\tabular(fun r -> [[r#l{A}; r#r{B}]; [r#c{C}; r#e]])(rule);}";
        let v = compile_via_loader_with_metrics("table-basic", src, &Mono)
            .expect("table.satyh should compile");
        match v {
            Value::InlineBoxes(boxes) => {
                assert_eq!(boxes.len(), 1);
                assert!(
                    matches!(&boxes[0], HorzBox::Pure(PureHorzBox::Tabular(_))),
                    "expected a Tabular pure box, got {:?}",
                    boxes[0]
                );
            }
            other => panic!("expected inline-boxes, got {other:?}"),
        }
    });
}

#[test]
fn require_tabular_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: tabular
@require: list
let-inline ctx \\math m = inline-nil
in
let rule xs ys = [] in
read-inline (get-initial-context 400pt (command \\math))
  {\\tabular(fun c m e -> [[c{A}; c{B}]; [c{C}; c{D}]])(rule);}";
        let v = compile_via_loader_with_metrics("tabular-basic", src, &Mono)
            .expect("tabular.satyh should compile");
        match v {
            Value::InlineBoxes(boxes) => {
                assert_eq!(boxes.len(), 1);
                assert!(
                    matches!(&boxes[0], HorzBox::Pure(PureHorzBox::Tabular(_))),
                    "expected a Tabular pure box, got {:?}",
                    boxes[0]
                );
            }
            other => panic!("expected inline-boxes, got {other:?}"),
        }
    });
}

// `code` (docs/plans/context-box-prims.md) — unblocked by the
// context-box-prims batch (`set-text-color`, `split-into-lines`,
// `block-frame-breakable`, `set-code-text-command`, `get-natural-length`).
// stdja.satyh `@require`s this directly, so it's the priority package.

#[test]
fn require_code_module_scheme_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: code
let-inline ctx \\math m = inline-nil
in
read-block (get-initial-context 400pt (command \\math)) '<+code(`let x = 1`);>";
        let v = compile_via_loader_with_metrics("code-basic", src, &Mono)
            .expect("code.satyh should compile");
        match v {
            Value::BlockBoxes(boxes) => {
                assert!(!boxes.is_empty(), "expected non-empty block-boxes");
            }
            other => panic!("expected block-boxes, got {other:?}"),
        }
    });
}

// ============================================================================
// `annot` (`lib-satysfi/dist/packages/annot.satyh`, ported verbatim) — every
// primitive it needs (`register-link-to-uri`, `register-link-to-location`,
// `register-destination`, `get-leftmost-script`, `get-rightmost-script`,
// `inline-frame-breakable`) already exists; `register-location-frame` is a
// plain stdlib fn `Annot` defines itself. `@require`s `pervasives`, `color`,
// `gr`, `option` — none define `\math`, so a local stub stands in for
// `get-initial-context`'s 2nd argument, exactly like the `code`/`table`/
// `tabular`/`tabularx` fixtures above.
// ============================================================================

#[test]
fn require_annot_href_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: annot
let-inline ctx \\math m = inline-nil
in
read-inline (get-initial-context 400pt (command \\math))
  {\\href?*(`https://example.com`){link text}}";
        let v = compile_via_loader_with_metrics("annot-href", src, &Mono)
            .expect("annot.satyh should compile");
        match v {
            Value::InlineBoxes(boxes) => {
                assert!(!boxes.is_empty(), "expected non-empty inline-boxes");
            }
            other => panic!("expected inline-boxes, got {other:?}"),
        }
    });
}

// ============================================================================
// `stdja` (`lib-satysfi/dist/packages/stdja.satyh`, ported verbatim) — the
// CAPSTONE: the real upstream document class, `@require`-ing pervasives, gr,
// list, math, code, color, option, annot. Reaching this proves every one of
// `StdJa`'s bindings (`document`, `+p`/`+pn`/`+section`/`+subsection`,
// `\ref`/`\ref-page`/`\figure`/`\emph`, the title/header/footer/page-break
// scheme) parses, typechecks, and evaluates through the production loader.
//
// `Mono` (above) rejects non-ASCII, but the footer stdja.satyh always
// builds (`— #pageno; —`, an em dash) is non-ASCII, so this test uses its
// own `Wide` stub that answers every character — this milestone still has
// no real CJK/Latin font metrics (`docs/plans/text-rendering.md`), so CJK
// text is never actually exercised here (`show-toc = false` avoids the
// `目次` table-of-contents heading; the body is plain Latin), but the em
// dash alone would otherwise trip the same "not in WinAnsi" guard `Mono`
// exists to exercise elsewhere.
// ============================================================================

struct Wide;

impl FontMetrics for Wide {
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

#[test]
fn require_stdja_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: stdja
in
document (|
  title = {Milestone Capstone};
  author = {SATySFi in Rust};
  show-toc = false;
  show-title = true;
|) ?* '<
  +p {
    Hello, world! This is a Latin paragraph long enough that greedy line
    breaking must wrap it onto more than one line, exercising the real
    line-break and page-break path end to end through StdJa.document.
  }
>";
        let v = compile_via_loader_with_metrics("stdja-document", src, &Wide)
            .expect("stdja.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page (title/body/footer content)"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}
