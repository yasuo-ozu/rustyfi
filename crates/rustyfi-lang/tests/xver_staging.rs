//! Multi-stage evaluation ACROSS the version boundary: what happens when a
//! `code` value — `&e`'s result, `Value::Code` — is built by one SATySFi
//! generation and forced by the other.
//!
//! `crates/rustyfi-lang/tests/staging.rs` and `staging_v1.rs` pin staging
//! within one generation; `xver_import.rs`/`xver_import_reverse.rs` pin
//! cross-version import for ordinary values. Nothing pinned the intersection,
//! and the intersection is where the two mechanisms could disagree: staging
//! DEFERS an expression past the point where it was written, and cross-version
//! import makes "where it was written" decide which generation's primitives
//! that expression means.
//!
//! Three questions, each with its tests below.
//!
//! 1. **Can a 0.0.6 package even PRODUCE a `code`-typed export?** Yes — a
//!    `@stage: 0` library may quote (`staging.rs`'s `a_stage_zero_binding_
//!    may_quote`), and `lib.rs`'s `note_stage` carries that header across the
//!    splice, so the binding really is read at stage 0 inside a 0.1 program
//!    and the 0.1 document reaches it through `~`. `code τ` has no 0.0.6
//!    surface spelling, so such an export is INFERRED, never written — and
//!    therefore invisible to the textual forked-type guard
//!    (`collect_free_globals` -> `xver_adapt::reject_type_names`). That is the
//!    right answer for an inferred one: `Value::Code { body, env }` is one
//!    struct with no version field, shared verbatim by both generations
//!    (`value.rs`), so the representation genuinely is identical.
//!
//!    A WRITTEN `code` is a different matter, and the reason `code` is now in
//!    the refused set for a 0.0.6 producer — see `a_zero_zero_six_package_
//!    that_writes_the_code_type_is_refused` and its control.
//!
//! 1c. **Which ARM reads that 0.0.6-authored text?** Both. Slice X4c's group
//!    below pins the reverse one. `elaborate` hoists every `type` declaration
//!    out of the `Ast` spine into `Program::type_decls`, where no
//!    `Ast::VersionScope` can reach it and one hard-coded-`V0_1` `Checker`
//!    registers it — so a 0.0.6 ENTRY's own declarations, and a native 0.0.6
//!    co-dependency's, are re-read with 0.1's vocabulary exactly as a spliced
//!    0.0.6 dependency's are on the forward arm. `math` therefore takes the
//!    SAME relabel in both directions (there is no mirror to take: the target
//!    vocabulary is `V0_1` either way), and `code`/`page`/`math-text` refuse
//!    with `slice: "X4c"`. The scan is narrower than the forward arm's,
//!    deliberately — `a_zero_zero_six_signature_and_ascription_are_still_not_
//!    guarded` says why.
//!
//! 2. **Is a quoted body version-correct wherever it is forced?** Yes, and the
//!    mechanism is that `Ast::Next`'s compile arm compiles the body EAGERLY,
//!    at the quote's own site, so every free primitive in it constant-folds
//!    against `Compiler::current_version` — the innermost enclosing
//!    `Ast::VersionScope` (`compile.rs`'s `Ast::Next`/`Ast::VersionScope`
//!    arms). The `Value::Code` that results carries an already-resolved
//!    closure, so forcing it from the other generation cannot re-resolve
//!    anything. Both directions are pinned below, each against a same-file
//!    CONTROL that writes the identical expression natively at the forcing
//!    site — the control is what makes the crossing test non-vacuous, because
//!    the two generations' readings of that expression are observably
//!    different (`get-graphics-bbox` returns a bare pair under 0.0.6 and an
//!    `option` under 0.1: `primitives.rs`'s `prim_get_graphics_bbox_v006`
//!    vs `_v01`).
//!
//! 3. **Do `Ast::StageScope` and `Ast::VersionScope` compose?** Yes. They are
//!    applied by the same `elaborate::walk_bindings` pass, `VersionScope`
//!    innermost (`maybe_v006_scope`, inside each arm) and `StageScope` outside
//!    it (`stage_wrap_item`, after the arm) — never the other way round — and
//!    both `elaborate::already_staged` and `typecheck::Checker::binding_stage`
//!    peel the same two wrappers looking for the stage, so they cannot
//!    disagree about which stage a doubly-wrapped binding is at. Pinned
//!    behaviourally: a `@stage: 0` 0.0.6 dependency whose binding is a
//!    `let-rec` inside a `module` puts all THREE wrappers
//!    (`ModuleScope`/`StageScope`/`VersionScope`) on one clause body.
//!
//! **No `@require:` resolution happens here.** Every fixture hands
//! `compile_document_v1`/`compile_document_v006_xver` a `LoadedFile` list it
//! builds itself, with the dependency's `LoadedCst` variant chosen explicitly
//! — so the crossing is structural and cannot silently stop crossing the way
//! a loader-driven fixture can when the other generation's corpus is visible
//! (`CLAUDE.md` §1's trap note).

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::CompileError;
use rustyfi_loader::{LoadedCst, LoadedFile};
use rustyfi_syntax::{parse_file, parse_file_v1, RustyfiVersion};

struct NoFonts;

impl FontMetrics for NoFonts {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size
    }
    fn descender(&self, _f: FontKey, _size: Length) -> Length {
        Length::ZERO
    }
}

fn v006_file(name: &str, src: &str) -> LoadedFile {
    LoadedFile {
        path: std::path::PathBuf::from(name),
        cst: parse_file(src)
            .map(LoadedCst::V0_0)
            .unwrap_or_else(|e| panic!("0.0.6 parse of {name} failed: {e}")),
        origin: Default::default(),
        version: RustyfiVersion::V0_0,
    }
}

fn v01_file(name: &str, src: &str) -> LoadedFile {
    LoadedFile {
        path: std::path::PathBuf::from(name),
        cst: parse_file_v1(src)
            .map(LoadedCst::V0_1)
            .unwrap_or_else(|e| panic!("0.1 parse of {name} failed: {e}")),
        origin: Default::default(),
        version: RustyfiVersion::V0_1,
    }
}

/// The type name of the value the merged program's tail expression evaluated
/// to.
///
/// None of these fixtures builds a real `document` envelope (that would need
/// fonts, a context and a page-break call, none of which this file is about),
/// so a program that gets all the way through the guard, the seal check, the
/// typechecker and evaluation surfaces as `CompileError::NotADocument(t)` —
/// with `t` being exactly `Value::type_name()` of what it did produce. That
/// makes `NotADocument` the ACCEPT signal (the same reading `staging_v1.rs`'s
/// `assert_sealed_accepts` uses) AND the observation: `"tuple"` and
/// `"variant"` below are two different readings of one expression.
fn tail_type(
    r: Result<std::rc::Rc<rustyfi_lang::value::DocumentValue>, CompileError>,
) -> Result<String, CompileError> {
    match r {
        Err(CompileError::NotADocument(t)) => Ok(t.to_string()),
        Err(e) => Err(e),
        Ok(_) => Ok("document".to_string()),
    }
}

/// FORWARD: a 0.1 document with one spliced 0.0.6 dependency.
fn forward(dep_v006: &str, doc_v01: &str) -> Result<String, CompileError> {
    let files = vec![
        v006_file("xdep.satyg", dep_v006),
        v01_file("doc.saty", doc_v01),
    ];
    tail_type(rustyfi_lang::compile_document_v1(&files, &NoFonts))
}

/// FORWARD with no dependency at all — a plain 0.1 document, for the
/// "what does 0.1 itself say" controls.
fn v01_alone(doc_v01: &str) -> Result<String, CompileError> {
    let files = vec![v01_file("doc.saty", doc_v01)];
    tail_type(rustyfi_lang::compile_document_v1(&files, &NoFonts))
}

/// REVERSE: a 0.0.6 document entry with one foreign 0.1 dependency.
fn reverse(dep_v01: &str, doc_v006: &str) -> Result<String, CompileError> {
    let files = vec![
        v01_file("xdep.satyh", dep_v01),
        v006_file("doc.saty", doc_v006),
    ];
    tail_type(rustyfi_lang::compile_document_v006_xver(&files, &NoFonts))
}

/// REVERSE with no dependency — the 0.0.6 entry alone, through the SAME
/// cross-version arm (so its own bindings and tail are still
/// `Ast::VersionScope(V0_0, _)`-wrapped), for the "what does 0.0.6 itself
/// say" controls.
fn v006_alone(doc_v006: &str) -> Result<String, CompileError> {
    let files = vec![v006_file("doc.saty", doc_v006)];
    tail_type(rustyfi_lang::compile_document_v006_xver(&files, &NoFonts))
}

/// REVERSE with a NATIVE 0.0.6 co-dependency alongside the foreign 0.1 one —
/// the other file class the reverse arm splices `Ast::VersionScope(V0_0,
/// _)`-wrapped (`lib.rs`'s `LoadedCst::V0_0` branch, as against the entry's
/// own prelude). Ordered co-dependency-first, as the loader's topological
/// order would deliver it.
fn reverse_with_v006_codep(
    dep_v01: &str,
    codep_v006: &str,
    doc_v006: &str,
) -> Result<String, CompileError> {
    let files = vec![
        v01_file("xdep.satyh", dep_v01),
        v006_file("xcodep.satyg", codep_v006),
        v006_file("doc.saty", doc_v006),
    ];
    tail_type(rustyfi_lang::compile_document_v006_xver(&files, &NoFonts))
}

/// The one expression whose READING differs between the generations without
/// needing a font, a context or a page: `get-graphics-bbox` is version-forked
/// (`primitives.rs`'s `v006`/`v01` rows) with the SAME arity and the same
/// argument type, so both versions accept it — but 0.0.6 answers with the bare
/// `point * point` pair (`Value::Tuple`, `type_name() == "tuple"`) and 0.1 with
/// an `option` of it (`Value::Ctor("Some", ..)`, `type_name() == "variant"`).
///
/// Same text under both grammars: application, tuples, `pt` lengths and the
/// `Gray` colour constructor are spelled identically in 0.0.6 and 0.1.
const BBOX_EXPR: &str = "get-graphics-bbox (fill (Gray 0.5) \
                         (terminate-path (line-to (10pt, 10pt) (start-path (0pt, 0pt)))))";

/// 0.0.6's reading of [`BBOX_EXPR`], and 0.1's — named so a failure message
/// says which generation's primitive actually ran.
const V006_READING: &str = "tuple";
const V01_READING: &str = "variant";

// ---------------------------------------------------------------------------
// Q1: a 0.0.6 package CAN produce a `code`-typed export, and it crosses
// ---------------------------------------------------------------------------

#[test]
fn a_zero_zero_six_stage_zero_export_is_code_the_zero_one_document_can_splice() {
    // The headline reachability answer. Every link in the chain has to hold
    // for this to compile at all: `@stage: 0` survives the splice
    // (`lib.rs`'s `note_stage`), the binding is wrapped in BOTH
    // `Ast::StageScope(Stage0, _)` and `Ast::VersionScope(V0_0, _)` and the
    // typechecker reads the stage through the version wrapper
    // (`Checker::binding_stage`'s peel), `&` is therefore legal on it, and
    // the 0.1 document's `~` reaches a stage-0 binding from stage 1 legally
    // because a splice reads its operand one stage earlier.
    assert_eq!(
        forward("@stage: 0\nlet xstaged = &(1 + 1)\n", "~xstaged").unwrap(),
        "int"
    );
}

#[test]
fn the_crossed_stage_zero_export_is_still_refused_without_the_splice() {
    // The pair that proves the stage genuinely crossed rather than being
    // dropped: if `note_stage` did not reach this binding it would be stage 1
    // like the document, and naming it bare would be perfectly legal (and
    // would hand the document a `code int` it cannot use). It is not.
    let err = forward("@stage: 0\nlet xstaged = &(1 + 1)\n", "xstaged").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid occurrence") && msg.contains("as to stage"),
        "expected a staging-occurrence error, got {msg}"
    );
}

#[test]
fn a_persistent_zero_zero_six_export_is_nameable_from_stage_zero_of_the_document() {
    // The other half of the same wiring, and the one that shows the stage is
    // carried as a VALUE rather than as "stage 0 or nothing".
    //
    // The occurrence is deliberately INSIDE the splice, i.e. at stage 0: a
    // bare `xp` at the document stage would be satisfied by a binding that
    // lost its header on the way in (stage 1 naming stage 1 is legal), so it
    // would prove nothing. Stage 0 naming stage 1 is exactly the cell the
    // matrix refuses, so this compiles only if `Persistent0` really crossed.
    assert_eq!(
        forward(
            "@stage: persistent\nlet xp = 40 + 2\n",
            "~(if xp > 41 then &(1) else &(2))"
        )
        .unwrap(),
        "int"
    );
}

// ---------------------------------------------------------------------------
// Q1b: an INFERRED `code` export crosses; a WRITTEN `code` type does not
// ---------------------------------------------------------------------------

#[test]
fn an_inferred_code_export_is_not_refused_by_the_forked_type_guard() {
    // `code` is in the refused set (the test below), and this is the pair that
    // keeps that from being a blanket refusal of staging across the boundary.
    // The guard is TEXTUAL — it intersects the dependency's export-position
    // type text with `xver_adapt::reject_type_names()` — and `code τ` has no
    // 0.0.6 spelling at all (`staging.rs`'s `the_code_type_has_no_zero_zero_
    // six_spelling`), so an inferred `code` export writes no such text and is
    // correctly untouched. This is a full render-path compile of a dependency
    // whose export type IS `code int`.
    // A TOP-LEVEL `let-rec` is one of the three sites the guard treats as
    // export-position text (`walk_top_binding`'s `LetRec` arm, `boundary =
    // true`), and `xmk`'s inferred type really is `'a -> code int` — so this
    // is a `code` at an export boundary with no `code` anywhere in the text.
    assert_eq!(
        forward(
            "@stage: 0\nlet-rec xmk x = &(1 + 1)\nlet xstaged = xmk 0\n",
            "~xstaged"
        )
        .unwrap(),
        "int"
    );
}

#[test]
fn a_zero_zero_six_package_that_writes_the_code_type_is_refused() {
    // The fork the automatic derivation cannot see. `typecheck::
    // forked_type_names()` builds its set by diffing `name_to_mono` per NAME,
    // which only ever lowers a BARE type atom; `code`'s version gate lives one
    // level up, in `lower_type_app`'s `"code" if .. version.
    // has_code_type_syntax()` arm, where a one-argument application is read as
    // the real `MonoType::Code` under 0.1 and left as the opaque nominal
    // `Variant("code", [τ])` under 0.0.6. So the diff reports nothing, exactly
    // as it reports nothing for `page` (whose fork is in the VALUE rep) — and
    // for the same reason `page` is added to `reject_type_names()` by hand,
    // so is `code`.
    //
    // What it protects: the merged program has ONE `Checker.version`, hard
    // coded to `V0_1` (`v1::module_check::check_program_inner`), so a 0.0.6
    // dependency's `type` declaration text is re-read with 0.1's vocabulary
    // when it crosses. `the_two_readings_of_a_written_code_really_differ`
    // below is the control that they are not the same reading.
    let err = forward("type xholder = | XC of int code\nlet xz = 1\n", "0").unwrap_err();
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "code");
            assert_eq!(slice, "X3");
        }
        other => panic!("expected a cross-version refusal naming `code`, got {other}"),
    }
}

#[test]
fn the_two_readings_of_a_written_code_really_differ() {
    // The control for the refusal above: the SAME 0.0.6 text, typechecked
    // under each generation's vocabulary. Under 0.0.6 `int code` is an opaque
    // nominal that nothing unifies with, so applying the constructor to a real
    // quote is a type error (upstream 0.0.6's manual-type decoder knows only
    // `list` and `ref`, `src/frontend/typeenv.ml:527-530`). Under 0.1 it is
    // the genuine staged type and the same text is accepted.
    //
    // Without this pair the refusal above could be justified by nothing at
    // all; with it, the refusal is exactly "this dependency's own text would
    // change meaning on the way in".
    let src = "type xholder = | XC of int code\nlet xz = XC (&(1)) in 0";
    let stage0 = std::collections::HashMap::from([(1usize, rustyfi_lang::types::Stage::Stage0)]);
    assert!(
        typecheck_v006_text(src, &stage0, RustyfiVersion::V0_0).is_err(),
        "0.0.6 must read `int code` as an opaque nominal"
    );
    typecheck_v006_text(src, &stage0, RustyfiVersion::V0_1)
        .expect("0.1 must read the same text as the real `code` type");
}

/// Elaborate + typecheck one 0.0.6-spelled source under `version`'s type
/// vocabulary — the two readings [`the_two_readings_of_a_written_code_really_
/// differ`] compares. `stages` marks prelude entries exactly as the loader's
/// merge does (`staging.rs`'s own harness).
fn typecheck_v006_text(
    src: &str,
    stages: &std::collections::HashMap<usize, rustyfi_lang::types::Stage>,
    version: RustyfiVersion,
) -> Result<(), String> {
    let file = parse_file(src).map_err(|e| format!("parse: {e}"))?;
    let env = rustyfi_lang::primitives::base_env_with_version(version);
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = rustyfi_lang::elaborate::Scope::new_with_version(&store, env.names(), version);
    let program = rustyfi_lang::elaborate::elaborate_program_with_stages(&file, &scope, stages)
        .map_err(|e| format!("elaborate: {e}"))?;
    rustyfi_lang::typecheck::typecheck_with_version(&program, version).map_err(|e| format!("{e}"))
}

#[test]
fn a_zero_one_dependency_may_still_write_the_code_type() {
    // The refusal above must be DIRECTIONAL, and this is what keeps it so.
    // `code` is only forked when 0.0.6-authored text is read under the merged
    // program's hard-coded `V0_1` `Checker` — a 0.1 dependency's own `code`
    // is already in exactly that vocabulary, so it crosses verbatim and
    // rejecting it would be a pure regression. (Reverse direction: a 0.0.6
    // entry with a foreign 0.1 dependency; `reject_type_names()` is shared by
    // both arms, so a naive addition there would have broken this.)
    assert_eq!(
        reverse(
            "module XM = struct\n  type xholder = | XC of code int\n  val xz = 1\nend\n",
            "XM.xz"
        )
        .unwrap(),
        "int"
    );
}

// ---------------------------------------------------------------------------
// Q1c (Slice X4c): the REVERSE arm guards 0.0.6-authored type text too
// ---------------------------------------------------------------------------
//
// The residual this group replaces (`the_reverse_arm_does_not_guard_zero_zero_
// six_type_text_at_all`) asserted that `int code`, `int math` and `page` alike
// spliced through the reverse arm unchecked. They no longer do.
//
// The misreading is NOT the mirror of the forward arm's — it is the SAME one,
// reached from the other side. `elaborate` hoists every `type` declaration out
// of the `Ast` spine into `Program::type_decls`/`synonym_decls`, so no
// declaration is ever inside an `Ast::VersionScope`, and the one `Checker` that
// registers them all has `version` hard-coded to `V0_1` on BOTH arms
// (`v1::module_check::check_program_inner`). Forward, the 0.0.6-authored text
// re-read under 0.1's vocabulary is a spliced dependency's; reverse, it is the
// ENTRY's own prelude and every native 0.0.6 co-dependency. Same reading,
// different files.
//
// So `math` needs the SAME relabel here, not the mirror one — see
// `a_zero_zero_six_entry_that_writes_math_is_relabeled_not_refused` — and
// everything else in `xver_adapt::reject_type_names_from_v006()` refuses,
// tagged `slice: "X4c"`. The scan is deliberately narrower than the forward
// arm's `collect_free_globals`: see `a_zero_zero_six_signature_and_ascription_
// are_still_not_guarded` for why widening it would refuse ordinary documents.

/// The 0.1 dependency every fixture in this group pairs with. Its only job is
/// to make the compile take the reverse arm at all — a pure 0.0.6 load never
/// reaches `compile_document_v006_xver`.
const XVER_LIB: &str = "module XM = struct\n  val xz = 1\nend\n";

#[test]
fn a_zero_zero_six_entry_that_writes_the_code_type_is_refused() {
    // The reverse twin of `a_zero_zero_six_package_that_writes_the_code_type_
    // is_refused`, and the reason the producer-keyed
    // `reject_type_names_from_v006()` (not the shared `reject_type_names()`)
    // is what this arm consults: `code`'s fork is a property of the text's
    // AUTHOR, not of the crossing. `the_two_readings_of_a_written_code_really_
    // differ` above is the control for both directions at once — it compares
    // the readings, not the arms.
    let err = reverse(
        XVER_LIB,
        "type xh = | XC of int code\nlet xq = 1\nin XM.xz",
    )
    .unwrap_err();
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "code");
            assert_eq!(slice, "X4c");
        }
        other => panic!("expected a cross-version refusal naming `code`, got {other}"),
    }
}

#[test]
fn a_zero_zero_six_entry_that_writes_page_is_refused() {
    // The sharp one. `page`'s bare name lowers to the SAME nominal
    // `Variant("page", [])` under both versions (`typecheck::name_to_mono` has
    // no `page` arm at all), so `forked_type_names()`'s per-name diff cannot
    // see it and the typechecker cannot either — a `page` mismatch is not a
    // type error, it is 0.0.6's 9-constructor `Value::Ctor` meeting 0.1's
    // `length * length` `Value::Product` at runtime. That is exactly why
    // `xver_adapt::reject_type_names()` adds it by hand, and why this arm has
    // to consult that set rather than trust unification to catch anything.
    let err = reverse(XVER_LIB, "type xh = | XC of page\nlet xq = 1\nin XM.xz").unwrap_err();
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, dep } => {
            assert_eq!(name, "page");
            assert_eq!(slice, "X4c");
            assert_eq!(dep, "doc.saty", "the diagnostic must name the 0.0.6 file");
        }
        other => panic!("expected a cross-version refusal naming `page`, got {other}"),
    }
}

#[test]
fn a_zero_zero_six_entry_that_writes_math_text_is_refused() {
    // The other half of the `math` story, and what keeps the relabel below
    // from being a blanket "math-ish names are fine". 0.0.6 has no `math-text`
    // spelling, so in 0.0.6-authored text it is an unrelated OPAQUE user
    // nominal; under the merged program's `V0_1` vocabulary the same word is
    // the real `Base(MathText)`. Nothing can relabel that back into an opaque
    // nominal, so it refuses — as `forked_note` says of it verbatim.
    let err = reverse(XVER_LIB, "type xh = | XC of math-text\nlet xq = 1\nin XM.xz").unwrap_err();
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "math-text");
            assert_eq!(slice, "X4c");
        }
        other => panic!("expected a cross-version refusal naming `math-text`, got {other}"),
    }
}

#[test]
fn a_zero_zero_six_entry_that_writes_math_is_relabeled_not_refused() {
    // The name that crosses, and the test that says WHICH relabel the reverse
    // arm needs. `math` is 0.0.6's undifferentiated math type and lowers to
    // `Base(MathText)` there; under the merged program's hard-coded `V0_1`
    // vocabulary the same word is unrecognized and falls to the nominal
    // `Variant("math", [])`. So the declaration below must be rewritten to
    // `math-text` — the FORWARD arm's relabel, `relabel_type_decls(_, V0_0,
    // V0_1)`, applied unchanged — and NOT the mirror `math-text` -> `math`
    // that `relabel_or_reject_name`'s `(V0_1, V0_0)` arm implements. There is
    // nothing to mirror: the target vocabulary is `V0_1` on both arms.
    //
    // `${x}` is what makes this discriminate. It is a real `math` value
    // (`Base(MathText)`), so `XC` accepts it only if the ctor payload was
    // relabeled; unrelabeled, the payload is an unbound nominal and the
    // application is a unification failure. Before X4c this errored.
    assert_eq!(
        reverse(
            XVER_LIB,
            "type xh = | XC of math\nlet xq = XC ${x}\nin XM.xz",
        )
        .unwrap(),
        "int"
    );
}

#[test]
fn a_native_zero_zero_six_co_dependency_is_guarded_too() {
    // The entry is not the only 0.0.6-authored file on this arm: a 0.0.6 entry
    // may `@require:` ordinary 0.0.6 packages alongside its 0.1 one, and their
    // `type` declarations land in the same `Program::type_decls` under the same
    // `V0_1` `Checker`. `lib.rs`'s `LoadedCst::V0_0` branch runs the identical
    // guard, and the diagnostic names the CO-DEPENDENCY, not the entry.
    let err = reverse_with_v006_codep(
        XVER_LIB,
        "type xh = | XC of page\nlet xq = 1\n",
        "XM.xz",
    )
    .unwrap_err();
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, dep } => {
            assert_eq!(name, "page");
            assert_eq!(slice, "X4c");
            assert_eq!(dep, "xcodep.satyg");
        }
        other => panic!("expected a cross-version refusal naming `page`, got {other}"),
    }
}

#[test]
fn a_zero_zero_six_signature_and_ascription_are_still_not_guarded() {
    // The precision statement, and the reason this arm does NOT reuse the
    // forward arm's `collect_free_globals`.
    //
    // Two 0.0.6 surface forms carry a `TypeExpr` besides a `type` declaration:
    // a `module .. : sig .. end`'s `val` items and a `let-rec`'s `: ty`
    // ascription. `elaborate.rs` parses and then entirely IGNORES both (only a
    // `direct` sig item does anything, and only its NAME), so neither ever
    // reaches a `Checker` and neither can be misread. Collecting from them —
    // which the forward arm does deliberately, as cheap conservatism about one
    // spliced dependency — would here refuse ordinary 0.0.6 documents for text
    // no phase reads: the bundled corpus writes forked names in exactly those
    // positions (`vdecoset.satyh`'s `val paper : deco-set`, `math.satyh`'s
    // `direct \frac : [math; math] math-cmd`, `progsynt.satyh`'s `val to-math
    // : int?-> t -> math`).
    //
    // So both spellings below still cross, with `page` — a name the same file
    // would be refused for writing in a `type` declaration — sitting in each.
    let entry = "module XN : sig\n  val xf : page\nend = struct\n  let xf = 1\nend\n\
                 let-rec xr : page -> int\n  | _ = 1\nin XM.xz";
    assert_eq!(reverse(XVER_LIB, entry).unwrap(), "int");
}

// ---------------------------------------------------------------------------
// Q2: a quoted body keeps the primitives of the generation it was WRITTEN in,
// whichever generation forces it
// ---------------------------------------------------------------------------

#[test]
fn the_two_generations_read_the_bbox_expression_differently() {
    // The premise every test in this section rests on, stated once: written
    // natively at each side, `BBOX_EXPR` really does produce two differently
    // shaped values. If this ever stops holding, the crossing tests below stop
    // discriminating anything and would keep passing while proving nothing.
    assert_eq!(v01_alone(BBOX_EXPR).unwrap(), V01_READING);
    assert_eq!(v006_alone(BBOX_EXPR).unwrap(), V006_READING);
}

#[test]
fn a_quote_written_in_a_zero_zero_six_dependency_keeps_its_own_primitive() {
    // Forward crossing. The quote is written inside a spliced 0.0.6
    // dependency — hence inside `Ast::VersionScope(V0_0, _)` — and forced by
    // a 0.1 document, entirely outside it. `compile.rs`'s `Ast::Next` arm
    // compiles the body THERE, under `current_version == V0_0`, so
    // `get-graphics-bbox` freezes to 0.0.6's `PrimDef` and the forced value is
    // 0.0.6's bare pair, not 0.1's `option`.
    //
    // This is the test that fails if `Ast::Next` ever stops compiling eagerly
    // at the quote site, or compiles it under the ambient version instead of
    // the enclosing `VersionScope`'s.
    assert_eq!(
        forward(&format!("@stage: 0\nlet xbb = &({BBOX_EXPR})\n"), "~xbb").unwrap(),
        V006_READING
    );
}

#[test]
fn a_quote_written_in_a_zero_one_dependency_keeps_its_own_primitive() {
    // Reverse crossing, the mirror: the quote is written in a foreign 0.1
    // dependency (spliced UNWRAPPED — ambient `V0_1`) and forced from a 0.0.6
    // ENTRY, whose whole document tail IS wrapped in `Ast::VersionScope(V0_0,
    // _)` (`compile_document_v006_xver`'s `wrap_body_version`). So the force
    // site is inside a 0.0.6 scope and the quote site is outside one — the
    // exact opposite arrangement of the test above — and the answer is still
    // the quote site's.
    //
    // Together the two directions say the property is about where a quote is
    // WRITTEN, not about which scope happens to be open when `~` runs.
    assert_eq!(
        reverse(
            &format!("module XM = struct\n  val ~xbb = &({BBOX_EXPR})\nend\n"),
            "~XM.xbb"
        )
        .unwrap(),
        V01_READING
    );
}

// ---------------------------------------------------------------------------
// Q3: `Ast::StageScope` and `Ast::VersionScope` on one binding
// ---------------------------------------------------------------------------

#[test]
fn the_file_stage_and_the_version_scope_compose_on_a_module_member() {
    // Three wrappers on one clause body: `Ast::ModuleScope` (a module
    // member's RHS), `Ast::StageScope` (the file's `@stage: 0`) and
    // `Ast::VersionScope` (the spliced 0.0.6 dependency). The `let-rec` arm
    // is the one that builds all three itself, in that nesting, while every
    // other arm gets its `StageScope` from `stage_wrap_item` afterwards — and
    // `stage_wrap_item` skips a binding `already_staged` already covered, so a
    // disagreement between that peel and `Checker::binding_stage`'s would
    // either double-wrap this clause or leave it at the wrong stage. Either
    // way the `&` below stops being legal.
    //
    // The `get-graphics-bbox` payload makes it prove the version wrapper
    // survived the same nesting, not just the stage one.
    assert_eq!(
        forward(
            &format!("@stage: 0\nmodule XM = struct\n  let-rec xf x = &({BBOX_EXPR})\nend\n"),
            "~(XM.xf 0)"
        )
        .unwrap(),
        V006_READING
    );
}

#[test]
fn a_quote_is_still_refused_at_the_document_stage_of_a_crossed_dependency() {
    // The negative that keeps the composition test above honest: WITHOUT the
    // `@stage: 0` header, the identical 0.0.6 dependency is at the document
    // stage and its `&` must be refused. A `VersionScope` wrapper that
    // swallowed the stage — or a `stage_wrap_item` that defaulted a spliced
    // binding to stage 0 — would accept this.
    let err = forward(
        "module XM = struct\n  let-rec xf x = &(1)\nend\n",
        "~(XM.xf 0)",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("only valid at stage 0"),
        "expected a staging error, got {err}"
    );
}
