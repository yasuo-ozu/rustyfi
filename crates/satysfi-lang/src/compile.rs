//! A closure-compiling ("JIT-style") evaluator that lowers an [`Ast`] into a
//! tree of Rust closures once, so that repeated evaluation chases compiled
//! closures instead of re-matching `Ast` nodes on every visit.
//!
//! This is an **additive, opt-in fast path**: [`crate::eval::Interp::eval`]
//! stays the reference tree-walking interpreter, and every compiled node is a
//! faithful, byte-for-byte reproduction of the corresponding `eval` match arm
//! (same evaluation order, same `env.child()` frames, same error messages and
//! spans). The two paths must always produce identical [`Value`]s; the
//! `compile.rs` unit tests below cross-check that on the shared eval test
//! programs.
//!
//! # Design (approach "a" + a safe slice of "b")
//!
//! * **Structure-only closures.** Each `Ast` node compiles to a
//!   [`CompiledExpr`] capturing its already-compiled children, so evaluation
//!   is pointer-chasing closures rather than `match ast { … }` dispatch +
//!   recursion. The runtime environment stays the existing `Rc<Frame>`
//!   HashMap chain ([`Env`]) unchanged — so variable *semantics*, shadowing,
//!   closures/let-rec/mutable-refs, and the exact "unbound variable …" error
//!   are all identical to the tree-walker.
//! * **Global constant-folding (a contained bit of slot-resolution's win).**
//!   A compile-time scope pass classifies each [`Ast::Var`]: a name bound by
//!   some enclosing `let`/`lambda`/`match`/… is a *local* and still resolves
//!   through `env.lookup` (identical behavior). A name that is **not** bound
//!   by any enclosing local frame — and is therefore provably unshadowed all
//!   the way down to the base environment — is a *global*; its `Value` is
//!   fetched from the base environment once, at compile time, and the compiled
//!   node just clones that captured value. This eliminates the frame-chain
//!   walk + HashMap lookup for primitives (`+`, `<`, `==`, …), which is where
//!   a compute-heavy workload like `fib` spends much of its time.
//!
//! Full slot-resolved (`Vec`-indexed) frames — the bigger "approach b" — would
//! need a parallel value/env world threaded through `primitives.rs`
//! (`Value::Closure`/`InlineText` capture an `Env`) and would risk the
//! byte-identical invariant; it is intentionally left as a follow-up.

use crate::ast::{Ast, Pattern};
use crate::eval::{available_fields, eval_error, match_pattern, EvalError, Interp};
use crate::value::{Env, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

/// A compiled expression: a reference-counted closure taking the runtime
/// environment and the interpreter and yielding a [`Value`] (or an
/// [`EvalError`]). Cloning is a cheap `Rc` bump, so a `CompiledExpr` can be
/// captured by several parent nodes and stored in caches / closure values.
///
/// This is a crate-internal type. It appears as the (opaque) body of the
/// public [`Value::CompiledClosure`] variant, but has no public constructor
/// and only a `pub(crate)` method — so external code can neither build nor
/// inspect it (see the `allow(private_interfaces)` note on `Value`).
#[derive(Clone)]
pub(crate) struct CompiledExpr(Rc<dyn Fn(&Env, &mut Interp<'_>) -> Result<Value, EvalError>>);

impl CompiledExpr {
    fn new(
        f: impl Fn(&Env, &mut Interp<'_>) -> Result<Value, EvalError> + 'static,
    ) -> CompiledExpr {
        CompiledExpr(Rc::new(f))
    }

    /// Run this compiled expression against `env`.
    pub(crate) fn run(&self, env: &Env, interp: &mut Interp<'_>) -> Result<Value, EvalError> {
        (self.0)(env, interp)
    }
}

impl std::fmt::Debug for CompiledExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<compiled>")
    }
}

/// The lowering pass. Carries the compile-time lexical scope (a stack of
/// name-sets, one per runtime frame that will exist at that point) and,
/// optionally, the base environment used for the global constant-folding
/// fast path.
struct Compiler<'b> {
    /// Innermost scope frame last. A name present in any frame is a *local*.
    scopes: Vec<HashSet<String>>,
    /// The base environment, when globals may be constant-folded. `None` for
    /// lazily-compiled command arguments (see [`compile_arg`]), where the
    /// captured environment's local frames are unknown, so *every* free name
    /// must fall back to `env.lookup` to stay correct.
    globals: Option<&'b Env>,
}

impl<'b> Compiler<'b> {
    fn new(globals: Option<&'b Env>) -> Compiler<'b> {
        Compiler {
            scopes: Vec::new(),
            globals,
        }
    }

    /// Is `name` bound by an enclosing local frame (and therefore *not* a
    /// constant-foldable global)?
    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().any(|frame| frame.contains(name))
    }

    /// Push a frame binding `names`, compile `body` inside it, then pop. The
    /// one-frame-per-binding-construct shape mirrors the tree-walker's
    /// `env.child()` calls exactly.
    fn in_frame<R>(
        &mut self,
        names: impl IntoIterator<Item = String>,
        body: impl FnOnce(&mut Compiler<'b>) -> R,
    ) -> R {
        self.scopes.push(names.into_iter().collect());
        let r = body(self);
        self.scopes.pop();
        r
    }

    /// If `ast` is a fully-applied call to an unshadowed base-environment
    /// primitive — `op a1 … aN` where `op` resolves (as a *global*, not a
    /// local) to a zero-args-applied [`Value::Prim`] whose arity is exactly
    /// `N` — compile it to a direct primitive-body invocation, skipping the
    /// per-argument `Value::Prim` clone + `applied`-vector churn the reference
    /// interpreter performs when currying the arguments in one at a time.
    ///
    /// This is byte-identical to the tree-walker's saturated application: the
    /// argument vector handed to `def.run` holds the same values in the same
    /// (left-to-right) order, so the primitive body — and any type error it
    /// raises — is unchanged. Under- and over-application, or a shadowed/local
    /// `op`, return `None` and fall back to ordinary nested application (which
    /// still specializes any *inner* saturated prim call as it recurses).
    fn try_saturated_prim(&mut self, ast: &Ast) -> Option<CompiledExpr> {
        let (head, args) = unfold_spine(ast);
        let Ast::Var(name, _) = head else {
            return None;
        };
        if self.is_local(name) {
            return None;
        }
        let Value::Prim { def, applied } = self.globals.and_then(|g| g.lookup(name))? else {
            return None;
        };
        if !applied.is_empty() || def.arity != args.len() {
            return None;
        }
        let run = def.run;
        let cargs: Vec<CompiledExpr> = args.iter().map(|a| self.compile(a)).collect();
        Some(CompiledExpr::new(move |env, interp| {
            let mut vals = Vec::with_capacity(cargs.len());
            for c in &cargs {
                vals.push(c.run(env, interp)?);
            }
            run(interp, vals)
        }))
    }

    fn compile(&mut self, ast: &Ast) -> CompiledExpr {
        match ast {
            Ast::Unit => CompiledExpr::new(|_, _| Ok(Value::Unit)),
            Ast::Bool(b) => {
                let b = *b;
                CompiledExpr::new(move |_, _| Ok(Value::Bool(b)))
            }
            Ast::Int(n) => {
                let n = *n;
                CompiledExpr::new(move |_, _| Ok(Value::Int(n)))
            }
            Ast::Float(x) => {
                let x = *x;
                CompiledExpr::new(move |_, _| Ok(Value::Float(x)))
            }
            Ast::Length(l) => {
                let l = *l;
                CompiledExpr::new(move |_, _| Ok(Value::Length(l)))
            }
            Ast::Str(s) => {
                let s = s.clone();
                // Mirror the tree-walker's per-eval `s.clone()`.
                CompiledExpr::new(move |_, _| Ok(Value::Str(s.clone())))
            }
            Ast::Var(name, span) => {
                if self.is_local(name) {
                    lookup_var(name.clone(), *span)
                } else if let Some(v) = self.globals.and_then(|g| g.lookup(name)) {
                    // Unshadowed base-environment name: fold to its value.
                    CompiledExpr::new(move |_, _| Ok(v.clone()))
                } else {
                    // Defensive: elaboration guarantees every `Var` is in
                    // scope, so this is unreachable for well-formed programs;
                    // fall back to the identical runtime lookup + error.
                    lookup_var(name.clone(), *span)
                }
            }
            Ast::Apply(f, arg) => {
                // Specialize a *saturated* call to an unshadowed base-env
                // primitive (`op a1 … aN` with N == arity), if any.
                if let Some(special) = self.try_saturated_prim(ast) {
                    special
                } else {
                    let cf = self.compile(f);
                    let ca = self.compile(arg);
                    CompiledExpr::new(move |env, interp| {
                        let func = cf.run(env, interp)?;
                        let arg = ca.run(env, interp)?;
                        interp.apply(func, arg)
                    })
                }
            }
            Ast::Lambda(param, body) => {
                let param = param.clone();
                let cbody = self.in_frame([param.clone()], |c| c.compile(body));
                CompiledExpr::new(move |env, _| {
                    Ok(Value::CompiledClosure {
                        opt_params: Vec::new(),
                        param: param.clone(),
                        body: cbody.clone(),
                        env: env.clone(),
                    })
                })
            }
            // `fun ?(l = x, …) p -> body` (SATySFi 0.1). The compiled body
            // sees the optional binders plus the positional param in scope
            // (they are bound at application by `Interp::apply_with_opts`).
            Ast::LambdaOpt { opts, param, body } => {
                let opts = opts.clone();
                let param = param.clone();
                let binders: Vec<String> = opts
                    .iter()
                    .map(|(_, b)| b.clone())
                    .chain(std::iter::once(param.clone()))
                    .collect();
                let cbody = self.in_frame(binders, |c| c.compile(body));
                CompiledExpr::new(move |env, _| {
                    Ok(Value::CompiledClosure {
                        opt_params: opts.clone(),
                        param: param.clone(),
                        body: cbody.clone(),
                        env: env.clone(),
                    })
                })
            }
            // `f ?(l = e, …) arg` (SATySFi 0.1) — mirror `Ast::Apply` minus
            // the saturated-prim fast path (prims reject optionals anyway).
            Ast::ApplyOpt { func, opts, arg } => {
                let cf = self.compile(func);
                let copts: Vec<(String, CompiledExpr)> = opts
                    .iter()
                    .map(|(l, e)| (l.clone(), self.compile(e)))
                    .collect();
                let ca = self.compile(arg);
                CompiledExpr::new(move |env, interp| {
                    let func = cf.run(env, interp)?;
                    let mut opt_vals = Vec::with_capacity(copts.len());
                    for (l, ce) in &copts {
                        opt_vals.push((l.clone(), ce.run(env, interp)?));
                    }
                    let arg = ca.run(env, interp)?;
                    interp.apply_with_opts(func, opt_vals, arg)
                })
            }
            Ast::LetIn(name, value, rest) => {
                let cvalue = self.compile(value);
                let name = name.clone();
                let crest = self.in_frame([name.clone()], |c| c.compile(rest));
                CompiledExpr::new(move |env, interp| {
                    let v = cvalue.run(env, interp)?;
                    let inner = env.child();
                    inner.define(name.clone(), v);
                    crest.run(&inner, interp)
                })
            }
            // Same run-time shape as `LetIn` (see `ast.rs`'s doc comment).
            Ast::LetMathIn(name, value, rest) => {
                let cvalue = self.compile(value);
                let name = name.clone();
                let crest = self.in_frame([name.clone()], |c| c.compile(rest));
                CompiledExpr::new(move |env, interp| {
                    let v = cvalue.run(env, interp)?;
                    let inner = env.child();
                    inner.define(name.clone(), v);
                    crest.run(&inner, interp)
                })
            }
            Ast::LetRecIn(bindings, body) => {
                let names: Vec<String> = bindings.iter().map(|(n, _)| n.clone()).collect();
                let (cbindings, cbody) = self.in_frame(names.clone(), |c| {
                    let cbindings: Vec<(String, CompiledExpr)> = bindings
                        .iter()
                        .map(|(n, value_ast)| (n.clone(), c.compile(value_ast)))
                        .collect();
                    let cbody = c.compile(body);
                    (cbindings, cbody)
                });
                CompiledExpr::new(move |env, interp| {
                    let inner = env.child();
                    for (name, cval) in &cbindings {
                        let v = cval.run(&inner, interp)?;
                        if !matches!(
                            v,
                            Value::Closure { .. } | Value::CompiledClosure { .. }
                        ) {
                            return eval_error(format!(
                                "let-rec binding '{name}' must be a function, got {}",
                                v.type_name()
                            ));
                        }
                        inner.define(name.clone(), v);
                    }
                    cbody.run(&inner, interp)
                })
            }
            Ast::IfThenElse(cond, then_e, else_e) => {
                let ccond = self.compile(cond);
                let cthen = self.compile(then_e);
                let celse = self.compile(else_e);
                CompiledExpr::new(move |env, interp| match ccond.run(env, interp)? {
                    Value::Bool(true) => cthen.run(env, interp),
                    Value::Bool(false) => celse.run(env, interp),
                    other => eval_error(format!(
                        "if-then-else condition must be bool, got {}",
                        other.type_name()
                    )),
                })
            }
            Ast::Record(fields) => {
                let cfields: Vec<(String, CompiledExpr)> = fields
                    .iter()
                    .map(|(name, e)| (name.clone(), self.compile(e)))
                    .collect();
                CompiledExpr::new(move |env, interp| {
                    let mut map = BTreeMap::new();
                    for (name, ce) in &cfields {
                        map.insert(name.clone(), ce.run(env, interp)?);
                    }
                    Ok(Value::Record(map))
                })
            }
            Ast::List(items) => {
                let citems: Vec<CompiledExpr> = items.iter().map(|e| self.compile(e)).collect();
                CompiledExpr::new(move |env, interp| {
                    let mut out = Vec::with_capacity(citems.len());
                    for ce in &citems {
                        out.push(ce.run(env, interp)?);
                    }
                    Ok(Value::List(out))
                })
            }
            Ast::Tuple(items) => {
                let citems: Vec<CompiledExpr> = items.iter().map(|e| self.compile(e)).collect();
                CompiledExpr::new(move |env, interp| {
                    let mut out = Vec::with_capacity(citems.len());
                    for ce in &citems {
                        out.push(ce.run(env, interp)?);
                    }
                    Ok(Value::Tuple(out))
                })
            }
            Ast::Ctor(name, arg) => {
                let name = name.clone();
                let carg = arg.as_ref().map(|a| self.compile(a));
                CompiledExpr::new(move |env, interp| {
                    let payload = match &carg {
                        Some(ce) => Some(Box::new(ce.run(env, interp)?)),
                        None => None,
                    };
                    Ok(Value::Ctor(name.clone(), payload))
                })
            }
            Ast::InlineText(elems) => {
                let elems = elems.clone();
                CompiledExpr::new(move |env, _| {
                    Ok(Value::InlineText {
                        elems: elems.clone(),
                        env: env.clone(),
                    })
                })
            }
            Ast::BlockText(elems) => {
                let elems = elems.clone();
                CompiledExpr::new(move |env, _| {
                    Ok(Value::BlockText {
                        elems: elems.clone(),
                        env: env.clone(),
                    })
                })
            }
            Ast::MathText(elems) => {
                let elems = elems.clone();
                CompiledExpr::new(move |env, _| {
                    Ok(Value::MathText {
                        elems: elems.clone(),
                        env: env.clone(),
                    })
                })
            }
            Ast::LetMutableIn(name, init, body) => {
                let cinit = self.compile(init);
                let name = name.clone();
                let cbody = self.in_frame([name.clone()], |c| c.compile(body));
                CompiledExpr::new(move |env, interp| {
                    let v = cinit.run(env, interp)?;
                    let inner = env.child();
                    inner.define(name.clone(), Value::Ref(Rc::new(RefCell::new(v))));
                    cbody.run(&inner, interp)
                })
            }
            Ast::Overwrite(name, span, value) => {
                let name = name.clone();
                let span = *span;
                let cvalue = self.compile(value);
                CompiledExpr::new(move |env, interp| {
                    let cell = env.lookup(&name).ok_or_else(|| EvalError {
                        span: Some(span),
                        msg: format!("unbound mutable variable '{name}' at run time"),
                    })?;
                    match cell {
                        Value::Ref(cell) => {
                            let v = cvalue.run(env, interp)?;
                            *cell.borrow_mut() = v;
                            Ok(Value::Unit)
                        }
                        other => Err(EvalError {
                            span: Some(span),
                            msg: format!(
                                "cannot overwrite an immutable variable '{name}' (got a value of type {})",
                                other.type_name()
                            ),
                        }),
                    }
                })
            }
            Ast::WhileDo(cond, body) => {
                let ccond = self.compile(cond);
                let cbody = self.compile(body);
                CompiledExpr::new(move |env, interp| loop {
                    match ccond.run(env, interp)? {
                        Value::Bool(true) => {
                            cbody.run(env, interp)?;
                        }
                        Value::Bool(false) => break Ok(Value::Unit),
                        other => {
                            return eval_error(format!(
                                "while-do condition must be bool, got {}",
                                other.type_name()
                            ))
                        }
                    }
                })
            }
            Ast::Sequential(e1, e2) => {
                let ce1 = self.compile(e1);
                let ce2 = self.compile(e2);
                CompiledExpr::new(move |env, interp| {
                    ce1.run(env, interp)?;
                    ce2.run(env, interp)
                })
            }
            Ast::AccessField(e, label, span) => {
                let ce = self.compile(e);
                let label = label.clone();
                let span = *span;
                CompiledExpr::new(move |env, interp| {
                    let v = ce.run(env, interp)?;
                    match v {
                        Value::Record(map) => {
                            map.get(&label).cloned().ok_or_else(|| EvalError {
                                span: Some(span),
                                msg: format!(
                                    "record has no field '{label}' (available fields: {})",
                                    available_fields(&map)
                                ),
                            })
                        }
                        other => Err(EvalError {
                            span: Some(span),
                            msg: format!(
                                "cannot access field '{label}' of a non-record value (got {})",
                                other.type_name()
                            ),
                        }),
                    }
                })
            }
            Ast::UpdateField(e, label, value) => {
                let ce = self.compile(e);
                let label = label.clone();
                let cvalue = self.compile(value);
                CompiledExpr::new(move |env, interp| {
                    let v = ce.run(env, interp)?;
                    let new_v = cvalue.run(env, interp)?;
                    match v {
                        Value::Record(mut map) => {
                            if !map.contains_key(&label) {
                                return eval_error(format!(
                                    "cannot update field '{label}': record has no such field \
                                     (available fields: {})",
                                    available_fields(&map)
                                ));
                            }
                            map.insert(label.clone(), new_v);
                            Ok(Value::Record(map))
                        }
                        other => eval_error(format!(
                            "cannot update field '{label}' of a non-record value (got {})",
                            other.type_name()
                        )),
                    }
                })
            }
            Ast::Match(scrutinee, arms) => {
                let cscrut = self.compile(scrutinee);
                let carms: Vec<CompiledArm> = arms
                    .iter()
                    .map(|arm| {
                        let mut vars = Vec::new();
                        pattern_vars(&arm.pat, &mut vars);
                        self.in_frame(vars, |c| CompiledArm {
                            pat: arm.pat.clone(),
                            guard: arm.guard.as_ref().map(|g| c.compile(g)),
                            body: c.compile(&arm.body),
                        })
                    })
                    .collect();
                CompiledExpr::new(move |env, interp| {
                    let v = cscrut.run(env, interp)?;
                    for arm in &carms {
                        let mut bindings = Vec::new();
                        if !match_pattern(&arm.pat, &v, &mut bindings) {
                            continue;
                        }
                        let inner = env.child();
                        for (name, val) in bindings {
                            inner.define(name, val);
                        }
                        if let Some(guard) = &arm.guard {
                            match guard.run(&inner, interp)? {
                                Value::Bool(true) => {}
                                Value::Bool(false) => continue,
                                other => {
                                    return eval_error(format!(
                                        "match guard must be bool, got {}",
                                        other.type_name()
                                    ))
                                }
                            }
                        }
                        return arm.body.run(&inner, interp);
                    }
                    eval_error(format!(
                        "non-exhaustive match: no arm matched a value of type {}",
                        v.type_name()
                    ))
                })
            }
        }
    }
}

/// One compiled match arm: the (uncompiled) pattern is kept for the runtime
/// [`match_pattern`] structural test; the guard and body are compiled in a
/// scope extended with the pattern's bound variables.
struct CompiledArm {
    pat: Pattern,
    guard: Option<CompiledExpr>,
    body: CompiledExpr,
}

/// The shared local-variable lookup: identical to the tree-walker's
/// `Ast::Var` arm (same span, same "unbound variable …" message).
fn lookup_var(name: String, span: satysfi_syntax::Span) -> CompiledExpr {
    CompiledExpr::new(move |env, _| {
        env.lookup(&name).ok_or_else(|| EvalError {
            span: Some(span),
            msg: format!("unbound variable '{name}' at run time"),
        })
    })
}

/// Unfold a left-nested application spine `((h a1) a2) … aN` into its head
/// `h` and the argument list `[a1, a2, …, aN]` in left-to-right (source)
/// order.
fn unfold_spine(ast: &Ast) -> (&Ast, Vec<&Ast>) {
    let mut args = Vec::new();
    let mut head = ast;
    while let Ast::Apply(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    args.reverse();
    (head, args)
}

/// Collect the variable names a pattern binds (order irrelevant — this only
/// feeds the compile-time scope's membership test, so that pattern-bound
/// names are treated as locals rather than constant-folded globals).
fn pattern_vars(pat: &Pattern, out: &mut Vec<String>) {
    match pat {
        Pattern::Wild
        | Pattern::Unit
        | Pattern::Bool(_)
        | Pattern::Int(_)
        | Pattern::Str(_)
        | Pattern::EmptyList => {}
        Pattern::Var(name) => out.push(name.clone()),
        Pattern::As(inner, name) => {
            pattern_vars(inner, out);
            out.push(name.clone());
        }
        Pattern::Tuple(ps) => {
            for p in ps {
                pattern_vars(p, out);
            }
        }
        Pattern::Cons(h, t) => {
            pattern_vars(h, out);
            pattern_vars(t, out);
        }
        Pattern::Ctor(_, Some(p)) => pattern_vars(p, out),
        Pattern::Ctor(_, None) => {}
    }
}

/// Compile a top-level program body against `base_env`. Names bound by the
/// program's own `let`s become locals as compilation descends; names that
/// remain free are the (unshadowed) base-environment primitives, which are
/// constant-folded to their captured values.
pub(crate) fn compile_program(ast: &Ast, base_env: &Env) -> CompiledExpr {
    Compiler::new(Some(base_env)).compile(ast)
}

/// Compile a command argument / embed expression evaluated later inside
/// `read_inline`/`read_block`. The captured environment's local frames are
/// not known here, so no global constant-folding is done — every free name
/// resolves through `env.lookup`, exactly as the tree-walker would.
pub(crate) fn compile_arg(ast: &Ast) -> CompiledExpr {
    Compiler::new(None).compile(ast)
}

#[cfg(test)]
mod tests {
    //! Cross-checks that the compiled path (this module) and the reference
    //! tree-walker ([`crate::eval::Interp::eval`]) produce identical results
    //! on the same programs, plus opt-in (`#[ignore]`) micro-benchmarks.
    //!
    //! Run the benchmarks with, e.g.:
    //! `cargo test -p satysfi-lang --release -- --ignored --nocapture bench_`

    use super::*;
    use crate::ast::{MatchArm, Pattern};
    use crate::eval::Interp;
    use crate::value::{Env, Value};
    use satysfi_backend::{FontKey, FontMetrics, Length};
    use satysfi_syntax::Span;

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

    // ---- small Ast builders (mirroring the eval_phase2 test helpers) -------

    fn var(name: &str) -> Ast {
        Ast::Var(name.to_string(), Span::default())
    }
    fn app1(f: Ast, a: Ast) -> Ast {
        Ast::Apply(Box::new(f), Box::new(a))
    }
    fn app2(name: &str, a: Ast, b: Ast) -> Ast {
        app1(app1(var(name), a), b)
    }

    /// `let rec fib n = if n < 2 then n else fib (n-1) + fib (n-2) in fib n`.
    fn fib_program(n: i64) -> Ast {
        let body = Ast::IfThenElse(
            Box::new(app2("<", var("n"), Ast::Int(2))),
            Box::new(var("n")),
            Box::new(app2(
                "+",
                app1(var("fib"), app2("-", var("n"), Ast::Int(1))),
                app1(var("fib"), app2("-", var("n"), Ast::Int(2))),
            )),
        );
        let fib_lambda = Rc::new(Ast::Lambda("n".to_string(), Rc::new(body)));
        Ast::LetRecIn(
            vec![("fib".to_string(), fib_lambda)],
            Box::new(app1(var("fib"), Ast::Int(n))),
        )
    }

    fn eval_tree(env: &Env, ast: &Ast) -> Result<Value, EvalError> {
        let mono = Mono;
        let mut interp = Interp::new(&mono);
        interp.eval(env, ast)
    }

    fn eval_compiled(env: &Env, ast: &Ast) -> Result<Value, EvalError> {
        let mono = Mono;
        let mut interp = Interp::new(&mono);
        compile_program(ast, env).run(env, &mut interp)
    }

    /// Assert the two evaluators agree on `ast`: identical `Value` (compared
    /// by structural `Debug`, which is byte-identical for every non-closure
    /// value) on success, identical error text on failure.
    fn assert_agree(ast: &Ast) {
        let env_t = crate::primitives::base_env();
        let env_c = crate::primitives::base_env();
        match (eval_tree(&env_t, ast), eval_compiled(&env_c, ast)) {
            (Ok(t), Ok(c)) => assert_eq!(
                format!("{t:?}"),
                format!("{c:?}"),
                "compiled value differs from tree-walker"
            ),
            (Err(t), Err(c)) => assert_eq!(
                t.to_string(),
                c.to_string(),
                "compiled error differs from tree-walker"
            ),
            (t, c) => panic!("ok/err mismatch: tree={t:?} compiled={c:?}"),
        }
    }

    /// SATySFi 0.1 labeled-optional lambda/application (`LambdaOpt`/
    /// `ApplyOpt`): the compiled path and the tree-walker must agree on the
    /// `Some`/`None` defaulting — a provided `?(bias = e)` binds `Some e`, an
    /// omitted one (a plain apply of an opt-closure) binds `None`.
    #[test]
    fn cross_check_labeled_optionals() {
        use std::rc::Rc;
        // `fun ?(bias = b) x -> x + (match b with None -> 0 | Some v -> v end)`
        let body = app2(
            "+",
            var("x"),
            Ast::Match(
                Box::new(var("b")),
                vec![
                    MatchArm {
                        pat: Pattern::Ctor("None".to_string(), None),
                        guard: None,
                        body: Ast::Int(0),
                    },
                    MatchArm {
                        pat: Pattern::Ctor(
                            "Some".to_string(),
                            Some(Box::new(Pattern::Var("v".to_string()))),
                        ),
                        guard: None,
                        body: var("v"),
                    },
                ],
            ),
        );
        let lam = Ast::LambdaOpt {
            opts: vec![("bias".to_string(), "b".to_string())],
            param: "x".to_string(),
            body: Rc::new(body),
        };
        // provided `?(bias = 40) 2` -> 42
        assert_agree(&Ast::ApplyOpt {
            func: Box::new(lam.clone()),
            opts: vec![("bias".to_string(), Ast::Int(40))],
            arg: Box::new(Ast::Int(2)),
        });
        // omitted (plain apply of an opt-closure) -> bias defaults None -> 2
        assert_agree(&app1(lam, Ast::Int(2)));
    }

    #[test]
    fn cross_check_literals_and_arithmetic() {
        assert_agree(&Ast::Int(42));
        assert_agree(&Ast::Str("hi".to_string()));
        assert_agree(&Ast::Bool(true));
        assert_agree(&app2("+", Ast::Int(2), Ast::Int(3)));
        assert_agree(&app2("*", Ast::Int(7), Ast::Int(6)));
        assert_agree(&app2("<", Ast::Int(2), Ast::Int(3)));
        assert_agree(&app2("^", Ast::Str("foo".into()), Ast::Str("bar".into())));
        // division by zero: both must error the same way.
        assert_agree(&app2("/", Ast::Int(1), Ast::Int(0)));
    }

    #[test]
    fn cross_check_let_lambda_and_capture() {
        // let const a b = a in const 1 2
        assert_agree(&Ast::LetIn(
            "id".into(),
            Box::new(Ast::Lambda("x".into(), Rc::new(var("x")))),
            Box::new(app1(var("id"), Ast::Int(7))),
        ));
        // capture of an outer let through a closure: let a = 5 in (fun x -> a + x) 3
        assert_agree(&Ast::LetIn(
            "a".into(),
            Box::new(Ast::Int(5)),
            Box::new(app1(
                Ast::Lambda("x".into(), Rc::new(app2("+", var("a"), var("x")))),
                Ast::Int(3),
            )),
        ));
    }

    #[test]
    fn cross_check_let_rec_fib_and_mutual() {
        assert_agree(&fib_program(15));
        // mutual even/odd
        let even_body = Ast::IfThenElse(
            Box::new(app2("==", var("n"), Ast::Int(0))),
            Box::new(Ast::Bool(true)),
            Box::new(app1(var("odd"), app2("-", var("n"), Ast::Int(1)))),
        );
        let odd_body = Ast::IfThenElse(
            Box::new(app2("==", var("n"), Ast::Int(0))),
            Box::new(Ast::Bool(false)),
            Box::new(app1(var("even"), app2("-", var("n"), Ast::Int(1)))),
        );
        let bindings = vec![
            (
                "even".to_string(),
                Rc::new(Ast::Lambda("n".into(), Rc::new(even_body))),
            ),
            (
                "odd".to_string(),
                Rc::new(Ast::Lambda("n".into(), Rc::new(odd_body))),
            ),
        ];
        assert_agree(&Ast::LetRecIn(
            bindings,
            Box::new(Ast::Tuple(vec![
                app1(var("even"), Ast::Int(10)),
                app1(var("odd"), Ast::Int(7)),
            ])),
        ));
        // a non-function let-rec binding errors identically in both paths.
        assert_agree(&Ast::LetRecIn(
            vec![("x".into(), Rc::new(Ast::Int(1)))],
            Box::new(var("x")),
        ));
    }

    #[test]
    fn cross_check_records_lists_tuples_and_fields() {
        assert_agree(&Ast::Record(vec![
            ("a".into(), Ast::Int(1)),
            ("b".into(), Ast::Str("x".into())),
        ]));
        assert_agree(&Ast::List(vec![Ast::Int(1), Ast::Int(2), Ast::Int(3)]));
        assert_agree(&Ast::Tuple(vec![Ast::Int(1), Ast::Bool(true)]));
        // field access, present and absent (absent => identical error)
        let rec = Ast::Record(vec![("a".into(), Ast::Int(9)), ("b".into(), Ast::Int(8))]);
        assert_agree(&Ast::AccessField(
            Box::new(rec.clone()),
            "a".into(),
            Span::default(),
        ));
        assert_agree(&Ast::AccessField(
            Box::new(rec.clone()),
            "zzz".into(),
            Span::default(),
        ));
        // functional update, present and absent
        assert_agree(&Ast::UpdateField(
            Box::new(rec.clone()),
            "a".into(),
            Box::new(Ast::Int(100)),
        ));
        assert_agree(&Ast::UpdateField(
            Box::new(rec),
            "nope".into(),
            Box::new(Ast::Int(1)),
        ));
    }

    #[test]
    fn cross_check_match_arms_guards_and_ctors() {
        // int literal + wildcard
        assert_agree(&Ast::Match(
            Box::new(Ast::Int(3)),
            vec![
                MatchArm {
                    pat: Pattern::Int(1),
                    guard: None,
                    body: Ast::Str("one".into()),
                },
                MatchArm {
                    pat: Pattern::Wild,
                    guard: None,
                    body: Ast::Str("other".into()),
                },
            ],
        ));
        // guard selecting the second arm
        assert_agree(&Ast::Match(
            Box::new(Ast::Int(4)),
            vec![
                MatchArm {
                    pat: Pattern::Var("x".into()),
                    guard: Some(app2(">", var("x"), Ast::Int(10))),
                    body: Ast::Str("big".into()),
                },
                MatchArm {
                    pat: Pattern::Var("x".into()),
                    guard: Some(app2(">", var("x"), Ast::Int(0))),
                    body: Ast::Str("small".into()),
                },
                MatchArm {
                    pat: Pattern::Wild,
                    guard: None,
                    body: Ast::Str("np".into()),
                },
            ],
        ));
        // cons/empty-list, `as`, and ctor payload
        assert_agree(&Ast::Match(
            Box::new(Ast::List(vec![Ast::Int(1), Ast::Int(2)])),
            vec![
                MatchArm {
                    pat: Pattern::EmptyList,
                    guard: None,
                    body: Ast::Int(-1),
                },
                MatchArm {
                    pat: Pattern::Cons(
                        Box::new(Pattern::Var("h".into())),
                        Box::new(Pattern::Var("t".into())),
                    ),
                    guard: None,
                    body: var("h"),
                },
            ],
        ));
        assert_agree(&Ast::Match(
            Box::new(Ast::Ctor("Some".into(), Some(Box::new(Ast::Int(5))))),
            vec![
                MatchArm {
                    pat: Pattern::Ctor("None".into(), None),
                    guard: None,
                    body: Ast::Int(0),
                },
                MatchArm {
                    pat: Pattern::Ctor("Some".into(), Some(Box::new(Pattern::Var("x".into())))),
                    guard: None,
                    body: var("x"),
                },
            ],
        ));
        // non-exhaustive => identical error
        assert_agree(&Ast::Match(
            Box::new(Ast::Int(5)),
            vec![MatchArm {
                pat: Pattern::Int(1),
                guard: None,
                body: Ast::Int(0),
            }],
        ));
    }

    #[test]
    fn cross_check_mutable_while_and_sequential() {
        // let-mutable acc <- 0 in
        // let-mutable i <- 0 in
        //   (while i < 5 do (acc <- acc + i before i <- i + 1)) before !acc
        let deref = |n: &str| app1(var("!"), var(n));
        let loop_body = Ast::Sequential(
            Box::new(Ast::Overwrite(
                "acc".into(),
                Span::default(),
                Box::new(app2("+", deref("acc"), deref("i"))),
            )),
            Box::new(Ast::Overwrite(
                "i".into(),
                Span::default(),
                Box::new(app2("+", deref("i"), Ast::Int(1))),
            )),
        );
        let while_loop = Ast::WhileDo(
            Box::new(app2("<", deref("i"), Ast::Int(5))),
            Box::new(loop_body),
        );
        let prog = Ast::LetMutableIn(
            "acc".into(),
            Box::new(Ast::Int(0)),
            Box::new(Ast::LetMutableIn(
                "i".into(),
                Box::new(Ast::Int(0)),
                Box::new(Ast::Sequential(Box::new(while_loop), Box::new(deref("acc")))),
            )),
        );
        assert_agree(&prog); // 0+1+2+3+4 = 10
    }

    // ---- document-level cross-check + shared prep for the doc benchmark ----

    /// Merge the `stdja-mini` prelude ahead of `src` (as the loader does),
    /// then elaborate + typecheck, returning `(base_env, elaborated body)`.
    fn prepare_document(src: &str) -> (Env, Ast) {
        let lib_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../lib-satysfi/dist/packages/stdja-mini.satyh");
        let lib_src = std::fs::read_to_string(&lib_path).unwrap();
        let lib_file = satysfi_syntax::parse_file(&lib_src).unwrap();
        let doc_file = satysfi_syntax::parse_file(src).unwrap();
        let mut prelude = lib_file.prelude;
        prelude.extend(doc_file.prelude);
        let merged = satysfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: doc_file.in_kw,
            body: doc_file.body,
            eoi: doc_file.eoi,
        };
        let env = crate::primitives::base_env();
        let scope = crate::elaborate::Scope::new(env.names());
        let program = crate::elaborate::elaborate_program(&merged, &scope).unwrap();
        crate::typecheck::typecheck(&program).unwrap();
        (env, program.body)
    }

    fn many_paragraph_src(n: usize) -> String {
        let mut body = String::new();
        for i in 0..n {
            body.push_str(&format!(
                "+p {{ paragraph number {i} with a few \\emph{{words}} to typeset here }}\n"
            ));
        }
        format!("document (||) '< {body} >")
    }

    #[test]
    fn cross_check_document_many_paragraphs() {
        let (env_t, body) = prepare_document(&many_paragraph_src(12));
        let doc_t = eval_tree(&env_t, &body).unwrap();
        let (env_c, _) = prepare_document(&many_paragraph_src(12));
        let doc_c = eval_compiled(&env_c, &body).unwrap();
        // The whole typeset document (pages/boxes) must be byte-identical.
        assert_eq!(
            format!("{doc_t:?}"),
            format!("{doc_c:?}"),
            "compiled document differs from tree-walker"
        );
        assert!(matches!(doc_t, Value::Document(_)));
    }

    // ---- benchmarks (opt-in) ----------------------------------------------

    fn bench_ns<F: FnMut()>(iters: u32, mut f: F) -> f64 {
        // warm up
        f();
        let start = std::time::Instant::now();
        for _ in 0..iters {
            f();
        }
        start.elapsed().as_nanos() as f64 / iters as f64
    }

    #[test]
    #[ignore = "benchmark; run with --release -- --ignored --nocapture"]
    fn bench_fib() {
        const N: i64 = 28;
        // fib call count = 2*fib(N+1) - 1
        let calls = {
            let (mut a, mut b) = (0u64, 1u64);
            for _ in 0..=N + 1 {
                let t = a + b;
                a = b;
                b = t;
            }
            2 * a - 1
        };
        let prog = fib_program(N);
        let env = crate::primitives::base_env();

        let tree = bench_ns(20, || {
            let _ = eval_tree(&env, &prog).unwrap();
        });
        // Compile once; time only repeated execution (compilation is one-off).
        let compiled = compile_program(&prog, &env);
        let mono = Mono;
        let mut interp = Interp::new(&mono);
        let comp = bench_ns(20, || {
            let _ = compiled.run(&env, &mut interp).unwrap();
        });

        println!("\n== fib({N}) : {calls} calls/eval ==");
        println!(
            "  tree-walk : {:>9.0} ns/eval  ({:>5.1} ns/call)",
            tree,
            tree / calls as f64
        );
        println!(
            "  compiled  : {:>9.0} ns/eval  ({:>5.1} ns/call)",
            comp,
            comp / calls as f64
        );
        println!("  speedup   : {:.2}x", tree / comp);
    }

    #[test]
    #[ignore = "benchmark; run with --release -- --ignored --nocapture"]
    fn bench_many_paragraph_document() {
        const PARAS: usize = 300;
        let (env, body) = prepare_document(&many_paragraph_src(PARAS));

        let tree = bench_ns(20, || {
            let _ = eval_tree(&env, &body).unwrap();
        });
        let comp = bench_ns(20, || {
            let _ = eval_compiled(&env, &body).unwrap();
        });

        println!("\n== document with {PARAS} paragraphs ==");
        println!("  tree-walk : {:>10.0} ns/doc", tree);
        println!("  compiled  : {:>10.0} ns/doc", comp);
        println!("  speedup   : {:.2}x", tree / comp);
    }
}
