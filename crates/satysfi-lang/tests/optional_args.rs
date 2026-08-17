//! End-to-end acceptance coverage for runtime optional args (build-order
//! step 3 — see `docs/plans/frontend-completion.md` Sub-area 2 and the
//! `?->`/`string?` type-grammar parts of
//! `docs/plans/class-signature-lang-gaps.md`): real SATySFi source text run
//! through the full pipeline — `parse_file` -> `elaborate::elaborate_program`
//! -> `typecheck::typecheck` -> `eval::Interp` — proving:
//!
//! 1. an inline command with an optional argument, called both `?:`-supplied
//!    and `?*`-omitted, type-checks and evaluates to observably different
//!    content;
//! 2. a `?->`-typed function (declared via a `type` synonym, since this
//!    milestone has no signature-*enforcement* pass yet — see that plan's
//!    Risks) unifies with a real function whose domain is a plain `option`,
//!    and with `?:`/`?*` call sites against it — the "one consistent
//!    optional-arg model" the two plans share.

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

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

/// Parse -> elaborate -> typecheck -> evaluate a whole file's document
/// expression, returning its final `Value`. Mirrors `compile_document_cst`
/// (`lib.rs`) minus the document-specific page-breaking tail, since these
/// fixtures compute a plain `int`/`inline-boxes` result rather than a full
/// document.
fn run(src: &str) -> Result<Value, CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &program.body)?)
}

fn int(src: &str) -> i64 {
    match run(src).unwrap() {
        Value::Int(n) => n,
        other => panic!("{src:?} evaluated to {other:?}, not an int"),
    }
}

// ============================================================================
// 1. An inline command with an optional argument: `?:` supplied vs `?*`
//    omitted, both parsed via `CmdTail::Args`'s leading-`AppArg` grammar.
// ============================================================================

const GREET_CMD: &str = "let-inline ctx \\greet name =
  read-inline ctx (
    match name with
    | Some(_) -> { Hello there, my dear friend! }
    | None -> { Hi. }
  )
let-inline ctx \\math m = inline-nil
in
";

#[test]
fn inline_command_optional_arg_supplied_and_omitted_typecheck_and_evaluate() {
    // Supplied (`?:(1)`) takes the `Some` branch (longer text); omitted
    // (`?*`) takes the `None` branch (shorter text) — both are well-typed
    // (the command's inferred `name` domain is `int option`, peeled by
    // `command_scheme` into `CmdArgType { optional: true, ty: int }`) and
    // both evaluate to `inline-boxes` with observably different widths.
    let src = format!(
        "{GREET_CMD}\
         let base = get-initial-context 200pt (command \\math) in
         let ib-yes = read-inline base {{ \\greet?:(1); }} in
         let ib-no = read-inline base {{ \\greet?*; }} in
         let (w-yes, _, _) = get-natural-metrics ib-yes in
         let (w-no, _, _) = get-natural-metrics ib-no in
         if w-yes >' w-no then 1 else 0"
    );
    assert_eq!(int(&src), 1, "supplied greeting should render wider than the omitted fallback");
}

#[test]
fn inline_command_with_only_an_omission_marker_still_evaluates() {
    // A command call whose only argument is the bare `?*` marker (no
    // trailing group arg) is a valid, complete `CmdTail::Args` on its own.
    let src = format!(
        "{GREET_CMD}\
         let base = get-initial-context 200pt (command \\math) in
         let ib = read-inline base {{ \\greet?*; }} in
         let (w, _, _) = get-natural-metrics ib in
         if w >' 0pt then 1 else 0"
    );
    assert_eq!(int(&src), 1);
}

// ============================================================================
// 2. A `?->`-typed function unifies with a real `option`-domain function and
//    with `?:`/`?*` call sites against it.
// ============================================================================

#[test]
fn optional_arrow_type_unifies_with_option_domain_and_call_sites() {
    // `greeter = string ?-> string -> int` (an optional leading domain, then
    // a *mandatory* one — `?->` chains always terminate in a plain `->`,
    // matching upstream `txfuncopts`/`txfunc`) lowers (`typecheck.rs`'s
    // `lower_type_expr`) to `Func(option(string), Func(string, int))` — bit-
    // for-bit the type a plain function using its first parameter as `string
    // option` infers on its own. Storing one inside a variant ctor (`MkBox`)
    // and pulling it back out via pattern match forces a REAL unification
    // between the two encodings; applying it via `?:`/`?*` then exercises
    // the call-site desugaring against that same domain.
    let src = "type greeter = string ?-> string -> int

type box-ty = | MkBox of greeter

let combine prefix-opt s = match prefix-opt with
  | Some(p) -> string-length p + string-length s
  | None -> string-length s

let apply-supplied b p s = match b with | MkBox(f) -> f ?:(p) s
let apply-omitted b s = match b with | MkBox(f) -> f ?* s

let bx = MkBox combine
in
let via-some = apply-supplied bx `ab` `cde` in
let via-none = apply-omitted bx `cde` in
if via-some == 5 then
  (if via-none == 3 then 1 else 0)
else 0";
    assert_eq!(int(src), 1);
}

#[test]
fn optional_arrow_type_declaration_alone_typechecks() {
    // The bracket-list command-type grammar (`ty?`) and the plain-function
    // `?->` arrow both parse + lower without needing any package that
    // actually consumes them — proving the grammar/lowering plumbing (`?->`,
    // `string?`) independent of the call-site runtime proof above.
    let src = "type greeter = string ?-> string -> int
type section-cmd = [string?; string?; inline-text; block-text] block-cmd
in
0";
    assert_eq!(int(src), 0);
}
