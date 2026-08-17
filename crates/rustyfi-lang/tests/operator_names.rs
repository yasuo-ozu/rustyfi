//! End-to-end coverage for parenthesized-operator NAMES (`( ‹op› )`) — the
//! gap that blocked porting `itemize.satyh`/`progsynt.satyh`/`proof.satyh`
//! (upstream's `let (+++>) = ..`/`val (-->) : ty` binding forms and the bare
//! `(+++)` atomic-expression reference). Real source text run through
//! `parse_file` -> `elaborate::elaborate_program` -> `typecheck::typecheck`
//! -> `eval::Interp`, mirroring `tests/type_synonym.rs`'s harness.

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

fn eval_str(src: &str) -> Result<Value, CompileError> {
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
    match eval_str(src) {
        Ok(Value::Int(n)) => n,
        Ok(other) => panic!("{src:?} evaluated to {other:?}, not an int"),
        Err(e) => panic!("{src:?} failed to parse/typecheck/evaluate: {e}"),
    }
}

fn boolean(src: &str) -> bool {
    match eval_str(src) {
        Ok(Value::Bool(b)) => b,
        Ok(other) => panic!("{src:?} evaluated to {other:?}, not a bool"),
        Err(e) => panic!("{src:?} failed to parse/typecheck/evaluate: {e}"),
    }
}

// ---- `let ( op ) = ..` / `let ( op ) param* = ..` -------------------------

#[test]
fn let_paren_op_binding_with_no_params() {
    // The task's first minimal repro (was a parse error before this change).
    assert_eq!(int("let (+++>) = 1 in 0"), 0);
}

#[test]
fn let_paren_op_binding_curried_infix_use() {
    // `(<+>)` is bound as an ordinary 2-ary function, then used infix — the
    // `OpChain` fold already resolves any user-bound operator name generically
    // (`elaborate.rs`'s `op_chain`), so this needs no elaborator change beyond
    // the `(op)` NAME grammar itself.
    assert_eq!(int("let (<+>) a b = a + b in 3 <+> 4"), 7);
}

#[test]
fn let_paren_op_binding_prefix_use() {
    // The same binding, applied prefix-style (`(op) x y`), matching how
    // `progsynt.satyh`-style packages mostly use their custom operators.
    assert_eq!(int("let (-->) a b = a * b in (-->) 3 4"), 12);
}

// ---- `( op )` as a bare atomic-expression value reference -----------------

#[test]
fn bare_paren_op_reference_to_a_builtin_primitive() {
    // `(+)` referencing the registered `"+"` primitive as a first-class
    // value (`ast::Atomic::OpRef`), same as `(+++)`/`(-->)` would.
    assert_eq!(int("let f = (+) in f 3 4"), 7);
}

#[test]
fn bare_paren_op_reference_applied_directly() {
    assert_eq!(int("(*) 6 7"), 42);
}

// ---- `val ( op ) : ty` in a module signature ------------------------------

#[test]
fn module_sig_val_paren_op_matches_struct_let_paren_op() {
    // The task's second minimal repro (was a parse error at `module`
    // before this change): a `val (op) : ty` signature item alongside a
    // matching `let (op) = ..` in the struct body. Module signatures are
    // parsed but not yet enforced against the struct (`elaborate.rs`'s
    // `TopBinding::Module` doc comment), so this exercises parsing +
    // elaboration + evaluation of the operator-named binding end-to-end via
    // `open`.
    let src = "\
        module M : sig val (-->) : int -> int -> int end = struct \
            let (-->) a b = a + b \
        end in open M in (-->) 3 4";
    assert_eq!(int(src), 7);
}

#[test]
fn module_sig_val_paren_op_parses_with_unresolved_type_name() {
    // The task's exact second repro, verbatim: an undeclared type `t` in the
    // signature. Since `sig .. end` is parsed but never consulted by the
    // (untyped) elaborator or the typechecker, this only needs to PARSE.
    let src = "module M : sig val (-->) : t -> t -> t end = struct \
        let (-->) a b = a end";
    rustyfi_syntax::parse_file(src).expect("the parenthesized-operator sig/struct form should parse");
}

// ---- `not` binds looser than application (real-world compat round 4) ------

#[test]
fn not_binds_looser_than_application() {
    // Upstream `not f x` is `not (f x)`, NOT `(not f) x` (which would apply
    // `not` to a function value and fail to typecheck). The blocker case was
    // `satysfi-xpath`'s `util.satyh`: `not float-zero-or-nan (a +. 1.)`.
    // `id true` is `true`, so `not id true` must be `not (id true)` = `false`.
    let src = "let id = fun b -> b in not id true";
    assert!(!boolean(src), "`not id true` must fold as `not (id true)`");
}

#[test]
fn not_still_first_class_in_argument_position() {
    // The looser-binding rule only fires when `not` is the HEAD of an
    // application; a `not` sitting in ARGUMENT position stays the ordinary
    // `not` primitive value, so it can still be passed to a higher-order
    // function. `apply not true` = `not true` = `false`.
    let src = "let apply = fun f x -> f x in apply not true";
    assert!(!boolean(src), "`not` in argument position must stay first-class");
}

// ---- nested-module operator via local-open (real-world compat round 4) ----

#[test]
fn nested_module_operator_local_open() {
    // `satysfi-fss`'s `fss/font/selection.satyg`: a nested `module Inner`
    // whose members (including an operator `(<)`) are bound under the fully-
    // qualified `Outer.Inner.<`, referenced from a sibling via a local-open
    // `Inner.(z < z)`. `open_module` must resolve the relative alias
    // `Inner.<` to its actual qualified binding key, or the overlaid `<`
    // reaches the typechecker unbound.
    let src = "module Outer : sig val run : int end = struct \
        module Inner : sig val (<) : int -> int -> bool val z : int end = struct \
            let (<) a b = true \
            let z = 5 \
        end \
        let run = if Inner.(z < z) then 1 else 2 \
    end in Outer.run";
    assert_eq!(int(src), 1);
}
