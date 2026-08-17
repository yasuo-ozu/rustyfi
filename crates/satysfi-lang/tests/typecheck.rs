//! End-to-end coverage for the phase-3 type inferencer: real SATySFi source
//! text run through `parse_file` -> `elaborate::elaborate_program` ->
//! `typecheck::typecheck`, exercising every typing rule against both
//! well-typed and ill-typed programs.

use satysfi_lang::{elaborate, primitives, typecheck, CompileError};

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    Ok(())
}

fn assert_well_typed(src: &str) {
    if let Err(e) = typecheck_str(src) {
        panic!("expected {src:?} to type-check, got error: {e}");
    }
}

fn assert_type_error(src: &str) -> CompileError {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected by the typechecker, but it passed"),
        Err(e @ CompileError::Type(_)) => e,
        Err(other) => panic!("expected {src:?} to fail with a type error, got: {other}"),
    }
}

// ============================================================================
// Basics: literals, arithmetic, if/tuple.
// ============================================================================

#[test]
fn arithmetic_basics_typecheck() {
    assert_well_typed("1 + 2 * 3");
}

#[test]
fn if_then_else_with_tuples_typechecks() {
    assert_well_typed("if true then (1, 2) else (3, 4)");
}

#[test]
fn if_branches_must_unify() {
    assert_type_error("if true then 1 else false");
}

// ============================================================================
// Let-polymorphism vs. lambda-bound monomorphism.
// ============================================================================

#[test]
fn polymorphic_id_used_at_two_types() {
    assert_well_typed(
        "let id = fun x -> x
         in
         (id 1, id true)",
    );
}

#[test]
fn lambda_bound_argument_rejects_polymorphic_use() {
    // `f` is monomorphic inside the lambda body — unlike a `let`-bound name,
    // it is never generalized, so using it at two different types is a
    // type error (classic HM lambda-vs-let distinction).
    assert_type_error("fun f -> (f 1, f true)");
}

#[test]
fn let_rec_mutual_recursion_typechecks() {
    assert_well_typed(
        "let-rec is-even n = if n == 0 then true else is-odd (n - 1)
         and is-odd n = if n == 0 then false else is-even (n - 1)
         in
         is-even 4",
    );
}

// ============================================================================
// Lists.
// ============================================================================

#[test]
fn homogeneous_list_typechecks() {
    assert_well_typed("[1; 2; 3]");
}

#[test]
fn list_with_mixed_element_types_is_rejected() {
    assert_type_error("[1; true]");
}

// ============================================================================
// Records: open-row polymorphism via field access, and missing labels.
// ============================================================================

#[test]
fn open_row_function_applies_to_a_record_with_extra_fields() {
    assert_well_typed("(fun r -> r#a + 1) (| a = 1; b = 2 |)");
}

#[test]
fn record_missing_a_required_label_is_rejected() {
    assert_type_error("(fun r -> r#a) (| b = 1 |)");
}

// ============================================================================
// Constructors: built-in `option`, and a user `type` declaration surfaced by
// `elaborate::elaborate_program`.
// ============================================================================

#[test]
fn builtin_option_ctor_round_trip() {
    assert_well_typed(
        "match Some 3 with
         | Some n -> n
         | None -> 0",
    );
}

#[test]
fn user_variant_round_trip_through_elaborate_program() {
    assert_well_typed(
        "type t = | A | B of int
         in
         match B 3 with
         | A -> 0
         | B n -> n",
    );
}

#[test]
fn user_variant_payload_type_mismatch_is_rejected() {
    assert_type_error(
        "type t = | A | B of int
         in
         B true",
    );
}

// ============================================================================
// Match: arm-type joining and guards.
// ============================================================================

#[test]
fn match_arms_join_to_a_common_type() {
    assert_well_typed(
        "match true with
         | true -> 1
         | false -> 2",
    );
}

#[test]
fn match_arms_that_disagree_in_type_are_rejected() {
    assert_type_error(
        "match true with
         | true -> 1
         | false -> false",
    );
}

#[test]
fn match_guard_must_be_boolean() {
    assert_type_error(
        "match 1 with
         | n when n -> n
         | _ -> 0",
    );
}

// ============================================================================
// Mutable references: the value restriction (no generalization), overwrite,
// while, and sequencing (`before`).
// ============================================================================

#[test]
fn overwrite_well_typed_case() {
    assert_well_typed(
        "let-mutable x <- 0
         in
         (x <- 5)",
    );
}

#[test]
fn overwrite_type_mismatch_is_rejected() {
    assert_type_error(
        "let-mutable x <- 0
         in
         x <- true",
    );
}

#[test]
fn mutable_ref_does_not_generalize_across_overwrites() {
    // The classic ML "value restriction" leak: if `let-mutable`'s binding
    // were (wrongly) generalized the way an ordinary `let` is, `r`'s
    // element type could be instantiated to `int` at the first overwrite
    // and, independently, to `bool` at the second — smuggling both through
    // the very same cell. It must instead stay monomorphic for the whole
    // body, so the second overwrite's `bool` conflicts with the first's
    // `int`.
    assert_type_error(
        "let-mutable r <- []
         in
         ((r <- (1 :: !r)) before (r <- (true :: !r)))",
    );
}

#[test]
fn while_with_boolean_condition_typechecks() {
    assert_well_typed("while false do ()");
}

#[test]
fn while_condition_must_be_boolean() {
    assert_type_error("while 1 do ()");
}

#[test]
fn sequential_well_typed_case() {
    assert_well_typed(
        "let-mutable c <- 0
         in
         ((c <- 5) before !c)",
    );
}

#[test]
fn sequential_requires_a_unit_left_hand_side() {
    assert_type_error("1 before 2");
}

// ============================================================================
// Inline/block commands and itemize.
// ============================================================================

#[test]
fn inline_command_with_matching_argument_type_typechecks() {
    assert_well_typed("{ \\emph{ ok } }");
}

#[test]
fn inline_command_argument_type_mismatch_is_rejected() {
    // `\emph : context -> inline-text -> inline-boxes` — passing a program-
    // mode `int` (via the active-mode `(...)`  escape) instead of
    // inline-text is a type error.
    assert_type_error("{ \\emph(4); }");
}

#[test]
fn itemize_value_is_not_inline_text() {
    // `{ * a }` elaborates to an `itemize` constructor value, not plain
    // inline-text — applying `+p` (which expects `inline-text`) to it must
    // be rejected, confirming itemize really does get its own nominal type
    // rather than silently degrading to `inline-text`.
    assert_type_error("'< +p { * a } >");
}

// ============================================================================
// Display: spot-check that error messages render both types involved.
// ============================================================================

#[test]
fn display_shows_both_types_for_an_arithmetic_mismatch() {
    let err = assert_type_error("1 + true");
    let msg = err.to_string();
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
    assert!(msg.contains("bool"), "message should mention `bool`: {msg}");
}

#[test]
fn display_shows_both_types_for_a_list_mismatch() {
    let err = assert_type_error("[1; true]");
    let msg = err.to_string();
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
    assert!(msg.contains("bool"), "message should mention `bool`: {msg}");
}

#[test]
fn display_includes_a_span_for_an_overwrite_mismatch() {
    let err = assert_type_error(
        "let-mutable x <- 0
         in
         x <- true",
    );
    let msg = err.to_string();
    // `Span`'s `Display` always renders as "line N, characters A-B" (or the
    // two-line variant) — see `satysfi_syntax::span::Span`.
    assert!(
        msg.contains("line"),
        "message should include a source location: {msg}"
    );
}

// ============================================================================
// Sanity: the hand-kept `typecheck::PRIMITIVE_NAMES` list (needed because
// `prim_types::primitive_type` has no way to enumerate its own domain, and
// `primitives.rs`'s `PRIM_DEFS` table is private) stays in sync with
// `primitives.rs`'s actual `prims!` registration table.
// ============================================================================

#[test]
fn primitive_names_are_cross_checked_against_primitives_source() {
    let src = include_str!("../src/primitives.rs");
    assert_eq!(
        typecheck::PRIMITIVE_NAMES.len(),
        43,
        "keep this in sync with primitives.rs's prims! table and \
         types_unify.rs's every_registered_primitive_has_a_type test"
    );
    for name in typecheck::PRIMITIVE_NAMES {
        // Escape backslashes the way they'd actually appear in Rust source
        // text (e.g. the value `\emph` — one backslash — is spelled
        // `"\\emph"` — two backslashes — in `primitives.rs`'s own source).
        let escaped = name.replace('\\', "\\\\");
        let quoted = format!("\"{escaped}\"");
        assert!(
            src.contains(&quoted),
            "primitive `{name}` not found in primitives.rs's source text \
             (PRIMITIVE_NAMES has drifted out of sync)"
        );
    }
}
