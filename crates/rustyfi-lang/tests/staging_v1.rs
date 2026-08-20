//! Multi-stage evaluation, SATySFi **0.1** surface: `&e`/`~e` operands and
//! the per-binding stage qualifier `val ~x` / `val persistent ~x`.
//!
//! The 0.0.6 half — the `Stage`/`code ty`/`Next`/`Prev` machinery itself, and
//! the whole-file `@stage:` header that reaches it — is pinned in
//! `staging.rs`. This file pins the 0.1 SURFACE on top of it: 0.1 dropped the
//! `@stage:` header and says the same thing per binding
//! (`parser_v1.mly:417-421`), and it keeps 0.0.6's two operand productions
//! unchanged (`:870-873`). Both were missing from the port's 0.1 grammar
//! entirely, so nothing below could even parse before.
//!
//! Everything here goes through `v1/lower.rs`, which is where the 0.1 halves
//! are joined to the 0.0.6 machinery — a lowering that dropped the stage on
//! the floor would still compile, still typecheck, and quietly run a macro at
//! the wrong stage, so each test states which side of that join it holds
//! down.

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

/// Parse a 0.1 library (`module … = struct … end`, `""` to skip) and a 0.1
/// document body, lower both, then run the real pipeline: elaborate ->
/// typecheck(V0_1) -> eval. The same shape `v01_lang_completeness.rs`'s
/// `eval_v01_with_lib` uses (reproduced locally per that file's own "no
/// shared test-support target" rationale), because every staging property
/// below is a property of the WHOLE pipeline: the parse alone cannot tell a
/// stage-0 binding from a stage-1 one, and the typechecker alone cannot tell
/// whether the lowering ever handed it the stage.
fn compile_v01(lib_src: &str, doc_src: &str) -> Result<Value, String> {
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

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The operand prefixes: `&e` and `~e` in a 0.1 expression
// ---------------------------------------------------------------------------

#[test]
fn a_quote_and_a_splice_round_trip_a_value_in_a_zero_one_file() {
    // The minimum proof that `&`/`~` exist in 0.1 at all: before this slice
    // the 0.1 grammar had no slot for either token, so this was a parse
    // error. `~(&e)` is legal at the document stage (stage 1: a splice is
    // allowed, and it reads its operand one stage earlier, where the quote
    // is), so it exercises both prefixes and both typechecker arms in one
    // expression.
    assert_eq!(as_int(compile_v01("", "~(&(1 + 1))").unwrap()), 2);
}

#[test]
fn a_staged_operand_may_sit_in_an_application_argument() {
    // Upstream puts `&`/`~` on `expr_un`, BELOW `expr_app`
    // (`parser_v1.mly:849-873`), so each argument of an application carries
    // its own prefix independently -- `f ~(&1)` is `f` applied to a spliced
    // quote, never `~(f (&1))`. The port flattens `expr_un`/`expr_app` into
    // one node, so this is the test that the flattening kept the reading:
    // it only passes if the prefix is honoured on the ARGUMENT, not just the
    // head.
    let v = compile_v01("", "(fun x -> x + 1) ~(&41)").unwrap();
    assert_eq!(as_int(v), 42);
}

#[test]
fn a_quote_at_the_document_stage_is_still_refused_in_zero_one() {
    // The discipline is the 0.0.6 typechecker's, reached through the 0.1
    // lowering: if `v1/lower.rs` dropped the prefix instead of lowering it,
    // this would compile happily and evaluate to 1.
    let err = compile_v01("", "&(1)").unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}

// ---------------------------------------------------------------------------
// The per-binding qualifier: `val ~x` / `val persistent ~x`
// ---------------------------------------------------------------------------

#[test]
fn a_stage_zero_val_may_quote() {
    // 0.1's replacement for 0.0.6's `@stage: 0` header, and the whole point
    // of the qualifier: the binding is read at stage 0, so its `&` is legal.
    // The paired rejection is the next test -- the two differ by one `~`.
    compile_v01("module M = struct val ~c = &(1) end", "0").expect("a `val ~x` binding may quote");
}

#[test]
fn a_default_stage_val_may_not_quote() {
    // Same text, no `~`: an ordinary `val` is stage 1 (the document stage),
    // where a quote has no meaning. Without this pair, a lowering that
    // marked EVERY 0.1 binding stage 0 would pass the test above.
    let err = compile_v01("module M = struct val c = &(1) end", "0").unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}

#[test]
fn a_persistent_val_may_not_quote() {
    // `persistent` is reachable from both stages, which is not the same as
    // being stage 0: upstream refuses `&` there too (`typechecker.ml:786-791`
    // reads `Stage1 | Persistent0` as the rejection case). This is the test
    // that `persistent ~x` maps to `Persistent0` and not to `Stage0` -- they
    // are otherwise spelled almost identically and behave identically
    // everywhere else in this file.
    let err = compile_v01("module M = struct val persistent ~c = &(1) end", "0").unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}

#[test]
fn a_stage_zero_val_is_code_the_document_stage_can_splice() {
    // The macro shape end to end, and the reason the qualifier is worth
    // having: a library computes at stage 0 and exports a `code int`; the
    // document splices it. This is the one test here that runs the VALUE
    // half across the binding boundary -- a stage that survived typechecking
    // but was lost before `compile.rs` would fail here, not above.
    let v = compile_v01("module M = struct val ~c = &(1 + 1) end", "~M.c").unwrap();
    assert_eq!(as_int(v), 2);
}

#[test]
fn the_stage_does_not_leak_to_the_next_binding() {
    // Per BINDING, not per file: 0.1 has no `@stage:` header, so a staged
    // `val` must not put its neighbours at stage 0 too. `d`'s quote is the
    // rejection; `c`'s is not.
    let err = compile_v01("module M = struct val ~c = &(1) val d = &(2) end", "0").unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "the following binding is still stage 1: {err}"
    );
}

#[test]
fn an_unstaged_zero_one_program_is_unaffected() {
    // The `Option<BindStageV1>`/`Option<StagePrefix>` fields are tried before
    // the name and before the atom respectively; on unstaged input they must
    // fail at the first token and steal nothing.
    let v = compile_v01("module M = struct val c = 1 + 1 end", "M.c").unwrap();
    assert_eq!(as_int(v), 2);
}

#[test]
fn persistent_is_not_a_keyword_in_zero_zero_six() {
    // The new token is version-gated (`lexer.rs`'s V0_1-only table). A 0.0.6
    // program that uses `persistent` as an ordinary variable name -- which it
    // is entitled to, the word means nothing there -- must keep parsing.
    rustyfi_syntax::parse_file("let persistent = 1 in persistent")
        .expect("`persistent` stays an identifier under 0.0.6");
}
