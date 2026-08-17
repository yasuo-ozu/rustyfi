//! End-to-end coverage for the match exhaustiveness/redundancy pass
//! (typechecker-completion plan, §Slice 1): real SATySFi source text run
//! through `parse_file` -> `elaborate::elaborate_program` ->
//! `typecheck::typecheck_verbose`, asserting on the `MatchWarning`s it
//! collects. Mirrors `tests/typecheck.rs`'s harness shape.

use satysfi_lang::{elaborate, primitives, typecheck};

fn warnings_for(src: &str) -> Vec<typecheck::MatchWarning> {
    let file = satysfi_syntax::parse_file(src).unwrap_or_else(|e| panic!("parse {src:?}: {e}"));
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)
        .unwrap_or_else(|e| panic!("elaborate {src:?}: {e}"));
    typecheck::typecheck_verbose(&program).unwrap_or_else(|e| panic!("typecheck {src:?}: {e}"))
}

fn non_exhaustive<'a>(warnings: &'a [typecheck::MatchWarning]) -> Vec<&'a typecheck::MatchWarning> {
    warnings
        .iter()
        .filter(|w| w.message.contains("not exhaustive"))
        .collect()
}

fn unreachable<'a>(warnings: &'a [typecheck::MatchWarning]) -> Vec<&'a typecheck::MatchWarning> {
    warnings
        .iter()
        .filter(|w| w.message.contains("unreachable"))
        .collect()
}

fn assert_exhaustive(src: &str) {
    let warnings = warnings_for(src);
    let hits = non_exhaustive(&warnings);
    assert!(
        hits.is_empty(),
        "expected {src:?} to be exhaustive, got: {hits:?}"
    );
}

fn assert_non_exhaustive(src: &str, witness_substr: &str) {
    let warnings = warnings_for(src);
    let hits = non_exhaustive(&warnings);
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one non-exhaustive warning for {src:?}, got: {warnings:?}"
    );
    assert!(
        hits[0].message.contains(witness_substr),
        "expected witness containing {witness_substr:?}, got message: {}",
        hits[0].message
    );
}

fn assert_no_warnings(src: &str) {
    let warnings = warnings_for(src);
    assert!(
        warnings.is_empty(),
        "expected {src:?} to produce no warnings, got: {warnings:?}"
    );
}

fn assert_has_unreachable(src: &str) {
    let warnings = warnings_for(src);
    assert!(
        !unreachable(&warnings).is_empty(),
        "expected {src:?} to warn about an unreachable arm, got: {warnings:?}"
    );
}

// ============================================================================
// bool
// ============================================================================

#[test]
fn bool_both_arms_is_exhaustive() {
    assert_exhaustive("match true with | true -> 1 | false -> 2");
}

#[test]
fn bool_one_arm_is_not_exhaustive() {
    assert_non_exhaustive("match true with | true -> 1", "false");
}

// ============================================================================
// int: never complete without a wildcard/var, however many literals are covered.
// ============================================================================

#[test]
fn int_with_wildcard_is_exhaustive() {
    assert_exhaustive("match 1 with | 1 -> 10 | 2 -> 20 | _ -> 0");
}

#[test]
fn int_without_wildcard_is_not_exhaustive() {
    let warnings = warnings_for("match 1 with | 1 -> 10 | 2 -> 20");
    let hits = non_exhaustive(&warnings);
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one non-exhaustive warning, got: {warnings:?}"
    );
}

// ============================================================================
// option: a built-in two-constructor variant.
// ============================================================================

#[test]
fn option_both_ctors_is_exhaustive() {
    assert_exhaustive(
        "match Some 3 with
         | Some n -> n
         | None -> 0",
    );
}

#[test]
fn option_missing_none_is_not_exhaustive() {
    assert_non_exhaustive("match Some 3 with | Some n -> n", "None");
}

// ============================================================================
// lists: `[]`/`::`.
// ============================================================================

#[test]
fn list_cons_and_empty_is_exhaustive() {
    assert_exhaustive(
        "match [1; 2; 3] with
         | [] -> 0
         | x :: rest -> x",
    );
}

#[test]
fn list_missing_empty_is_not_exhaustive() {
    assert_non_exhaustive("match [1; 2; 3] with | x :: rest -> x", "[]");
}

#[test]
fn nested_cons_pattern_missing_singleton_list() {
    // `x :: y :: rest` only covers lists of length >= 2; combined with `[]`
    // it still leaves length-1 lists uncovered.
    assert_non_exhaustive(
        "match [1; 2; 3] with
         | x :: y :: rest -> x
         | [] -> 0",
        "::",
    );
}

// ============================================================================
// tuples: a gap between two partial arms.
// ============================================================================

#[test]
fn tuple_partial_arms_leave_a_gap() {
    assert_non_exhaustive(
        "match (true, true) with
         | (true, _) -> 1
         | (_, false) -> 2",
        "(false, true)",
    );
}

#[test]
fn tuple_fully_covered_is_exhaustive() {
    assert_exhaustive(
        "match (true, true) with
         | (true, _) -> 1
         | (false, _) -> 2",
    );
}

// ============================================================================
// A user-declared (three-constructor) variant type: the full ctor set comes
// from `Checker::variants`, not the built-ins.
// ============================================================================

#[test]
fn user_variant_all_ctors_is_exhaustive() {
    assert_exhaustive(
        "type shape = | Circle of int | Square of int | Point
         in
         match Circle 3 with
         | Circle r -> r
         | Square s -> s
         | Point -> 0",
    );
}

#[test]
fn user_variant_missing_ctor_is_not_exhaustive() {
    assert_non_exhaustive(
        "type shape = | Circle of int | Square of int | Point
         in
         match Circle 3 with
         | Circle r -> r
         | Square s -> s",
        "Point",
    );
}

// ============================================================================
// `as`-patterns: normalized to their inner pattern for coverage purposes.
// ============================================================================

#[test]
fn as_pattern_does_not_hide_coverage() {
    assert_exhaustive(
        "match Some 3 with
         | Some n as s -> n
         | None -> 0",
    );
}

#[test]
fn as_pattern_missing_arm_still_detected() {
    assert_non_exhaustive(
        "match Some 3 with
         | Some n as s -> n",
        "None",
    );
}

// ============================================================================
// Redundancy: a wildcard arm makes any later arm unreachable.
// ============================================================================

#[test]
fn arm_after_a_wildcard_is_unreachable() {
    let warnings = warnings_for("match true with | _ -> 1 | true -> 2");
    assert!(
        !unreachable(&warnings).is_empty(),
        "expected an unreachable-arm warning, got: {warnings:?}"
    );
    // The leading wildcard already makes this exhaustive — only the
    // redundancy warning should fire.
    assert!(
        non_exhaustive(&warnings).is_empty(),
        "did not expect a non-exhaustive warning too, got: {warnings:?}"
    );
}

#[test]
fn no_redundant_arm_when_every_arm_is_reachable() {
    assert_no_warnings("match true with | true -> 1 | false -> 2");
}

#[test]
fn duplicate_ctor_arm_is_unreachable() {
    assert_has_unreachable(
        "match Some 3 with
         | Some n -> n
         | Some m -> m
         | None -> 0",
    );
}

// ============================================================================
// Guards: a guarded arm never contributes coverage (may fail at runtime), so
// it never counts toward exhaustiveness and never shadows a later arm.
// ============================================================================

#[test]
fn guarded_catchall_still_warns_non_exhaustive() {
    // `_ when true` is syntactically a catch-all but must not count: only
    // `true` is unconditionally covered, so `false` is still missing.
    assert_non_exhaustive("match true with | true -> 1 | _ when true -> 2", "false");
}

#[test]
fn guard_on_a_non_final_arm_does_not_block_exhaustiveness() {
    assert_no_warnings("match true with | true when true -> 1 | true -> 2 | false -> 3");
}
