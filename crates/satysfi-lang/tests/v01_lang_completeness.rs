//! Language-completeness sweep: four small V0_1-only gaps surfaced by
//! vendoring real 0.1 stdlib packages (`lib-satysfi/dist-v01/packages/*`).
//! Each gap is additive (no `types.rs`/`unify.rs` edits) — this file proves
//! each one end-to-end (parse V0_1 -> `v1::lower` -> `elaborate` ->
//! `typecheck` -> `eval::Interp::eval`, the same harness `v01_modules.rs`'s
//! `elaborate_with_lib` and `v01_stdlib.rs`'s `compile_v01_via_loader`
//! already use, reproduced locally per those files' own "no shared
//! test-support library target" rationale):
//!
//!  1. float comparison operators `>.`/`<.`/`>=.`/`<=.` (missing prim
//!     registration — `primitives.rs`'s `prims!` table / `prim_types.rs` /
//!     `typecheck::PRIMITIVE_NAMES`, v01-only, confirmed absent from 0.0.6
//!     upstream by grepping gfngfn/SATySFi's v0.0.6 tag AND its
//!     dev-0-1-0 branch).
//!  2. wildcard `_` as a lambda parameter (`fun _ -> …`).
//!  3. tuple-destructuring lambda parameters (`fun (a, b) -> …`).
//!
//! Gaps 2+3 share one fix: `cst_v1::ast::Expr::Fun.params` widened from
//! `Vec<VarTok>` to `Vec<PatBot>` (the same cross-root DAG edge
//! `RecClauseV1::params` already makes), with `v1/lower.rs`'s `Fun` arm
//! switched to `lower_pat_bot`. `elaborate.rs`'s `c::Expr::Fun` arm already
//! supported arbitrary `PatBot` params (it delegates to the same
//! `rec_clause_value` a `let rec` clause's pattern params use) — `cst.rs`/
//! `elaborate.rs` needed NO change at all, confirming this was purely a
//! `cst_v1` grammar restriction.
//!
//! Gap 4 (`command \Mod.cmd` in program position) is a pure lexer fix —
//! see `satysfi-syntax`'s `tests/lexer.rs::program_mode_qualified_command`
//! and `tests/cst_v1.rs::document_qualified_command_reference` for the
//! lex/parse-level proof; this file adds the semantic (typecheck+eval)
//! layer on top.

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use satysfi_syntax::cst;
use satysfi_syntax::leaf::KwIn;
use satysfi_syntax::{parse_file_v1, SatysfiVersion, Span};

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

/// Parse `src` as a package-free V0_1 document body, then run the full
/// elaborate -> typecheck -> eval pipeline against V0_1's base environment.
/// Mirrors `v01_stdlib.rs`'s `compile_v01_via_loader`, minus the loader
/// (no `@require:` dependency needed for any of these gaps).
fn eval_v01(src: &str) -> Result<Value, String> {
    eval_v01_with_lib("", src)
}

/// Same as [`eval_v01`], but first lowers `lib_src` (a `module … = struct …
/// end` source, parsed as a V0_1 library file) into the prelude — the
/// `v01_modules.rs::elaborate_with_lib` shape, extended through
/// typecheck+eval. Pass `""` for `lib_src` to skip the library entirely.
fn eval_v01_with_lib(lib_src: &str, doc_src: &str) -> Result<Value, String> {
    let mut prelude = Vec::new();
    if !lib_src.is_empty() {
        let lib_file = parse_file_v1(lib_src).map_err(|e| format!("lib parse: {e}"))?;
        prelude = lower::lower_file_v1(&lib_file).map_err(|e| format!("lower_file_v1: {e}"))?;
    }

    let doc_file = parse_file_v1(doc_src).map_err(|e| format!("parse: {e}"))?;
    let body = lower::lower_document_v1(&doc_file).map_err(|e| format!("lower_document_v1: {e}"))?;
    let eoi = match &doc_file {
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("entry must parse as a V0_1 document".to_string()),
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
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, SatysfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&NoFonts);
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

fn as_bool(v: Value) -> bool {
    match v {
        Value::Bool(b) => b,
        other => panic!("expected a bool, got {other:?}"),
    }
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

// ============================================================================
// Gap 1: float comparison operators.
// ============================================================================

#[test]
fn float_greater_than_and_less_than() {
    assert!(as_bool(eval_v01("3.0 >. 2.0").expect("should compile and evaluate")));
    assert!(!as_bool(eval_v01("2.0 >. 3.0").expect("should compile and evaluate")));
    assert!(as_bool(eval_v01("2.0 <. 3.0").expect("should compile and evaluate")));
    assert!(!as_bool(eval_v01("3.0 <. 2.0").expect("should compile and evaluate")));
}

#[test]
fn float_greater_or_equal_and_less_or_equal() {
    assert!(as_bool(eval_v01("3.0 >=. 3.0").expect("should compile and evaluate")));
    assert!(as_bool(eval_v01("3.0 >=. 2.0").expect("should compile and evaluate")));
    assert!(!as_bool(eval_v01("2.0 >=. 3.0").expect("should compile and evaluate")));
    assert!(as_bool(eval_v01("2.0 <=. 2.0").expect("should compile and evaluate")));
    assert!(as_bool(eval_v01("2.0 <=. 3.0").expect("should compile and evaluate")));
    assert!(!as_bool(eval_v01("3.0 <=. 2.0").expect("should compile and evaluate")));
}

#[test]
fn float_comparisons_are_unbound_under_v0_0_6() {
    // Confirms these really are V0_1-only registrations, not a shared
    // `Both` retype — mirrors `types_unify.rs`'s
    // `every_v01_only_primitive_has_a_type_under_v0_1_and_none_under_v0_0_6`.
    for name in [">.", "<.", ">=.", "<=."] {
        assert!(
            satysfi_lang::prim_types::primitive_type_with_version(name, SatysfiVersion::V0_0_6)
                .is_none(),
            "`{name}` must be unbound under V0_0_6"
        );
        assert!(
            satysfi_lang::prim_types::primitive_type_with_version(name, SatysfiVersion::V0_1)
                .is_some(),
            "`{name}` must be bound under V0_1"
        );
    }
}

// ============================================================================
// Gap 2: wildcard `_` as a lambda parameter.
// ============================================================================

#[test]
fn fun_wildcard_parameter_ignores_its_argument() {
    let v = eval_v01("(fun _ -> 42) 7").expect("`fun _ -> …` should compile and evaluate");
    assert_eq!(as_int(v), 42);
}

#[test]
fn fun_wildcard_parameter_mixed_with_a_plain_variable() {
    let v = eval_v01("(fun _ x -> x + 1) 100 6").expect("should compile and evaluate");
    assert_eq!(as_int(v), 7);
}

// ============================================================================
// Gap 3: tuple-destructuring lambda parameters.
// ============================================================================

#[test]
fn fun_tuple_destructuring_parameter() {
    let v = eval_v01("(fun (a, b) -> a + b) (3, 4)").expect("should compile and evaluate");
    assert_eq!(as_int(v), 7);
}

#[test]
fn fun_tuple_destructuring_parameter_mixed_with_a_plain_variable() {
    // The `list.satyg` `mapi-adjacent`-shaped case: a tuple-destructuring
    // accumulator parameter followed by a plain-variable parameter, in one
    // `fun`.
    let v = eval_v01("(fun (i, acc) x -> i + acc + x) (1, 2) 3").expect("should compile and evaluate");
    assert_eq!(as_int(v), 6);
}

// ============================================================================
// Gap 4: `command \Mod.cmd` in program position.
// ============================================================================

/// A `module Mod = struct val inline ctx \cmd m = … end` library, the same
/// `val inline ctx \math m = …` shape `v01-mini.satyh` (line 27) already
/// uses for real — reused here so `\Mod.cmd`'s type is a real `inline-cmd
/// [math-text]`, exactly what `get-initial-context`'s second argument
/// wants.
const CMD_LIB_SRC: &str = "\
module Mod = struct
  val inline ctx \\cmd m = embed-math ctx (read-math ctx m)
end
";

#[test]
fn qualified_command_reference_typechecks_and_evaluates() {
    // `(command \Mod.cmd)` — a module-qualified inline-command reference
    // used as a first-class value (upstream `parser_v1.mly:906-908`'s
    // `backslash_cmd` accepts `LONG_HORZCMD`), passed to
    // `get-initial-context`, the one prim whose signature actually wants an
    // `inline-cmd [math-text]` argument. Reaching a `Value` at all is the
    // proof: the lexer emitted one `HorzCmdWithMod` token instead of
    // splitting on the `.` (gap 4), `elaborate.rs`'s `horz_cmd_key`
    // resolved it against `Mod.\cmd`'s qualified binding key (unchanged,
    // shared with 0.0.6's own `\Mod.cmd` support), and `typecheck`/`eval`
    // accepted it as an ordinary `inline-cmd`-typed value.
    let v = eval_v01_with_lib(
        CMD_LIB_SRC,
        "let ctx = get-initial-context 100pt (command \\Mod.cmd) in
         get-text-width ctx",
    )
    .expect("`(command \\Mod.cmd)` should compile and evaluate");
    // The exact width is incidental (a freshly built context's configured
    // paragraph width) — reaching here at all is the proof.
    let _ = v;
}
