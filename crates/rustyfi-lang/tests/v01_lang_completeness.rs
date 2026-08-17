//! Language-completeness sweep: four small V0_1-only gaps surfaced by
//! vendoring real 0.1 stdlib packages (`lib-rustyfi/dist-v01/packages/*`).
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
//! see `rustyfi-syntax`'s `tests/lexer.rs::program_mode_qualified_command`
//! and `tests/cst_v1.rs::document_qualified_command_reference` for the
//! lex/parse-level proof; this file adds the semantic (typecheck+eval)
//! layer on top.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use rustyfi_syntax::cst;
use rustyfi_syntax::leaf::KwIn;
use rustyfi_syntax::{parse_file_v1, RustyfiVersion, Span};

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
    let body =
        lower::lower_document_v1(&doc_file).map_err(|e| format!("lower_document_v1: {e}"))?;
    let eoi = match &doc_file {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => return Err("entry must parse as a V0_1 document".to_string()),
    };
    let file = cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(KwIn(Span::default())),
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
    let mut interp = eval::Interp::new(&NoFonts);
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&elaborated.body, &store))
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
    assert!(as_bool(
        eval_v01("3.0 >. 2.0").expect("should compile and evaluate")
    ));
    assert!(!as_bool(
        eval_v01("2.0 >. 3.0").expect("should compile and evaluate")
    ));
    assert!(as_bool(
        eval_v01("2.0 <. 3.0").expect("should compile and evaluate")
    ));
    assert!(!as_bool(
        eval_v01("3.0 <. 2.0").expect("should compile and evaluate")
    ));
}

#[test]
fn float_greater_or_equal_and_less_or_equal() {
    assert!(as_bool(
        eval_v01("3.0 >=. 3.0").expect("should compile and evaluate")
    ));
    assert!(as_bool(
        eval_v01("3.0 >=. 2.0").expect("should compile and evaluate")
    ));
    assert!(!as_bool(
        eval_v01("2.0 >=. 3.0").expect("should compile and evaluate")
    ));
    assert!(as_bool(
        eval_v01("2.0 <=. 2.0").expect("should compile and evaluate")
    ));
    assert!(as_bool(
        eval_v01("2.0 <=. 3.0").expect("should compile and evaluate")
    ));
    assert!(!as_bool(
        eval_v01("3.0 <=. 2.0").expect("should compile and evaluate")
    ));
}

#[test]
fn float_comparisons_are_unbound_under_v0_0_6() {
    // Confirms these really are V0_1-only registrations, not a shared
    // `Both` retype — mirrors `types_unify.rs`'s
    // `every_v01_only_primitive_has_a_type_under_v0_1_and_none_under_v0_0_6`.
    for name in [">.", "<.", ">=.", "<=."] {
        assert!(
            rustyfi_lang::prim_types::primitive_type_with_version(name, RustyfiVersion::V0_0_6)
                .is_none(),
            "`{name}` must be unbound under V0_0_6"
        );
        assert!(
            rustyfi_lang::prim_types::primitive_type_with_version(name, RustyfiVersion::V0_1)
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
    let v =
        eval_v01("(fun (i, acc) x -> i + acc + x) (1, 2) 3").expect("should compile and evaluate");
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

// ============================================================================
// G9 (vendoring wave 2, `inline.satyh`'s banner): `typecheck.rs`'s
// `PRIMITIVE_NAMES` list omitted `"inline-frame-inner"` even though the
// primitive itself (`primitives.rs`'s `prim_inline_frame_inner`) and its
// type (`prim_types.rs`, identical shape to its already-listed sibling
// `"inline-frame-outer"`) were both already registered for both versions —
// referencing it produced "internal error: unbound variable
// 'inline-frame-inner' reached the typechecker". Fixed by adding the one
// missing name to `PRIMITIVE_NAMES` (see that list's own comment at the fix
// site). `inline.satyh`'s `Inline.frame-inner` member (dropped by the
// vendoring wave that found this gap) is restored alongside this fix.
// ============================================================================

#[test]
fn inline_frame_inner_typechecks_and_evaluates() {
    // `inline-frame-inner : paddings -> deco -> inline-boxes -> inline-boxes`
    // — `deco` is `point -> length -> length -> length -> graphics`; built
    // here with `draw-text`/`inline-nil` (both bare base primitives, no
    // `@require:` needed) purely to exercise the type, since the stand-in
    // body never actually invokes the callback.
    let v = eval_v01(
        "inline-frame-inner (0pt, 0pt, 0pt, 0pt) \
         (fun p w h d -> draw-text (0pt, 0pt) inline-nil) inline-nil",
    )
    .expect("`inline-frame-inner` should now be bound and typecheck (G9 fixed)");
    match v {
        Value::InlineBoxes(_) => {}
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

// ============================================================================
// G10 (vendoring wave 2, `hdecoset.satyh`'s banner): an expression-level
// named `let NAME param* = value in body` was ORIGINALLY reported to only
// accept plain variable params (`Vec<VarTok>`), unlike top-level `val`
// binds and `let rec`, forcing `hdecoset.satyh`/`vdecoset.satyh` to rewrite
// `let decoS (x, y) w h d = …` as `let decoS = fun (x, y) w h d -> …`.
// ALREADY FIXED by the time this sweep ran: the optional-arg-rows
// increments (2/3a) generalized `cst_v1::ast::Param`/`ParamBody` — which
// `Expr::Fun`, `RecClauseV1`, AND `Expr::LetIn` all share as
// `params: Vec<Param>` — to carry a full `PatBot` (`ParamBody::Pat`, plus
// an ascribed-pattern variant), and `v1/lower.rs`'s `lower_param_units` /
// `lower_param_body` / `lower_pat_bot` chain already lowers all three forms
// identically. No `cst_v1`/`lower.rs` edit was needed for this gap —
// confirmed by the tests below, which exercise the ORIGINAL failing shapes
// (wildcard and tuple params on an expression-level named `let`) end to
// end.
// ============================================================================

#[test]
fn let_binding_wildcard_parameter_ignores_its_argument() {
    let v = eval_v01("let f _ = 1 in f 5").expect("`let f _ = …` should compile and evaluate");
    assert_eq!(as_int(v), 1);
}

#[test]
fn let_binding_tuple_destructuring_parameter() {
    let v = eval_v01("let g (a, b) = a in g (3, 4)")
        .expect("`let g (a, b) = …` should compile and evaluate");
    assert_eq!(as_int(v), 3);
}

// ============================================================================
// G11 (vendoring wave 2, `footnote-scheme.satyh`'s test section): a flat
// program containing BOTH a `command \math`-shaped value (built via a
// user-defined `val inline ctx \math m = …` command, the only way this
// harness has to synthesize a real `context`) AND a `+++` (`block-boxes`
// concat) application elsewhere was reported to spuriously fail the `+++`
// site with "type mismatch: expected `int`, found `block-boxes`".
//
// INVESTIGATED, NOT AN ENGINE BUG: the failing minimal repro quoted by the
// original report only reproduces when `\math` is brought into scope via
// `let open V01Mini in` — and `lib-rustyfi/dist-v01/packages/v01-mini.satyh`
// (the ONLY source of a bare, unqualified `\math` binding in this port's
// test fixtures) ALSO defines its own test-only `val (+++) a b = a + b * 2`
// (added for the Sub-slice 2b "`val ( binop )` binds" coverage). `let open
// V01Mini in` therefore shadows the global block-boxes-concatenating
// `+++` with V01Mini's own int-typed one for the rest of that scope —
// textbook `open` shadowing, not a type-inference defect. Confirmed three
// ways below: (1) a `command \math` built WITHOUT opening V01Mini (a fresh
// module that does NOT redefine `+++`) never triggers the symptom; (2) the
// actual blocked scenario — `FootnoteScheme.main` (which itself uses `+++`
// internally) applied to a real context built through `command \math` via
// `V01Mini`, all in one flat program — compiles AND evaluates cleanly
// end-to-end through the real loader (see
// `v01_stdlib_graphics.rs::footnote_scheme_main_with_a_command_math_context_compiles_and_evaluates`,
// which supersedes that file's old "GAP FOUND (G11)" comment); (3) the
// exact literal repro quoted by the report still fails for the diagnosed
// (expected) reason when reproduced verbatim (not asserted here — see (2)
// for the regression proof that actually matters: the real blocked test
// case now works).
// ============================================================================

#[test]
fn command_math_value_does_not_shadow_the_global_plus_plus_plus_operator() {
    // A module that binds a real `\math` command WITHOUT redefining `+++`
    // (unlike `v01-mini.satyh`'s own test-only `val (+++)`) — proves the
    // combination of "a `command \math`-shaped value exists" and "`+++` is
    // used elsewhere" is fine on its own; the G11 report's symptom needs
    // the V01Mini-specific `+++` shadow, not just the two idioms coexisting.
    let lib = "module M = struct
  val inline ctx \\math m = embed-math ctx (read-math ctx m)
end
";
    let src = "let m = command \\M.math in
let cc = block-skip 1pt +++ block-skip 2pt in
get-natural-metrics inline-nil";
    eval_v01_with_lib(lib, src)
        .expect("`command \\M.math` alongside `+++` should typecheck (G11: not an engine bug)");
}
