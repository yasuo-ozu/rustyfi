//! End-to-end coverage for the phase-3 type inferencer: real SATySFi source
//! text run through `parse_file` -> `elaborate::elaborate_program` ->
//! `typecheck::typecheck`, exercising every typing rule against both
//! well-typed and ill-typed programs.

use rustyfi_lang::{elaborate, primitives, typecheck, CompileError};

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
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
// `color` built-in variant (frontend-completion.md §Slice1-B): `Gray of
// float | RGB of (float*float*float) | CMYK of (float*float*float*float)` —
// no base type, no primitive, ordinary `Ast::Ctor`/`Value::Ctor` plumbing.
// ============================================================================

#[test]
fn color_variant_ctors_typecheck() {
    assert_well_typed("let c = Gray 0.5 in c");
    assert_well_typed("let c = RGB (0.5, 0.5, 0.5) in c");
    assert_well_typed("let c = CMYK (0.1, 0.2, 0.3, 0.4) in c");
}

#[test]
fn color_variant_ctors_are_pattern_matchable() {
    assert_well_typed(
        "match Gray 0.5 with
         | Gray(x)      -> x
         | RGB(r, g, b) -> r
         | CMYK(c, m, y, k) -> c",
    );
}

#[test]
fn color_variant_payload_type_mismatch_is_rejected() {
    assert_type_error("RGB (true, 0.5, 0.5)");
}

#[test]
fn color_variant_wrong_ctor_arity_is_rejected() {
    // `Gray` takes exactly one `float` payload, not a 3-tuple.
    assert_type_error("Gray (0.1, 0.2, 0.3)");
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

// `\emph`/`+p` are no longer built-in primitives as of phase 4 (they moved
// to the real `stdja-mini` stdlib package, loaded through the multi-file
// loader) — these tests define local stand-ins with the same shape
// (`context -> inline-text -> inline-boxes` / `.. -> block-boxes`) so the
// typechecking rules they exercise (command-argument unification) are
// unaffected by that move.

#[test]
fn inline_command_with_matching_argument_type_typechecks() {
    assert_well_typed(
        "let-inline ctx \\emph it = read-inline ctx it
         in
         { \\emph{ ok } }",
    );
}

#[test]
fn inline_command_argument_type_mismatch_is_rejected() {
    // `\emph : context -> inline-text -> inline-boxes` — passing a program-
    // mode `int` (via the active-mode `(...)`  escape) instead of
    // inline-text is a type error.
    assert_type_error(
        "let-inline ctx \\emph it = read-inline ctx it
         in
         { \\emph(4); }",
    );
}

#[test]
fn itemize_value_is_not_inline_text() {
    // `{ * a }` elaborates to an `itemize` constructor value, not plain
    // inline-text — applying `+p` (which expects `inline-text`) to it must
    // be rejected, confirming itemize really does get its own nominal type
    // rather than silently degrading to `inline-text`.
    assert_type_error(
        "let-block ctx +p it = line-break true true ctx (read-inline ctx it)
         in
         '< +p { * a } >",
    );
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
    // two-line variant) — see `rustyfi_syntax::span::Span`.
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
        202,
        "keep this in sync with primitives.rs's prims! table and \
         types_unify.rs's every_registered_primitive_has_a_type test \
         (math-split spec §2.2 added 8: read-math, stringify-math, \
         set-math-char, set-math-char-class, get-math-char-class, \
         embed-inline-to-math, get-math-axis-height-ratio, \
         %math-attach-scripts; prim-retype-sweep §2 L5a added 11: <<, >>, \
         band, bor, bxor, bnot, normalize-string-to-nfc, \
         normalize-string-to-nfd, split-grapheme-cluster, read-file, \
         register-document-information; prim-retype-sweep §3 L5b added 2: \
         unite-graphics, clip-graphics-by-path; language-completeness sweep \
         gap 1 added 4: >., <., >=., <=.; G6 (…/tmp/g6-g7-standins.md §1) \
         added 5: load-hyphenation-dictionary, load-unicode-char-database, \
         set-hyphenation-dictionary, set-unicode-char-database, here; G9 \
         added 1: inline-frame-inner — the primitive was already registered \
         in primitives.rs/prim_types.rs, only this list omitted it; \
         docs/plans/design-reflow-s4-lists.md §4.1 added 2: list-mark, \
         inline-mark)"
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

// ============================================================================
// Phase 4/2: user-defined `let-inline`/`let-block` bindings get real command
// types (`MonoType::InlineCmd`/`BlockCmd`, `Checker::command_scheme`) rather
// than being unified as plain "context-curried" functions — and a command
// application (`IText::Cmd`/`BText::Cmd`) is checked against that command
// type's argument list directly (`Checker::check_cmd_args`): exact arity,
// then one unification per argument.
//
// (Polymorphic commands are intentionally not exercised here: every command
// argument this milestone's grammar can produce is a mandatory, monomorphic-
// enough type by construction — `inline-text`/`block-text` literals, or a
// program-mode expression whose own type is unrelated to `command_scheme`'s
// generalization step — so there is no case among these rules that actually
// needs a *quantified* command-argument type variable to exercise; nothing
// here would tell "poly command" apart from "any other command".)
// ============================================================================

#[test]
fn user_defined_inline_command_gets_an_inline_cmd_type_and_applies() {
    assert_well_typed(
        "let-inline ctx \\bracket it = read-inline ctx it
         in
         { \\bracket{ Bracketed text. } }",
    );
}

#[test]
fn user_defined_block_command_gets_a_block_cmd_type_and_applies() {
    assert_well_typed(
        "let-block ctx +box it = read-block ctx it
         let-block ctx +p it = line-break true true ctx (read-inline ctx it)
         in
         '< +box< +p{ Boxed text. } > >",
    );
}

#[test]
fn lightweight_ctx_less_inline_form_still_yields_an_inline_cmd_type() {
    // The ctx-less form elaborates to `Lambda(%context, Lambda(it,
    // read-inline %context it))` (`elaborate_let_inline`'s `None` branch) —
    // structurally just another `context -> inline-text -> inline-boxes`
    // function, so it must be picked up by the very same `command_scheme`
    // peeling as the explicit-context form above.
    assert_well_typed(
        "let-inline \\whisper it = it
         in
         { \\whisper{ hi } }",
    );
}

#[test]
fn inline_command_called_with_too_few_arguments_is_rejected() {
    let err = assert_type_error(
        "let-inline ctx \\pair a b = read-inline ctx a
         in
         { \\pair{x} }",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("expects 2 argument") && msg.contains("got 1"),
        "expected an exact-arity message, got: {msg}"
    );
}

#[test]
fn inline_command_called_with_too_many_arguments_is_rejected() {
    let err = assert_type_error(
        "let-inline ctx \\pair a b = read-inline ctx a
         in
         { \\pair{x}{y}{z} }",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("expects 2 argument") && msg.contains("got 3"),
        "expected an exact-arity message, got: {msg}"
    );
}

#[test]
fn block_command_called_with_wrong_arity_is_rejected() {
    let err = assert_type_error(
        "let-block ctx +duo a b = read-block ctx a
         in
         '< +duo{x} >",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("expects 2 argument") && msg.contains("got 1"),
        "expected an exact-arity message, got: {msg}"
    );
}

#[test]
fn inline_command_argument_type_mismatch_names_the_argument_position() {
    // A custom command (rather than the built-in `\emph`) whose single
    // parameter is `inline-text`, called with a program-mode `int` instead
    // (via the active-mode `(...)` escape) — the message should name the
    // argument position and both types involved, not just "some unify
    // failed somewhere".
    let err = assert_type_error(
        "let-inline ctx \\only it = read-inline ctx it
         in
         { \\only(4); }",
    );
    let msg = err.to_string();
    assert!(msg.contains("argument 1"), "message should name the argument position: {msg}");
    assert!(msg.contains("\\only"), "message should name the command: {msg}");
    assert!(msg.contains("inline-text"), "message should mention `inline-text`: {msg}");
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
}

#[test]
fn emph_given_an_int_is_still_rejected_via_the_command_path() {
    // Regression: `\emph`'s signature moved from a plain `context ->
    // inline-text -> inline-boxes` function (unified against `IText::Cmd`'s
    // whole application) to `MonoType::InlineCmd([inline-text])` (checked
    // argument-by-argument by `check_cmd_args`) — the same ill-typed program
    // that `inline_command_argument_type_mismatch_is_rejected` already
    // covers must still be rejected, now via the new code path, with a
    // message in the new shape. (`\emph` itself moved to the `stdja-mini`
    // stdlib package in phase 4 — see this file's comment above
    // `inline_command_with_matching_argument_type_typechecks` — so it's
    // locally re-declared here with the same shape.)
    let err = assert_type_error(
        "let-inline ctx \\emph it = read-inline ctx it
         in
         { \\emph(4); }",
    );
    let msg = err.to_string();
    assert!(msg.contains("argument 1"), "message should name the argument position: {msg}");
    assert!(msg.contains("inline-text"), "message should mention `inline-text`: {msg}");
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
}

#[test]
fn block_command_argument_type_mismatch_is_rejected() {
    let err = assert_type_error(
        "let-block ctx +only it = read-block ctx it
         in
         '< +only(4); >",
    );
    let msg = err.to_string();
    assert!(msg.contains("argument 1"), "message should name the argument position: {msg}");
    assert!(msg.contains("block-text"), "message should mention `block-text`: {msg}");
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
}

#[test]
fn inline_command_binding_not_context_headed_is_rejected() {
    // `\bad`'s first (and only) lambda-bound parameter is used as an `int`
    // (via `+`) rather than passed through to any context-consuming
    // primitive, so it can never unify with `context` — the binding itself
    // must be rejected, independent of whether `\bad` is ever applied.
    assert_type_error(
        "let-inline ctx \\bad = ctx + 1
         in
         ()",
    );
}

#[test]
fn inline_command_binding_with_wrong_result_type_is_rejected() {
    // `\bad`'s body is a bare `int`, never routed through `read-inline` (or
    // anything else that would force `inline-boxes`) — the peeled result
    // type can't unify with `inline-boxes`.
    assert_type_error(
        "let-inline ctx \\bad it = 4
         in
         ()",
    );
}

#[test]
fn module_exported_inline_command_via_open_still_applies() {
    // `Helper.\shout` (bound by `export_alias` as `LetIn(\"M.\\shout\",
    // Ast::Var(...), ..)`) and then `open`'s own alias-rebinding (another
    // `Ast::Var`-valued `LetIn`) must both be *transparent* to the command
    // type `command_scheme` already gave `\shout` at its original
    // `let-inline` site — see `command_scheme`'s alias branch.
    assert_well_typed(
        "module Helper = struct
           let-inline ctx \\shout it = read-inline ctx it
         end
         in
         open Helper
         in
         { \\shout{ hi } }",
    );
}

#[test]
fn module_qualified_inline_command_reference_has_a_command_type() {
    // Same as above but via the qualified `M.\cmd` form directly, without an
    // intervening `open` — exercises `export_alias`'s own `Ast::Var`-valued
    // `LetIn` in isolation.
    assert_well_typed(
        "module Helper = struct
           let-inline ctx \\shout it = read-inline ctx it
         end
         in
         { \\Helper.shout{ hi } }",
    );
}

// ============================================================================
// Slice 1: raster images (docs/plans/math-images.md). These only exercise
// typechecking — `load-image` is never actually evaluated here, so no real
// file needs to exist on disk (a runtime round trip against a real decoded
// PNG lives in `crates/rustyfi-lang/tests/images.rs`).
// ============================================================================

#[test]
fn image_primitives_typecheck_end_to_end() {
    // `load-image : string -> image`, `use-image-by-width : image -> length
    // -> inline-boxes` — chained together and used where `inline-boxes` is
    // expected (a command argument), exactly like `use-image-by-width
    // (load-image \`fig.png\`) 40pt` would appear in real source.
    assert_well_typed(
        "let-inline ctx \\fig it = use-image-by-width (load-image `fig.png`) 40pt
         in
         { \\fig{ ignored } }",
    );
}

// Slice 1 graphics primitives (docs/plans/graphics-subsystem.md §2/§5): no
// `@require`, no type synonyms (`point` isn't parsed as a synonym yet — see
// the plan's §5) — just the seven new prims' own signatures, exercised by
// inference alone, exactly the "minimal self-contained module" the plan's
// acceptance criterion asks for.
// ============================================================================

#[test]
fn graphics_path_fill_stroke_typecheck() {
    // `point = length * length` unifies against `start-path`/`line-to`'s
    // `point` domain via plain tuple literals (no synonym needed); `fill`/
    // `stroke` both consume the resulting `path` and the built-in `color`
    // variant (`Gray`/`RGB`).
    assert_well_typed(
        "let p = close-with-line (line-to (1pt, 1pt) (start-path (0pt, 0pt))) in
         let g = fill (Gray(0.)) p in
         stroke 1pt (RGB(0., 0., 0.)) p",
    );
}

#[test]
fn use_image_by_width_rejects_a_non_image_first_argument() {
    // `image` is a real, distinct base type — passing an `int` where
    // `use-image-by-width` expects the `image` `load-image` returns must be
    // rejected, not silently accepted via some other type.
    let err = assert_type_error("use-image-by-width 3 40pt");
    let msg = err.to_string();
    assert!(msg.contains("image"), "message should mention `image`: {msg}");
}

#[test]
fn use_image_by_width_rejects_a_non_length_second_argument() {
    assert_type_error("use-image-by-width (load-image `fig.png`) `not-a-length`");
}

#[test]
fn load_image_rejects_a_non_string_argument() {
    assert_type_error("load-image 3");
}

#[test]
fn terminate_path_is_also_a_valid_path_source() {
    assert_well_typed("terminate-path (start-path (0pt, 0pt))");
}

#[test]
fn inline_graphics_callback_typechecks() {
    // `(point -> graphics list)` — a function argument nested inside
    // `inline-graphics`'s arrow chain; the callback here ignores its point
    // argument and returns an empty list, exactly Slice 1's eager-callback
    // shortcut (see `prim_inline_graphics`'s doc comment).
    assert_well_typed("inline-graphics 1pt 1pt 1pt (fun pt -> [])");
}

#[test]
fn fill_rejects_a_non_color_first_argument() {
    assert_type_error("fill 1 (terminate-path (start-path (0pt, 0pt)))");
}

// ============================================================================
// Slice 1: `tabular` + the `cell` variant
// (docs/plans/table-subsystem.md §Slice 1/§5) — a self-contained
// `\tabular`-shaped `let-inline` command, mirroring `tabular.satyh`'s real
// `\tabular` (positional cell builders, no record/option front-end — that's
// roadmap G) exercises `NormalCell`/`MultiCell`/`EmptyCell` inferring
// `cell`, `cellssf cellf multif empty` inferring `(cell list) list`, and
// `tabular … rulef` unifying `rulef` against `length list -> length list ->
// graphics list`.
// ============================================================================

const TABULAR_CMD: &str = "let-inline ctx \\tabular cellssf rulef =
       let pads = (5pt, 5pt, 2pt, 2pt) in
       let cellf it = NormalCell (pads, inline-fil ++ read-inline ctx it ++ inline-fil) in
       let multif n m it = MultiCell (n, m, pads, inline-fil ++ read-inline ctx it ++ inline-fil) in
       let empty = EmptyCell in
         tabular (cellssf cellf multif empty) rulef";

#[test]
fn tabular_command_shape_typechecks_end_to_end() {
    // The trailing `;` closes the lexer's "active area" opened by `\tabular`
    // once its last argument is a program-mode `(...)` value rather than a
    // `{...}`/`<...>` text group (`cst.rs`'s `CmdTail::Args`'s doc comment);
    // it is required here for exactly the same reason `\tabular(...)(...)`
    // needs it in `table.satyh`/`tabular.satyh`'s real front-ends.
    assert_well_typed(&format!(
        "{TABULAR_CMD}
         in
         {{ \\tabular(fun c m e -> [[c{{A}}; c{{B}}]; [e; c{{D}}]])(fun xs ys -> []); }}"
    ));
}

#[test]
fn tabular_rejects_a_rule_callback_with_the_wrong_result_type() {
    // The rule callback must return `graphics list`, not `inline-boxes`.
    let err = assert_type_error(&format!(
        "{TABULAR_CMD}
         in
         {{ \\tabular(fun c m e -> [[c{{A}}]])(fun xs ys -> inline-nil); }}"
    ));
    let _ = err;
}

#[test]
fn multi_cell_ctor_infers_the_cell_type() {
    assert_well_typed(
        "let pads = (0pt, 0pt, 0pt, 0pt) in
         match MultiCell (1, 2, pads, inline-nil) with
         | NormalCell(p, ib) -> p
         | EmptyCell -> pads
         | MultiCell(nr, nc, p, ib) -> p",
    );
}

// ============================================================================
// Slice 1 hooks + cross-references
// (docs/plans/hooks-annotations-crossref.md §Slice 1) — `hook-page-break`'s
// closure argument receives a `page-break-info` closed record row (`{|
// page-number : int |}`) with no nominal type needed, and
// `register-cross-reference`/`get-cross-reference` round-trip through the
// `string option` the built-in `option` variant provides.
// ============================================================================

#[test]
fn hook_page_break_closure_typechecks_against_the_pbinfo_record_row() {
    // `#page-number` structurally unifies the lambda's `pbinfo` parameter
    // against the closed row `hook-page-break` expects — no `tPBINFO`
    // nominal type needed (the plan's §5 point).
    assert_well_typed(
        "hook-page-break (fun pbinfo pt -> register-cross-reference `p` (arabic pbinfo#page-number))",
    );
}

#[test]
fn hook_page_break_rejects_a_closure_missing_the_page_number_field() {
    // A closure that never uses `#page-number` still has to accept an
    // argument shaped like a `page-break-info`; passing one that's used
    // some other way entirely (here, as a `string`) must be rejected.
    let err = assert_type_error(r#"hook-page-break (fun pbinfo pt -> string-length pbinfo)"#);
    let _ = err; // any type error is acceptable; message content isn't pinned.
}

#[test]
fn register_then_get_cross_reference_round_trip_typechecks() {
    assert_well_typed(
        "register-cross-reference `k` `v` before
         (match get-cross-reference `k` with
          | None -> `absent`
          | Some(s) -> s)",
    );
}

#[test]
fn register_cross_reference_rejects_a_non_string_argument() {
    assert_type_error("register-cross-reference `k` 3");
}

#[test]
fn get_cross_reference_rejects_a_non_string_argument() {
    assert_type_error("get-cross-reference 3");
}

// ============================================================================
// Slice 1: the real 4-arg `page-break`
// (docs/plans/document-page-model.md §Slice 1) — `page-content-scheme`
// (`{| text-origin : point; text-height : length |}`) and `page-parts`
// (`{| header-origin; header-content; footer-origin; footer-content |}`)
// are structural closed rows, same as `pbinfo` itself: no nominal scheme
// type needed, just two ordinary `fun pbinfo -> record` closures.
// ============================================================================

#[test]
fn page_break_typechecks_over_two_content_and_parts_scheme_closures() {
    assert_well_typed(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break A4Paper content parts block-nil",
    );
}

#[test]
fn page_break_rejects_a_content_scheme_missing_text_height() {
    let err = assert_type_error(
        "let content pbinfo = (| text-origin = (0pt, 0pt) |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break A4Paper content parts block-nil",
    );
    let _ = err; // any type error is acceptable; message content isn't pinned.
}

#[test]
fn page_break_rejects_a_non_page_first_argument() {
    assert_type_error(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break 3 content parts block-nil",
    );
}

#[test]
fn user_defined_paper_takes_a_length_pair() {
    assert_well_typed(
        "let content pbinfo = (| text-origin = (0pt, 0pt); text-height = 100pt |) in
         let parts pbinfo =
           (| header-origin = (0pt, 0pt); header-content = block-nil;
              footer-origin = (0pt, 0pt); footer-content = block-nil |)
         in
         page-break (UserDefinedPaper(210mm, 297mm)) content parts block-nil",
    );
}

// ============================================================================
// `docs/plans/class-signature-lang-gaps.md` gap 1: first-class command
// values `(command \cmd)` — elaborates to a plain `Var` referencing the
// command's own `let-inline` binding, so it infers exactly that binding's
// `InlineCmd` scheme (`Checker::command_scheme`).
// ============================================================================

#[test]
fn command_value_typechecks_and_unifies_with_another_of_the_same_shape() {
    // Two independently-defined, shape-identical inline commands: their
    // `(command \cmd)` values must unify (both `InlineCmd([inline-text])`),
    // proving `(command \cmd)` really does carry the command's own
    // `MonoType`, not some untyped/opaque placeholder.
    assert_well_typed(
        "let-inline ctx \\m it = read-inline ctx it
         let-inline ctx \\n it = read-inline ctx it
         in
         if true then (command \\m) else (command \\n)",
    );
}

#[test]
fn command_value_of_mismatched_arity_is_rejected() {
    // `\m` is `[inline-text] inline-cmd`, `\pair` is `[inline-text;
    // inline-text] inline-cmd` — pinning that `(command \cmd)`'s type is
    // the *specific* `InlineCmd` argument list, not some generic stand-in
    // that would let any two commands unify.
    assert_type_error(
        "let-inline ctx \\m it = read-inline ctx it
         let-inline ctx \\pair a b = read-inline ctx a
         in
         if true then (command \\m) else (command \\pair)",
    );
}

#[test]
fn command_value_of_an_undefined_command_is_rejected() {
    // `scoped_var`'s ordinary scope check fires for `Atomic::Command` too
    // (elaboration-time, before typechecking ever runs).
    let file = rustyfi_syntax::parse_file("(command \\nonexistent)").unwrap();
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let err = elaborate::elaborate_program(&file, &scope)
        .expect_err("an undefined command reference should be rejected");
    assert!(
        err.to_string().contains("\\nonexistent"),
        "error should name the unbound command: {err}"
    );
}

#[test]
fn get_standard_context_construct_typechecks_with_stand_in_bindings() {
    // Mirrors `stdja.satyh:115-121`'s `get-standard-context`:
    //   let get-standard-context wid =
    //     get-initial-context wid (command \math)
    //       |> set-code-text-command (command \code)
    //       |> ...
    // using *locally*-defined stand-ins for `\math`/`\code`/
    // `get-initial-context`/`set-code-text-command` (restoring the real
    // `get-initial-context` primitive's `[math] inline-cmd` type lives in
    // `prim_types.rs`, out of this wave's file boundary — a sibling agent
    // owns that file concurrently; see `class-signature-lang-gaps.md`'s
    // Slice 1 note on this exact risk). This still proves the *construct*
    // end-to-end: `(command \cmd)` flowing into a `[…] inline-cmd`-typed
    // parameter and back out through a pipe chain.
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         let-inline ctx \\code s = inline-nil
         let stub-get-initial-context wid m = wid
         let stub-set-code-text-command cmd c = c
         let get-standard-context wid =
           stub-get-initial-context wid (command \\math)
             |> stub-set-code-text-command (command \\code)
         in
         get-standard-context 400pt",
    );
}
