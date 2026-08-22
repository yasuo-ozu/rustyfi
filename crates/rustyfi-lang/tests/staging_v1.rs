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
//!
//! The qualifier applies to EVERY binding shape, not just the plain
//! non-recursive `val`: upstream puts it before the whole `bind_value`
//! (`parser_v1.mly:417-421`), and `bind_value` is what selects between the
//! plain form and `rec`/`mutable`/`inline`/`block`/`math` (`:581-593`). The
//! last group of tests covers the `code τ` TYPE, 0.1's surface spelling of
//! `MonoType::Code` (`dev-0-1-0 src/frontend/manualTypeDecoder.ml:31-36`) —
//! without it a signature cannot describe a staged member at all.

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, v1::lower};
use rustyfi_loader::{LoadedCst, LoadedFile};
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
    // `new_with_version(V0_1)`, not `Scope::new` (which is V0_0): the
    // elaborate scope's version is what gates 0.1-only surface, and the
    // per-binding stage qualifier is now one of those gates -- a 0.0.6-scoped
    // elaboration refuses `val ~x` outright (`elaborate.rs`'s `binding_stage`).
    // This is also what the real 0.1 pipeline does (`lib.rs:714`, `:1002`).
    let scope = elaborate::Scope::new_with_version(&store, env.names(), RustyfiVersion::V0_1);
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck_with_version(&elaborated, RustyfiVersion::V0_1)
        .map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&NoFonts);
    interp
        .eval(&env, &rustyfi_lang::ast::debrand(&elaborated.body, &store))
        .map_err(|e| format!("eval: {e}"))
}

/// Run the FULL public 0.1 pipeline (which, unlike [`compile_v01`], also runs
/// `v1::module_check` — the `:>` seal check, the only consumer that lowers a
/// SIGNATURE's declared type). Same `LoadedFile`/`NotADocument` shape
/// `v01_sealing.rs` uses and for the same reasons: `check_program` is
/// `pub(crate)`, so an integration test reaches it only through
/// `compile_document_v1`, and a program that type-checks but is not a real
/// document envelope surfaces as `NotADocument` — reachable only once the
/// seal check has already accepted it.
/// (`CompileError` is boxed only to keep `clippy::result_large_err` quiet.)
fn compile_v01_sealed(lib_src: &str, doc_src: &str) -> Result<(), Box<rustyfi_lang::CompileError>> {
    let files = vec![
        LoadedFile {
            path: std::path::PathBuf::from("lib.satyh"),
            cst: LoadedCst::V0_1(
                parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}")),
            ),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
        LoadedFile {
            path: std::path::PathBuf::from("doc.saty"),
            cst: LoadedCst::V0_1(
                parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}")),
            ),
            origin: Default::default(),
            version: RustyfiVersion::V0_1,
        },
    ];
    rustyfi_lang::compile_document_v1(&files, &NoFonts)
        .map(|_| ())
        .map_err(Box::new)
}

fn assert_sealed_accepts(lib_src: &str, doc_src: &str) {
    match compile_v01_sealed(lib_src, doc_src) {
        Ok(()) => {}
        Err(e) => match *e {
            rustyfi_lang::CompileError::NotADocument(_) => {}
            other => panic!("expected the seal check to accept, got: {other}"),
        },
    }
}

fn assert_sealed_type_error(lib_src: &str, doc_src: &str) -> String {
    match compile_v01_sealed(lib_src, doc_src) {
        Err(e) => match *e {
            rustyfi_lang::CompileError::Type(t) => t.to_string(),
            other => panic!("expected a type error, got: {other}"),
        },
        Ok(()) => panic!("expected a type error, the program was accepted"),
    }
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
    // The minimum proof that `&`/`~` exist in 0.1 at all: before this
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

// ---------------------------------------------------------------------------
// The occurrence matrix, through the 0.1 qualifier
//
// `staging.rs` pins all nine cells against 0.0.6's whole-file `@stage:`. These
// three pin that the 0.1 PER-BINDING qualifier feeds the same matrix — a
// lowering that recorded the stage well enough for `&`/`~` but dropped it
// before the environment would pass every test above this line and still let a
// document read a stage-0 macro directly.
// ---------------------------------------------------------------------------

#[test]
fn a_stage_zero_val_is_not_nameable_from_the_document_stage() {
    // The same `val ~c` that `~M.c` reaches legally two tests up. Without the
    // splice, this is a stage-1 occurrence of a stage-0 binding, and it is not
    // merely bad style: `c` is a `code int`, so what the document would get is
    // not even the value it looks like it is asking for.
    let err = compile_v01("module M = struct val ~c = &(1 + 1) end", "M.c").unwrap_err();
    assert!(
        err.contains("invalid occurrence") && err.contains("as to stage"),
        "expected a staging-occurrence error, got {err}"
    );
}

#[test]
fn a_persistent_val_is_nameable_from_the_document_stage() {
    // `persistent` earns its keyword here: the SAME reference the test above
    // refuses is legal, because a persistent binding is the one kind nameable
    // from every stage. Both tests are needed — either alone is satisfied by
    // a lowering that maps both qualifiers to the same stage.
    let v = compile_v01("module M = struct val persistent ~c = 40 + 2 end", "M.c").unwrap();
    assert_eq!(as_int(v), 42);
}

#[test]
fn a_document_stage_val_is_not_nameable_from_inside_a_splice() {
    // The reverse crossing, which the 0.1 surface can express in one file: a
    // splice reads its operand at stage 0, where a plain (stage-1) `val` is
    // not yet bound. Upstream refuses it on exactly the same matrix cell.
    //
    // The occurrence has to be refused BEFORE the splice's own `code b`
    // unification — `M.c` is an `int`, so a port that checked the type first
    // would report a type mismatch and hide the staging error underneath it.
    let err = compile_v01("module M = struct val c = 1 end", "~M.c").unwrap_err();
    assert!(
        err.contains("invalid occurrence") && err.contains("as to stage"),
        "expected a staging-occurrence error, got {err}"
    );
}

/// The `(Stage1, Persistent0)` cell is the ONE cell `dev-0-1-0` compiles to a
/// distinct node — `Persistent(rng, evid)` rather than `ContentOf(rng, evid)`
/// (`typechecker.ml:346-347`; 0.0.6 emits it for all three persistent rows,
/// `typechecker.ml:670-671`). `staging.rs`'s own `(Stage1, Persistent0)` block
/// says at length what that node is FOR and why this port's closure-valued
/// quote is its equivalent; this is the 0.1 twin, because the cell is reached
/// through a different surface here (`val persistent ~p`, per binding) and
/// because 0.1 is the generation that RESTRICTED the node to this cell —
/// `interpret_0` reports a bug if it ever sees one (`evaluator.cppo.ml:404-405`).
#[test]
fn a_persistent_val_named_from_inside_a_quote_evaluates_to_its_value() {
    // `val ~c = &(p)`: the quote is at stage 0, its body at stage 1, and `p` is
    // persistent — exactly the cell. The document then splices it.
    let v = compile_v01(
        "module M = struct val persistent ~p = 10  val ~c = &(p) end",
        "~M.c",
    )
    .unwrap();
    assert_eq!(as_int(v), 10);
}

#[test]
fn a_quoted_persistent_val_is_not_captured_at_the_splice_site() {
    // The hygiene half: upstream's `CdPersistent` carries the `EvalVarID`, so
    // no same-named binding at the splice site can intercept it. Here the port
    // must answer 10, not 99 — the quote resolved `p` in the scope it was
    // written in and carried that environment with it.
    let v = compile_v01(
        "module M = struct val persistent ~p = 10  val ~c = &(p) end",
        "let p = 99 in ~M.c",
    )
    .unwrap();
    assert_eq!(as_int(v), 10);
}

#[test]
fn an_unstaged_zero_one_program_is_unaffected() {
    // The `Option<BindStageV1>`/`Option<StagePrefix>` fields are tried before
    // the name and before the atom respectively; on unstaged input they must
    // fail at the first token and steal nothing.
    let v = compile_v01("module M = struct val c = 1 + 1 end", "M.c").unwrap();
    assert_eq!(as_int(v), 2);
}

// ---------------------------------------------------------------------------
// The qualifier on the OTHER binding shapes
// ---------------------------------------------------------------------------
//
// Upstream puts the stage before the whole `bind_value`
// (`parser_v1.mly:417-421`), and `bind_value` is what selects between the
// plain non-recursive form and `rec`/`mutable`/`inline`/`block`/`math`
// (`:581-593`) -- so all six shapes take the prefix, not just the first.

#[test]
fn a_stage_zero_val_rec_may_quote() {
    // Closed quote: `&(x)` would name a stage-0 parameter from inside the
    // quote (stage 1), which the stage-reference matrix refuses in its own
    // right -- see `staging.rs`'s `a_stage_zero_let_rec_may_quote`.
    compile_v01("module M = struct val ~rec f x = &(1) end", "0")
        .expect("a `val ~rec` binding may quote");
}

#[test]
fn a_default_stage_val_rec_may_not_quote() {
    // The pair that proves the prefix is what did it, not `val rec` as such.
    let err = compile_v01("module M = struct val rec f x = &(1) end", "0").unwrap_err();
    assert!(
        err.contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}

#[test]
fn a_stage_zero_val_rec_covers_the_whole_and_chain() {
    // One qualifier, one `UTBindValue(stage, UTRec(binds))` upstream: the
    // second clause is at stage 0 too, so ITS quote is legal as well. A
    // lowering that wrapped only the first clause would fail here.
    compile_v01(
        "module M = struct val ~rec f x = &(1) and g y = &(2) end",
        "0",
    )
    .expect("every clause of a staged `val ~rec` is at that stage");
}

#[test]
fn a_stage_zero_val_mutable_may_quote() {
    compile_v01("module M = struct val ~mutable r <- &(1) end", "0")
        .expect("a `val ~mutable` binding may quote");
}

#[test]
fn a_stage_zero_command_val_may_quote() {
    // `val ~inline` / `val ~block` / `val ~math` -- the three command shapes,
    // each built by its own `walk_bindings` arm and so each needing its own
    // wrap. A bare `&` is the probe (see `staging.rs`'s twin): legal at stage
    // 0 and nowhere else.
    for lib in [
        "module M = struct val ~inline \\c = let n = &(1) in { } end",
        "module M = struct val ~block +c = let n = &(1) in '< > end",
        "module M = struct val ~math ctx \\c = let n = &(1) in read-math ctx ${} end",
    ] {
        compile_v01(lib, "0").unwrap_or_else(|e| panic!("{lib} -> {e}"));
    }
}

#[test]
fn a_default_stage_command_val_may_not_quote() {
    for lib in [
        "module M = struct val inline \\c = let n = &(1) in { } end",
        "module M = struct val block +c = let n = &(1) in '< > end",
        "module M = struct val math ctx \\c = let n = &(1) in read-math ctx ${} end",
    ] {
        let err = compile_v01(lib, "0").unwrap_err();
        assert!(
            err.contains("only valid at stage 0"),
            "expected a staging error for {lib}, got {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// The `code` type, written out
// ---------------------------------------------------------------------------

#[test]
fn a_signature_may_name_the_code_type() {
    // `code τ` is 0.1's surface spelling of `MonoType::Code`
    // (`dev-0-1-0 src/frontend/manualTypeDecoder.ml:31-36`, decoded as a
    // one-argument type application right beside `list` and `ref`). Before
    // this, no `TypeExpr` production yielded `MonoType::Code` at all, so the
    // name fell through to an unknown nominal `Variant("code", [int])` and a
    // signature simply could not describe a staged member.
    assert_sealed_accepts(
        "module M :> sig val ~c : code int end = struct val ~c = &(1) end",
        "0",
    );
}

#[test]
fn the_code_type_is_checked_not_merely_parsed() {
    // The mirror: `code int` must not unify with a `code string`. If the
    // annotation were still an opaque nominal type this would be accepted
    // (nothing would unify with anything), which is the failure mode worth
    // guarding.
    let err = assert_sealed_type_error(
        "module M :> sig val ~c : code string end = struct val ~c = &(1) end",
        "0",
    );
    assert!(
        err.contains("code"),
        "expected the mismatch to name `code`, got {err}"
    );
}

#[test]
fn the_code_type_takes_exactly_one_argument() {
    // A bare `code` (no argument) is not the code type -- upstream reaches
    // its `CodeType` branch only with `[ ty ]` and reports
    // `IllegalNumberOfTypeArguments` otherwise. Here the zero-argument
    // spelling stays an unknown nominal name, so it cannot stand in for
    // `code int`.
    assert_sealed_type_error(
        "module M :> sig val ~c : code end = struct val ~c = &(1) end",
        "0",
    );
}

// ---------------------------------------------------------------------------
// The stage as part of `:>` conformance
//
// A signature declares a member's STAGE as well as its type, and the two are
// checked separately: `sig val ~c : int end = struct val c = 1 end` has
// matching types throughout and still promises something the struct does not
// provide. Upstream checks the stage first and the type second
// (`dev-0-1-0 signatureSubtyping.ml:279-298`); so does this.
// ---------------------------------------------------------------------------

#[test]
fn a_signature_stage_is_enforced_against_the_implementation() {
    // The headline case, and the one that used to slip through: `int` and
    // `int` unify, so a conformance check that compared only types accepted
    // this. It is caught now BECAUSE the stages differ, not incidentally
    // because the types happened to.
    let err = assert_sealed_type_error(
        "module M :> sig val ~c : int end = struct val c = 1 end",
        "0",
    );
    assert!(
        err.contains("stage 1") && err.contains("stage 0"),
        "the error must name both stages, got {err}"
    );
}

#[test]
fn a_matching_stage_pair_is_accepted() {
    // The other direction, without which the test above is satisfied by
    // refusing every staged member: the same signature, now honestly
    // implemented.
    assert_sealed_accepts(
        "module M :> sig val ~c : int end = struct val ~c = 1 end",
        "0",
    );
}

#[test]
fn a_stage_zero_implementation_does_not_satisfy_a_plain_val() {
    // The mirror of the headline case. A plain `val c : int` is a stage-1
    // declaration, so a stage-0 binding under-delivers exactly as a stage-1
    // one does against `val ~c`.
    let err = assert_sealed_type_error(
        "module M :> sig val c : int end = struct val ~c = 1 end",
        "0",
    );
    assert!(
        err.contains("stage 0") && err.contains("stage 1"),
        "the error must name both stages, got {err}"
    );
}

#[test]
fn a_persistent_implementation_satisfies_any_declared_stage() {
    // Stage conformance is a SUBSUMPTION, not an equality: a persistent
    // binding is nameable from every stage, so it delivers whatever a
    // signature asks for (upstream's `(Persistent0, _)` rows). Getting this
    // wrong would make `persistent` unusable in a sealed module — the only
    // place a real 0.1 library writes it.
    for sig in ["val ~c : int", "val persistent ~c : int", "val c : int"] {
        assert_sealed_accepts(
            &format!("module M :> sig {sig} end = struct val persistent ~c = 1 end"),
            "0",
        );
    }
}

#[test]
fn a_stage_zero_implementation_does_not_satisfy_a_persistent_declaration() {
    // The one asymmetry worth pinning: `persistent` in a signature promises
    // the document stage may name the member, which a stage-0 binding cannot
    // honour. So the subsumption above does not run backwards.
    let err = assert_sealed_type_error(
        "module M :> sig val persistent ~c : int end = struct val ~c = 1 end",
        "0",
    );
    assert!(
        err.contains("stage 0") && err.contains("persistent stage"),
        "the error must name both stages, got {err}"
    );
}

#[test]
fn persistent_is_not_a_keyword_in_zero_zero_six() {
    // The new token is version-gated (`lexer.rs`'s V0_1-only table). A 0.0.6
    // program that uses `persistent` as an ordinary variable name -- which it
    // is entitled to, the word means nothing there -- must keep parsing.
    rustyfi_syntax::parse_file("let persistent = 1 in persistent")
        .expect("`persistent` stays an identifier under 0.0.6");
}
