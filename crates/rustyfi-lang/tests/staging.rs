//! Multi-stage evaluation: `&e` quotes, `~e` splices, and `@stage:` says which
//! stage a file's bindings are written at.
//!
//! Both halves are pinned here because they fail differently. The TYPE half is
//! a discipline — a quote outside stage 0 is refused, a splice of a non-`code`
//! value is a type error — and the VALUE half is what the quote/splice pair
//! actually computes. A staging bug that only breaks the second half compiles
//! silently and renders the wrong document, which is the failure worth
//! guarding against.

use std::collections::{HashMap, HashSet};

use rustyfi_lang::types::Stage;
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck};
use rustyfi_syntax::RustyfiVersion;

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

/// Typecheck AND evaluate, with per-entry stages — the two halves above run
/// separately (`eval_str` deliberately skips the typechecker so it can hold a
/// code value at stage 1), which is exactly wrong for the question "does an
/// occurrence the stage matrix ACCEPTS also compute the right value?". Nothing
/// but the whole pipeline can answer that.
fn run_str(src: &str, stages: &HashMap<usize, Stage>) -> Result<Value, String> {
    let file = rustyfi_syntax::parse_file(src).map_err(|e| format!("parse: {e}"))?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program_with_stages(&file, &scope, stages)
        .map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_verbose(&program).map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&NoMetrics);
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))
        .map_err(|e| format!("eval: {e}"))
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

/// A `@stage:` header covers the file, so it has to reach EVERY binding shape
/// the file can hold — not just plain `let`.
///
/// This is the silent half of the staging gap: `let-rec`/`let-inline`/
/// `let-block`/`let-math`/`let-mutable` used to be built without ever
/// consulting the file's stage, so a `@stage: 0` library's `let-rec` was read
/// at stage 1 and its `&` refused. Nothing warned; the library simply could
/// not be written.
#[test]
fn a_stage_zero_let_rec_may_quote() {
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Stage0);
    // The quote is CLOSED deliberately. `&(x)` would name `x` -- a parameter
    // bound at stage 0 -- from inside the quote, which is stage 1, and that is
    // independently illegal under the stage-reference matrix (upstream's
    // `typechecker.ml` refuses it the same way). Writing it that way would
    // make this test fail for a reason that has nothing to do with what it is
    // about: whether the FILE's stage reached a `let-rec` at all.
    typecheck_str("let-rec f x = &(1) in 0", &stages).expect("a stage-0 `let-rec` may quote");
}

#[test]
fn a_stage_zero_let_mutable_may_quote() {
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Stage0);
    typecheck_str("let-mutable r <- &(1) in 0", &stages)
        .expect("a stage-0 `let-mutable` may quote");
}

#[test]
fn a_stage_zero_command_binding_may_quote() {
    // `let-inline`/`let-block`/`let-math` all funnel through the same
    // command-binding arms; each is built by its own `walk_bindings` arm, so
    // each needs its own wrap.
    //
    // A BARE `&` is the probe that tells the two apart: legal at stage 0, and
    // "only valid at stage 0" anywhere else. (`~(&e)` would not -- it is legal
    // at stage 1 too, which is exactly how it hides a lost stage.)
    for src in [
        "let-inline \\c = let n = &(1) in { } in 0",
        "let-block +c = let n = &(1) in '< > in 0",
        "let-math \\c = let n = &(1) in ${} in 0",
    ] {
        let mut stages = HashMap::new();
        stages.insert(0usize, Stage::Stage0);
        typecheck_str(src, &stages)
            .unwrap_or_else(|e| panic!("the file's stage must reach this binding: {src} -> {e}"));
    }
}

/// The 0.1 per-binding stage rides on the shared `cst::TopLet`, which made
/// `let ~x = e` parse in a genuine 0.0.6 file too. Upstream 0.0.6 has no such
/// form -- its `EXACT_TILDE` is a splice (`parser.mly:797`) or macro syntax
/// (`:608`, `:1199`), never a binding qualifier -- so elaborating one under
/// 0.0.6 must be refused, not silently honoured.
#[test]
fn a_staged_let_is_rejected_in_a_zero_zero_six_file() {
    // The qualifier sits right after the binding keyword in every shape (it
    // precedes the whole `bind_value` upstream), so `let-inline ~ctx \c` is
    // the 0.0.6-token spelling of 0.1's `val ~inline ctx \c`.
    for src in [
        "let ~x = 1 in 0",
        "let-rec ~f x = x in 0",
        "let-mutable ~r <- 1 in 0",
        "let-inline ~ctx \\c = { } in 0",
        "let-block ~ctx +c = '< > in 0",
        "let-math ~\\c = ${} in 0",
    ] {
        let err = typecheck_str(src, &HashMap::new()).unwrap_err();
        assert!(
            err.contains("SATySFi 0.1 syntax"),
            "expected a version error for {src}, got {err}"
        );
    }
}

/// The `code` TYPE has no 0.0.6 surface spelling, and adding 0.1's must not
/// give it one.
///
/// Upstream 0.0.6's manual-type decoder special-cases exactly `list` and `ref`
/// (`src/frontend/typeenv.ml:527-530`) and reports `UndefinedTypeName` for
/// anything else it cannot find; `code` is added only in 0.1
/// (`dev-0-1-0 src/frontend/manualTypeDecoder.ml:31-36`). So an `int code`
/// annotation here stays an unknown nominal name and does NOT describe the
/// value a `&` produces.
#[test]
fn the_code_type_has_no_zero_zero_six_spelling() {
    let mut stages = HashMap::new();
    stages.insert(1usize, Stage::Stage0);
    let err = typecheck_str("type t = C of int code\nlet x = C (&(1)) in 0", &stages).unwrap_err();
    assert!(
        err.contains("mismatch"),
        "`int code` must not name the code type under 0.0.6, got {err}"
    );
}

#[test]
fn an_unstaged_program_is_unaffected() {
    typecheck_str("let x = 1 + 1 in x", &HashMap::new()).expect("no staging, no change");
    assert!(matches!(eval_str("let x = 1 + 1 in x").unwrap(), Value::Int(2)));
}

// ---------------------------------------------------------------------------
// The occurrence matrix: which stage may NAME a binding of which stage
//
// `&`/`~` police the OPERATORS; this half polices ordinary variable
// references, which upstream refuses independently (`typechecker.ml:667-681`
// in 0.0.6, `:340-353` on `dev-0-1-0` — the two agree on the accept/reject
// split). Nine cells, all nine pinned below, because the cheapest way to
// "pass" a rejection test is to reject everything: five of these tests exist
// only to prove the check is not a blanket refusal, and `an_unstaged_program_
// is_unaffected` above plus the whole rest of the suite pin the ninth
// (stage 1 naming stage 1) at scale.
//
// The harness marks prelude ENTRY INDICES, so entry 0 is the "library"
// binding under test and entry 1 the one that references it; an index absent
// from the map is stage 1, the document's own stage.
// ---------------------------------------------------------------------------

/// The two entries' stages, as `typecheck_str` wants them. `None` means "not
/// marked", i.e. the default stage 1.
fn two_entries(bound: Option<Stage>, user: Option<Stage>) -> HashMap<usize, Stage> {
    let mut stages = HashMap::new();
    if let Some(st) = bound {
        stages.insert(0usize, st);
    }
    if let Some(st) = user {
        stages.insert(1usize, st);
    }
    stages
}

/// `let a = 1` at `bound`, then `let b = a` at `user` — the minimal shape of
/// one occurrence crossing a stage boundary.
fn reference_across(bound: Option<Stage>, user: Option<Stage>) -> Result<(), String> {
    typecheck_str("let a = 1\nlet b = a in 0", &two_entries(bound, user))
}

fn assert_stage_rejected(bound: Option<Stage>, user: Option<Stage>) {
    let err = reference_across(bound, user).expect_err("this occurrence must be refused");
    assert!(
        err.contains("invalid occurrence") && err.contains("as to stage"),
        "expected a staging-occurrence error, got {err}"
    );
}

#[test]
fn a_stage_zero_binding_is_not_nameable_from_stage_one() {
    // The headline gap this closes: before per-binding stages, a document
    // could name (and the evaluator would happily run) a library binding that
    // exists only at the earlier stage.
    assert_stage_rejected(Some(Stage::Stage0), None);
}

#[test]
fn a_stage_one_binding_is_not_nameable_from_stage_zero() {
    // The mirror. A stage-0 library runs BEFORE the document stage, so a
    // stage-1 binding is not merely wrong to name — it does not exist yet.
    assert_stage_rejected(None, Some(Stage::Stage0));
}

#[test]
fn a_stage_zero_binding_is_not_nameable_from_the_persistent_stage() {
    // `persistent` is not a superset of stage 0: it is its own stage, and the
    // only one nameable from everywhere is the one being POINTED AT, never the
    // one pointing.
    assert_stage_rejected(Some(Stage::Stage0), Some(Stage::Persistent0));
}

#[test]
fn a_stage_one_binding_is_not_nameable_from_the_persistent_stage() {
    // Same asymmetry, the other neighbour — this is the rule that makes a
    // `@stage: persistent` library self-contained.
    assert_stage_rejected(None, Some(Stage::Persistent0));
}

#[test]
fn same_stage_references_are_accepted() {
    // Stage 0 -> stage 0 and persistent -> persistent. Without these two the
    // rejections above would be satisfied by refusing every staged reference,
    // which would break `list.satyg` (a real `@stage: persistent` library
    // whose members call each other).
    reference_across(Some(Stage::Stage0), Some(Stage::Stage0))
        .expect("stage 0 may name stage 0");
    reference_across(Some(Stage::Persistent0), Some(Stage::Persistent0))
        .expect("the persistent stage may name itself");
}

#[test]
fn a_persistent_binding_is_nameable_from_every_stage() {
    // The whole point of `persistent`, and the row of the matrix that has to
    // stay open for a document to use `List.map` at all. Upstream compiles
    // these to a `Persistent` node so they survive its stage-1 preprocess
    // pass; this port evaluates every stage in one environment, so an accepted
    // occurrence is just an occurrence.
    reference_across(Some(Stage::Persistent0), None).expect("stage 1 may name persistent");
    reference_across(Some(Stage::Persistent0), Some(Stage::Stage0))
        .expect("stage 0 may name persistent");
    reference_across(Some(Stage::Persistent0), Some(Stage::Persistent0))
        .expect("persistent may name persistent");
}

#[test]
fn a_primitive_is_nameable_from_every_stage() {
    // Primitives are registered `Persistent0` upstream
    // (`primitives.cppo.ml:596`); binding them at the default stage instead
    // would make `+` unreachable from a `@stage: 0` library — a rejection that
    // would look like a staging bug in the library rather than in the port.
    for st in [Stage::Persistent0, Stage::Stage0, Stage::Stage1] {
        let mut stages = HashMap::new();
        stages.insert(0usize, st);
        typecheck_str("let a = 1 + 1 in 0", &stages)
            .unwrap_or_else(|e| panic!("`+` must be nameable at {}: {e}", st.as_str()));
    }
}

#[test]
fn a_staged_binding_may_name_its_own_siblings_through_the_aliases_it_mints() {
    // One source item is not one binding: a module member also mints a
    // qualified alias, and a `let-rec` group mints one per clause. Those
    // aliases are code from the same file and so carry the same stage — if
    // they were left at the default, this (the shape `list.satyg`'s `let
    // reverse lst = fold-left .. lst` has) would be refused for naming its own
    // neighbour.
    let mut stages = HashMap::new();
    for i in 0..2 {
        stages.insert(i, Stage::Persistent0);
    }
    // `M.twice` -> `double`: a `let` member naming a `let-rec` sibling, which
    // is `list.satyg`'s `let reverse lst = fold-left .. lst` exactly.
    // `N.four` -> `M.twice`: a member of one persistent library naming
    // ANOTHER's qualified alias, which is `list.satyg`'s `Option.is-none`
    // use. Only the first is caught by wrapping the member value; the second
    // needs the alias `M.twice` itself to carry the stage.
    typecheck_str(
        "module M : sig val twice : int -> int end = struct\n\
         \x20 let-rec double x = x * 2\n\
         \x20 let twice x = double x\n\
         end\n\
         module N : sig val four : int end = struct\n\
         \x20 let four = M.twice 2\n\
         end\n\
         let n = N.four in n",
        &stages,
    )
    .expect("a persistent module's members may name each other and be named from the document");
}

// ---------------------------------------------------------------------------
// The `(Stage1, Persistent0)` cell, as a VALUE
//
// Upstream does not merely PERMIT that cell: it compiles it to a distinct node
// (`Persistent(rng, evid)`, `typechecker.ml:670-671` in 0.0.6, `:346-347` on
// `dev-0-1-0`) instead of the `ContentOf(rng, evid)` every other accepted cell
// gets. That node exists for one reason, and it is an artefact of upstream's
// code REPRESENTATION rather than of the semantics:
//
//   * upstream's stage-1 pass (`interpret_1`, `evaluator.cppo.ml:429` in 0.0.6,
//     `:609` on `dev-0-1-0`) does not evaluate — it BUILDS a first-order
//     `code_value`, alpha-renaming every binder it walks under into a fresh
//     `CodeSymbol` and resolving an ordinary `ContentOf` through `find_symbol`;
//   * a persistent binding is not one of those binders. It was evaluated at
//     stage 0 (`interpret_bindings_0`'s `Persistent0 | Stage0` arm,
//     `dev-0-1-0 evaluator.cppo.ml:1177-1195`) and lives in the VALUE
//     environment, so `find_symbol` would miss it and upstream would
//     `report_bug_ast "symbol not found"`;
//   * so it is carried through verbatim as `CdPersistent(rng, evid)`
//     (`interpret_1`'s own arm) and `unlift_code` maps it straight back to
//     `ContentOf(rng, evid)` (`types.cppo.ml:1506` in 0.0.6, `:1340` on
//     `dev-0-1-0`) — an ordinary environment lookup, by the SAME `EvalVarID`
//     the typechecker resolved, once the generated code finally runs.
//
// So `Persistent` is capture-avoidance bookkeeping for a first-order code
// representation, plus the escape hatch that lets a persistent name survive a
// pass whose whole job is to replace names. This port has neither: a quote is a
// CLOSURE (`compile.rs`'s `Ast::Next` — the compiled body paired with the
// environment reaching it), every free name in it was already resolved to a
// static slot in the scope the quote was WRITTEN in, and there is no renaming
// pass to survive. The equivalent of `Persistent` here is that closure, and the
// two tests below are what pin it: together they say the accepted cell computes
// the persistent binding's value, and computes it from the quote's scope rather
// than the splice's.
// ---------------------------------------------------------------------------

#[test]
fn a_persistent_binding_named_from_inside_a_quote_evaluates_to_its_value() {
    // The cell, end to end. `&(p)` is read at stage 1 (a quote reads its body
    // one stage later), and `p` is persistent — upstream's `(Stage1,
    // Persistent0)` row, the ONLY one that mints a `Persistent` node on
    // `dev-0-1-0`. The occurrence tests above prove the port accepts it; this
    // proves the acceptance is worth something, which is the half a
    // permission-only implementation gets wrong silently.
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Persistent0);
    stages.insert(1usize, Stage::Stage0);
    let v = run_str("let p = 10\nlet c = &(p) in ~c", &stages)
        .expect("a quote may name a persistent binding");
    assert!(matches!(v, Value::Int(10)), "expected 10, got {v:?}");
}

#[test]
fn a_quoted_persistent_reference_is_not_captured_at_the_splice_site() {
    // The property upstream buys with the `EvalVarID` inside `CdPersistent`,
    // stated as a value: the reference is to the BINDING, not to the name, so a
    // same-named binding in scope where the code is spliced cannot intercept
    // it. Here `~c` is forced under a `p` of 99, once as another top-level
    // (spine) binding and once as a local, and both answers must still be the
    // persistent 10.
    //
    // The port's stand-in for the `EvalVarID` is the compile-time slot: a
    // top-level binding gets its OWN `Globals` slot (`compile.rs`'s
    // `alloc_global`) and the quote body's occurrence is compiled against the
    // slot in scope where the quote was WRITTEN. Give the shadowing binding the
    // same slot instead — the "obvious" name-keyed spine table, which is a
    // shape this codebase has genuinely shipped elsewhere — and both cases
    // below return 99.
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Persistent0);
    stages.insert(1usize, Stage::Stage0);
    for src in [
        "let p = 10\nlet c = &(p)\nlet p = 99 in ~c",
        "let p = 10\nlet c = &(p) in let p = 99 in ~c",
    ] {
        let v = run_str(src, &stages).expect("a quote may name a persistent binding");
        assert!(
            matches!(v, Value::Int(10)),
            "the quote's `p` is the persistent binding, not the splice site's: {src} -> {v:?}"
        );
    }
}

#[test]
fn a_quoted_persistent_reference_survives_being_carried_out_of_scope() {
    // The sharper shape of the same property, and the one upstream's
    // architecture makes non-obvious: the code value is built at stage 0 inside
    // a function, returned, and spliced somewhere the persistent name is
    // shadowed by an unrelated binding of a DIFFERENT TYPE. A name-based
    // resolution would not merely give the wrong number here, it would fail to
    // typecheck or return a string.
    let mut stages = HashMap::new();
    stages.insert(0usize, Stage::Persistent0);
    stages.insert(1usize, Stage::Stage0);
    let v = run_str(
        "let p = 10\nlet c = (let hide = 1 in &(p * 2)) in let p = `no` in ~c",
        &stages,
    )
    .expect("a quote may name a persistent binding");
    assert!(matches!(v, Value::Int(20)), "expected 20, got {v:?}");
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

/// The version gate on `let ~x` is per ITEM, not per file.
///
/// In a cross-version compile the elaborate scope carries ONE version --
/// `V0_1` for both roots (`lib.rs:714`, `:1002`) -- while `ItemOrigins::v006`
/// marks the individual prelude slots a 0.0.6 dependency contributed. A gate
/// that only consulted the scope would let a 0.0.6 library acquire `let ~x`
/// merely by being `@require:`d from a 0.1 document.
#[test]
fn a_staged_let_is_rejected_in_a_spliced_zero_zero_six_dependency() {
    let file = rustyfi_syntax::parse_file("let ~x = 1 in 0").expect("parse");
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new_with_version(&store, env.names(), RustyfiVersion::V0_1);

    // Unmarked: 0.1-authored, and so legal.
    elaborate::elaborate_program_with_versions(
        &file,
        &scope,
        &HashSet::new(),
        &HashMap::new(),
        None,
    )
    .expect("a V0_1-authored `let ~x` elaborates");

    // The same node, now attributed to a spliced 0.0.6 file.
    let err = elaborate::elaborate_program_with_versions(
        &file,
        &scope,
        &HashSet::from([0usize]),
        &HashMap::new(),
        None,
    )
    .expect_err("a V0_0-authored `let ~x` must be refused even under a V0_1 scope")
    .to_string();
    assert!(
        err.contains("SATySFi 0.1 syntax"),
        "expected a version error, got {err}"
    );
}
