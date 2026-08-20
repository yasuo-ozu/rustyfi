//! Multi-stage evaluation: `&e` quotes, `~e` splices, and `@stage:` says which
//! stage a file's bindings are written at.
//!
//! Both halves are pinned here because they fail differently. The TYPE half is
//! a discipline — a quote outside stage 0 is refused, a splice of a non-`code`
//! value is a type error — and the VALUE half is what the quote/splice pair
//! actually computes. A staging bug that only breaks the second half compiles
//! silently and renders the wrong document, which is the failure worth
//! guarding against.

use std::collections::HashMap;

use rustyfi_lang::types::Stage;
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck};

/// Evaluate a whole file's tail expression (no typechecking) — the value half.
fn eval_str(src: &str) -> Result<Value, String> {
    let file = rustyfi_syntax::parse_file(src).map_err(|e| format!("parse: {e}"))?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let ast = elaborate::elaborate(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    let mut interp = eval::Interp::new(&NoMetrics);
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&ast, &store))
        .map_err(|e| format!("eval: {e}"))
}

/// Typecheck a whole file — the discipline half. `stages` marks prelude entries
/// as belonging to a file that declared a non-default `@stage:`, exactly as the
/// loader's merge does.
fn typecheck_str(src: &str, stages: &HashMap<usize, Stage>) -> Result<(), String> {
    let file = rustyfi_syntax::parse_file(src).map_err(|e| format!("parse: {e}"))?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program_with_stages(&file, &scope, stages)
        .map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_verbose(&program)
        .map(|_| ())
        .map_err(|e| format!("{e}"))
}

struct NoMetrics;

impl rustyfi_backend::FontMetrics for NoMetrics {
    fn advance(
        &self,
        _f: rustyfi_backend::FontKey,
        _c: char,
        size: rustyfi_backend::Length,
    ) -> Option<rustyfi_backend::Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: rustyfi_backend::FontKey, size: rustyfi_backend::Length) -> rustyfi_backend::Length {
        size * 0.75
    }
    fn descender(&self, _f: rustyfi_backend::FontKey, size: rustyfi_backend::Length) -> rustyfi_backend::Length {
        size * 0.25
    }
}

// ---------------------------------------------------------------------------
// The value half
// ---------------------------------------------------------------------------

#[test]
fn a_splice_of_a_quote_is_the_quoted_value() {
    assert!(matches!(eval_str("~(&(1 + 1))").unwrap(), Value::Int(2)));
}

#[test]
fn a_quote_is_not_run_when_it_is_built() {
    // If `&` evaluated its body, this would divide by zero rather than
    // producing a value that is simply never forced.
    let v = eval_str("let c = &(1 / 0) in 7").unwrap();
    assert!(matches!(v, Value::Int(7)), "an unforced quote must not run");
}

#[test]
fn a_quote_sees_the_environment_it_was_written_in() {
    // `x` resolves where the quote was WRITTEN (10), not where it is forced
    // (99) -- the property that lets a code value be handed around at all.
    //
    // The doubled quote is what lets this harness HOLD a code value at stage
    // 1: `~(&(&(x)))` forces one level and leaves the other. That makes the
    // program ill-staged, which is fine here and only here -- `eval_str`
    // deliberately skips the typechecker, and the discipline that would refuse
    // it is pinned by its own tests below.
    let v = eval_str("let f = (let x = 10 in ~(&(&(x)))) in let x = 99 in ~f");
    assert!(matches!(v.unwrap(), Value::Int(10)));
}

#[test]
fn quotes_compose_through_a_stage_zero_function() {
    // The macro shape: stage-0 code that builds a function, spliced into the
    // document stage and then applied.
    let v = eval_str("let twice = ~(&( fun s -> ~(&(fun t -> t ^ t)) s )) in twice `ab`");
    assert!(matches!(v.unwrap(), Value::Str(ref s) if s == "abab"));
}

#[test]
fn splicing_a_non_code_value_is_a_runtime_error_not_a_wrong_answer() {
    // The typechecker rejects this first in a real compile; the evaluator must
    // not quietly do something else if it ever gets here.
    let err = eval_str("~(1)").unwrap_err();
    assert!(err.contains("code"), "expected a code-value error, got {err}");
}

// ---------------------------------------------------------------------------
// The discipline half
// ---------------------------------------------------------------------------

#[test]
fn a_quote_at_the_document_stage_is_refused() {
    let err = typecheck_str("let c = &(1) in 0", &HashMap::new()).unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}

#[test]
fn a_splice_at_stage_zero_is_refused() {
    // The mirror of the rule above: a stage-0 binding may quote, and may not
    // splice -- there is no earlier stage for a splice to run at.
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Stage0);
    let err = typecheck_str("let c = ~(1) in 0", &stages).unwrap_err();
    assert!(
        err.contains("only valid at stage 1"),
        "expected a stage-1-only error, got {err}"
    );
}

#[test]
fn a_splice_needs_a_code_value() {
    let err = typecheck_str("let x = ~(1) in 0", &HashMap::new()).unwrap_err();
    assert!(err.contains("code"), "expected a `code` mismatch, got {err}");
}

#[test]
fn a_stage_zero_binding_may_quote() {
    // What `@stage: 0` in a library buys: the merged prelude entry is read at
    // stage 0, so its `&` is legal -- the same text at the default stage is
    // the rejection pinned above.
    let src = "let c = &(1) in 0";
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Stage0);
    typecheck_str(src, &stages).expect("a stage-0 binding may quote");
}

#[test]
fn a_persistent_binding_may_not_quote() {
    // Persistent is reachable from both stages, which is not the same as being
    // stage 0: upstream refuses `&` there too (`typechecker.ml`'s `UTNext`).
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Persistent0);
    let err = typecheck_str("let c = &(1) in 0", &stages).unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}

#[test]
fn the_stage_does_not_leak_past_the_binding_it_was_declared_for() {
    // Entry 0 is the "library" binding (stage 0); entry 1 stands for the
    // document's own, which must still be stage 1 and so must still refuse a
    // quote.
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Stage0);
    let err = typecheck_str("let a = &(1)\nlet b = &(2) in 0", &stages).unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "the second binding is still stage 1: {err}"
    );
}

#[test]
fn an_unstaged_program_is_unaffected() {
    typecheck_str("let x = 1 + 1 in x", &HashMap::new()).expect("no staging, no change");
    assert!(matches!(eval_str("let x = 1 + 1 in x").unwrap(), Value::Int(2)));
}

// ---------------------------------------------------------------------------
// Across generations
// ---------------------------------------------------------------------------

/// A 0.0.6 library's `@stage:` has to survive being spliced into a 0.1
/// document, not just a 0.0.6 one.
///
/// The stage is recorded at the prelude MERGE, and there are three merges (the
/// 0.0-rooted one in the CLI and two cross-version ones here). Miss one and the
/// same library compiles from one generation and is refused from the other —
/// which is exactly what happened when only the first was wired.
#[test]
fn a_stage_header_survives_the_cross_version_splice() {
    // Both merges record `start..end` for the spliced dependency, so a stage-0
    // entry keeps quoting rights wherever it was spliced from.
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Stage0);
    typecheck_str("let c = &(1)\nlet d = 2 in 0", &stages)
        .expect("a spliced stage-0 binding may quote");

    // And the entry AFTER it (the consuming document's own) is untouched.
    let err = typecheck_str("let c = &(1)\nlet d = &(2) in 0", &stages).unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "the consumer's own bindings stay stage 1: {err}"
    );
}
