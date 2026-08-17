//! End-to-end acceptance coverage for Gap 3 (`docs/plans/math-mode-
//! language-gaps.md`): `|`-separated math lists `${| a | b |}`. Upstream
//! desugars a LEADING `|` in-grammar to an ordinary list literal of `math`
//! values (`mathblock`, parser.mly:1059-1066) — this is a `math list`
//! LITERAL, not a matrix/grid. Pure-pipeline `run`/`int` helpers copied from
//! `optional_args.rs` (per that file's own copy-not-share convention, since
//! test harness files are intentionally standalone).

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

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

fn run(src: &str) -> Result<Value, CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))?)
}

fn int(src: &str) -> i64 {
    match run(src).unwrap() {
        Value::Int(n) => n,
        other => panic!("{src:?} evaluated to {other:?}, not an int"),
    }
}

fn assert_compile_error_contains(src: &str, needle: &str) {
    match run(src) {
        Ok(v) => panic!("expected {src:?} to be rejected, but it evaluated to {v:?}"),
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains(needle),
                "expected error for {src:?} to contain {needle:?}, got: {msg}"
            );
        }
    }
}

// ============================================================================
// 1. `${|a|b|}` is a 2-elem math list.
// ============================================================================

#[test]
fn leading_sep_math_list_has_two_elements() {
    let src = "match ${| a | b |} with
  | x :: y :: [] -> 1
  | _ -> 0";
    assert_eq!(int(src), 1);
}

// ============================================================================
// 2. Degenerate list-mode shapes.
// ============================================================================

#[test]
fn empty_bars_is_the_empty_list() {
    let src = "match ${|} with
  | [] -> 1
  | _ -> 0";
    assert_eq!(int(src), 1);
}

#[test]
fn double_bar_is_one_empty_cell() {
    let src = "match ${||} with
  | x :: [] -> 1
  | _ -> 0";
    assert_eq!(int(src), 1);
}

// ============================================================================
// 3. Rejections.
// ============================================================================

#[test]
fn non_leading_sep_is_rejected() {
    // Not list mode (no leading `|`): the interior `|` hits `math_bot`'s
    // upgraded `Sep` error, which mentions both "starts with '|'" and
    // "mid-formula".
    assert_compile_error_contains("${ a | b }", "starts with '|'");
    assert_compile_error_contains("${ a | b }", "mid-formula");
}

#[test]
fn missing_trailing_bar_is_rejected() {
    assert_compile_error_contains("${| a }", "trailing '|'");
}

#[test]
fn sep_inside_a_math_group_is_rejected() {
    assert_compile_error_contains("${x^{a|b}}", "math group");
}

#[test]
fn sep_cannot_carry_a_script() {
    assert_compile_error_contains("${| a |^2 b|}", "cannot carry a script");
}

#[test]
fn math_list_cannot_be_embedded_in_inline_text() {
    assert_compile_error_contains("{ text ${| a |} }", "cannot be embedded");
}

// ============================================================================
// 5. Round-trip smoke (parse-only; full token round-trip lives in
//    `rustyfi-syntax`'s own `roundtrip.rs`) — proves this pipeline's parser
//    accepts the same source `assert_roundtrip` covers.
// ============================================================================

#[test]
fn math_list_sources_parse() {
    rustyfi_syntax::parse_file("${| a | b |}").expect("${| a | b |} should parse");
    rustyfi_syntax::parse_file("${|}").expect("${|} should parse");
}
