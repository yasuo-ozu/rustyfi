//! Tier 0 stdlib port proof: `@require: list` (transitively
//! `option`) and `@require: option` alone must compile end-to-end through
//! the real multi-file loader with `lib-rustyfi/` as `lib_root` — the same
//! pipeline `rustyfi`'s `main.rs` uses, not a hand-rolled shortcut.
//! `option.satyg`/`list.satyg` are copied byte-for-byte from upstream.

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

fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

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

/// Never consulted — Tier-0 packages do no text/box rendering; exists only
/// because `eval::Interp::new` requires a `&dyn FontMetrics`.
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

/// A real (if crude) `FontMetrics`, for tests that call `read-inline` on
/// actual text — `NoFonts`'s always-`None` `advance` would make any
/// non-empty word a dynamic error (see `primitives.rs`'s `text_to_boxes`).
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

fn compile_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    compile_via_loader_with_metrics(tag, src, &NoFonts)
}

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

/// Runs `f` on a big-stack thread: `gr.satyh` (205 lines) needs more depth
/// than the default stack allows through syan's recursive-descent parser.
/// `Value` holds `Rc`s (not `Send`), so the compile call AND every assertion
/// on its result must run entirely *inside* `f` — nothing `Value`-shaped can
/// cross back out to the caller's thread.
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

// `@require: list` (transitively pulls in `option` via `list.satyg`'s own
// header).

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
    let src = "@require: list
in
Option.from 0 (Some 5)";
    let v = compile_via_loader("list-nested-option", src).expect("should compile");
    assert_eq!(as_int(v), 5);
}

#[test]
fn require_list_uses_pipe_internally() {
    // `list.satyg`'s `reverse`/`map-adjacent`/`map-with-ends` use `|>`
    // internally (Blocker B).
    let src = "@require: list
in
List.map-adjacent (fun x left right -> x) [1; 2; 3]";
    let v = compile_via_loader("list-pipe-internal", src).expect("should compile");
    assert_eq!(as_int_list(v), vec![1, 2, 3]);
}

#[test]
fn require_list_mapi_adjacent_uses_a_tuple_pattern_lambda_correctly() {
    // `List.mapi-adjacent`'s own definition destructures a tuple in its
    // first parameter (`fun (i, acc) x leftopt rightopt -> ..`) — the one
    // grammar gap hit porting `list.satyg` verbatim (`Expr::Fun` took
    // only plain variables; see `cst::ast::Expr::Fun`'s doc comment).
    let src = "@require: list
in
List.mapi-adjacent (fun i x leftopt rightopt -> i) [10; 20; 30]";
    let v = compile_via_loader("list-mapi-adjacent", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![0, 1, 2]);
}

#[test]
fn require_list_map_with_ends_calls_option_is_none_across_module_boundary() {
    let src = "@require: list
in
List.map-with-ends
  (fun is-first is-last x -> if is-first then 100 else if is-last then 200 else x)
  [1; 2; 3]";
    let v = compile_via_loader("list-map-with-ends", src).expect("list.satyg should compile");
    assert_eq!(as_int_list(v), vec![100, 2, 200]);
}

// `@require: option` alone (the second, minimal case).

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

// `color` built-in variant via the loader — no `@require:`
// needed (seeded by `prim_types::builtin_variants` before any package
// loads).

#[test]
fn a_color_value_compiles_via_the_loader() {
    // Zero-dependency entry document — no `@require:` at all.
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
    let src = "@require: list
in
let cs = [RGB (1.0, 0.0, 0.0); Gray 0.5; CMYK (0.0, 0.0, 0.0, 1.0)] in
List.length cs";
    let v = compile_via_loader("color-and-list", src).expect("should compile");
    assert_eq!(as_int(v), 3);
}

// `@require: color` (`lib-rustyfi/dist/packages/color.satyh`, ported
// verbatim) — wraps the built-in `Gray`/`RGB`/`CMYK` ctors in a `Color`
// module.

#[test]
fn require_color_module_constant_red_compiles() {
    // `Color.red = rgb 1. 0. 0. = RGB(1., 0., 0.)`.
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
    let src = "@require: color
in
let g = (match Color.gray 0.5 with Gray(_) -> 1 | _ -> 0) in
let c = (match Color.rgb 0.1 0.2 0.3 with RGB(_, _, _) -> 1 | _ -> 0) in
g + c";
    let v = compile_via_loader("color-fns", src).expect("color.satyh should compile");
    assert_eq!(as_int(v), 2);
}

// `@require: pervasives` (`lib-rustyfi/dist/packages/pervasives.satyh`,
// ported verbatim) — exercises the 5 primitives it needed that this port
// didn't already have: `get-natural-metrics`, `inline-frame-outer`,
// `set-manual-rising`, `script-guard`, `discretionary` (see primitives.rs's
// "pervasives.satyh unblockers" section).

#[test]
fn require_pervasives_compiles_and_evaluates_math_pi() {
    // Cheapest proof the WHOLE prelude typechecks: every top-level binding
    // in pervasives.satyh sits ahead of `body` in the same nested-let chain,
    // so success here means the entire file typechecked, not just this one
    // binding.
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
    // `no-break` calls `inline-frame-outer`, `mandatory-break` calls
    // `discretionary`; wrapping in `get-natural-metrics` exercises both plus
    // itself. `ctx`'s 400pt width makes `mandatory-break`'s `inline-skip
    // (get-text-width ctx *' 2.)` a fixed 800pt no-break box; `no-break`'s
    // zero padding leaves it unchanged.
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
    // Exercises `set-manual-rising`/`script-guard` via a real `read-inline`
    // pass (hence the `Mono` font stub — `NoFonts`'s always-`None` `advance`
    // would reject any glyph). The trailing `;` is required SATySFi syntax
    // (`lexer.rs`'s "active area"/`EndActive`): a command with no bracket
    // arguments must be explicitly terminated before following text.
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

// `@require: geom` (`lib-rustyfi/dist/packages/geom.satyh`, ported verbatim,
// `@require: pervasives`) — needs no new primitives.

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

// `@require: gr` (`lib-rustyfi/dist/packages/gr.satyh`, ported verbatim;
// `@require: pervasives`/`geom`/`list`) — the graphics prim
// additions: `bezier-to`, `close-with-bezier`,
// `shift-path`, `linear-transform-path`, `shift-graphics`,
// `linear-transform-graphics`, `get-graphics-bbox`, `dashed-stroke`, and
// `draw-text` (FAITHFUL, see `GraphicsElem::Text`'s doc
// comment).

#[test]
fn require_gr_rectangle_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `close-with-line` on 3 `line-to`s from `start-path` yields one
        // closed subpath.
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
        // `close-with-bezier` — the ONLY bundled use of either prim.
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
        // `Gr.scale-path` = shift(-center) |> linear-transform(scalex, 0, 0,
        // scaley) |> shift(center) — the ONLY bundled use of `shift-path`/
        // `linear-transform-path`. With `center` at the origin, both shifts
        // are no-ops, isolating the matrix math: `(x, y) |-> (2x, 3y)`.
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
        // `Gr.dashed-arrow` — the ONLY bundled use of `dashed-stroke`;
        // returns `[stroke-shaft; fill-head]`.
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
        // A rectangle's bbox is exactly its own corners.
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
        // `draw-text` is FAITHFUL (`GraphicsElem::Text`): an
        // EMPTY run has zero `natural_metrics`, so shifting a `Text{pt: (1,
        // 2), width: 0, height: 0, depth: 0}` by `(5, 5)` gives bbox
        // `(6, 7)-(6, 7)`.
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
        // `inline-skip 5pt` is a `FixedEmpty{5pt}` box — no glyphs, so
        // `NoFonts` suffices; its `natural_metrics` are exactly
        // `(5pt, 0pt, 0pt)`.
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
        // The `gr.satyh` consumer path exists for:
        // `Gr.text-centering` needs `get-graphics-bbox` to report a real
        // width for a `draw-text` run.
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
        // The exact cubic-extrema bbox is the circle's own tight
        // bounds `((40,40),(60,60))`; the old control-point hull would
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

// Tier-2 decoration/graphics packages: `deco`/`hdecoset`/`vdecoset`/
// `picture` — all copied verbatim except `picture.satyh`'s one unparseable
// sig line (bracket command-type syntax; see that file's own comment).

#[test]
fn require_deco_simple_frame_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `Deco.simple-frame` is `deco.satyh`'s only non-trivial export.
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
        // `deco-set` is a plain 4-tuple of `deco`s, `(decoS, decoH, decoM,
        // decoT)`.
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
        // `VDecoSet.simple-frame` is the richest of `vdecoset.satyh`'s
        // deco-sets short of `paper`/`quote-round`.
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
        // `Picture.draw-line` runs `draw-line-scheme`'s bbox-vs-slope
        // geometry between two synthetic `node`s built from EMPTY
        // `draw-text` runs (no real font metrics needed); yields one
        // stroked, open, single-segment line.
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

// `tabular` needs the `tabular` primitive/`cell` type. Upstream
// `tabular.satyh`'s own module wrapper is entirely commented out
// (`%`-prefixed dead code), so it defines a bare `\tabular` inline command
// positionally (`lstf cellf multif empty`, then `rulef`). Renders real cell
// text, hence `Mono`.
//
// `tabularx.satyh`/`table.satyh` need `cst.rs`'s `TypeAtom::Record`
// accepting `(| l : ty; … |)` in ordinary type position
// (`record_types.rs`).

#[test]
fn require_tabularx_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `cellf : cell-record option -> inline-text -> cell` — a `None`
        // alignment override exercises `make-alignments`'s `Option.from`
        // default-record path.
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

#[test]
fn require_table_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // `cellssf` receives a record VALUE (fields `l`/`r`/`c`/`m`/`e`);
        // the sig's record type sits inside the arrow chain of its first
        // cmd-type element — the other half of the record-types-in-
        // `TypeExpr` gap `tabularx` exercises.
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

// `code` — needs the context-box prims (`set-text-color`,
// `split-into-lines`, `block-frame-breakable`, `set-code-text-command`,
// `get-natural-length`). stdja.satyh `@require`s this directly.

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

// `annot` (`lib-rustyfi/dist/packages/annot.satyh`, ported verbatim) — every
// primitive it needs already exists; `register-location-frame` is a plain
// stdlib fn `Annot` defines itself.

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

// `stdja` (`lib-rustyfi/dist/packages/stdja.satyh`, ported verbatim) — the
// CAPSTONE: the real upstream document class, `@require`-ing pervasives, gr,
// list, math, code, color, option, annot.
//
// Uses `Wide` (not `Mono`) because the footer always renders an em dash
// (non-ASCII); `show-toc = false` avoids the CJK ToC heading since there is
// still no real CJK/Latin font metrics.

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

// `footnote-scheme` (ported verbatim; `@require: color gr`) —
// `add-footnote` is FAITHFUL (wraps the block in a zero-metric
// `PureHorzBox::Footnote` marker); this only proves the module
// compiles/evaluates, not that footnote text lands on the page (see
// `crates/rustyfi/tests/e2e.rs`'s footnote fixture for that).

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

// `proof` (ported verbatim; `@require: gr`) — `\derive`/`\derive-multi` are
// `direct`, `math-cmd`-typed; loading proves the whole file typechecks, not
// that `\derive` is ever invoked (needs a real `${…}`-embedded math-cmd
// call — `reflect_math_elem`/`as_math`).

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

/// `\derive : [math?; bool?; math list; math] math-cmd`
/// called BARE with only its two mandatory arguments — `optional_arity`
/// auto-pads `[None, None, <math list>, <math>]` (`math_block_ast`
/// turns `{|A|}` into `[MathText([A])]`). `math-concat` forces `as_math` ->
/// `reflect_math_elem`, which actually APPLIES `\derive`'s closure — real
/// evaluation, not a parse/typecheck smoke test. `derive`'s body is itself
/// `text-in-math` (out of scope here), so this only forces evaluation via
/// `math-concat`, never layout.
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

// `cd` (ported verbatim; `@require: gr color geom option`) —
// `draw-arr-scheme` uses def-site optional params (`?:t-name-opt`) and the
// `\diagram` sig uses `?->` optional-arrow record field types — the
// machinery `optional_args.rs` proves end to end.

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

// `mitou-detail` (ported verbatim; `@require: pervasives gr list math
// color`) — structurally close to `stdja`'s capstone (title/section/
// subsection/figure/hook-page-break/cross-reference), simpler (no TOC, no
// footnotes). Title block renders `– #subtitle; –` (EN DASH), hence `Wide`.

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

// `mitou-report` — needs the `clear-page` primitive (`VertBox::ClearPage`).
// Its `document` takes a 5-field record
// (project/year/creators/manager/jouzai-number).
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

// `stdjareport` — needs hook-page-break-block + page-break-multicolumn;
// `@require: footnote-scheme`. `document` takes a title/author record + a
// `?->` optional config (omitted via `?*`), same shape as `stdjabook`.
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

// `mdja` — `@require: pervasives code math itemize color hdecoset vdecoset
// annot`. `document` takes a title/author record.
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

// `stdjabook` (ported verbatim; `@require: pervasives gr list math code
// color option annot footnote-scheme`) — like `stdja`'s capstone, plus a
// real `\footnote` command and `Code.(command \code)` local-open. `\*`
// inside the footnote mark is a lexer-level escape for literal `*`
// (`lexer.rs`'s "symbol" class), not a module command. Uses `Wide` (both
// the footer's em dash and `\footnote`'s mark text need non-ASCII
// tolerance); `show-toc = false` again avoids the CJK ToC heading.

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

// `standalone` (ported verbatim; no `@require:` of its own) — the minimal
// one-function document class, `standalone : block-text -> document`. `+p`
// is not a primitive (every doc class defines its own), so the test source
// defines a local one.

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

// `itemize` (ported verbatim; `@require: pervasives list option gr`) —
// built on the `itemize` variant's `Item` ctor and the `{ * .. ** .. }`
// bullet-marker literal syntax. Exercises the parenthesized operator name
// `let (+++>) = List.fold-left (+++)`, `listing-item`'s type-ascribed
// multi-pattern `let-rec` (`type_ascribed_letrec.rs`), and `+listing`'s
// def-site `?:` optional binder (`marker_less_optional.rs`).

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

// `progsynt` (ported verbatim; `@require: pervasives list math`) — `Term`/
// `Type` build `${..}` MATH-TEXT LITERALS via internal bare and explicit
// leading-`?:` calls (`marker_less_optional.rs`). `Ast::MathText` is quoted
// (typesetting is handled elsewhere), so building these values is safe; only
// actually RENDERING one (`embed-math`/`read-inline`) would hit the "math
// command needs the math package" gap — this exercises every value-building
// function but stops short of rendering.

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

// `bnf` (ported verbatim; `@require: math`) — uses `Math`-package math
// commands UNQUALIFIED inside its own `${..}` — the cross-module
// math-command exposure `elaborate.rs`'s `direct_cmd_name`/
// `TopBinding::Module` machinery generalizes to (see `math_cmd_exposure.rs`).
// `\mid` renders to non-ASCII `∣` (U+2223), hence `Wide`.

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
