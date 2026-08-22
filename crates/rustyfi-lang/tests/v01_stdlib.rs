//! Vendoring the 17 upstream 0.1 `stdlib` package modules —
//! `lib-rustyfi/dist-v01/packages/{color,basic,paper-size,ref,cross-ref,
//! length,ordering,option,pair,int,float,list,string,vector,point,path,
//! context}.satyh|.satyg` — transliterated from
//! `saphe-split@b836d512:lib-rustyfi/packages/stdlib/stdlib.0.0.1/src/*`,
//! PROVEN through the real production loader (`rustyfi_loader::load`,
//! `lib_root = dist-v01/packages`, `RustyfiVersion::V0_1`) — not merely
//! parsed.
//!
//! `compile_v01_via_loader[_with_metrics]` compiles a package member to a
//! `Value` through the real loader/elaborate/typecheck/eval pipeline.
//! `assert_bare_access_unbound` is the qualified-export negative probe
//! every package gets once: referencing a member's bare name after only
//! `@require:`ing the package must fail "unbound variable", proving the
//! loader-resolved dependency was wrapped as a real module and not spliced
//! flat.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_backend::{FontKey, FontMetrics, Length};
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
            "rustyfi-lang-v01-stdlib-{tag}-{}-{}.saty",
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

/// Stub — never consulted; these fixtures never render real text
/// (`embed-string` builds a `Value::InlineText` without rendering it).
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

/// Real (ASCII-only) metrics — `color`'s document capstone DOES render
/// text via `V01Mini.document`'s `read-inline` pass.
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
        LoadedCst::V0_0(_) => unreachable!("this test's helper is V0_1-only"),
    }
}

/// Reproduces `compile_document_v1_with_trials` (`lib.rs:165-195`) minus
/// the sealing check `v1::module_check::check_program` (`pub(crate)`,
/// unreachable from an integration test; sealing is proven separately by
/// `v01_sealing.rs`).
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
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&elaborated.body, &store))
        .map_err(|e| format!("eval: {e}"))
}

/// The qualified-export negative probe every Wave-0 package gets once; see
/// this module's doc comment for what it proves and why.
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

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

fn as_bool(v: Value) -> bool {
    match v {
        Value::Bool(b) => b,
        other => panic!("expected a bool, got {other:?}"),
    }
}

fn as_float(v: Value) -> f64 {
    match v {
        Value::Float(f) => f,
        other => panic!("expected a float, got {other:?}"),
    }
}

fn as_str(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected a string, got {other:?}"),
    }
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

/// Needs a big stack: `list.satyg` (280+ lines) exceeds the default
/// recursion depth of syan's recursive-descent parser.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

// `color.satyh` — the fully-specced first package.

#[test]
fn color_bare_gray_is_unbound_without_qualification() {
    assert_bare_access_unbound("color-bare", "color", "gray 0.5");
}

#[test]
fn color_module_constants_and_functions_resolve_qualified() {
    let src = "@require: color
match Color.red with
| RGB(_, _, _)     -> 1
| Gray(_)          -> 2
| CMYK(_, _, _, _) -> 3
end";
    let v = compile_v01_via_loader("color-red", src).expect("color.satyh should compile");
    assert_eq!(as_int(v), 1);

    let src2 = "@require: color
let g = (match Color.gray 0.5 with Gray(_) -> 1 | _ -> 0 end) in
let c = (match Color.rgb 0.1 0.2 0.3 with RGB(_, _, _) -> 1 | _ -> 0 end) in
g + c";
    let v2 = compile_v01_via_loader("color-fns", src2).expect("color.satyh should compile");
    assert_eq!(as_int(v2), 2);
}

/// `Color.red`'s payload is pattern-matched INTO the rendered text,
/// proving the sealed value round-trips as a real `color`, not just a name.
#[test]
fn color_document_capstone_loads_and_compiles_via_v01_mini() {
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: color

let ib = embed-string (match Color.red with
  | RGB(r, g, b) -> `warm`
  | Gray(_)      -> `gray`
  | CMYK(_, _, _, _) -> `cmyk`
  end) in
let open V01Mini in
document (| title = `color` |) '<
  +p { Color says #ib;. }
>";
        let doc = TempDoc::new("color-capstone", src);
        let opts = LoadOptions {
            lib_root: Some(lib_root()),
            version: RustyfiVersion::V0_1,
            ..Default::default()
        };
        let program = rustyfi_loader::load(&doc.0, &opts).expect("v01-mini + color should load");
        assert_eq!(
            program.files.len(),
            3,
            "expected color.satyh + v01-mini.satyh + the entry"
        );
        assert!(matches!(program.files[0].cst, LoadedCst::V0_1(_)));
        assert!(matches!(program.files[1].cst, LoadedCst::V0_1(_)));

        let doc_value = rustyfi_lang::compile_document_v1(&program.files, &Mono)
            .expect("color.satyh + v01-mini.satyh should compile to a document");
        assert_eq!(doc_value.pages.len(), 1);
        assert!(
            doc_value.pages[0].lines.len() >= 2,
            "expected the +p paragraph plus v01-mini's footer line, got {}",
            doc_value.pages[0].lines.len()
        );
    });
}

// `basic.satyg` — UNSEALED (no `:>`); only `type` synonyms + ctors. Cross-
// file consumer: `ordering.satyg`'s tests below.

#[test]
fn basic_ctors_become_available_once_required() {
    let src = "@require: basic
match Less with
| Less    -> 1
| Equal   -> 2
| Greater -> 3
end";
    let v = compile_v01_via_loader("basic-ctors", src).expect("basic.satyg should compile");
    assert_eq!(as_int(v), 1);
}

// `paper-size.satyh` — plain UNSEALED module of `length * length` constants.

#[test]
fn paper_size_bare_a4_is_unbound_without_qualification() {
    assert_bare_access_unbound("paper-size-bare", "paper-size", "a4");
}

#[test]
fn paper_size_a4_is_the_iso_216_dimensions() {
    let src = "@require: paper-size
PaperSize.a4";
    let v = compile_v01_via_loader("paper-size-a4", src).expect("paper-size.satyh should compile");
    let vs = as_tuple(v);
    assert_eq!(vs.len(), 2);
    let w = as_length(vs[0].clone());
    let h = as_length(vs[1].clone());
    assert!((w.0 - Length::from_unit(210.0, "mm").unwrap().0).abs() < 1e-6);
    assert!((h.0 - Length::from_unit(297.0, "mm").unwrap().0).abs() < 1e-6);
}

// `ref.satyg` — sealed; `<-`/`!` generically overwrite a `Value::Ref` held
// by a function PARAMETER, not just a `let mutable`-introduced local.

#[test]
fn ref_bare_increment_is_unbound_without_qualification() {
    assert_bare_access_unbound(
        "ref-bare",
        "ref",
        "let mutable r <- 0 in let () = increment r in !r",
    );
}

#[test]
fn ref_increment_and_decrement_mutate_through_a_parameter() {
    let src = "@require: ref
let mutable r <- 5 in
let () = Ref.increment r in
let () = Ref.increment r in
let () = Ref.decrement r in
!r";
    let v = compile_v01_via_loader("ref-inc-dec", src).expect("ref.satyg should compile");
    assert_eq!(as_int(v), 6);
}

// `cross-ref.satyg` — sealed; `register`/`probe` round-trip through the
// real `register-cross-reference`/`probe-cross-reference` primitives
// (Group E).

#[test]
fn cross_ref_bare_register_is_unbound_without_qualification() {
    assert_bare_access_unbound("cross-ref-bare", "cross-ref", "register `k` `v`");
}

#[test]
fn cross_ref_register_then_probe_round_trips() {
    let src = "@require: cross-ref
let () = CrossRef.register `k` `v` in
match CrossRef.probe `k` with
| Some(s) -> s
| None    -> `MISSING`
end";
    let v =
        compile_v01_via_loader("cross-ref-roundtrip", src).expect("cross-ref.satyg should compile");
    assert_eq!(as_str(v), "v");
}

// `length.satyh` — sealed; `max`/`min`/`abs`/`atan2` over the built-in
// `length` type.

#[test]
fn length_bare_max_is_unbound_without_qualification() {
    assert_bare_access_unbound("length-bare", "length", "max 1pt 2pt");
}

#[test]
fn length_max_min_abs_compile_and_evaluate() {
    let src = "@require: length
Length.max (Length.min 3pt 5pt) (Length.abs (0pt -' 9pt))";
    let v = compile_v01_via_loader("length-max-min-abs", src).expect("length.satyh should compile");
    assert_eq!(as_length(v), Length::pt(9.0));
}

// `ordering.satyg` — sealed; a smoke test for cross-file qualified-type
// identity (`Basic.ordering`, matched/constructed here in a DIFFERENT
// vendored file).

#[test]
fn ordering_bare_compare_is_unbound_without_qualification() {
    assert_bare_access_unbound("ordering-bare", "ordering", "compare Less Greater");
}

#[test]
fn ordering_compare_and_show_cross_file_basic_ordering() {
    let src = "@require: ordering
Ordering.show (Ordering.compare Less Greater)";
    let v = compile_v01_via_loader("ordering-compare-show", src)
        .expect("ordering.satyg should compile (R4: cross-file Basic.ordering)");
    assert_eq!(as_str(v), "Less");
}

#[test]
fn ordering_equal_is_reflexive_and_show_round_trips_all_three_ctors() {
    let src = "@require: ordering
if Ordering.equal (Ordering.compare Equal Equal) Equal
then Ordering.show (Ordering.compare Greater Less)
else `WRONG`";
    let v = compile_v01_via_loader("ordering-equal-reflexive", src)
        .expect("ordering.satyg should compile");
    assert_eq!(as_str(v), "Greater");
}

#[test]
fn option_bare_map_is_unbound_without_qualification() {
    assert_bare_access_unbound("option-bare", "option", "map (fun x -> x + 1) (Some 41)");
}

#[test]
fn option_map_bind_from_compile_and_evaluate() {
    let src = "@require: option
Option.from 0 (Option.bind (Option.map (fun x -> x + 1) (Some 41)) (fun x -> Some (x + 1)))";
    let v =
        compile_v01_via_loader("option-map-bind-from", src).expect("option.satyg should compile");
    assert_eq!(as_int(v), 43);
}

#[test]
fn pair_bare_first_is_unbound_without_qualification() {
    assert_bare_access_unbound("pair-bare", "pair", "first (1, 2)");
}

#[test]
fn pair_map_second_and_second_compile_and_evaluate() {
    let src = "@require: pair
Pair.second (Pair.map-second (fun y -> y + 1) (1, 41))";
    let v = compile_v01_via_loader("pair-map-second", src).expect("pair.satyg should compile");
    assert_eq!(as_int(v), 42);
}

// `int.satyg` — sealed; `equal`'s `( == )` operator section transliterates
// to the lambda fallback `fun n1 n2 -> n1 == n2` (no `Atomic::OpRef`
// in this port's v1 grammar).

#[test]
fn int_bare_compare_is_unbound_without_qualification() {
    assert_bare_access_unbound("int-bare", "int", "compare 3 5");
}

#[test]
fn int_compare_matches_ordering_ctors() {
    let src = "@require: int
match Int.compare 3 5 with
| Less    -> 1
| Equal   -> 2
| Greater -> 3
end";
    let v = compile_v01_via_loader("int-compare", src).expect("int.satyg should compile");
    assert_eq!(as_int(v), 1);
}

#[test]
fn int_equal_operator_section_lambda_fallback_works() {
    let src = "@require: int
if Int.equal 4 4 then Int.equal 4 5 else true";
    let v = compile_v01_via_loader("int-equal", src).expect("int.satyg should compile");
    assert!(!as_bool(v));
}

// `float.satyg` — sealed, fully vendored.

#[test]
fn float_bare_power_is_unbound_without_qualification() {
    assert_bare_access_unbound("float-bare", "float", "power 2. 3.");
}

#[test]
fn float_power_sqrt_and_pi_compile_and_evaluate() {
    let src = "@require: float
Float.sqrt (Float.power 2. 3.)";
    let v = compile_v01_via_loader("float-power-sqrt", src).expect("float.satyg should compile");
    let f = as_float(v);
    // power 2. 3. = 3^2 = 9; sqrt 9 = 3.
    assert!((f - 3.0).abs() < 1e-6, "expected 3.0, got {f}");
}

#[test]
fn float_pi_is_the_expected_constant() {
    let src = "@require: float
Float.pi";
    let v = compile_v01_via_loader("float-pi", src).expect("float.satyg should compile");
    let f = as_float(v);
    assert!(
        (f - std::f64::consts::PI).abs() < 1e-9,
        "expected pi, got {f}"
    );
}

#[test]
fn float_abs_of_a_negative_value_negates_it() {
    let src = "@require: float
Float.abs (0. -. 3.5)";
    let v = compile_v01_via_loader("float-abs", src).expect("float.satyg should compile");
    let f = as_float(v);
    assert!((f - 3.5).abs() < 1e-9, "expected 3.5, got {f}");
}

#[test]
fn float_max_and_min_pick_the_expected_operand() {
    let src = "@require: float
Float.max (Float.min 4. 9.) 2.";
    let v = compile_v01_via_loader("float-max-min", src).expect("float.satyg should compile");
    let f = as_float(v);
    // min 4. 9. = 4.; max 4. 2. = 4.
    assert!((f - 4.0).abs() < 1e-9, "expected 4.0, got {f}");
}

// `list.satyg` — sealed; requires `option` transitively (`map-with-ends`
// calls `Option.is-none`). `find`'s recursive branch is upstream-buggy —
// only head-matches/empty are exercised here.

#[test]
fn list_bare_fold_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound(
            "list-bare",
            "list",
            "fold (fun acc x -> acc + x) 0 [1, 2, 3]",
        );
    });
}

#[test]
fn list_fold_of_map_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: list
List.fold (fun acc x -> acc + x) 0 (List.map (fun x -> x + 1) [1, 2, 3])";
        let v = compile_v01_via_loader("list-fold-map", src).expect("list.satyg should compile");
        assert_eq!(as_int(v), 9);
    });
}

#[test]
fn list_map_with_ends_calls_option_is_none_across_the_require_boundary() {
    run_with_big_stack(|| {
        let src = "@require: list
List.length (List.map-with-ends
  (fun is-first is-last x -> if is-first then 100 else if is-last then 200 else x)
  [1, 2, 3])";
        let v = compile_v01_via_loader("list-map-with-ends", src)
            .expect("list.satyg (and its transitive option.satyg) should compile");
        assert_eq!(as_int(v), 3);
    });
}

#[test]
fn list_find_matches_the_head_without_hitting_the_upstream_recursion_bug() {
    run_with_big_stack(|| {
        let src = "@require: list
match List.find (fun x -> x == 1) [1, 2, 3] with
| Some(n) -> n
| None    -> -1
end";
        let v = compile_v01_via_loader("list-find-head", src).expect("list.satyg should compile");
        assert_eq!(as_int(v), 1);
    });
}

// `string.satyg` — sealed; requires `basic`/`int`/`option`/`list`
// transitively. `append`'s `( ^ )` operator section transliterates to the
// lambda fallback.

#[test]
fn string_bare_length_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("string-bare", "string", "length `abc`");
    });
}

#[test]
fn string_append_and_length_compile_and_evaluate() {
    run_with_big_stack(|| {
        let src = "@require: string
String.length (String.append `foo` `bar`)";
        let v = compile_v01_via_loader("string-append-length", src)
            .expect("string.satyg should compile");
        assert_eq!(as_int(v), 6);
    });
}

#[test]
fn string_chop_prefix_uses_list_and_option_transitively() {
    run_with_big_stack(|| {
        let src = "@require: string
match String.chop-prefix `foo` `foobar` with
| Some(s) -> s
| None    -> `MISSING`
end";
        let v = compile_v01_via_loader("string-chop-prefix", src)
            .expect("string.satyg (and its transitive list/option) should compile");
        assert_eq!(as_str(v), "bar");
    });
}

// `vector.satyg` — sealed; `Basic.vector` qualification.

#[test]
fn vector_bare_add_is_unbound_without_qualification() {
    assert_bare_access_unbound("vector-bare", "vector", "add (1pt, 2pt) (3pt, 4pt)");
}

#[test]
fn vector_add_and_get_x_compile_and_evaluate() {
    let src = "@require: vector
Vector.get-x (Vector.add (1pt, 2pt) (3pt, 4pt))";
    let v = compile_v01_via_loader("vector-add-get-x", src).expect("vector.satyg should compile");
    assert_eq!(as_length(v), Length::pt(4.0));
}

// `point.satyh` — sealed; `Basic.point`/`Basic.vector`. `add`/`get-x`/
// `get-y` are aliases of `Vector`'s own — the two are the SAME structural
// synonym, so they unify transparently.

#[test]
fn point_bare_get_y_is_unbound_without_qualification() {
    assert_bare_access_unbound("point-bare", "point", "get-y (1pt, 2pt)");
}

#[test]
fn point_add_and_get_y_compile_and_evaluate() {
    let src = "@require: point
Point.get-y (Point.add (1pt, 2pt) (3pt, 4pt))";
    let v = compile_v01_via_loader("point-add-get-y", src).expect("point.satyh should compile");
    assert_eq!(as_length(v), Length::pt(6.0));
}

#[test]
fn point_dividing_point_and_atan2_compile_and_evaluate() {
    let src = "@require: point
Point.get-x (Point.dividing-point 0.5 (0pt, 0pt) (10pt, 0pt))";
    let v =
        compile_v01_via_loader("point-dividing-point", src).expect("point.satyh should compile");
    assert_eq!(as_length(v), Length::pt(5.0));
}

// `path.satyh` — sealed; `Basic.point`. `rectangle`/`get-bounding-box`
// exercise `start-path`/`line-to`/`close-with-line`/`get-path-bbox`.

#[test]
fn path_bare_rectangle_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("path-bare", "path", "rectangle (0pt, 0pt) (10pt, 10pt)");
    });
}

#[test]
fn path_rectangle_bounding_box_is_its_own_corners() {
    run_with_big_stack(|| {
        let src = "@require: path
Path.get-bounding-box (Path.rectangle (0pt, 0pt) (10pt, 20pt))";
        let v =
            compile_v01_via_loader("path-rectangle-bbox", src).expect("path.satyh should compile");
        let corners = as_tuple(v);
        assert_eq!(corners.len(), 2);
        let p0 = as_tuple(corners[0].clone());
        let p1 = as_tuple(corners[1].clone());
        assert_eq!(as_length(p0[0].clone()), Length::pt(0.0));
        assert_eq!(as_length(p0[1].clone()), Length::pt(0.0));
        assert_eq!(as_length(p1[0].clone()), Length::pt(10.0));
        assert_eq!(as_length(p1[1].clone()), Length::pt(20.0));
    });
}

#[test]
fn path_circle_compiles_and_evaluates() {
    run_with_big_stack(|| {
        let src = "@require: path
Path.get-bounding-box (Path.circle (50pt, 50pt) 10pt)";
        let v = compile_v01_via_loader("path-circle-bbox", src).expect("path.satyh should compile");
        let corners = as_tuple(v);
        assert_eq!(corners.len(), 2);
    });
}

// `context.satyh` — sealed, PARTIAL vendor (7/28 members dropped).
// GAP: a module-qualified command reference in PROGRAM-AREA position
// (`command \Mod.cmd`) does not lex — `lexer.rs`'s `lex_program` only
// scans an unqualified name, so the following `.` hits "illegal token '.'
// in a program area" (`lexer.rs:723`). Worked around below with `let open
// V01Mini in …`, which re-exposes `\math` bare so `command \math` lexes
// with no dot involved.

#[test]
fn context_bare_set_font_size_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound(
            "context-bare",
            "context",
            "set-font-size 20pt (initial 400pt (command \\math))",
        );
    });
}

#[test]
fn context_set_and_get_font_size_round_trip() {
    run_with_big_stack(|| {
        let src = "@require: context
@require: v01-mini
let open V01Mini in
Context.get-font-size (Context.set-font-size 20pt (Context.initial 400pt (command \\math)))";
        let v = compile_v01_via_loader("context-font-size-roundtrip", src)
            .expect("context.satyh should compile");
        assert_eq!(as_length(v), Length::pt(20.0));
    });
}

#[test]
fn context_set_and_get_text_width_and_leading_round_trip() {
    run_with_big_stack(|| {
        let src = "@require: context
@require: v01-mini
let open V01Mini in
let ctx = Context.initial 400pt (command \\math) in
Context.get-text-width (Context.set-leading 15pt ctx)";
        let v = compile_v01_via_loader("context-leading-roundtrip", src)
            .expect("context.satyh should compile");
        assert_eq!(as_length(v), Length::pt(400.0));
    });
}

// `unidata.satyh`/`hyph-english.satyh` — both `val`s EVALUATE `load-*`
// at module load (not lazily), so reaching them via `@require:` proves the
// loader stand-ins are accept-and-return, not hard errors.

#[test]
fn hyph_english_bare_hyphenation_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("hyph-english-bare", "hyph-english", "hyphenation");
    });
}

#[test]
fn unidata_bare_unidata_is_unbound_without_qualification() {
    run_with_big_stack(|| {
        assert_bare_access_unbound("unidata-bare", "unidata", "unidata");
    });
}

#[test]
fn hyph_unidata_packages_seal_and_load() {
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: unidata
@require: hyph-english

let h = HyphEnglish.hyphenation in
let u = Unidata.unidata in
let open V01Mini in
document (| title = `hyph-unidata` |) '<
  +p { ok. }
>";
        let doc = TempDoc::new("hyph-unidata-capstone", src);
        let opts = LoadOptions {
            lib_root: Some(lib_root()),
            version: RustyfiVersion::V0_1,
            ..Default::default()
        };
        let program = rustyfi_loader::load(&doc.0, &opts)
            .expect("v01-mini + unidata + hyph-english should load");
        assert_eq!(
            program.files.len(),
            4,
            "expected unidata.satyh + hyph-english.satyh + v01-mini.satyh + the entry"
        );

        let doc_value = rustyfi_lang::compile_document_v1(&program.files, &Mono).expect(
            "unidata.satyh + hyph-english.satyh + v01-mini.satyh should compile to a document \
             (the load-* stand-ins must be accept-and-return, evaluated at module load)",
        );
        assert_eq!(doc_value.pages.len(), 1);
    });
}

// The 4 font envelope stand-in packages. `font` is saphe-split's real
// `BaseType(FontType)` (`typecheck::name_to_mono`'s `"font"` arm ->
// `t_font_key`), an OPAQUE handle, not a `t_string()` stand-in — members
// type as `font`, evaluate to `Value::Font(FontKey)`, and flow into
// `set-font`/`set-math-font`.

/// Gives every abbrev its OWN `FontKey`, unlike `NoFonts`/`Mono`'s default
/// `resolve_font_abbrev` (`None` for everything, collapsing all faces onto
/// one heuristic) — needed so the assertions below can tell abbrevs apart.
struct NamedFaces;

impl NamedFaces {
    /// The nine abbrevs the four bundled stand-ins name, in a fixed order
    /// that doubles as their `FontKey` numbering.
    const ABBREVS: &'static [&'static str] = &[
        "Junicode",
        "Junicode-b",
        "Junicode-it",
        "Junicode-bi",
        "lmmono",
        "lmsans",
        "lmodern",
        "ipaexm",
        "ipaexg",
    ];

    fn key_of(abbrev: &str) -> FontKey {
        FontKey(
            Self::ABBREVS
                .iter()
                .position(|a| *a == abbrev)
                .unwrap_or_else(|| panic!("NamedFaces has no key for `{abbrev}`"))
                as u16,
        )
    }
}

impl FontMetrics for NamedFaces {
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
    fn resolve_font_abbrev(&self, abbrev: &str) -> Option<FontKey> {
        Self::ABBREVS
            .iter()
            .position(|a| *a == abbrev)
            .map(|i| FontKey(i as u16))
    }
}

/// Deliberately NOT `as_str`: a `font` is not a string, so a test
/// that could still read one back would be pinning the old stand-in.
fn as_font(v: Value) -> FontKey {
    match v {
        Value::Font(key) => key,
        other => panic!("expected an opaque `font` handle, got {other:?}"),
    }
}

#[test]
fn font_junicode_bare_normal_is_unbound_without_qualification() {
    assert_bare_access_unbound("font-junicode-bare", "font-junicode", "normal");
}

#[test]
fn font_latin_modern_math_bare_main_is_unbound_without_qualification() {
    assert_bare_access_unbound("font-lmm-bare", "font-latin-modern-math", "main");
}

#[test]
fn font_packages_seal_and_resolve() {
    run_with_big_stack(|| {
        let src = "@require: v01-mini
@require: font-junicode
@require: font-latin-modern-math

let open V01Mini in
let ctx0 = get-initial-context 440pt (command \\math) in
let ctx = ctx0
  |> set-font Latin (FontJunicode.normal, 1., 0.)
  |> set-math-font FontLatinModernMath.main in
let n = embed-string (arabic 1) in
document (| title = `f` |) '<
  +p { Font #n;. }
>";
        let v = compile_v01_via_loader_with_metrics("font-packages-seal-resolve", src, &Mono)
            .expect(
                "font-junicode.satyh + font-latin-modern-math.satyh + v01-mini.satyh should \
                 compile (FontJunicode.normal : font must seal as the opaque handle and flow \
                 through set-font's `font * float * float`, and set-math-font must accept the \
                 bare font member — saphe-split's `tFONTWR` and `tFONTKEY` respectively)",
            );
        match v {
            Value::Document(_) => {}
            other => panic!("expected a document, got {other:?}"),
        }
    });
}

#[test]
fn font_junicode_normal_is_not_a_length() {
    run_with_big_stack(|| {
        let src = "@require: font-junicode
FontJunicode.normal +' 1pt";
        let err = compile_v01_via_loader("font-junicode-not-length", src)
            .err()
            .unwrap_or_else(|| {
                panic!("expected `FontJunicode.normal +' 1pt` to fail to typecheck, it compiled")
            });
        assert!(
            err.contains("typecheck"),
            "expected a typecheck error, got: {err}"
        );
    });
}

/// Central negative: 0.1's `font` is NOT `string`. Would have passed under
/// the old `"font" => t_string()` stand-in; `string-length` is the
/// cheapest witness the two types do not unify.
#[test]
fn font_is_not_string_in_either_direction() {
    run_with_big_stack(|| {
        let font_as_string = "@require: font-junicode
string-length FontJunicode.normal";
        let err = compile_v01_via_loader("font-not-string-a", font_as_string)
            .err()
            .unwrap_or_else(|| {
                panic!(
                    "`string-length FontJunicode.normal` must not typecheck — 0.1's `font` is \
                     saphe-split's opaque `BaseType(FontType)`, not `string`"
                )
            });
        assert!(
            err.contains("typecheck"),
            "expected a typecheck error: {err}"
        );

        // A bare abbrev is not a `font`: `set-math-font` takes `tFONTKEY` in 0.1.
        let string_as_font = "@require: v01-mini
let open V01Mini in
set-math-font `lmodern` (get-initial-context 440pt (command \\math))";
        let err = compile_v01_via_loader("font-not-string-b", string_as_font)
            .err()
            .unwrap_or_else(|| {
                panic!(
                    "`set-math-font `lmodern`` must not typecheck under 0.1 — that is the \
                     0.0.6 signature (`tS @-> tCTX @-> tCTX`)"
                )
            });
        assert!(
            err.contains("typecheck"),
            "expected a typecheck error: {err}"
        );
    });
}

/// `set-math-font` under 0.0.6 still takes the abbrev string — guards the
/// corpus, which calls it that way throughout.
#[test]
fn v006_set_math_font_still_takes_the_abbrev_string() {
    let ty = rustyfi_lang::prim_types::primitive_type_with_version(
        "set-math-font",
        rustyfi_syntax::RustyfiVersion::V0_0,
    )
    .expect("set-math-font is registered under V0_0");
    assert_eq!(
        format!("{}", rustyfi_lang::types::instantiate(&ty, 0)),
        "string -> (context -> context)"
    );
    let ty01 = rustyfi_lang::prim_types::primitive_type_with_version(
        "set-math-font",
        rustyfi_syntax::RustyfiVersion::V0_1,
    )
    .expect("set-math-font is registered under V0_1");
    assert_eq!(
        format!("{}", rustyfi_lang::types::instantiate(&ty01, 0)),
        "font -> (context -> context)"
    );
}

#[test]
fn font_latin_modern_bare_sans_is_unbound_without_qualification() {
    assert_bare_access_unbound("font-latin-modern-bare", "font-latin-modern", "sans");
}

#[test]
fn font_latin_modern_members_are_the_expected_006_corpus_abbrevs() {
    let src = "@require: font-latin-modern
(FontLatinModern.mono, FontLatinModern.sans)";
    let v = compile_v01_via_loader_with_metrics("font-latin-modern-abbrevs", src, &NamedFaces)
        .expect("font-latin-modern.satyh should compile");
    let vs = as_tuple(v);
    assert_eq!(as_font(vs[0].clone()), NamedFaces::key_of("lmmono"));
    assert_eq!(as_font(vs[1].clone()), NamedFaces::key_of("lmsans"));
}

#[test]
fn font_ipa_ex_bare_mincho_is_unbound_without_qualification() {
    assert_bare_access_unbound("font-ipa-ex-bare", "font-ipa-ex", "mincho");
}

#[test]
fn font_ipa_ex_members_are_the_expected_006_corpus_abbrevs() {
    let src = "@require: font-ipa-ex
(FontIpaEx.mincho, FontIpaEx.gothic)";
    let v = compile_v01_via_loader_with_metrics("font-ipa-ex-abbrevs", src, &NamedFaces)
        .expect("font-ipa-ex.satyh should compile");
    let vs = as_tuple(v);
    assert_eq!(as_font(vs[0].clone()), NamedFaces::key_of("ipaexm"));
    assert_eq!(as_font(vs[1].clone()), NamedFaces::key_of("ipaexg"));
}

#[test]
fn font_junicode_members_are_the_expected_006_corpus_abbrevs() {
    let src = "@require: font-junicode
(FontJunicode.normal, FontJunicode.bold, FontJunicode.italic, FontJunicode.bold-italic)";
    let v = compile_v01_via_loader_with_metrics("font-junicode-abbrevs", src, &NamedFaces)
        .expect("font-junicode.satyh should compile");
    let vs = as_tuple(v);
    assert_eq!(as_font(vs[0].clone()), NamedFaces::key_of("Junicode"));
    assert_eq!(as_font(vs[1].clone()), NamedFaces::key_of("Junicode-b"));
    assert_eq!(as_font(vs[2].clone()), NamedFaces::key_of("Junicode-it"));
    assert_eq!(as_font(vs[3].clone()), NamedFaces::key_of("Junicode-bi"));
}

#[test]
fn font_latin_modern_math_main_is_the_expected_006_corpus_abbrev() {
    let src = "@require: font-latin-modern-math
FontLatinModernMath.main";
    let v = compile_v01_via_loader_with_metrics("font-lmm-abbrev", src, &NamedFaces)
        .expect("font-latin-modern-math.satyh should compile");
    assert_eq!(as_font(v), NamedFaces::key_of("lmodern"));
}
