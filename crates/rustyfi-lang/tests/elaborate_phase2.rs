//! End-to-end coverage for the phase-2 elaborator: real SATySFi *source
//! text* run through `parse_file` -> `elaborate` -> `eval::Interp`,
//! exercising the operator-precedence fold, `let-rec`/`if`/`match`/tuples,
//! unary minus, list `::`, inline-text `#var;` embeds, and `let-inline`
//! (both the context-taking and "lightweight" forms).

use rustyfi_backend::{FontKey, FontMetrics, Length, PureHorzBox};
use rustyfi_lang::value::{DocumentValue, Value};
use rustyfi_lang::{compile_document_cst, elaborate, eval, primitives};
use std::path::Path;
use std::rc::Rc;

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn eval_str(src: &str) -> Result<Value, rustyfi_lang::CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let ast = elaborate::elaborate(&file, &scope)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &ast)?)
}

fn int(src: &str) -> i64 {
    match eval_str(src).unwrap() {
        Value::Int(n) => n,
        other => panic!("{src:?} evaluated to {other:?}, not an int"),
    }
}

// ---- operator precedence / associativity ---------------------------------

#[test]
fn precedence_times_binds_tighter_than_plus() {
    assert_eq!(int("1 + 2 * 3"), 7);
    assert_eq!(int("2 * 3 + 1"), 7);
}

#[test]
fn minus_is_left_associative() {
    assert_eq!(int("10 - 3 - 2"), 5);
}

#[test]
fn divides_is_right_associative_fidelity_quirk() {
    // v0.0.6's `nxrtimes` is genuinely right-recursive: 16 / (4 / 2) = 16 / 2 = 8,
    // not (16 / 4) / 2 = 2.
    assert_eq!(int("16 / 4 / 2"), 8);
}

#[test]
fn cons_operator_builds_a_list() {
    let Value::List(items) = eval_str("1 :: 2 :: []").unwrap() else {
        panic!("expected a list")
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0], Value::Int(1)));
    assert!(matches!(items[1], Value::Int(2)));
}

#[test]
fn unary_minus_wraps_the_whole_application() {
    assert_eq!(int("- (2 + 3)"), -5);
}

// ---- `|>` reverse application (frontend-completion.md Blocker B) ----------
//
// `|>` has no `Ast`-level identity to build directly (unlike every other
// primitive/operator): elaboration lowers `a |> f` straight to `Apply(f,
// a)`, so this is the one place in the pure-primitives batch that can only
// be exercised through the real parser + elaborator, not `prims_phase4.rs`'s
// hand-built-`Ast` harness.

#[test]
fn pipe_lowers_to_reverse_application() {
    assert_eq!(int("1 |> (fun x -> x + 1)"), 2);
}

#[test]
fn pipe_accepts_a_bare_function_reference_not_just_a_lambda() {
    // Matches how the bundled `list.satyg` actually uses it (`lst |>
    // fold-left-adjacent (...) [] |> reverse`): the right-hand side is an
    // ordinary application-chain result, not necessarily a literal `fun`.
    let src = "let-rec double x = x * 2 in let-rec inc x = x + 1 in 3 |> double |> inc";
    assert_eq!(int(src), 7);
}

#[test]
fn pipe_is_left_associative() {
    // (3 |> double) |> inc = inc (double 3) = 7, not double (inc 3) = 8.
    let src = "let-rec double x = x * 2 in let-rec inc x = x + 1 in 3 |> double |> inc";
    assert_eq!(int(src), 7);
    assert_ne!(int(src), 8);
}

#[test]
fn pipe_sits_at_the_loosest_precedence_level() {
    // `|>` (level 1) is looser than `+` (level 5): `1 + 1 |> double` parses
    // as `(1 + 1) |> double`, i.e. `double 2` = 4, not `1 + (1 |> double)`
    // (which double-applies `+` at the wrong point and would give 3).
    let src = "let-rec double x = x * 2 in 1 + 1 |> double";
    assert_eq!(int(src), 4);
}

// ---- if / let-rec / match / tuple, from source -----------------------------

#[test]
fn if_then_else_from_source() {
    assert_eq!(int("if true then 1 else 2"), 1);
    assert_eq!(int("if false then 1 else 2"), 2);
}

#[test]
fn let_rec_factorial_from_source() {
    let src = "let-rec fact n = if n == 0 then 1 else n * fact (n - 1) in fact 5";
    assert_eq!(int(src), 120);
}

#[test]
fn match_some_none_from_source() {
    assert_eq!(int("match Some 3 with | None -> 0 | Some y -> y"), 3);
    assert_eq!(int("match None with | None -> 0 | Some y -> y"), 0);
}

#[test]
fn tuple_construction_from_source() {
    let Value::Tuple(items) = eval_str("(1, 2)").unwrap() else {
        panic!("expected a tuple")
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0], Value::Int(1)));
    assert!(matches!(items[1], Value::Int(2)));
}

// ---- inline-text embeds through a document ---------------------------------

/// `document`/`+p`/`\emph` are no longer hardcoded Rust natives (phase 4):
/// they're now ordinary bindings in the real `stdja-mini` stdlib package
/// (`lib-rustyfi/dist/packages/stdja-mini.satyh`). Compile `src` the same
/// way the multi-file loader's `merge_program` does — concatenate the
/// package's prelude ahead of `src`'s own — rather than pulling in the
/// whole loader crate for a single-file test.
fn compile_document_with_stdlib(
    src: &str,
    metrics: &dyn rustyfi_backend::FontMetrics,
) -> Result<Rc<DocumentValue>, rustyfi_lang::CompileError> {
    let lib_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/packages/stdja-mini.satyh");
    let lib_src = std::fs::read_to_string(&lib_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lib_path.display()));
    let lib_file = rustyfi_syntax::parse_file(&lib_src)?;
    let doc_file = rustyfi_syntax::parse_file(src)?;

    let mut prelude = lib_file.prelude;
    prelude.extend(doc_file.prelude);
    let merged = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: doc_file.in_kw,
        body: doc_file.body,
        eoi: doc_file.eoi,
    };
    compile_document_cst(&merged, metrics)
}

/// Collects every text run on page 0, body first. Since
/// `docs/plans/document-page-model.md` Slice 1 rewrote `stdja-mini`'s
/// `document` to call the real 4-arg `page-break`, every one-page fixture
/// now also carries a footer rendering `arabic pbinfo#page-number` — placed
/// (via `place_block_at`) *after* the body's lines, so it always shows up
/// as this vec's trailing `"1"` element on a single-page document.
fn document_words(src: &str) -> Vec<String> {
    let doc = compile_document_with_stdlib(src, &Mono).unwrap();
    doc.pages[0]
        .lines
        .iter()
        .flat_map(|line| {
            line.contents.iter().filter_map(|(_, b)| match b {
                PureHorzBox::InnerString { text, .. } => Some(text.clone()),
                _ => None,
            })
        })
        .collect()
}

#[test]
fn inline_embed_splices_a_let_bound_inline_text() {
    let src = "let greeting = { world } in \
               document (||) '< +p { hello #greeting; } >";
    assert_eq!(document_words(src), vec!["hello", "world", "1"]);
}

// ---- let-inline, both forms -------------------------------------------------

#[test]
fn let_inline_with_explicit_context() {
    // The explicit-context form's value is already inline-boxes typed, so
    // it can call `read-inline` itself.
    let src = "let-inline ctx \\shout it = read-inline ctx it in \
               document (||) '< +p { \\shout{ loud } } >";
    assert_eq!(document_words(src), vec!["loud", "1"]);
}

#[test]
fn let_inline_lightweight_form_without_context() {
    // The context-less "lightweight" form's value is inline-text typed;
    // v0.0.6's parser.mly (nxhorzdec) implicitly reads it under a
    // synthesized `%context`, so plain inline text is a valid body.
    let src = "let-inline \\whisper it = it in \
               document (||) '< +p { \\whisper{ hi } } >";
    assert_eq!(document_words(src), vec!["hi", "1"]);
}

#[test]
fn let_block_with_explicit_context() {
    // Block text can only contain commands/embeds (no bare characters), so
    // the nested block-text argument re-uses `+p` for its content.
    let src = "let-block ctx +shout it = read-block ctx it in \
               document (||) '< +shout< +p{ loud } > >";
    assert_eq!(document_words(src), vec!["loud", "1"]);
}
