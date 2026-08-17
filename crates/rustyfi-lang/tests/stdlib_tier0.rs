//! Slice 1 / Tier 0 stdlib port proof (`docs/plans/stdlib-port.md` §Slice 1):
//! `@require: list` (hence, transitively, `@require: option`) — and
//! `@require: option` alone — must PARSE, ELABORATE, TYPECHECK, and EVALUATE
//! through the real multi-file loader with this repo's `lib-rustyfi/` as
//! `lib_root`. This mirrors `rustyfi-cli`'s own production pipeline
//! (`main.rs`'s `cmd_compile`: `rustyfi_loader::load` -> merge preludes ->
//! `compile_document_cst`) rather than a bespoke single-file harness, so it
//! genuinely exercises `@require:` resolution (including the NESTED
//! `list.satyg -> @require: option` edge) through the production loader
//! crate — not just a hand-rolled shortcut.
//!
//! `option.satyg`/`list.satyg` under `lib-rustyfi/dist/packages/` are copied
//! byte-for-byte from upstream (the plan's "copy-verbatim" policy) — this
//! test is the proof the compiler now *accepts* them (the Slice-1
//! acceptance bar is "compiles", i.e. evaluates to a value, not merely
//! "parses" — see the plan's Verification table).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{
    natural_metrics, Closing, FontKey, FontMetrics, GraphicsElem, HorzBox, Length, PathSeg,
    PureHorzBox,
};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck};
use rustyfi_loader::{LoadOptions, LoadedProgram};

/// This repo's `lib-rustyfi/` (the real Tier-0 packages' home), resolved
/// relative to this crate's own manifest directory — the same convention
/// `compile.rs`'s private `prepare_document` test helper already uses for
/// `stdja-mini.satyh`.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// A uniquely-named temp `.saty` file, cleaned up on drop — scaled down from
/// `rustyfi-loader/tests/loader.rs`'s `TempDir` fixture pattern to the one
/// entry file each test here needs (the packages themselves already live on
/// disk under `lib_root()`, so there is no fixture tree to build).
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-stdlib-tier0-{tag}-{}-{}.saty",
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
/// `cst::File`, exactly like `rustyfi-cli`'s private `merge_program`
/// (`main.rs`): the loader guarantees dependency-first order with the entry
/// document last, so every library's prelude is spliced ahead of the
/// entry's own, in that order (the v0.0.6 analog typechecks each library
/// into a shared environment in dependency order; untyped elaboration gets
/// the same scoping by prelude concatenation).
fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        rustyfi_loader::LoadedCst::V0_0_6(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's merge_program is the V0_0_6-only path")
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

/// Run `f` (a self-contained compile-and-assert closure) on a thread with a
/// generously large stack. `gr.satyh` (205 lines — considerably bigger than
/// any package loaded so far) needs more depth than the default stack
/// allows through syan's recursive-descent parser, the same reason
/// `rustyfi-syntax/tests/roundtrip.rs`'s deep-nesting test spawns a
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
    let v = compile_via_loader("list-mapi-adjacent", src).expect("list.satyg should compile");
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
// (`lib-rustyfi/dist/packages/color.satyh`, byte-for-byte from upstream). It
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
// verbatim to `lib-rustyfi/dist/packages/pervasives.satyh`. These tests
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
    let v = compile_via_loader("pervasives-math-pi", src).expect("pervasives.satyh should compile");
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
    let v =
        compile_via_loader("pervasives-no-break", src).expect("pervasives.satyh should compile");
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
fn require_pervasives_rustyfi_command_renders_via_read_inline() {
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
    let v = compile_via_loader_with_metrics("pervasives-rustyfi-cmd", src, &Mono)
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
// `@require: geom` (`lib-rustyfi/dist/packages/geom.satyh`, ported verbatim,
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
// `@require: gr` (`lib-rustyfi/dist/packages/gr.satyh`, ported verbatim;
// `@require: pervasives`/`geom`/`list`) — the graphics hub package, and the
// point of the whole graphics-roadmap prim additions
// (`docs/plans/graphics-subsystem.md` §Full roadmap A/B/C/D): `bezier-to`,
// `close-with-bezier`, `shift-path`, `linear-transform-path`,
// `shift-graphics`, `linear-transform-graphics`, `get-graphics-bbox`,
// `dashed-stroke`, and `draw-text` (all FAITHFUL — `draw-text` as of roadmap
// C1, see `GraphicsElem::Text`'s doc comment). These tests prove the triple
// nested `@require:` resolves and exercise every new prim through REAL
// `Gr.*` module code (not bare primitive calls), except `draw-text`/
// `get-graphics-bbox`, which `gr.satyh` only ever composes inside
// `Gr.text-centering`/`-leftward` — covered separately, below, via bare
// primitive calls and (for `Gr.text-centering` itself) a direct `Gr.*` call
// with real-width content (`inline-skip`, no font metrics needed).
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
        // Exercise the `Fill`/`Stroke` bbox path directly: a rectangle's
        // bbox is exactly its own corners. (`draw-text`'s own bbox is
        // covered separately below, now that it's FAITHFUL — roadmap C1 —
        // and `Gr.text-centering`/`-leftward` are exercised via
        // `require_gr_text_centering_centers_on_the_runs_real_width`.)
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
fn require_draw_text_composes_with_shift_and_bbox_on_an_empty_run() {
    run_with_big_stack(|| {
        // `draw-text` is now FAITHFUL (`GraphicsElem::Text`, roadmap C1) —
        // prove it still composes correctly with `shift-graphics`/
        // `get-graphics-bbox` (exactly how `Gr.text-centering`/`-leftward`
        // use them), with no real font/line-break pass needed: an EMPTY run
        // (`inline-nil`) has zero `natural_metrics`, so shifting a
        // `Text{pt: (1, 2), width: 0, height: 0, depth: 0}` by `(5, 5)` then
        // taking its bbox still gives the single point `(6, 7)` — the same
        // assertion the pre-C1 stand-in proved, now for the right reason
        // (zero-size run, not "content dropped").
        let src = "@require: gr
in
get-graphics-bbox (shift-graphics (5pt, 5pt) (draw-text (1pt, 2pt) inline-nil))";
        let v = compile_via_loader("gr-draw-text-empty-run", src).expect("gr.satyh should compile");
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

#[test]
fn draw_text_bbox_is_the_runs_real_natural_width() {
    run_with_big_stack(|| {
        // `draw-text (0pt,0pt) (inline-skip 5pt)`: a `FixedEmpty{5pt}` box
        // needs no font metrics (no glyphs), so `NoFonts` suffices, and its
        // `natural_metrics` are exactly `(5pt, 0pt, 0pt)` — real width, not
        // the pre-C1 stand-in's always-zero-size box.
        let src = "get-graphics-bbox (draw-text (0pt, 0pt) (inline-skip 5pt))";
        let v = compile_via_loader("draw-text-real-width", src).expect("should compile");
        match v {
            Value::Tuple(vs) if vs.len() == 2 => {
                let (x0, y0) = as_point_f64(&vs[0]);
                let (x1, y1) = as_point_f64(&vs[1]);
                assert!(
                    (x0 - 0.0).abs() < 1e-6
                        && (y0 - 0.0).abs() < 1e-6
                        && (x1 - 5.0).abs() < 1e-6
                        && (y1 - 0.0).abs() < 1e-6,
                    "expected bbox (0,0)-(5,0), got ({x0},{y0})-({x1},{y1})"
                );
            }
            other => panic!("expected a (point*point) tuple, got {other:?}"),
        }
    });
}

#[test]
fn require_gr_text_centering_centers_on_the_runs_real_width() {
    run_with_big_stack(|| {
        // The `gr.satyh` consumer path this whole roadmap item exists for
        // (C1's design summary): `Gr.text-centering` needs `get-graphics-
        // bbox` to report a REAL width for a `draw-text` run — previously
        // meaningless under the always-zero-size stand-in. `inline-skip 5pt`
        // needs no font metrics either.
        let src = "@require: gr
in
Gr.text-centering (0pt, 0pt) (inline-skip 5pt)";
        let v = compile_via_loader("gr-text-centering-real-width", src)
            .expect("gr.satyh should compile");
        match v {
            Value::Graphics(g) => {
                let (pmin, pmax) = rustyfi_backend::graphics_bbox(&g).expect("nonempty graphics");
                assert!(
                    (pmin.0 .0 - (-2.5)).abs() < 1e-6
                        && (pmin.1 .0 - 0.0).abs() < 1e-6
                        && (pmax.0 .0 - 2.5).abs() < 1e-6
                        && (pmax.1 .0 - 0.0).abs() < 1e-6,
                    "expected bbox (-2.5,0)-(2.5,0), got {pmin:?}-{pmax:?}"
                );
            }
            other => panic!("expected a Graphics value, got {other:?}"),
        }
    });
}

#[test]
fn get_path_bbox_of_a_rectangle_is_its_own_corners() {
    run_with_big_stack(|| {
        let src = "@require: gr
in
get-path-bbox (Gr.rectangle (0pt, 0pt) (10pt, 20pt))";
        let v =
            compile_via_loader("get-path-bbox-rectangle", src).expect("gr.satyh should compile");
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
fn get_path_bbox_of_a_circle_is_exact_not_the_control_hull() {
    run_with_big_stack(|| {
        // C3b: `Gr.circle` is 3 `bezier-to`s + `close-with-bezier`
        // (`k = 0.55228`), the same shape `require_gr_circle_exercises_
        // bezier_to_and_close_with_bezier` above pins structurally. The
        // exact cubic-extrema bbox is the circle's own tight bounds
        // `((40,40),(60,60))`; the (pre-C3b) control-point hull would
        // overshoot the diagonal corners by `10 * 0.55228 ≈ 5.52pt`.
        let src = "@require: gr
in
get-path-bbox (Gr.circle (50pt, 50pt) 10pt)";
        let v = compile_via_loader("get-path-bbox-circle", src).expect("gr.satyh should compile");
        match v {
            Value::Tuple(vs) if vs.len() == 2 => {
                let (x0, y0) = as_point_f64(&vs[0]);
                let (x1, y1) = as_point_f64(&vs[1]);
                assert!(
                    (x0 - 40.0).abs() < 1e-3
                        && (y0 - 40.0).abs() < 1e-3
                        && (x1 - 60.0).abs() < 1e-3
                        && (y1 - 60.0).abs() < 1e-3,
                    "expected exact bbox (40,40)-(60,60), got ({x0},{y0})-({x1},{y1})"
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
                assert!(matches!(
                    &items[1],
                    Value::Graphics(GraphicsElem::Stroke(..))
                ));
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
                assert!(matches!(
                    &items[0],
                    Value::Graphics(GraphicsElem::Stroke(..))
                ));
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
                assert!(matches!(
                    &items[1],
                    Value::Graphics(GraphicsElem::Stroke(..))
                ));
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
        // synthetic `node`s (`point * graphics`, built directly from
        // `draw-text (.., inline-nil)` — an EMPTY run, so no real font
        // metrics are needed, same trick
        // `require_draw_text_composes_with_shift_and_bbox_on_an_empty_run`
        // above uses) and renders one stroked, open, single-segment
        // connecting line.
        let src = "@require: picture
in
Picture.draw-line 1pt (Gray(0.))
  ((0pt, 0pt), draw-text (0pt, 0pt) inline-nil)
  ((10pt, 10pt), draw-text (10pt, 10pt) inline-nil)";
        let v = compile_via_loader("picture-draw-line", src).expect("picture.satyh should compile");
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
// `annot` (`lib-rustyfi/dist/packages/annot.satyh`, ported verbatim) — every
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
// `stdja` (`lib-rustyfi/dist/packages/stdja.satyh`, ported verbatim) — the
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

// ============================================================================
// `footnote-scheme` (`lib-rustyfi/dist/packages/footnote-scheme.satyh`,
// ported verbatim; `@require: color gr`) — every primitive it needs
// (`register-cross-reference`/`get-cross-reference`/`hook-page-break`/
// `no-break`/`add-footnote`, all already proven above or in
// `hooks_crossref.rs`) already exists. `FootnoteScheme.main` is exercised
// directly (its `sig` exposes only plain `val`s, no `direct` command) with
// trivial `ibf`/`bbf` callbacks. `add-footnote` is now FAITHFUL (wraps the
// block in a zero-metric `PureHorzBox::Footnote` marker,
// docs/plans/document-page-model.md §C); `get-natural-metrics` measures the
// marker's OWN zero width/height/depth (it never re-enters the wrapped
// block), so the asserted metrics are unaffected by that change — this
// still only proves the module compiles/evaluates end to end, not that the
// footnote text lands on the page (that needs a real `chop_page` run,
// covered by `crates/rustyfi-cli/tests/e2e.rs`'s footnote fixture).
// ============================================================================

#[test]
fn require_footnote_scheme_main_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: footnote-scheme
let-inline ctx \\math m = inline-nil
in
let ctx = get-initial-context 400pt (command \\math) in
let () = FootnoteScheme.initialize () in
let () = FootnoteScheme.start-page () in
let ibf num = inline-nil in
let bbf num = block-nil in
get-natural-metrics (FootnoteScheme.main ctx ibf bbf)";
        let v = compile_via_loader("footnote-scheme-main", src)
            .expect("footnote-scheme.satyh should compile");
        match v {
            Value::Tuple(vs) => assert_eq!(vs.len(), 3, "expected (width, height, depth)"),
            other => panic!("expected a tuple, got {other:?}"),
        }
    });
}

// ============================================================================
// `proof` (`lib-rustyfi/dist/packages/proof.satyh`, ported verbatim;
// `@require: gr`) — its only two exports (`\derive`/`\derive-multi`) are
// `direct`, `math-cmd`-typed bindings; both bodies are plain closures
// (`let-math ... = derive nameopt bopt ... mlst m`, itself a curried plain
// `let`) that are never forced to evaluate their insides just by loading the
// module — same "cheapest whole-module proof" rationale as
// `require_gr_rectangle_compiles_and_evaluates`/`require_math_compiles_and_
// evaluates` above: success here means the WHOLE file (sig match included)
// typechecked, not merely parsed. Actually invoking `\derive` needs a real
// `${…}`-embedded user math-cmd call under a document context — the
// production `EmbedMath` path (`read_inline`'s installed-`Context::
// math_command` arm, Gap 1) resolves `MathElem::Cmd` fine via
// `reflect_math_elem`/`as_math`; exercising `\derive` end to end is simply
// out of scope for porting this one package file (no fixture builds one).
// ============================================================================

#[test]
fn require_proof_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: proof
in
0";
        let v = compile_via_loader("proof-basic", src).expect("proof.satyh should compile");
        assert_eq!(as_int(v), 0);
    });
}

/// Gap 4 (`docs/plans/math-mode-language-gaps.md`) flagship: `\derive :
/// [math?; bool?; math list; math] math-cmd` called BARE (no `?:`/`?*`
/// marker at all) with only its two mandatory arguments — `\derive`'s
/// `?:nameopt ?:bopt` leading params register `optional_arity("\derive")
/// == 2`, so `math_bot`'s `Cmd` arm auto-pads `[None, None, <math list>,
/// <math>]` before applying (Gap 3's `math_block_ast` turns `{|A|}` into
/// the `math list` literal `[MathText([A])]`). `math-concat` forces
/// `as_math` -> `reflect_math_elem`, which actually APPLIES `\derive`'s
/// closure to those four arguments and runs `derive` all the way to its
/// `text-in-math` result value — real evaluation, not just a parse/
/// typecheck smoke test. `derive`'s body is itself `text-in-math` (Gap 6,
/// out of scope): laying THAT out would hard-error, so this only forces
/// evaluation via `math-concat`, never `embed-math`/layout on the result.
#[test]
fn gap4_derive_marker_less_optional_args_evaluate_via_math_concat() {
    run_with_big_stack(|| {
        let src = "@require: proof
in
let _ = math-concat ${\\derive{|A|}{B}} ${x} in 1";
        let v = compile_via_loader("gap4-derive-bare", src)
            .expect("bare \\derive{|A|}{B} should elaborate, typecheck, and evaluate");
        assert_eq!(as_int(v), 1);
    });
}

// ============================================================================
// `cd` (`lib-rustyfi/dist/packages/cd.satyh`, ported verbatim;
// `@require: gr color geom option`) — its `draw-arr-scheme` helper is a
// PLAIN (non-`let-inline`) function with two def-site optional parameters
// (`?:t-name-opt`/`?:len-name-opt`), and its `\diagram` sig's record field
// types use the `?->` optional-arrow arrow-type grammar
// (`draw-arr : math -> float?-> length ?-> obj -> obj -> graphics list`) —
// exactly the machinery `optional_args.rs` proves end to end. Every other
// primitive it needs (`get-graphics-bbox`/`embed-math`/`get-text-color`/
// `Geom.atan2-point`/`Geom.div-perp`/`Gr.arrow`/`Gr.dashed-arrow`/
// `Gr.text-rightward`/`-leftward`/`-centering`/`length-abs`) already exists.
// ============================================================================

#[test]
fn require_cd_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: cd
in
0";
        let v = compile_via_loader("cd-basic", src).expect("cd.satyh should compile");
        assert_eq!(as_int(v), 0);
    });
}

// ============================================================================
// `mitou-detail` (`lib-rustyfi/dist/packages/mitou-detail.satyh`, ported
// verbatim; `@require: pervasives gr list math color`) — a full document
// class, structurally close to `stdja.satyh`'s own capstone (title/
// section/subsection/figure/hook-page-break/cross-reference scheme) but
// simpler (no TOC, no footnotes). Every primitive it needs already exists
// (proven by `stdja.satyh`'s own capstone test above using the same set).
// `MitouDetail.document`'s title block always renders `– #subtitle; –`
// (U+2013 EN DASH), so this needs the `Wide` (non-ASCII-tolerant) font stub,
// exactly like the `stdja` capstone test.
// ============================================================================

#[test]
fn require_mitou_detail_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: mitou-detail
in
document (|
  project = {Mitou Detail Capstone};
  subtitle = {A minimal report};
|) '<
  +p {
    Hello, world! This is a Latin paragraph long enough that greedy line
    breaking must wrap it onto more than one line, exercising the real
    line-break and page-break path end to end through MitouDetail.document.
  }
>";
        let v = compile_via_loader_with_metrics("mitou-detail-document", src, &Wide)
            .expect("mitou-detail.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

// `mitou-report` — now PORTED (the `clear-page` primitive it needs was wired
// by the page-prims wave: `VertBox::ClearPage`). Its `document` takes a
// 5-field record (project/year/creators/manager/jouzai-number).
#[test]
fn require_mitou_report_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: mitou-report
in
document (|
  project = {Mitou Report Capstone};
  year = 2026;
  creators = [{Alice}; {Bob}];
  manager = {Carol};
  jouzai-number = 7;
|) '<
  +p {
    Hello, world! This is a Latin paragraph long enough that greedy line
    breaking must wrap it onto more than one line, exercising the real
    line-break and page-break path end to end through MitouReport.document.
  }
>";
        let v = compile_via_loader_with_metrics("mitou-report-document", src, &Wide)
            .expect("mitou-report.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

// `stdjareport` — PORTED (needs hook-page-break-block + page-break-multicolumn,
// both wired by the page-prims wave; @require footnote-scheme). `document`
// takes a title/author record + a `?->` optional config (omitted via `?*`),
// same shape as `stdjabook`.
#[test]
fn require_stdjareport_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: stdjareport
in
document (|
  title = {Report Capstone};
  author = {SATySFi in Rust};
|) ?* '<
  +p {
    Hello, world! This is a Latin paragraph long enough that greedy line
    breaking must wrap it onto more than one line, exercising the real
    line-break and page-break path end to end through StdJaReport.document.
  }
>";
        let v = compile_via_loader_with_metrics("stdjareport-document", src, &Wide)
            .expect("stdjareport.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

// `mdja` — PORTED (@require pervasives code math itemize color hdecoset
// vdecoset annot; all now available — itemize was the last holdout).
// `document` takes a title/author record.
#[test]
fn require_mdja_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: mdja
in
MDJa.document (|
  title = {MDJa Capstone};
  author = {SATySFi in Rust};
|) '<
  +p {
    Hello, world! This is a Latin paragraph long enough that greedy line
    breaking must wrap it onto more than one line, exercising the real
    line-break and page-break path end to end through MDJa.document.
  }
>";
        let v = compile_via_loader_with_metrics("mdja-document", src, &Wide)
            .expect("mdja.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

// ============================================================================
// `stdjabook` (`lib-rustyfi/dist/packages/stdjabook.satyh`, ported verbatim;
// `@require: pervasives gr list math code color option annot
// footnote-scheme`) — the same shape as `stdja.satyh`'s own capstone test
// above, plus a real `\footnote` command (`FootnoteScheme.main`, proven
// standalone above) and `Code.(command \code)` (`Mod.(e)` local-open —
// already-supported grammar, `cst.rs`'s `Atomic::OpenModule`). `\*` inside
// `FootnoteScheme`'s footnote-mark inline text is a lexer-level escape for
// a literal `*` (`lexer.rs`'s "symbol" class), not a module command. Uses
// the `Wide` font stub (defined above, ahead of `stdja`'s own capstone
// test): both the footer's em dash and `\footnote`'s mark text need
// non-ASCII tolerance the same way `stdja`'s capstone does; `show-toc =
// false` again avoids the `目次` heading (no real CJK metrics yet).
// ============================================================================

#[test]
fn require_stdjabook_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: stdjabook
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
    line-break and page-break path end to end through StdJaBook.document.
    Here is a footnote\\footnote{A minimal footnote body.} right in the middle.
  }
>";
        let v = compile_via_loader_with_metrics("stdjabook-document", src, &Wide)
            .expect("stdjabook.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

// ============================================================================
// `standalone` (`lib-rustyfi/dist/packages/standalone.satyh`, ported
// verbatim; the file itself has no `@require:` at all) — the minimal
// one-function document class, `standalone : block-text -> document`. Needs
// `embed-math`, `get-initial-context [math]`, `set-dominant-narrow-script`/
// `set-dominant-wide-script`, `command`, and the `A4Paper`/`Latin`/`Kana`
// variants — all confirmed present (final-coverage sweep). `+p` is NOT a
// primitive (every doc class, e.g. `stdja-mini.satyh`, defines its own), and
// `standalone.satyh` defines no block commands at all, so — like the
// `pervasives`/`footnote-scheme` tests above defining their own local
// `\math` — the test source defines a local `+p` (`stdja-mini.satyh`'s own
// one-liner) to build block-text content; no extra `@require:` needed.
// ============================================================================

#[test]
fn require_standalone_document_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: standalone
let-block ctx +p it = line-break true true ctx (read-inline ctx it ++ inline-fil)
in
standalone '<
  +p {
    Hello, world! This is a Latin paragraph long enough that greedy line
    breaking must wrap it onto more than one line, exercising the real
    line-break and page-break path end to end through standalone.
  }
>";
        let v = compile_via_loader_with_metrics("standalone-document", src, &Mono)
            .expect("standalone.satyh should compile");
        match v {
            Value::Document(doc) => {
                assert!(!doc.pages.is_empty(), "expected at least one page");
                assert!(
                    doc.pages.iter().any(|p| !p.lines.is_empty()),
                    "expected at least one non-empty page"
                );
            }
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

// ============================================================================
// `itemize` (`lib-rustyfi/dist/packages/itemize.satyh`, ported verbatim;
// `@require: pervasives list option gr`) — `Itemize.+listing`/`\listing`/
// `+enumerate`/`\enumerate`, built on the `itemize` variant type's `Item`
// constructor and the `{ * .. ** .. }` bullet-marker literal syntax
// (`rustyfi-syntax/tests/roundtrip.rs`'s `itemize_markers`). Its own
// blockers are all cleared: the parenthesized operator name `let (+++>) =
// List.fold-left (+++)`, and `listing-item`/`listing-item-breakable`'s
// type-ascribed, multi-pattern `let-rec .. : context -> int -> bool -> bool
// -> itemize -> block-boxes | ctx depth is-first is-last (Item(..)) = ..`
// (`type_ascribed_letrec.rs`), and `+listing`/`\listing`'s def-site `?:`
// optional binder (`marker_less_optional.rs`).
// ============================================================================

#[test]
fn require_itemize_listing_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: itemize
let-inline ctx \\math m = inline-nil
in
let ctx = get-initial-context 400pt (command \\math) in
get-natural-length (read-block ctx '<
  +listing?*{
    * Item one is long enough to wrap onto more than one line so that greedy
      line breaking must actually run over this item's text.
    * Item two
  }
>)";
        let v = compile_via_loader_with_metrics("itemize-listing", src, &Mono)
            .expect("itemize.satyh should compile");
        match v {
            Value::Length(len) => {
                assert!(len.0 > 0.0, "expected positive block length, got {len:?}")
            }
            other => panic!("expected a length, got {other:?}"),
        }
    });
}

// ============================================================================
// `progsynt` (`lib-rustyfi/dist/packages/progsynt.satyh`, ported verbatim;
// `@require: pervasives list math`) — two small math-object modules (`Term`,
// `Type`), built on internal `let to-math ?:iopt e = ..` calls, both bare
// (module-internal leading-`?:` bare calls, `marker_less_optional.rs`) and
// explicit (`to-math ?:1 e1`). Needs no new primitives (`math-char`/
// `math-color`/`math-char-class`/`math-group`/`text-in-math`/
// `get-font-size` all already exist).
//
// `Term.var`/`app`/`abs`/`letin`/`paren`/`meta` all build `${..}` MATH-TEXT
// LITERALS (`\token{..}`/`\sp` `MathElem::Cmd` nodes) — `Ast::MathText` is
// quoted (`eval.rs`: "captures the environment only, typesetting is phase
// 7's job"), so building these values is safe; only ACTUALLY RENDERING one
// (`embed-math`/`read-inline` walking into `layout_math_elem`) would hit the
// same "math command needs the math package (phase 7 roadmap A)" gap
// `math_package.rs`'s `require_proof_compiles_and_evaluates` already
// documents for `\derive` — out of scope here (no `.rs` edits). So, like
// that test, this exercises every one of `Term`/`Type`'s VALUE-building
// functions (not just a trivial `0` body) but stops short of rendering.
// ============================================================================

#[test]
fn require_progsynt_term_and_type_compile_and_evaluate() {
    run_with_big_stack(|| {
        let src = "@require: progsynt
in
let mk s = Term.var (math-char MathOrd s) in
let e1 = mk `x` in
let e2 = mk `y` in
let eapp = Term.app e1 e2 in
let eabs = Term.abs e1 eapp in
let elet = Term.letin e1 e2 eapp in
let epar = Term.paren eapp in
let emeta = Term.meta (math-char MathOrd `m`) in
let tb = Type.base (math-char MathOrd `A`) in
let tm = Type.meta (math-char MathOrd `T`) in
let tarr = open Type in (-->) tb tm in
eapp#assoc + eabs#assoc + elet#assoc + epar#assoc + emeta#assoc + tarr#assoc";
        let v =
            compile_via_loader("progsynt-term-type", src).expect("progsynt.satyh should compile");
        assert_eq!(as_int(v), 5);
    });
}

// ============================================================================
// `bnf` (`lib-rustyfi/dist/packages/bnf.satyh`, ported verbatim; `@require:
// math`) — the final bundled package. `BNF.insert-bars`/`tabular-of-math`
// use `Math`-package math-commands UNQUALIFIED inside their own `${..}`
// (`Math.join ${\mid} mlst`, `embed-math ctx ${#mnontm \mathrel{: : =}}`) —
// exactly the cross-module math-command exposure this milestone's
// `direct_cmd_name`/`TopBinding::Module` machinery (`elaborate.rs`) already
// generalizes to (see `math_cmd_exposure.rs`'s minimal fixture proving the
// mechanism in isolation). Calling `+BNF` through `read-block` (rather than
// stopping at a trivial body) forces the WHOLE pipeline: `Math`'s `\mid`/
// `\mathrel` resolved unqualified, typechecked as `math-cmd`, and evaluated
// through `embed-math`'s real `reflect_math_elem`/`MathElem::Cmd` forcing
// path (not merely built as an unforced `Value::MathText`) — `\mid` renders
// to non-ASCII `∣` (U+2223), hence the `Wide` stub (defined above, for
// `stdja`'s em dash), not `Mono`.
// ============================================================================

#[test]
fn require_bnf_direct_math_cmd_renders_via_embed_math() {
    run_with_big_stack(|| {
        let src = "@require: bnf
let-inline ctx \\math m = inline-nil
in
let ctx = get-initial-context 200pt (command \\math) in
let mnontm = math-char MathOrd `E` in
let mlstlst = [[math-char MathOrd `a`]; [math-char MathOrd `b`]] in
get-natural-length (read-block ctx '<+BNF(mnontm)(mlstlst);>)";
        let v = compile_via_loader_with_metrics("bnf-direct-math-cmd", src, &Wide)
            .expect("bnf.satyh should compile");
        match v {
            Value::Length(len) => {
                assert!(len.0 > 0.0, "expected positive block length, got {len:?}")
            }
            other => panic!("expected a length, got {other:?}"),
        }
    });
}
