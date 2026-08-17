//! End-to-end acceptance coverage for Gap 4 (`docs/plans/math-mode-
//! language-gaps.md`): optional/omitted math-command args (`\cmd?:{…}`/
//! `?*`) and auto-`None` padding for a marker-less bare math-command call.
//! Pure-pipeline `run`/`int` helpers copied from `optional_args.rs`/
//! `math_lists.rs` (per those files' own copy-not-share convention, since
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
/// with the supplied math value `n` instead. Patterned on `math_package.rs`'s
/// `gap2_`-prefixed tests' own inline stub style.
const FOO_MATH_CMD: &str = "let-math \\foo ?:o m = (match o with None -> m | Some(n) -> n)
let-inline ctx \\dummy m = inline-nil
in
";

#[test]
fn math_optional_arg_supplied_is_wider_than_omitted_and_bare() {
    // `?:{yy}` supplies a 2-char override (`yy`, wider); `?*` explicitly
    // omits, falling back to the 1-char mandatory `x`; a marker-less bare
    // call (`\foo{x}`, no `?:`/`?*` at all) must auto-pad a `None` for the
    // leading optional slot and elaborate BYTE-IDENTICALLY to the explicit
    // `?*` call — so `w-omitted == w-bare`, and both are narrower than
    // `w-supplied`. (`==`/`>'` aren't polymorphic over `length` in this
    // language, so the three widths are returned as a tuple and compared in
    // Rust — same style as `math_package.rs`'s `gap2_pull_in_scripts_
    // resolver_receives_the_actual_scripts`.)
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
    // Same three call shapes as above, but comparing the actual `Length`s
    // directly (rather than folding the comparison into the evaluated
    // program) for a clearer failure message if this ever regresses.
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
