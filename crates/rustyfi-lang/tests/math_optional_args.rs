//! End-to-end acceptance coverage for optional/omitted math-command
//! args (`\cmd?:{…}`/ `?*`) and auto-`None` padding for a marker-less bare
//! math-command call. Pure pipeline, standalone helpers.

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

fn as_length(v: Value) -> Length {
    match v {
        Value::Length(l) => l,
        other => panic!("expected a length, got {other:?}"),
    }
}

/// `get-natural-metrics`'s `(width, height, depth)` result — the width only.
fn natural_width(v: Value) -> Length {
    match v {
        Value::Tuple(vs) if vs.len() == 3 => as_length(vs.into_iter().next().unwrap()),
        other => panic!("expected a (length * length * length) tuple, got {other:?}"),
    }
}

/// A `let-math` command with one leading `?:`-marked optional parameter:
/// `None` (omitted) falls back to the mandatory `m`, `Some(n)` overrides it
/// with the supplied math value `n` instead.
const FOO_MATH_CMD: &str = "let-math \\foo ?:o m = (match o with None -> m | Some(n) -> n)
let-inline ctx \\dummy m = inline-nil
in
";

#[test]
fn math_optional_arg_supplied_is_wider_than_omitted_and_bare() {
    // A marker-less bare call (`\foo{x}`) must auto-pad `None` for the
    // leading optional slot and elaborate BYTE-IDENTICALLY to an explicit
    // `?*` call. `==`/`>'` aren't polymorphic over `length` here, so the
    // three widths are returned as a tuple and compared in Rust.
    let src = format!(
        "{FOO_MATH_CMD}\
         let ctx = get-initial-context 200pt (command \\dummy) in
         let (w-supplied, _, _) = get-natural-metrics (embed-math ctx ${{\\foo?:{{yy}}{{x}}}}) in
         let (w-omitted, _, _) = get-natural-metrics (embed-math ctx ${{\\foo?*{{x}}}}) in
         let (w-bare, _, _) = get-natural-metrics (embed-math ctx ${{\\foo{{x}}}}) in
         (w-supplied, w-omitted, w-bare)"
    );
    let (w_supplied, w_omitted, w_bare) = match run(&src).unwrap() {
        Value::Tuple(vs) if vs.len() == 3 => {
            let mut it = vs.into_iter();
            (
                as_length(it.next().unwrap()),
                as_length(it.next().unwrap()),
                as_length(it.next().unwrap()),
            )
        }
        other => panic!("expected a (length * length * length) tuple, got {other:?}"),
    };
    assert_eq!(
        w_omitted, w_bare,
        "explicit ?* omission and marker-less padding must elaborate identically"
    );
    assert!(
        w_supplied > w_omitted,
        "supplied override (2 chars) should be wider than the omitted/bare 1-char fallback"
    );
}

#[test]
fn math_optional_arg_supplied_omitted_bare_widths_directly() {
    // Same three call shapes, but comparing `Length`s directly for a
    // clearer failure message if this regresses.
    let base = |cmd: &str| {
        let src = format!(
            "{FOO_MATH_CMD}\
             let ctx = get-initial-context 200pt (command \\dummy) in
             get-natural-metrics (embed-math ctx {cmd})"
        );
        natural_width(run(&src).unwrap())
    };
    let w_supplied = base(r"${\foo?:{yy}{x}}");
    let w_omitted = base(r"${\foo?*{x}}");
    let w_bare = base(r"${\foo{x}}");
    assert_eq!(
        w_omitted, w_bare,
        "explicit ?* omission and marker-less padding must elaborate identically"
    );
    assert!(
        w_supplied > w_omitted,
        "supplied override (2 chars) should be wider than the omitted/bare 1-char fallback"
    );
}
