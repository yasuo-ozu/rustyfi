//! Acceptance coverage for Gap 2 (`class-signature-lang-gaps.md`-style):
//! marker-less optional-argument defaulting — calling an ordinary (non-
//! command) function with a leading `?:`-optional parameter (`Param::
//! Optional`, `cst.rs`) *without* any `?:`/`?*` marker at the call site, e.g.
//! `progsynt.satyh`'s `let to-math ?:iopt e = .. in .. to-math e1 ..`. Closed
//! entirely in `elaborate.rs`: `Scope` now tracks each `let`/`let .. in`
//! binding's leading-optional-parameter count (`Scope::optional_arity`), and
//! `app_chain_generic` synthesizes a `None` argument for every such slot a
//! bare call site leaves unmarked — so the elaborated `Ast` for a bare call
//! is byte-for-byte what an explicit `?*`/`?:` call already produced, and
//! neither `typecheck.rs` nor `eval.rs` needed any change.

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

/// The task's acceptance example: `g`'s only parameter marked `?:` is bare-
/// called with the single remaining (mandatory) argument, no marker at all.
#[test]
fn bare_call_omits_leading_optional() {
    let src = "let g ?:a b = (match a with None -> b | Some(x) -> x + b) in
g 5";
    assert_eq!(int(src), 5);
}

/// The same binding, called with an explicit `?:`-supplied argument, must
/// still work exactly as before (the new bare-call path must never fire when
/// a marker is actually present).
#[test]
fn explicit_supplied_call_still_works() {
    let src = "let g ?:a b = (match a with None -> b | Some(x) -> x + b) in
g ?:(10) 5";
    assert_eq!(int(src), 15);
}

/// And an explicit `?*` omission — pre-existing behavior, must be unchanged
/// (elaborates the same way the new bare-call path now also does).
#[test]
fn explicit_omission_marker_still_works() {
    let src = "let g ?:a b = (match a with None -> b | Some(x) -> x + b) in
g ?* 5";
    assert_eq!(int(src), 5);
}

/// Two leading optionals, partially covered: the first is explicitly
/// supplied, the second is left for the bare-call path to auto-omit, and the
/// mandatory third argument follows — proving the insertion isn't limited to
/// a single optional slot.
#[test]
fn partial_explicit_prefix_then_auto_omitted_second_optional() {
    let src = "let h ?:a ?:b c =
  (match a with None -> 0 | Some(x) -> x) + (match b with None -> 0 | Some(x) -> x) + c in
h ?:(1) 5";
    assert_eq!(int(src), 6);
}

/// A non-leading `?:` (the marker isn't the *first* parameter, matching
/// `stdja.satyh`'s `document record ?:configopt inner`) still works with an
/// EXPLICIT `?*`/`?:` marker exactly as before this change — the fix must
/// leave every marked call site byte-identical.
#[test]
fn non_leading_optional_marker_is_unaffected() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
f 1 ?* 2";
    assert_eq!(int(src), 3);
}

/// THE BUG (elabfix): a function with a NON-leading optional
/// (`stdja.satyh`/`stdjabook.satyh`'s `document record ?:configopt inner` —
/// `'a -> config ?-> block-text -> document`) is bare-called omitting the
/// optional but SUPPLYING the following positional, with NO marker at all.
/// Before the fix, the following positional (`2` here, `body` in the real
/// document) was mis-bound into the OMITTED optional's slot (`config-option`
/// vs `block-text` mismatch in the stdjabook capstone); the optional must
/// instead default to `None` and the positional resume at the next
/// parameter. So `f 1 2` must bind `x=1`, `y=None`, `z=2` (⇒ 3), exactly as
/// `f 1 ?* 2` already did above.
#[test]
fn bare_call_omits_non_leading_optional_then_supplies_positional() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
f 1 2";
    assert_eq!(int(src), 3);
}

/// The same non-leading-optional binding, with the optional EXPLICITLY
/// supplied between the two positionals, must still bind `y=Some(9)` (⇒
/// 1+2+9 = 12) — the marked-call path is unchanged by the fix.
#[test]
fn explicit_supplied_non_leading_optional_still_works() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
f 1 ?:(9) 2";
    assert_eq!(int(src), 12);
}

/// Partial application that STOPS before reaching the optional position must
/// not eagerly insert a `None`: `f 1` is a genuine partial application
/// (still awaiting `?:y` and `z`), so a later `?* 2` supplies them. If the
/// fix wrongly padded `None` the moment arguments ran out at an optional
/// slot, `f 1` would over-apply and this would break.
#[test]
fn partial_application_before_optional_is_not_padded() {
    let src = "let f x ?:y z = (match y with None -> x + z | Some(n) -> x + z + n) in
let g = f 1 in g ?* 2";
    assert_eq!(int(src), 3);
}

/// `progsynt.satyh`'s real shape: a `to-math`-like function with a leading
/// `?:` parameter, bare-called from ANOTHER function defined in the *same*
/// module — the call site elaborate.rs actually needs to fix, since
/// `to-math`'s own arity is only ever registered in the enclosing struct's
/// running `Scope`, not bubbled through a qualified cross-module lookup (a
/// documented limitation — calling the leading-optional function via its
/// *qualified* name from outside its own module/`let .. in` body still falls
/// back to old marker-only behavior).
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
