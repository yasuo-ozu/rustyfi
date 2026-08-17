//! `docs/plans/context-box-prims.md` §Slice 1: the 10 context-setter/
//! box-combinator prims `code.satyh`/`itemize.satyh` need. Two halves:
//! - **Typecheck** (real source text through `parse_file` ->
//!   `elaborate::elaborate_program` -> `typecheck::typecheck`, mirroring
//!   `tests/typecheck.rs`'s own `typecheck_str` helper) — pins each new
//!   prim's declared signature end-to-end, including the surface syntax
//!   that has to parse to satisfy it.
//! - **Eval** (direct `Ast` apply chains through `eval::Interp` +
//!   `primitives::base_env()`, mirroring `tests/prims_phase4.rs`'s style) —
//!   round-trips the faithful stores (`set`/`get-text-color`,
//!   `set-hyphen-penalty`, `set-space-ratio`, `split-into-lines`,
//!   `get-natural-length`).

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::ast::Ast;
use rustyfi_lang::eval;
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, primitives, typecheck, CompileError};
use rustyfi_syntax::Span;

// ============================================================================
// Typecheck half
// ============================================================================

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

fn assert_type_error(src: &str) {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected by the typechecker, but it passed"),
        Err(CompileError::Type(_)) => {}
        Err(other) => panic!("expected {src:?} to fail with a type error, got: {other}"),
    }
}

#[test]
fn set_and_get_text_color_typecheck() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         get-text-color (set-text-color (Gray 0.5) (get-initial-context 100pt (command \\math)))",
    );
}

#[test]
fn set_text_color_rejects_a_non_color_argument() {
    assert_type_error(
        "let-inline ctx \\math m = inline-nil
         in
         set-text-color 3 (get-initial-context 100pt (command \\math))",
    );
}

#[test]
fn set_hyphen_penalty_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         get-text-width (set-hyphen-penalty 100000 (get-initial-context 100pt (command \\math)))",
    );
}

#[test]
fn set_space_ratio_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         get-text-width (set-space-ratio 0.4 0.1 0.2 (get-initial-context 100pt (command \\math)))",
    );
}

#[test]
fn split_into_lines_typechecks_over_a_string_literal() {
    assert_well_typed("split-into-lines `abc`");
}

#[test]
fn block_frame_breakable_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         let mydeco pt l1 l2 l3 = []
         in
         block-frame-breakable (get-initial-context 100pt (command \\math)) (0pt, 0pt, 0pt, 0pt)
           (mydeco, mydeco, mydeco, mydeco) (fun ctx -> block-nil)",
    );
}

#[test]
fn block_frame_breakable_rejects_a_three_element_paddings_tuple() {
    assert_type_error(
        "let-inline ctx \\math m = inline-nil
         let mydeco pt l1 l2 l3 = []
         in
         block-frame-breakable (get-initial-context 100pt (command \\math)) (0pt, 0pt, 0pt)
           (mydeco, mydeco, mydeco, mydeco) (fun ctx -> block-nil)",
    );
}

#[test]
fn embed_block_top_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         embed-block-top (get-initial-context 100pt (command \\math)) 100pt (fun ctx -> block-nil)",
    );
}

#[test]
fn set_font_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         set-font Latin (`lmmono`, 1.0, 0.0) (get-initial-context 100pt (command \\math))",
    );
}

#[test]
fn set_code_text_command_typechecks_with_a_first_class_command_value() {
    // `(command \code)` (docs/plans/class-signature-lang-gaps.md gap 1)
    // constructs the `[string] inline-cmd` value `set-code-text-command`
    // expects — `\code`'s param is inferred `string` because its body
    // funnels it through `embed-string : string -> inline-text`.
    assert_well_typed(
        "let-inline ctx \\code s = read-inline ctx (embed-string s)
         let-inline ctx \\math m = inline-nil
         in
         set-code-text-command (command \\code) (get-initial-context 100pt (command \\math))",
    );
}

#[test]
fn get_natural_length_typechecks() {
    assert_well_typed("get-natural-length (block-skip 10pt)");
}

#[test]
fn dominant_script_getters_typecheck() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         get-dominant-wide-script
           (set-dominant-wide-script Kana (get-initial-context 100pt (command \\math)))",
    );
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         get-dominant-narrow-script
           (set-dominant-narrow-script Latin (get-initial-context 100pt (command \\math)))",
    );
}

#[test]
fn dominant_script_getter_rejects_a_context_argument_shuffle() {
    // `get-dominant-wide-script : context -> script`, not `script -> context`.
    assert_type_error(
        "let-inline ctx \\math m = inline-nil
         in
         get-dominant-wide-script Latin",
    );
}

#[test]
fn get_language_typechecks_and_the_stdja_set_language_chain_still_typechecks() {
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         get-language Kana (get-initial-context 100pt (command \\math))",
    );
    // Regression: the `set-language Kana Japanese` chain (as `stdja.satyh`
    // writes it) must still typecheck now that the setter is a faithful
    // store rather than a drop no-op — its declared signature is unchanged.
    assert_well_typed(
        "let-inline ctx \\math m = inline-nil
         in
         set-language Kana Japanese (get-initial-context 100pt (command \\math))",
    );
}

// ============================================================================
// Eval half — direct `Ast` apply chains (no parser), mirroring
// `prims_phase4.rs`'s style.
// ============================================================================

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn var(name: &str) -> Ast {
    Ast::Var(name.to_string(), Span::default())
}

fn app1(f: Ast, a: Ast) -> Ast {
    Ast::Apply(Box::new(f), Box::new(a))
}

fn app2(name: &str, a: Ast, b: Ast) -> Ast {
    app1(app1(var(name), a), b)
}

fn app3(name: &str, a: Ast, b: Ast, c: Ast) -> Ast {
    app1(app1(app1(var(name), a), b), c)
}

fn app4(name: &str, a: Ast, b: Ast, c: Ast, d: Ast) -> Ast {
    app1(app1(app1(app1(var(name), a), b), c), d)
}

fn len(pt: f64) -> Ast {
    Ast::Length(Length::pt(pt))
}

/// `get-initial-context width ()` — the second (math-command) argument is
/// ignored at runtime (see `primitives.rs`'s `prim_get_initial_context`).
fn initial_ctx(width_pt: f64) -> Ast {
    app2("get-initial-context", len(width_pt), Ast::Unit)
}

fn run(ast: &Ast) -> Value {
    let env = primitives::base_env();
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    interp.eval(&env, ast).expect("evaluation should succeed")
}

fn assert_len_close(v: Value, expect_pt: f64) {
    match v {
        Value::Length(l) => assert!(
            (l.0 - expect_pt).abs() < 1e-9,
            "expected {expect_pt}pt, got {}pt",
            l.0
        ),
        other => panic!("expected a length, got {other:?}"),
    }
}

#[test]
fn set_text_color_then_get_text_color_round_trips_rgb() {
    let color = Ast::Ctor(
        "RGB".to_string(),
        Some(Box::new(Ast::Tuple(vec![
            Ast::Float(0.0),
            Ast::Float(0.25),
            Ast::Float(1.0),
        ]))),
    );
    let ast = app1(
        var("get-text-color"),
        app2("set-text-color", color, initial_ctx(100.0)),
    );
    match run(&ast) {
        Value::Ctor(name, Some(payload)) => {
            assert_eq!(name, "RGB");
            match *payload {
                Value::Tuple(vs) => {
                    let floats: Vec<f64> = vs
                        .into_iter()
                        .map(|v| match v {
                            Value::Float(f) => f,
                            other => panic!("expected a float, got {other:?}"),
                        })
                        .collect();
                    assert_eq!(floats, vec![0.0, 0.25, 1.0]);
                }
                other => panic!("expected the RGB payload to be a tuple, got {other:?}"),
            }
        }
        other => panic!("expected a color ctor value, got {other:?}"),
    }
}

#[test]
fn get_text_color_defaults_to_black_gray() {
    // `Context::initial`'s default (v0.0.6 `DeviceGray 0.`).
    let ast = app1(var("get-text-color"), initial_ctx(100.0));
    match run(&ast) {
        Value::Ctor(name, Some(payload)) => {
            assert_eq!(name, "Gray");
            match *payload {
                Value::Float(f) => assert_eq!(f, 0.0),
                other => panic!("expected the Gray payload to be a float, got {other:?}"),
            }
        }
        other => panic!("expected a color ctor value, got {other:?}"),
    }
}

#[test]
fn set_hyphen_penalty_round_trip_via_context_value() {
    let ast = app2("set-hyphen-penalty", Ast::Int(100_000), initial_ctx(100.0));
    match run(&ast) {
        Value::Context(ctx) => assert_eq!(ctx.hyphen_badness, 100_000),
        other => panic!("expected a context, got {other:?}"),
    }
}

#[test]
fn set_space_ratio_round_trip_via_context_value() {
    let ast = app4(
        "set-space-ratio",
        Ast::Float(0.4),
        Ast::Float(0.1),
        Ast::Float(0.2),
        initial_ctx(100.0),
    );
    match run(&ast) {
        Value::Context(ctx) => {
            assert_eq!(ctx.space_natural, 0.4);
            assert_eq!(ctx.space_shrink, 0.1);
            assert_eq!(ctx.space_stretch, 0.2);
        }
        other => panic!("expected a context, got {other:?}"),
    }
}

#[test]
fn split_into_lines_matches_chop_space_indent() {
    let ast = app1(var("split-into-lines"), Ast::Str("a\n  bc\n".to_string()));
    match run(&ast) {
        Value::List(items) => {
            let pairs: Vec<(i64, String)> = items
                .into_iter()
                .map(|v| match v {
                    Value::Tuple(vs) if vs.len() == 2 => {
                        let mut it = vs.into_iter();
                        let i = match it.next().unwrap() {
                            Value::Int(i) => i,
                            other => panic!("expected an int, got {other:?}"),
                        };
                        let s = match it.next().unwrap() {
                            Value::Str(s) => s,
                            other => panic!("expected a string, got {other:?}"),
                        };
                        (i, s)
                    }
                    other => panic!("expected an (int * string) pair, got {other:?}"),
                })
                .collect();
            assert_eq!(
                pairs,
                vec![(0, "a".to_string()), (2, "bc".to_string()), (0, String::new())]
            );
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn set_font_switches_to_the_bold_face_by_name_heuristic() {
    let script = Ast::Ctor("Latin".to_string(), None);
    let font = Ast::Tuple(vec![
        Ast::Str("lmroman-bold".to_string()),
        Ast::Float(1.0),
        Ast::Float(0.0),
    ]);
    let ast = app3("set-font", script, font, initial_ctx(100.0));
    match run(&ast) {
        Value::Context(ctx) => assert_eq!(ctx.font, FontKey(1), "\"bold\" in the abbrev should select FONT_BOLD"),
        other => panic!("expected a context, got {other:?}"),
    }
}

#[test]
fn get_natural_length_of_a_single_skip_is_its_own_length() {
    let ast = app1(var("get-natural-length"), app1(var("block-skip"), len(10.0)));
    assert_len_close(run(&ast), 10.0);
}

// ============================================================================
// group E2: dominant-script / language context stores
// ============================================================================

fn ctor(name: &str) -> Ast {
    Ast::Ctor(name.to_string(), None)
}

fn assert_ctor_eq(v: Value, expect: &str) {
    match v {
        Value::Ctor(name, None) => assert_eq!(name, expect),
        other => panic!("expected Ctor({expect:?}, None), got {other:?}"),
    }
}

#[test]
fn dominant_wide_script_round_trips_through_setter_and_getter() {
    let ast = app1(
        var("get-dominant-wide-script"),
        app2("set-dominant-wide-script", ctor("Kana"), initial_ctx(100.0)),
    );
    assert_ctor_eq(run(&ast), "Kana");
}

#[test]
fn dominant_narrow_script_round_trips_through_setter_and_getter() {
    let ast = app1(
        var("get-dominant-narrow-script"),
        app2("set-dominant-narrow-script", ctor("Latin"), initial_ctx(100.0)),
    );
    assert_ctor_eq(run(&ast), "Latin");
}

#[test]
fn dominant_script_getters_default_to_other_script() {
    let wide = app1(var("get-dominant-wide-script"), initial_ctx(100.0));
    assert_ctor_eq(run(&wide), "OtherScript");
    let narrow = app1(var("get-dominant-narrow-script"), initial_ctx(100.0));
    assert_ctor_eq(run(&narrow), "OtherScript");
}

#[test]
fn get_language_defaults_to_no_language_system() {
    let ast = app2("get-language", ctor("Latin"), initial_ctx(100.0));
    assert_ctor_eq(run(&ast), "NoLanguageSystem");
}

#[test]
fn set_language_is_a_per_script_map_insert_not_a_scalar_overwrite() {
    // `set-language Kana Japanese |> set-language Latin English`
    let ctx = app3("set-language", ctor("Kana"), ctor("Japanese"), initial_ctx(100.0));
    let ctx = app3("set-language", ctor("Latin"), ctor("English"), ctx);

    let get = |script: &str, ctx: &Ast| app2("get-language", ctor(script), ctx.clone());

    assert_ctor_eq(run(&get("Kana", &ctx)), "Japanese");
    assert_ctor_eq(run(&get("Latin", &ctx)), "English");
    assert_ctor_eq(
        run(&get("HanIdeographic", &ctx)),
        "NoLanguageSystem",
        // proves the insert is per-script, not a scalar overwrite of every
        // script's language system.
    );
}

#[test]
fn setting_dominant_wide_script_leaves_the_rest_of_the_context_equal() {
    // `let c = get-initial-context ... in (c, set-dominant-wide-script Kana c)`
    // — a single shared `initial_ctx` *value*, bound once, so both tuple
    // components carry the SAME `math_command` `MathCmdId` (calling
    // `get-initial-context` a second time would install a second closure
    // under a fresh id, making an independently-evaluated "base" spuriously
    // unequal to `updated` for a reason that has nothing to do with
    // `set-dominant-wide-script`).
    let ast = Ast::LetIn(
        "c".to_string(),
        Box::new(initial_ctx(100.0)),
        Box::new(Ast::Tuple(vec![
            var("c"),
            app2("set-dominant-wide-script", ctor("Kana"), var("c")),
        ])),
    );
    let (base, updated) = match run(&ast) {
        Value::Tuple(vs) if vs.len() == 2 => {
            let mut it = vs.into_iter();
            let base = match it.next().unwrap() {
                Value::Context(ctx) => *ctx,
                other => panic!("expected a context, got {other:?}"),
            };
            let updated = match it.next().unwrap() {
                Value::Context(ctx) => *ctx,
                other => panic!("expected a context, got {other:?}"),
            };
            (base, updated)
        }
        other => panic!("expected a 2-tuple, got {other:?}"),
    };
    assert_eq!(updated.dominant_wide_script, rustyfi_backend::Script::Kana);
    let expected = rustyfi_backend::Context { dominant_wide_script: updated.dominant_wide_script, ..base };
    assert_eq!(updated, expected, "only `dominant_wide_script` should differ from the base context");
}
