//! Acceptance coverage for marker-less optional-argument defaulting
//! — calling an ordinary (non-command) function with a leading `?:`-
//! optional parameter (`Param::Optional`, `cst.rs`) *without* any `?:`/`?*`
//! marker at the call site, e.g. `progsynt.satyh`'s `let to-math ?:iopt e
//! = .. in .. to-math e1 ..`. Handled in `elaborate.rs`: `Scope` tracks
//! each binding's leading-optional-parameter count (`Scope::
//! optional_arity`), and `app_chain_generic` synthesizes a `None` for
//! every unmarked slot — the elaborated `Ast` for a bare call is byte-for-
//! byte what an explicit `?*`/`?:` call produces.

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

#[test]
fn bare_call_omits_leading_optional() {
    let src = "let g ?:a b = (match a with None -> b | Some(x) -> x + b) in
g 5";
    assert_eq!(int(src), 5);
}

/// The bare-call padding path must never fire when a marker is present.
#[test]
fn explicit_supplied_call_still_works() {
    let src = "let g ?:a b = (match a with None -> b | Some(x) -> x + b) in
g ?:(10) 5";
    assert_eq!(int(src), 15);
}

/// An explicit `?*` omission elaborates the same way the bare-call path does.
#[test]
fn explicit_omission_marker_still_works() {
    let src = "let g ?:a b = (match a with None -> b | Some(x) -> x + b) in
g ?* 5";
    assert_eq!(int(src), 5);
}

/// Two leading optionals, partially covered: the first is explicit, the
/// second auto-omitted — proving the insertion isn't limited to a single
/// optional slot.
#[test]
fn partial_explicit_prefix_then_auto_omitted_second_optional() {
    let src = "let h ?:a ?:b c =
  (match a with None -> 0 | Some(x) -> x) + (match b with None -> 0 | Some(x) -> x) + c in
h ?:(1) 5";
    assert_eq!(int(src), 6);
}

/// A non-leading `?:` (matching `stdja.satyh`'s `document record
/// ?:configopt inner`) still works with an EXPLICIT `?*`/`?:` marker
/// exactly as before — the fix must leave every marked call site
/// byte-identical.
#[test]
fn non_leading_optional_marker_is_unaffected() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
f 1 ?* 2";
    assert_eq!(int(src), 3);
}

/// THE BUG (elabfix): a function with a NON-leading optional
/// (`stdja.satyh`/`stdjabook.satyh`'s `document record ?:configopt inner`)
/// bare-called with NO marker mis-bound the following positional into the
/// OMITTED optional's slot instead of defaulting it to `None`. So `f 1 2`
/// must bind `x=1`, `y=None`, `z=2` (⇒ 3), exactly as `f 1 ?* 2` does.
#[test]
fn bare_call_omits_non_leading_optional_then_supplies_positional() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
f 1 2";
    assert_eq!(int(src), 3);
}

/// Control: same binding, optional EXPLICITLY supplied between the two
/// positionals — the marked-call path is unchanged by the fix.
#[test]
fn explicit_supplied_non_leading_optional_still_works() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
f 1 ?:(9) 2";
    assert_eq!(int(src), 12);
}

/// Partial application that STOPS before the optional slot must not
/// eagerly insert `None`: `f 1` is a genuine partial application (still
/// awaiting `?:y` and `z`). A fix that padded `None` as soon as arguments
/// ran out at an optional slot would over-apply here.
#[test]
fn partial_application_before_optional_is_not_padded() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
let g = f 1 in g ?* 2";
    assert_eq!(int(src), 3);
}

/// `progsynt.satyh`'s real shape: a `to-math`-like function bare-called
/// from ANOTHER function in the SAME module — the site the fix actually
/// targets, since arity is tracked in the enclosing `Scope` and does not
/// bubble through a qualified cross-module lookup (calling via a
/// qualified name from outside the module still falls back to
/// marker-only behavior).
#[test]
fn module_internal_bare_call_to_leading_optional_function() {
    let src = "module M : sig
val g : int -> int
end = struct
  let to-math ?:iopt e = match iopt with
    | None -> e
    | Some(i) -> e + i
  let g e = to-math e
end
in M.g 5";
    assert_eq!(int(src), 5);
}
