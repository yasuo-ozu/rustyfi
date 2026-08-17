//! `direct` command exposure (`docs/plans/typechecker-completion.md` §4):
//! a `module M : sig direct \cmd : ty ... end = struct ... end` must expose
//! `\cmd`/`+cmd` UNQUALIFIED at the enclosing scope, aliasing the module's
//! own qualified binding (`elaborate.rs`'s `direct_cmd_name` +
//! `TopBinding::Module` arm) — while non-`direct` sig items (`val`/`type`)
//! stay module-qualified only, exactly as before this change.
//!
//! End-to-end: real source text through `parse_file` -> `elaborate_program`
//! -> `typecheck` (type-checks) and, separately, `parse_file` ->
//! `elaborate` -> `eval::Interp` (evaluates), mirroring
//! `tests/typecheck.rs`'s and `tests/elaborate_phase2b.rs`'s own helpers.

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

fn assert_compile_error(src: &str) -> CompileError {
    match typecheck_str(src) {
        Ok(()) => panic!("expected {src:?} to be rejected, but it passed"),
        Err(e) => e,
    }
}

fn eval_str(src: &str) -> Result<Value, CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let ast = elaborate::elaborate(&file, &scope)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &ast)?)
}

// ============================================================================
// Acceptance fixture: a `direct \cmd : [string] inline-cmd` sig item exposes
// `\greet` unqualified at the module's enclosing scope.
// ============================================================================

const GREET_MODULE: &str = "
    module M : sig
      direct \\greet : [string] inline-cmd
    end = struct
      let-inline ctx \\greet s = read-inline ctx (embed-string s)
    end
";

#[test]
fn direct_inline_command_is_usable_unqualified_and_typechecks() {
    let src = format!("{GREET_MODULE} in {{ \\greet(`hi`); }}");
    assert_well_typed(&src);
}

#[test]
fn direct_inline_command_is_usable_unqualified_and_evaluates() {
    let src = format!("{GREET_MODULE} in {{ \\greet(`hi`); }}");
    // A bare `{ .. }` literal evaluates to `Value::InlineText` (a closure
    // awaiting a `read-inline ctx` to render it — see `eval_phase2b.rs`'s
    // own tests); the point here is just that evaluation *succeeds* at all
    // (no "unbound `\greet`" `EvalError`), which only happens if the
    // unqualified alias this change adds actually resolved to `M`'s own
    // `\greet` closure.
    let v = eval_str(&src).expect("direct-exposed \\greet should elaborate and evaluate");
    assert!(
        matches!(v, Value::InlineText { .. }),
        "expected an inline-text value, got {v:?}"
    );
}

#[test]
fn direct_inline_command_also_still_reachable_qualified() {
    // `direct` ADDS the unqualified alias; the qualified path stays valid.
    let src = format!("{GREET_MODULE} in {{ \\M.greet(`hi`); }}");
    assert_well_typed(&src);
}

// ============================================================================
// `direct +cmd : [..] block-cmd` — the block-command form of the same rule.
// ============================================================================

#[test]
fn direct_block_command_is_usable_unqualified_and_typechecks() {
    let src = "
        module M : sig
          direct +box : [inline-text] block-cmd
        end = struct
          let-block ctx +box it = line-break true true ctx (read-inline ctx it)
        end
        in
        '< +box{ hi } >
    ";
    assert_well_typed(src);
}

// ============================================================================
// Non-`direct` sig items stay module-qualified only (regression: `direct`
// must not leak every command, just the ones the signature marks).
// ============================================================================

#[test]
fn non_direct_command_in_a_signature_stays_qualified_only() {
    let src = "
        module M : sig
          val dummy : int
        end = struct
          let dummy = 1
          let-inline ctx \\shout it = read-inline ctx it
        end
        in
        { \\shout{ hi } }
    ";
    let err = assert_compile_error(src);
    assert!(
        err.to_string().contains("unbound") || err.to_string().contains("\\shout"),
        "expected an unbound-command error for the un-exposed \\shout, got: {err}"
    );
}

#[test]
fn non_direct_command_in_a_signature_is_still_reachable_qualified() {
    let src = "
        module M : sig
          val dummy : int
        end = struct
          let dummy = 1
          let-inline ctx \\shout it = read-inline ctx it
        end
        in
        { \\M.shout{ hi } }
    ";
    assert_well_typed(src);
}

// ============================================================================
// A `direct`-declared command the struct never actually defines is a
// (cheap, `direct`-only) sig-preservation error, not a silently-dangling
// alias — `typechecker-completion.md` §3's fuller `val`/`type` reflects
// check stays deferred; this is the narrow slice §4 asks for.
// ============================================================================

#[test]
fn direct_declared_but_unimplemented_command_is_rejected() {
    let src = "
        module M : sig
          direct \\greet : [string] inline-cmd
        end = struct
          let dummy = 1
        end
        in
        { \\greet(`hi`); }
    ";
    let err = assert_compile_error(src);
    let msg = err.to_string();
    assert!(
        msg.contains("greet"),
        "expected the missing-\\greet error to name it, got: {msg}"
    );
}

// ============================================================================
// Nested modules: a `direct` item declared on an inner module bubbles its
// unqualified exposure out through the enclosing module too (the same
// `exported` bubbling qualified names already get).
// ============================================================================

#[test]
fn direct_command_from_a_nested_module_is_usable_unqualified_at_the_top_level() {
    let src = "
        module Outer = struct
          module M : sig
            direct \\greet : [string] inline-cmd
          end = struct
            let-inline ctx \\greet s = read-inline ctx (embed-string s)
          end
        end
        in
        { \\greet(`hi`); }
    ";
    assert_well_typed(src);
}
