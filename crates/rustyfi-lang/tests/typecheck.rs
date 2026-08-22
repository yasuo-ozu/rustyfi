//! End-to-end coverage for the type inferencer: real SATySFi source
//! text run through `parse_file` -> `elaborate::elaborate_program` ->
//! `typecheck::typecheck`, exercising every typing rule against both
//! well-typed and ill-typed programs.

use rustyfi_lang::{elaborate, primitives, typecheck, CompileError};

fn typecheck_str(src: &str) -> Result<(), CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
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
    // Lambda-bound `f` is monomorphic (never generalized like `let`), so
    // using it at two types is a type error — the classic HM distinction.
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


#[test]
fn homogeneous_list_typechecks() {
    assert_well_typed("[1; 2; 3]");
}

#[test]
fn list_with_mixed_element_types_is_rejected() {
    assert_type_error("[1; true]");
}


#[test]
fn open_row_function_applies_to_a_record_with_extra_fields() {
    assert_well_typed("(fun r -> r#a + 1) (| a = 1; b = 2 |)");
}

#[test]
fn record_missing_a_required_label_is_rejected() {
    assert_type_error("(fun r -> r#a) (| b = 1 |)");
}


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

// `color` built-in variant — no base type, no primitive, just
// ordinary `Ast::Ctor`/`Value::Ctor` plumbing.

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
    // The classic ML "value restriction" leak: if `let-mutable` were
    // (wrongly) generalized like `let`, `r`'s element type could be
    // instantiated to `int` at one overwrite and `bool` at the other,
    // smuggling both through the same cell — it must stay monomorphic.
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


// `\emph`/`+p` live in the `stdja-mini` stdlib package, not the primitive
// table, so these tests use local stand-ins of the same shape instead.

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
    // `\emph : context -> inline-text -> inline-boxes` — the active-mode
    // `(...)` escape passes a program-mode `int` where inline-text is expected.
    assert_type_error(
        "let-inline ctx \\emph it = read-inline ctx it
         in
         { \\emph(4); }",
    );
}

#[test]
fn itemize_value_is_not_inline_text() {
    // `{ * a }` elaborates to an `itemize` value, not inline-text — `+p`
    // (which expects inline-text) applied to it must be rejected.
    assert_type_error(
        "let-block ctx +p it = line-break true true ctx (read-inline ctx it)
         in
         '< +p { * a } >",
    );
}


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
    // `Span::Display` renders "line N, characters A-B" (or the two-line form).
    assert!(
        msg.contains("line"),
        "message should include a source location: {msg}"
    );
}

// Sanity: the hand-kept `typecheck::PRIMITIVE_NAMES` list (needed because
// `prim_types::primitive_type` can't enumerate its own domain, and
// `primitives.rs`'s `PRIM_DEFS` table is private) stays in sync with
// `primitives.rs`'s `prims!` registration table.

#[test]
fn primitive_names_are_cross_checked_against_primitives_source() {
    let src = include_str!("../src/primitives.rs");
    assert_eq!(
        typecheck::PRIMITIVE_NAMES.len(),
        211,
        "real-world-compat round 6 added 3: regexp-of-string, string-match, \
         split-on-regexp (satysfi-base char.satyg / figbox). keep this in sync \
         with primitives.rs's prims! table and \
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
         reflow S4 lists added 2: list-mark, inline-mark; the 0.1 `font` \
         build-out added 1: load-single-font, V0_1-only; the registry-corpus \
         sweep added 2: get-font (vminstdef.yaml:1350, version-forked like \
         set-font) and line-stack-top (:1109), which is what `ruby` and \
         `quotation` were missing)"
    );
    for name in typecheck::PRIMITIVE_NAMES {
        // Escape backslashes as they appear in Rust source text (e.g. one
        // backslash in `\emph` becomes two in `primitives.rs`'s `"\\emph"`).
        let escaped = name.replace('\\', "\\\\");
        let quoted = format!("\"{escaped}\"");
        assert!(
            src.contains(&quoted),
            "primitive `{name}` not found in primitives.rs's source text \
             (PRIMITIVE_NAMES has drifted out of sync)"
        );
    }
}

// User-defined `let-inline`/`let-block` bindings get real command
// types (`MonoType::InlineCmd`/`BlockCmd`, `Checker::command_scheme`), and a
// command application is checked via `Checker::check_cmd_args`: exact arity,
// then one unification per argument. Polymorphic commands aren't exercised
// here — nothing in this port's grammar can tell one apart from another.

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
    // Ctx-less form elaborates to `Lambda(%context, Lambda(it, read-inline
    // %context it))` (`elaborate_let_inline`'s `None` branch) — same shape.
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
    // Distinct from `\emph` (covered below via the command path): the
    // message should name the argument position and both types.
    let err = assert_type_error(
        "let-inline ctx \\only it = read-inline ctx it
         in
         { \\only(4); }",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("argument 1"),
        "message should name the argument position: {msg}"
    );
    assert!(
        msg.contains("\\only"),
        "message should name the command: {msg}"
    );
    assert!(
        msg.contains("inline-text"),
        "message should mention `inline-text`: {msg}"
    );
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
}

#[test]
fn emph_given_an_int_is_still_rejected_via_the_command_path() {
    // Regression: `\emph` is checked as `MonoType::InlineCmd([inline-text])`
    // argument-by-argument (`check_cmd_args`), not by unifying the whole
    // `IText::Cmd` application against one plain function type.
    let err = assert_type_error(
        "let-inline ctx \\emph it = read-inline ctx it
         in
         { \\emph(4); }",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("argument 1"),
        "message should name the argument position: {msg}"
    );
    assert!(
        msg.contains("inline-text"),
        "message should mention `inline-text`: {msg}"
    );
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
    assert!(
        msg.contains("argument 1"),
        "message should name the argument position: {msg}"
    );
    assert!(
        msg.contains("block-text"),
        "message should mention `block-text`: {msg}"
    );
    assert!(msg.contains("int"), "message should mention `int`: {msg}");
}

#[test]
fn inline_command_binding_not_context_headed_is_rejected() {
    // `ctx` is used as an `int` (via `+`), never passed to anything
    // context-consuming, so it can't unify with `context`.
    assert_type_error(
        "let-inline ctx \\bad = ctx + 1
         in
         ()",
    );
}

#[test]
fn inline_command_binding_with_wrong_result_type_is_rejected() {
    // Bare `int` body, never routed through anything forcing `inline-boxes`.
    assert_type_error(
        "let-inline ctx \\bad it = 4
         in
         ()",
    );
}

#[test]
fn module_exported_inline_command_via_open_still_applies() {
    // `export_alias` and `open`'s rebinding are both `Ast::Var`-valued
    // `LetIn`s that must stay transparent to `command_scheme`'s alias branch.
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
    // Same as above via the qualified `M.\cmd` form, without `open`.
    assert_well_typed(
        "module Helper = struct
           let-inline ctx \\shout it = read-inline ctx it
         end
         in
         { \\Helper.shout{ hi } }",
    );
}

// Raster images — typechecking only, `load-image` is never
// evaluated (no file needs to exist on disk); the runtime round trip
// against a real decoded PNG lives in `tests/images.rs`.

#[test]
fn image_primitives_typecheck_end_to_end() {
    assert_well_typed(
        "let-inline ctx \\fig it = use-image-by-width (load-image `fig.png`) 40pt
         in
         { \\fig{ ignored } }",
    );
}

// Graphics primitives: no `@require`, no type synonyms — `point`
// isn't parsed as a synonym yet, just the seven new prims' own signatures.

#[test]
fn graphics_path_fill_stroke_typecheck() {
    // `point = length * length` unifies via plain tuple literals (no
    // synonym needed); `fill`/`stroke` consume the resulting `path`.
    assert_well_typed(
        "let p = close-with-line (line-to (1pt, 1pt) (start-path (0pt, 0pt))) in
         let g = fill (Gray(0.)) p in
         stroke 1pt (RGB(0., 0., 0.)) p",
    );
}

#[test]
fn use_image_by_width_rejects_a_non_image_first_argument() {
    // `image` is a real, distinct base type, not an alias for anything else.
    let err = assert_type_error("use-image-by-width 3 40pt");
    let msg = err.to_string();
    assert!(
        msg.contains("image"),
        "message should mention `image`: {msg}"
    );
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
    // `(point -> graphics list)` callback — the eager-callback
    // shortcut (see `prim_inline_graphics`'s doc comment).
    assert_well_typed("inline-graphics 1pt 1pt 1pt (fun pt -> [])");
}

#[test]
fn fill_rejects_a_non_color_first_argument() {
    assert_type_error("fill 1 (terminate-path (start-path (0pt, 0pt)))");
}

// `tabular` + the `cell` variant — a self-contained, `tabular.
// satyh`-shaped `let-inline` command (positional cell builders; record/option
// front-ends aren't exercised here) exercising `NormalCell`/`MultiCell`/
// `EmptyCell` inferring `cell`, and `rulef` unifying against `graphics list`.

const TABULAR_CMD: &str = "let-inline ctx \\tabular cellssf rulef =
       let pads = (5pt, 5pt, 2pt, 2pt) in
       let cellf it = NormalCell (pads, inline-fil ++ read-inline ctx it ++ inline-fil) in
       let multif n m it = MultiCell (n, m, pads, inline-fil ++ read-inline ctx it ++ inline-fil) in
       let empty = EmptyCell in
         tabular (cellssf cellf multif empty) rulef";

#[test]
fn tabular_command_shape_typechecks_end_to_end() {
    // Trailing `;` closes the lexer's "active area" opened by `\tabular`
    // (`cst.rs`'s `CmdTail::Args` doc comment) — same as real front-ends.
    assert_well_typed(&format!(
        "{TABULAR_CMD}
         in
         {{ \\tabular(fun c m e -> [[c{{A}}; c{{B}}]; [e; c{{D}}]])(fun xs ys -> []); }}"
    ));
}

#[test]
fn tabular_rejects_a_rule_callback_with_the_wrong_result_type() {
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

// Hooks + cross-references — `hook-page-break`'s closure gets a
// `page-break-info` closed record row with no nominal type needed;
// `register`/`get-cross-reference` round-trip through `string option`.

#[test]
fn hook_page_break_closure_typechecks_against_the_pbinfo_record_row() {
    assert_well_typed(
        "hook-page-break (fun pbinfo pt -> register-cross-reference `p` (arabic pbinfo#page-number))",
    );
}

#[test]
fn hook_page_break_rejects_a_closure_missing_the_page_number_field() {
    // Never uses `#page-number`, but still must be shaped like `page-break-info`.
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

// The real 4-arg `page-break` — `page-content-scheme` and
// `page-parts` are structural closed rows, same as `pbinfo`: no nominal
// scheme type needed, just two ordinary `fun pbinfo -> record` closures.

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

// First-class command values `(command \cmd)` elaborate to a plain
// `Var` referencing the binding, inferring its `InlineCmd` scheme directly.

#[test]
fn command_value_typechecks_and_unifies_with_another_of_the_same_shape() {
    // Two shape-identical commands' `(command \cmd)` values must unify,
    // proving the value carries the command's real `MonoType`.
    assert_well_typed(
        "let-inline ctx \\m it = read-inline ctx it
         let-inline ctx \\n it = read-inline ctx it
         in
         if true then (command \\m) else (command \\n)",
    );
}

#[test]
fn command_value_of_mismatched_arity_is_rejected() {
    // `\m` and `\pair` have different `InlineCmd` argument-list arities —
    // pinning that `(command \cmd)`'s type isn't a generic stand-in.
    assert_type_error(
        "let-inline ctx \\m it = read-inline ctx it
         let-inline ctx \\pair a b = read-inline ctx a
         in
         if true then (command \\m) else (command \\pair)",
    );
}

#[test]
fn command_value_of_an_undefined_command_is_rejected() {
    // `scoped_var`'s scope check fires for `Atomic::Command` too (at elaboration).
    let file = rustyfi_syntax::parse_file("(command \\nonexistent)").unwrap();
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
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
    // using local stand-ins, proving `(command \cmd)` flows end-to-end into
    // an `inline-cmd`-typed parameter and back out through a pipe chain.
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
