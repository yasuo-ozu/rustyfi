//! SATySFi 0.1 value-level labeled optional arguments (optional-arg-rows
//! increment 1): `?(l = e, …)` application bundles and `?(l = x, …)`
//! parameter bundles, end-to-end (parse V0_1 -> `v1::lower` -> `elaborate`
//! (V0_1 scope) -> `typecheck` -> `eval::Interp::eval`), plus the frozen
//! 0.0.6 version-gate. Mirrors `v01_lang_completeness.rs`'s harness, but the
//! elaborate scope is V0_1-tagged so the `?(…)` nodes are accepted.

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use satysfi_syntax::leaf::KwIn;
use satysfi_syntax::{cst, parse_file, parse_file_v1, SatysfiVersion, Span};

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

/// Parse `src` as a V0_1 document body, then run the full elaborate (under a
/// V0_1 scope) -> typecheck -> eval pipeline against V0_1's base environment.
fn eval_v01(src: &str) -> Result<Value, String> {
    let doc_file = parse_file_v1(src).map_err(|e| format!("parse: {e}"))?;
    let body = lower::lower_document_v1(&doc_file).map_err(|e| format!("lower_document_v1: {e}"))?;
    let eoi = match &doc_file {
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("entry must parse as a V0_1 document".to_string()),
    };
    let file = cst::File {
        headers: Vec::new(),
        prelude: Vec::new(),
        in_kw: Some(KwIn(Span::default())),
        body: Some(body),
        eoi,
    };
    let env = primitives::base_env_with_version(SatysfiVersion::V0_1);
    let scope = elaborate::Scope::new_with_version(env.names(), SatysfiVersion::V0_1);
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, SatysfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&NoFonts);
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

/// Parse `src` as a FROZEN 0.0.6 document and elaborate it under a default
/// (V0_0_6) scope — returns the elaboration error text (or the value on the
/// unexpected success). Used to pin the version gate.
fn elaborate_v006_err(src: &str) -> String {
    let file = parse_file(src).unwrap_or_else(|e| panic!("0.0.6 parse of {src:?}: {e}"));
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    match elaborate::elaborate_program(&file, &scope) {
        Ok(_) => panic!("expected a version-gate error, but {src:?} elaborated"),
        Err(e) => e.to_string(),
    }
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

fn as_tuple(v: Value) -> Vec<Value> {
    match v {
        Value::Tuple(xs) => xs,
        other => panic!("expected a tuple, got {other:?}"),
    }
}

// ============================================================================
// T1 — defaulting + subset + label order (a param bundle at a plain `let`).
// ============================================================================

#[test]
fn t1_defaulting_subset_and_order() {
    // `x * scale + bias`, scale defaulting 1, bias defaulting 0.
    let src = "\
let add ?(bias = b, scale = s) x =
  let bv = match b with None -> 0 | Some v -> v end in
  let sv = match s with None -> 1 | Some v -> v end in
  x * sv + bv
in (add 1, add ?(bias = 10) 1, add ?(scale = 3, bias = 1) 2)";
    let vals = as_tuple(eval_v01(src).expect("T1 should compile and evaluate"));
    let got: Vec<i64> = vals.into_iter().map(as_int).collect();
    assert_eq!(
        got,
        vec![1, 11, 7],
        "plain-call default-all; subset (bias only); non-declaration order (scale, bias)"
    );
}

// ============================================================================
// T2 — higher-order / row polymorphism.
// ============================================================================

#[test]
fn t2_higher_order_row_poly() {
    let src = "\
let f ?(bias = b) x = x + (match b with None -> 0 | Some v -> v end) in
let apply-plain g y = g y in
(apply-plain f 5, f ?(bias = 2) 5)";
    let vals = as_tuple(eval_v01(src).expect("T2 should compile and evaluate"));
    let got: Vec<i64> = vals.into_iter().map(as_int).collect();
    // `apply-plain`'s row var instantiates against `f`'s closed `(bias:int)`
    // row; a plain apply of an opt-closure defaults `bias` to `None`.
    assert_eq!(got, vec![5, 7]);
}

// ============================================================================
// T3 — record row polymorphism through the V0_1 surface (pre-existing
// `AccessField` machinery, now pinned reachable alongside opt rows).
// ============================================================================

#[test]
fn t3_record_row_poly() {
    let src = "let getx r = r#x in (getx (| x = 1, y = 2 |)) + (getx (| x = 3, z = 4 |))";
    assert_eq!(as_int(eval_v01(src).expect("T3 should compile and evaluate")), 4);
}

// ============================================================================
// T5 — unknown-label rejection (typecheck error).
// ============================================================================

#[test]
fn t5_unknown_label_rejected() {
    let err = eval_v01("let f ?(a = x) n = n in f ?(b = 1) 2")
        .expect_err("supplying an undeclared optional label `b` must not typecheck");
    assert!(err.starts_with("typecheck:"), "expected a typecheck error, got: {err}");
}

// ============================================================================
// T6 — duplicate-label rejection (elaborate error), app + param bundles.
// ============================================================================

#[test]
fn t6_duplicate_label_rejected() {
    let app_err = eval_v01("let f ?(a = x) n = n in f ?(a = 1, a = 2) 0")
        .expect_err("a duplicate label in one `?(…)` application bundle must be rejected");
    assert!(app_err.contains("duplicate optional label"), "got: {app_err}");

    let param_err = eval_v01("let g = fun ?(a = x, a = y) n -> n in g 0")
        .expect_err("a duplicate label in one `?(…)` binder list must be rejected");
    assert!(param_err.contains("duplicate optional label"), "got: {param_err}");
}

// ============================================================================
// T7 — the frozen 0.0.6 version gate. The additive `cst` nodes make these
// PARSE under 0.0.6 (they used to be parse errors); elaboration then rejects
// them with a version error rather than silently accepting.
// ============================================================================

#[test]
fn t7_v006_version_gate() {
    let fun_err = elaborate_v006_err("let f = fun ?(a = x) p -> p in 0");
    assert!(
        fun_err.contains("SATySFi 0.1 syntax"),
        "0.0.6 must reject a `?(…)` param bundle with a version error, got: {fun_err}"
    );
    let app_err = elaborate_v006_err("let f x = x in f ?(a = 1) 2");
    assert!(
        app_err.contains("SATySFi 0.1 syntax"),
        "0.0.6 must reject a `?(…)` application bundle with a version error, got: {app_err}"
    );
}

// ============================================================================
// T9 — lower placeholders: an empty `?()` bundle is a lower error.
// ============================================================================

#[test]
fn t9_empty_bundle_is_error() {
    let err = eval_v01("let f ?(a = x) n = n in f ?() 2")
        .expect_err("an empty `?()` bundle must be a lower error");
    assert!(err.contains("optional-argument bundle"), "got: {err}");
}

// ============================================================================
// optional-arg-rows increment 2 — ascribed params `( pat : τ )`. The
// ascription's type is parsed but DROPPED (documented carve-out, precedent
// `cst::ast::RecBinding.ascription`'s own parse-and-ignore) — enforcing it
// needs an `Ast`-level ascription node, a typechecker-completion follow-up,
// not this increment.
// ============================================================================

#[test]
fn ascribed_param_pattern_is_accepted() {
    let v = eval_v01("let f (x : int) = x + 1 in f 1").expect("ascribed param should compile");
    assert_eq!(as_int(v), 2);
}

#[test]
fn ascribed_param_takes_a_full_pattern_not_just_a_patbot() {
    // `x :: xs` is a CONS pattern — one level above `patbot` in the grammar
    // (`PatCons`, not reachable as a bare `PatBot`) — proving the ascribed
    // form's `pat` really does route through the full-pattern lowering, not
    // `lower_pat_bot`. (Type `list int` is PREFIX application — SATySFi 0.1's
    // own order — and list literals use `,`, not 0.0.6's `;`.)
    let v = eval_v01("let f (x :: xs : list int) = x in f [1, 2, 3]")
        .expect("a cons-pattern ascription should compile");
    assert_eq!(as_int(v), 1);
}

#[test]
fn ascribed_param_type_is_not_enforced() {
    // A deliberately WRONG ascription (`string`, used with an `int`) is
    // accepted — because the annotation is dropped, not consulted. Pins the
    // documented carve-out.
    let v = eval_v01("let f (x : string) = x + 1 in f 1")
        .expect("the ascription's type is dropped, so this must still compile");
    assert_eq!(as_int(v), 2);
}

// ============================================================================
// optional-arg-rows increment 2 — `?(l : ty) dom -> cod` optional-argument
// TYPE domains (upstream `typ_opt_dom typ_prod ARROW typ`, `parser_v1.mly:688`
// — the `?(…)` prefix is directly followed by the mandatory domain, with NO
// arrow between them; the only arrow is the usual `dom -> cod` one).
// `t_opt_row_fun_type_domain_v006_version_gate` is the type-level analogue of
// T7 (the additive `cst.rs::TypeExpr::OptRowFun` node is version-blind at
// PARSE time — this is new 0.0.6 accept-surface, gated at `typecheck.rs`'s
// `check_type_expr_v0_1_only`, not at parse time).
// ============================================================================

/// A 0.0.6 program declaring a `?(bias : int) int -> int` synonym parses
/// (the additive `cst.rs` node), but `typecheck`'s dual-version `Checker`
/// path (`declare_synonym`) rejects it with a version error rather than
/// silently building a nonsense type — mirrors `type_synonym.rs`'s own
/// harness (`parse_file` -> `elaborate_program` -> `typecheck`).
#[test]
fn t_opt_row_fun_type_domain_v006_version_gate() {
    let file = parse_file("type adder = ?(bias : int) int -> int in 0")
        .unwrap_or_else(|e| panic!("0.0.6 parse failed: {e}"));
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)
        .expect("elaborate carries the raw type decl through unchecked");
    let err = typecheck::typecheck(&program)
        .expect_err("a `?(l:ty)->` type domain must be rejected under 0.0.6");
    assert!(
        err.to_string().contains("SATySFi 0.1 syntax"),
        "expected a version-gate message, got: {err}"
    );
}

/// The same `?(bias : int) int -> int` type declaration, this time inside
/// a V0_1 library — `declare_synonym`'s `check_type_expr_v0_1_only` gate is
/// a no-op once `has_row_polymorphism()` is true, so this compiles cleanly
/// (proving BOTH dual-version entry points — this one and the sealed-sig
/// path `v01_sealing.rs` pins — accept the SAME node under V0_1).
#[test]
fn t_opt_row_fun_type_domain_declares_cleanly_under_v01() {
    let lib_file = parse_file_v1(
        "module M = struct\n\
         type adder = ?(bias : int) int -> int\n\
         val x = 1\n\
         end",
    )
    .unwrap_or_else(|e| panic!("lib parse failed: {e}"));
    let prelude =
        lower::lower_file_v1(&lib_file).unwrap_or_else(|e| panic!("lower lib failed: {e}"));

    let doc_file = parse_file_v1("M.x").unwrap_or_else(|e| panic!("doc parse failed: {e}"));
    let body =
        lower::lower_document_v1(&doc_file).unwrap_or_else(|e| panic!("lower doc failed: {e}"));
    let eoi = match &doc_file {
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => panic!("doc must parse as a V0_1 document"),
    };
    let file = cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(KwIn(Span::default())),
        body: Some(body),
        eoi,
    };
    let env = primitives::base_env_with_version(SatysfiVersion::V0_1);
    let scope = elaborate::Scope::new(env.names());
    let elaborated = elaborate::elaborate_program(&file, &scope)
        .unwrap_or_else(|e| panic!("elaborate failed: {e}"));
    typecheck::typecheck_with_version(&elaborated, SatysfiVersion::V0_1)
        .unwrap_or_else(|e| panic!("expected the `?(bias:int) int -> int` synonym to typecheck under V0_1: {e}"));
}
