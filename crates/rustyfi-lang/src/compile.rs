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
use crate::quoted;
use crate::value::{BaseEnv, Env, Value};
use rustyfi_syntax::RustyfiVersion;
use std::cell::RefCell;
use std::collections::BTreeMap;
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

/// The flat table of TOP-LEVEL ("spine") binding values — Phase 2 of
/// (§6).
///
/// The loader concatenates every library prelude into one synthetic file, and
/// `elaborate::nest` folds every top-level binding into a nested
/// `LetIn`/`LetRecIn` **spine** around the document body. Each of those is one
/// runtime `env.child()` frame, so a document reference to an early prelude
/// function used to walk one frame per intervening top-level binding: measured
/// at 13-90 frames per compiled variable lookup, 109M-208M frame probes on a
/// corpus document. That was the single largest evaluator cost.
///
/// A spine binding therefore also gets a **slot index**, assigned at compile
/// time, and a compiled reference to it reads `table[slot]` instead of walking.
/// Phase 2 built the frame chain alongside this table, because quoted text
/// still resolved command names by string against a captured `Env`. Phase 3
/// removed the last such lookup and Phase 4 the chain itself, so a spine
/// binding now writes ONLY its slot — a corpus prelude no longer allocates one
/// frame per top-level binding per fixpoint trial.
///
/// Slots are written before they can be read (the spine is unconditional and
/// executes in order), so nothing needs clearing between fixpoint trials: each
/// trial simply overwrites every slot as the spine re-executes.
#[derive(Clone, Default)]
struct Globals(Rc<RefCell<Vec<Value>>>);

impl Globals {
    #[inline]
    fn get(&self, slot: usize) -> Value {
        self.0.borrow()[slot].clone()
    }

    #[inline]
    fn set(&self, slot: usize, v: Value) {
        self.0.borrow_mut()[slot] = v;
    }

    /// Size the table once compilation has assigned every slot. The compiled
    /// closures captured the same `Rc`, so they see this.
    fn finish(&self, len: usize) {
        self.0.borrow_mut().resize(len, Value::Unit);
    }
}

/// One entry of the compiler's lexical stack.
enum Scope {
    /// A real runtime frame, its names in slot order. Contributes one level
    /// of `depth` to every reference resolved past it.
    Frame(Vec<String>),
    /// A top-level (spine) binding and the [`Globals`] slot it was assigned.
    /// Contributes NO depth — spine bindings build no frame (Phase 4).
    Global(String, usize),
}

/// What [`Compiler::resolve`] found for a name.
enum Binding {
    /// A local: `(depth, index)` into the runtime frame chain.
    Local(u16, u16),
    /// A top-level binding: an index into the [`Globals`] table.
    Global(usize),
}

/// The lowering pass. Carries the compile-time lexical scope and, optionally,
/// the base environment used for the constant-folding fast path.
struct Compiler<'b> {
    /// ONE lexical stack, innermost last, holding both kinds of binding the
    /// compiler can resolve — see [`Scope`].
    ///
    /// It has to be one stack. A top-level (spine) binding and a local frame
    /// can shadow each other in either direction: the cross-version deco
    /// coercion (`v1::xver_adapt::deco_coercion_prelude`) splices a top-level
    /// `let` that shadows a `let rec` which — because its right-hand sides
    /// are not syntactic lambdas — had to fall back to a real frame. Resolving
    /// locals before globals (or the reverse) gets that backwards; only
    /// walking a single stack innermost-first gives the binding order the
    /// name-keyed frame chain used to give for free.
    scopes: Vec<Scope>,
    /// Slots assigned so far; also the table's final length.
    n_globals: usize,
    /// The table those slots index into, shared with every compiled node that
    /// reads or writes one.
    globals: Globals,
    /// The `V0_1`-slot base environment, when unshadowed names may be
    /// constant-folded.
    ///
    /// For a PURE (non-cross-version) compile this is simply *the* base
    /// environment, regardless of which language generation it actually
    /// binds — [`Compiler::new`] always parks it here and leaves
    /// `globals_v006`/`current_version` at their defaults, so `globals_for`
    /// always resolves back to this same field and the fold is unchanged
    /// from before Slice X2a.
    globals_v01: Option<&'b BaseEnv>,
    /// The `V0_0`-slot base environment — `Some` only for a cross-version
    /// splice compile ([`Compiler::new_xver`]), used exclusively while
    /// `current_version` is `V0_0` (i.e. while folding inside an
    /// `Ast::VersionScope(V0_0, _)` subtree). `None` on every pure path,
    /// so `globals_for(V0_0)` there is `None` — irrelevant, since no
    /// `VersionScope` node is ever emitted on a pure path (§X2.2.2).
    globals_v006: Option<&'b BaseEnv>,
    /// Which of the two envs above is active for the primitive fold right
    /// now — `V0_1` outside any `VersionScope`, or the tag of the innermost
    /// enclosing one (`Ast::VersionScope`'s compile arm below). Only ever
    /// changes away from its initial value on a cross-version splice
    /// compile, where `Ast::VersionScope(V0_0, _)` nodes actually occur.
    current_version: RustyfiVersion,
}

impl<'b> Compiler<'b> {
    /// The ordinary (pre-X2a-shaped) constructor: one base environment,
    /// constant-folded as before. `current_version` never moves off its
    /// initial `V0_1` slot here (no `VersionScope` node exists in a tree
    /// compiled through this path).
    fn new(globals: Option<&'b BaseEnv>) -> Compiler<'b> {
        Compiler {
            scopes: Vec::new(),
            n_globals: 0,
            globals: Globals::default(),
            globals_v01: globals,
            globals_v006: None,
            current_version: RustyfiVersion::V0_1,
        }
    }

    /// The cross-version-splice constructor (Slice X2a): `env_v01` folds
    /// every `Ast::Var` OUTSIDE a `VersionScope`; `env_v006` folds every
    /// `Ast::Var` INSIDE an `Ast::VersionScope(V0_0, _)` subtree — see the
    /// `Ast::VersionScope` compile arm below. The top-level program body
    /// always starts un-wrapped (`V0_1`), so `current_version` starts there
    /// too.
    fn new_xver(env_v01: &'b BaseEnv, env_v006: &'b BaseEnv) -> Compiler<'b> {
        Compiler {
            scopes: Vec::new(),
            n_globals: 0,
            globals: Globals::default(),
            globals_v01: Some(env_v01),
            globals_v006: Some(env_v006),
            current_version: RustyfiVersion::V0_1,
        }
    }

    /// The base environment currently active for the primitive fold —
    /// `V0_1`'s slot outside any `VersionScope`, `V0_0`'s slot inside one
    /// (only ever populated by [`Compiler::new_xver`]).
    fn globals_for(&self, version: RustyfiVersion) -> Option<&'b BaseEnv> {
        match version {
            RustyfiVersion::V0_1 => self.globals_v01,
            RustyfiVersion::V0_0 => self.globals_v006,
            // `RustyfiVersion` is `#[non_exhaustive]` (future 0.1.z-era
            // variants); this crate only ever constructs the two matched
            // above, and `Compiler`'s two env slots are exactly those two —
            // there is no third slot to resolve to.
            _ => None,
        }
    }

    /// Where `name` lives in the runtime frame chain, if a local frame binds
    /// it: `(depth, index)` — `depth` frames out from the innermost, then that
    /// position in the frame.
    ///
    /// Frames are searched innermost-first and each frame's names LAST-first,
    /// so a rebinding shadows the binding it overwrote — matching the
    /// name-keyed environment this replaced, where a second `define` of the
    /// same name in one frame simply overwrote the first. (Reachable for a
    /// pattern that binds one name twice, and for a `let rec` group that
    /// repeats one.)
    /// Where the program binds `name`, walking the lexical stack
    /// innermost-first so the most recent binding wins — the same answer the
    /// name-keyed frame chain used to give at run time.
    ///
    /// Within one frame, names are scanned LAST-first, so a frame that binds
    /// one name twice resolves to the later slot — matching the environment
    /// this replaced, where a second `define` of the same name simply
    /// overwrote the first. (Reachable for a pattern binding one name twice,
    /// and for a `let rec` group that repeats one.)
    fn resolve(&self, name: &str) -> Option<Binding> {
        let mut depth = 0u16;
        for entry in self.scopes.iter().rev() {
            match entry {
                Scope::Frame(names) => {
                    if let Some(index) = names.iter().rposition(|n| n == name) {
                        return Some(Binding::Local(depth, index as u16));
                    }
                    depth += 1;
                }
                Scope::Global(n, slot) => {
                    if n == name {
                        return Some(Binding::Global(*slot));
                    }
                }
            }
        }
        None
    }

    /// Is `name` bound by the program at all? This, not "is it a local", is
    /// what guards the base-environment constant fold: a top-level binding
    /// that shadows a primitive name must win.
    fn is_bound(&self, name: &str) -> bool {
        self.resolve(name).is_some()
    }

    /// Assign `name` the next spine slot and record it on the lexical stack.
    fn alloc_global(&mut self, name: &str) -> usize {
        let slot = self.n_globals;
        self.n_globals += 1;
        self.scopes.push(Scope::Global(name.to_string(), slot));
        slot
    }

    /// Push a frame binding `names` (in slot order), compile `body` inside it,
    /// then pop. This must stay 1:1 with where the emitted code calls
    /// `env.child(..)` — that correspondence is what makes `(depth, index)`
    /// mean the same thing at compile time and at run time.
    fn in_frame<R>(
        &mut self,
        names: impl IntoIterator<Item = String>,
        body: impl FnOnce(&mut Compiler<'b>) -> R,
    ) -> R {
        // Truncate rather than pop: `body` may have pushed `Scope::Global`
        // entries of its own (the spine continues inside a `let rec` group's
        // fallback frame), and those belong to this region too.
        let mark = self.scopes.len();
        self.scopes.push(Scope::Frame(names.into_iter().collect()));
        let r = body(self);
        self.scopes.truncate(mark);
        r
    }

    /// If `ast` is a fully-applied call to an unshadowed base-environment
    /// primitive — `op a1 … aN` where `op` resolves (as a *global*, not a
    /// local) to a zero-args-applied [`Value::Prim`] whose arity is exactly
    /// `N` — compile it to a direct primitive-body invocation, skipping the
    /// per-argument `Value::Prim` clone + `applied`-vector churn the reference
    /// interpreter performs when currying the arguments in one at a time.
    ///
    /// The argument vector handed to `def.run` holds the same values in the
    /// same (left-to-right) order as currying them in one at a time would, so
    /// the primitive body — and any type error it raises — is unchanged. Under- and over-application, or a shadowed/local
    /// `op`, return `None` and fall back to ordinary nested application (which
    /// still specializes any *inner* saturated prim call as it recurses).
    fn try_saturated_prim(&mut self, ast: &Ast) -> Option<CompiledExpr> {
        let (head, args) = unfold_spine(ast);
        let Ast::Var(name, _) = head else {
            return None;
        };
        if self.is_bound(name) {
            return None;
        }
        let Value::Prim { def, applied } = self
            .globals_for(self.current_version)
            .and_then(|g| g.lookup(name))?
        else {
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
                // One `String` clone per evaluation, as before.
                CompiledExpr::new(move |_, _| Ok(Value::Str(s.clone())))
            }
            Ast::Var(name, span) => self.compile_var_read(name, *span, "variable"),
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
                // The parameter is slot 0 of the frame `apply` pushes; its
                // name is needed only to compile the body.
                let cbody = self.in_frame([param.clone()], |c| c.compile(body));
                CompiledExpr::new(move |env, _| {
                    Ok(Value::CompiledClosure {
                        opt_labels: Vec::new(),
                        body: cbody.clone(),
                        env: env.clone(),
                    })
                })
            }
            // `fun ?(l = x, …) p -> body` (SATySFi 0.1). The compiled body
            // sees the optional binders plus the positional param in scope
            // (they are bound at application by `Interp::apply_with_opts`).
            Ast::LambdaOpt { opts, param, body } => {
                // Slot order at application: the optional binders in
                // declaration order, then the positional parameter last —
                // which is exactly the order `in_frame` records here and
                // `apply_with_opts` fills.
                let binders: Vec<String> = opts
                    .iter()
                    .map(|(_, b)| b.clone())
                    .chain(std::iter::once(param.clone()))
                    .collect();
                let cbody = self.in_frame(binders, |c| c.compile(body));
                let opt_labels: Vec<String> = opts.iter().map(|(l, _)| l.clone()).collect();
                CompiledExpr::new(move |env, _| {
                    Ok(Value::CompiledClosure {
                        opt_labels: opt_labels.clone(),
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
                let crest = self.in_frame([name.clone()], |c| c.compile(rest));
                CompiledExpr::new(move |env, interp| {
                    let v = cvalue.run(env, interp)?;
                    crest.run(&env.child(vec![v]), interp)
                })
            }
            // Same run-time shape as `LetIn` (see `ast.rs`'s doc comment).
            Ast::LetMathIn(name, value, rest) => {
                let cvalue = self.compile(value);
                let crest = self.in_frame([name.clone()], |c| c.compile(rest));
                CompiledExpr::new(move |env, interp| {
                    let v = cvalue.run(env, interp)?;
                    crest.run(&env.child(vec![v]), interp)
                })
            }
            Ast::LetRecIn(bindings, body) => {
                let names: Vec<String> = bindings.iter().map(|(n, _)| n.clone()).collect();
                let (cbindings, cbody) = self.in_frame(names.clone(), |c| {
                    let cbindings: Vec<(std::rc::Rc<str>, CompiledExpr)> = bindings
                        .iter()
                        .map(|(n, value_ast)| (n.as_str().into(), c.compile(value_ast)))
                        .collect();
                    let cbody = c.compile(body);
                    (cbindings, cbody)
                });
                let_rec_frame(cbindings, cbody)
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
            // Quoted text is compiled EAGERLY, here, in the lexical scope of
            // the quote site (Phase 3, `crate::quoted`): command names and
            // embedded expressions are resolved now rather than by string
            // against the captured environment at layout time.
            Ast::InlineText(elems) => {
                let elems = Rc::new(elems.iter().map(|e| self.compile_itext(e)).collect());
                CompiledExpr::new(move |env, _| {
                    Ok(Value::InlineText {
                        elems: Rc::clone(&elems),
                        env: env.clone(),
                    })
                })
            }
            Ast::BlockText(elems) => {
                let elems = Rc::new(elems.iter().map(|e| self.compile_btext(e)).collect());
                CompiledExpr::new(move |env, _| {
                    Ok(Value::BlockText {
                        elems: Rc::clone(&elems),
                        env: env.clone(),
                    })
                })
            }
            Ast::MathText(elems) => {
                let elems = Rc::new(elems.iter().map(|e| self.compile_melem(e)).collect());
                CompiledExpr::new(move |env, _| {
                    Ok(Value::MathText {
                        elems: Rc::clone(&elems),
                        env: env.clone(),
                    })
                })
            }
            Ast::LetMutableIn(name, init, body) => {
                let cinit = self.compile(init);
                let cbody = self.in_frame([name.clone()], |c| c.compile(body));
                CompiledExpr::new(move |env, interp| {
                    let v = cinit.run(env, interp)?;
                    let cell = Value::Ref(Rc::new(RefCell::new(v)));
                    cbody.run(&env.child(vec![cell]), interp)
                })
            }
            Ast::Overwrite(name, span, value) => {
                // `let-mutable` binds a `Value::Ref` cell; overwriting it is a
                // read of that binding (local slot, top-level slot, or — for
                // an unresolvable name — the same error as before) followed by
                // a write THROUGH the shared cell, so no frame is mutated.
                let cell_of = self.compile_var_read(name, *span, "mutable variable");
                let name = name.clone();
                let span = *span;
                let cvalue = self.compile(value);
                CompiledExpr::new(move |env, interp| {
                    let cell = cell_of.run(env, interp)?;
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
                        Value::Record(map) => map.get(&label).cloned().ok_or_else(|| EvalError {
                            span: Some(span),
                            msg: format!(
                                "record has no field '{label}' (available fields: {})",
                                available_fields(&map)
                            ),
                        }),
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
                        // `match_pattern` pushes bindings in exactly the order
                        // `pattern_vars` collected the names above, so position
                        // i in this vector IS slot i of the arm's frame.
                        let mut bindings = Vec::new();
                        if !match_pattern(&arm.pat, &v, &mut bindings) {
                            continue;
                        }
                        let inner = env.child(bindings);
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
            // Slice X2a (Option C): push the tag, compile `body`
            // (recursively — every nested `Ast::Var`/saturated-prim fold
            // reached from here, INCLUDING inside a nested `Ast::Lambda`'s
            // body, since `compile` recurses eagerly at COMPILE time, not
            // lazily at apply time — sees `current_version == v`), pop. This
            // is the whole mechanism: a version-forked primitive name folds
            // to `v`'s `PrimDef` here, and nowhere else does
            // version-sensitive resolution happen (X2.0). `is_local`
            // (checked first, in every `Var`/ `try_saturated_prim` arm
            // above) still shadows this — a local binding of the same name
            // is untouched regardless of the cursor. Never reached on a pure
            // single-version compile: no `Ast::VersionScope` node is ever
            // produced there (`elaborate_program_with_versions`'s empty
            // `v006_indices`).
            Ast::VersionScope(v, body) => {
                let prev = std::mem::replace(&mut self.current_version, *v);
                let c = self.compile(body);
                self.current_version = prev;
                c
            }
            // Ctor-scoping marker — transparent to compilation (typecheck runs
            // on the uncompiled body; see `Ast::ModuleScope`).
            Ast::ModuleScope(_, body) => self.compile(body),
        }
    }

    /// Resolve a quoted-text command name (`\emph`, `+p`, a math command) to
    /// the expression that yields its value — the same three-way
    /// classification [`Compiler::compile`]'s `Ast::Var` arm performs, but
    /// falling back to a lookup that reproduces the exact "unbound {kind}
    /// command '…' at run time" message `read_inline`/`read_block`/
    /// `reflect_math_elem` used to raise themselves.
    /// Resolve a NAME REFERENCE to the expression that yields its value —
    /// the compiler's single three-way classification, shared by `Ast::Var`,
    /// `Ast::Overwrite`'s cell lookup, and quoted text's command names:
    ///
    /// 1. the program binds it — a local frame -> a static `(depth, index)`
    ///    slot read, a top-level (spine) binding -> its [`Globals`] slot,
    ///    whichever [`Compiler::resolve`] reaches first;
    /// 2. an unshadowed base-environment name -> constant-folded to its value,
    ///    against `current_version`'s slot so a version-forked primitive
    ///    referenced inside an `Ast::VersionScope` freezes to THAT version's
    ///    `PrimDef` (Slice X2a);
    /// 3. otherwise nothing can be resolved — elaboration rejects unbound
    ///    names long before here, so this is unreachable for a well-formed
    ///    program; raise the same "unbound {what} '…' at run time" error the
    ///    runtime name lookup used to.
    ///
    /// `what` only shapes that last error: "variable", "mutable variable",
    /// "inline command", …
    fn compile_var_read(
        &mut self,
        name: &str,
        span: rustyfi_syntax::Span,
        what: &'static str,
    ) -> CompiledExpr {
        match self.resolve(name) {
            Some(Binding::Local(depth, index)) => {
                return CompiledExpr::new(move |env: &Env, _| Ok(env.slot(depth, index)))
            }
            Some(Binding::Global(slot)) => {
                let globals = self.globals.clone();
                return CompiledExpr::new(move |_, _| Ok(globals.get(slot)));
            }
            None => {}
        }
        if let Some(v) = self
            .globals_for(self.current_version)
            .and_then(|g| g.lookup(name))
        {
            return CompiledExpr::new(move |_, _| Ok(v.clone()));
        }
        let name = name.to_string();
        CompiledExpr::new(move |_, _| {
            Err(EvalError {
                span: Some(span),
                msg: format!("unbound {what} '{name}' at run time"),
            })
        })
    }

    fn compile_cmd_name(
        &mut self,
        name: &str,
        span: rustyfi_syntax::Span,
        kind: &'static str,
    ) -> CompiledExpr {
        self.compile_var_read(name, span, kind)
    }

    fn compile_cmd_arg(&mut self, a: &crate::ast::CmdArg) -> quoted::CmdArg {
        quoted::CmdArg {
            opts: a
                .opts
                .iter()
                .map(|(l, e)| (l.clone(), self.compile(e)))
                .collect(),
            arg: self.compile(&a.arg),
        }
    }

    fn compile_itext(&mut self, e: &crate::ast::IText) -> quoted::IText {
        use crate::ast::IText as A;
        match e {
            A::Text(s) => quoted::IText::Text(s.clone()),
            A::CodeText(s) => quoted::IText::CodeText(s.clone()),
            A::Cmd { name, span, args } => quoted::IText::Cmd {
                cmd: self.compile_cmd_name(name, *span, "inline command"),
                args: args.iter().map(|a| self.compile_cmd_arg(a)).collect(),
            },
            A::Embed { expr, span } => quoted::IText::Embed {
                expr: self.compile(expr),
                span: *span,
            },
            A::EmbedMath { elems, span } => quoted::IText::EmbedMath {
                elems: Rc::new(elems.iter().map(|m| self.compile_melem(m)).collect()),
                span: *span,
            },
        }
    }

    fn compile_btext(&mut self, e: &crate::ast::BText) -> quoted::BText {
        use crate::ast::BText as A;
        match e {
            A::Cmd { name, span, args } => quoted::BText::Cmd {
                cmd: self.compile_cmd_name(name, *span, "block command"),
                args: args.iter().map(|a| self.compile_cmd_arg(a)).collect(),
            },
            A::Embed { expr, span } => quoted::BText::Embed {
                expr: self.compile(expr),
                span: *span,
            },
        }
    }

    fn compile_melem(&mut self, e: &crate::ast::MathElem) -> quoted::MathElem {
        use crate::ast::MathElem as A;
        match e {
            A::Chars(s) => quoted::MathElem::Chars(s.clone()),
            A::Group(es) => {
                quoted::MathElem::Group(es.iter().map(|x| self.compile_melem(x)).collect())
            }
            A::Sub(b, s) => quoted::MathElem::Sub(
                Box::new(self.compile_melem(b)),
                s.iter().map(|x| self.compile_melem(x)).collect(),
            ),
            A::Sup(b, s) => quoted::MathElem::Sup(
                Box::new(self.compile_melem(b)),
                s.iter().map(|x| self.compile_melem(x)).collect(),
            ),
            A::Primes(b, n) => quoted::MathElem::Primes(Box::new(self.compile_melem(b)), *n),
            A::Cmd { name, span, args } => quoted::MathElem::Cmd {
                cmd: self.compile_cmd_name(name, *span, "math command"),
                name: name.as_str().into(),
                span: *span,
                args: args.iter().map(|a| self.compile_cmd_arg(a)).collect(),
            },
            A::Embed { expr, span } => quoted::MathElem::Embed {
                expr: self.compile(expr),
                span: *span,
            },
        }
    }

    /// Compile the top-level **spine** — the unbroken chain of Let-shaped
    /// nodes `elaborate::nest` wraps around the document body, one per
    /// top-level/`@require`d binding — giving each binding a [`Globals`] slot
    /// so that references to it compile to an index instead of a frame-chain
    /// walk (Phase 2, §6).
    ///
    /// Each arm evaluates its right-hand side in the same order its
    /// [`Compiler::compile`] counterpart does, and raises the same errors —
    /// but writes the result to a slot instead of into a frame. Phase 2 still
    /// built the frame chain alongside, because quoted text resolved command
    /// names (`\emph`, `+p` — themselves top-level bindings) by string
    /// against a captured `Env` at layout time. Phase 3 removed the last such
    /// lookup, so the chain is gone too: a corpus prelude no longer allocates
    /// one frame per top-level binding per fixpoint trial.
    ///
    /// The first node that is not Let-shaped is the document body; it and
    /// everything under it compile normally.
    fn compile_spine(&mut self, ast: &Ast) -> CompiledExpr {
        match ast {
            Ast::LetIn(name, value, rest) => self.spine_let(name, value, rest, false),
            // Same runtime shape as `LetIn` (see `ast.rs`).
            Ast::LetMathIn(name, value, rest) => self.spine_let(name, value, rest, false),
            Ast::LetMutableIn(name, init, body) => self.spine_let(name, init, body, true),
            Ast::LetRecIn(bindings, body) => self.spine_let_rec(bindings, body),
            // Not a binding — this is the document body.
            other => self.compile(other),
        }
    }

    /// The shared `LetIn`/`LetMathIn`/`LetMutableIn` spine arm. `mutable`
    /// selects the `let-mutable` form, whose bound value is a fresh `Ref`
    /// cell (the SAME cell goes into both the frame and the slot, so an
    /// `Ast::Overwrite` reached through either sees the other's writes).
    ///
    /// Note the ordering: `value` is compiled BEFORE the slot is allocated, so
    /// a reference to `name` inside its own right-hand side still resolves to
    /// whatever `name` meant before this binding — matching the runtime, which
    /// evaluates `value` in the outer env and only then creates the frame.
    fn spine_let(&mut self, name: &str, value: &Ast, rest: &Ast, mutable: bool) -> CompiledExpr {
        let cvalue = self.compile(value);
        let slot = self.alloc_global(name);
        let crest = self.compile_spine(rest);
        let globals = self.globals.clone();
        CompiledExpr::new(move |env, interp| {
            let v = cvalue.run(env, interp)?;
            globals.set(
                slot,
                if mutable {
                    Value::Ref(Rc::new(RefCell::new(v)))
                } else {
                    v
                },
            );
            crest.run(env, interp)
        })
    }

    /// The `LetRecIn` spine arm.
    ///
    /// Slots are allocated for the whole group BEFORE its values are compiled,
    /// since every name is in scope in every body — but that is only sound
    /// when no value can *read* a sibling while the group is still being
    /// filled. At run time the group's frame is populated one binding at a
    /// time, so a read of a not-yet-defined sibling falls through to the outer
    /// scope, whereas a slot read would see the previous trial's value. A
    /// syntactic `fun`/`fun ?(..)` right-hand side cannot read anything at
    /// definition time (evaluating a lambda never runs its body), which is the
    /// case every working program is in — the runtime rejects a non-function
    /// `let-rec` binding anyway. When some value is *not* syntactically a
    /// lambda, this falls back to an ordinary local frame for the group (the
    /// spine continues for the body).
    fn spine_let_rec(&mut self, bindings: &[(String, Rc<Ast>)], body: &Ast) -> CompiledExpr {
        let all_lambda = bindings
            .iter()
            .all(|(_, v)| matches!(**v, Ast::Lambda(..) | Ast::LambdaOpt { .. }));
        if !all_lambda {
            let names: Vec<String> = bindings.iter().map(|(n, _)| n.clone()).collect();
            let (cbindings, cbody) = self.in_frame(names, |c| {
                let cbindings: Vec<(Rc<str>, CompiledExpr)> = bindings
                    .iter()
                    .map(|(n, value_ast)| (n.as_str().into(), c.compile(value_ast)))
                    .collect();
                let cbody = c.compile_spine(body);
                (cbindings, cbody)
            });
            return let_rec_frame(cbindings, cbody);
        }
        let slots: Vec<usize> = bindings.iter().map(|(n, _)| self.alloc_global(n)).collect();
        let cbindings: Vec<(Rc<str>, CompiledExpr)> = bindings
            .iter()
            .map(|(n, value_ast)| (n.as_str().into(), self.compile(value_ast)))
            .collect();
        let cbody = self.compile_spine(body);
        let globals = self.globals.clone();
        CompiledExpr::new(move |env, interp| {
            for ((name, cval), slot) in cbindings.iter().zip(slots.iter()) {
                let v = cval.run(env, interp)?;
                if !matches!(v, Value::CompiledClosure { .. }) {
                    return eval_error(format!(
                        "let-rec binding '{name}' must be a function, got {}",
                        v.type_name()
                    ));
                }
                globals.set(*slot, v);
            }
            cbody.run(env, interp)
        })
    }
}

/// The ordinary (non-spine) `let-rec` runtime shape, shared by
/// [`Compiler::compile`]'s arm and [`Compiler::spine_let_rec`]'s fallback.
fn let_rec_frame(cbindings: Vec<(Rc<str>, CompiledExpr)>, cbody: CompiledExpr) -> CompiledExpr {
    CompiledExpr::new(move |env, interp| {
        // Pre-sized with placeholders and back-patched in order: a closure
        // built by an earlier binding captures this frame and sees the later
        // fills, which is what makes the group mutually recursive. The names
        // survive only for the "must be a function" message.
        //
        // A value that EAGERLY reads a not-yet-filled sibling therefore sees
        // the `Unit` placeholder, where the name-keyed chain would have fallen
        // through to an outer binding of the same name. Only reachable in a
        // program the next line rejects anyway (a `let rec` right-hand side
        // that is not a function), and arguably the more faithful answer: the
        // sibling IS the binding in scope there, so resolving it to an outer
        // one was accidental.
        let inner = env.child(vec![Value::Unit; cbindings.len()]);
        for (i, (name, cval)) in cbindings.iter().enumerate() {
            let v = cval.run(&inner, interp)?;
            if !matches!(v, Value::CompiledClosure { .. }) {
                return eval_error(format!(
                    "let-rec binding '{name}' must be a function, got {}",
                    v.type_name()
                ));
            }
            inner.set_slot(0, i as u16, v);
        }
        cbody.run(&inner, interp)
    })
}

/// One compiled match arm: the (uncompiled) pattern is kept for the runtime
/// [`match_pattern`] structural test; the guard and body are compiled in a
/// scope extended with the pattern's bound variables.
struct CompiledArm {
    pat: Pattern,
    guard: Option<CompiledExpr>,
    body: CompiledExpr,
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
pub(crate) fn compile_program(ast: &Ast, base_env: &BaseEnv) -> CompiledExpr {
    let mut c = Compiler::new(Some(base_env));
    let compiled = c.compile_spine(ast);
    c.globals.finish(c.n_globals);
    compiled
}

/// Compile a top-level program body that may contain `Ast::VersionScope`
/// nodes (Slice X2a — a cross-version splice, `lib.rs`'s
/// `compile_document_v1_with_trials`): `base_env` folds every unshadowed
/// `Ast::Var` OUTSIDE a `VersionScope`, `base_env_v006` folds every one
/// INSIDE an `Ast::VersionScope(V0_0, _)` subtree. See
/// [`Compiler::new_xver`].
pub(crate) fn compile_program_xver(
    ast: &Ast,
    base_env: &BaseEnv,
    base_env_v006: &BaseEnv,
) -> CompiledExpr {
    let mut c = Compiler::new_xver(base_env, base_env_v006);
    let compiled = c.compile_spine(ast);
    c.globals.finish(c.n_globals);
    compiled
}

#[cfg(test)]
mod tests {
    //! Determinism checks over a broad set of programs, plus opt-in
    //! (`#[ignore]`) micro-benchmarks.
    //!
    //! These used to be a DIFFERENTIAL harness: each program was run through
    //! both this module's compiler and a reference tree-walking interpreter,
    //! and the two results compared. Phase 3 of retired the tree-walker
    //! (quoted text is compiled eagerly now, so a tree-walker cannot build a
    //! `Value::InlineText` without invoking the compiler), which removed one
    //! side of the comparison. Rather than leave a tautology behind, the same
    //! programs now check the property that actually still has teeth and that
    //! the project's byte-identical-output constraint depends on: two
    //! INDEPENDENT compiles, against two freshly built base environments,
    //! must produce identical output. That is what catches nondeterminism
    //! leaking in from hash iteration order, allocation addresses, or the
    //! shared `Globals` table.
    //!
    //! Run the benchmarks with, e.g.:
    //! `cargo test -p rustyfi-lang --release -- --ignored --nocapture bench_`

    use super::*;
    use crate::ast::{MatchArm, Pattern};
    use crate::eval::Interp;
    use crate::value::{BaseEnv, Env, Value};
    use rustyfi_backend::{FontKey, FontMetrics, Length};
    use rustyfi_syntax::Span;

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

    /// Compile `ast` against the compile-time environment `base`, then run it
    /// in a fresh (empty) runtime frame chain — the same two-environment split
    /// `lib.rs` uses.
    fn eval_compiled(base: &BaseEnv, ast: &Ast) -> Result<Value, EvalError> {
        let mono = Mono;
        let mut interp = Interp::new(&mono);
        compile_program(ast, base).run(&Env::root(), &mut interp)
    }

    /// Compile and run `ast` twice — separate compiles, separate base
    /// environments, separate interpreters — and require the two runs to agree
    /// exactly: identical `Value` (compared by structural `Debug`) on success,
    /// identical error text on failure, and the same success/failure verdict.
    ///
    /// See the module comment for why this replaced the compiled-vs-
    /// tree-walker differential check.
    fn assert_deterministic(ast: &Ast) {
        let env_a = crate::primitives::base_env();
        let env_b = crate::primitives::base_env();
        match (eval_compiled(&env_a, ast), eval_compiled(&env_b, ast)) {
            (Ok(a), Ok(b)) => assert_eq!(
                format!("{a:?}"),
                format!("{b:?}"),
                "two independent compiles produced different values"
            ),
            (Err(a), Err(b)) => assert_eq!(
                a.to_string(),
                b.to_string(),
                "two independent compiles produced different errors"
            ),
            (a, b) => panic!("ok/err mismatch between two runs: {a:?} vs {b:?}"),
        }
    }

    /// SATySFi 0.1 labeled-optional lambda/application (`LambdaOpt`/
    /// `ApplyOpt`): the compiled path and the tree-walker must agree on the
    /// `Some`/`None` defaulting — a provided `?(bias = e)` binds `Some e`, an
    /// omitted one (a plain apply of an opt-closure) binds `None`.
    #[test]
    fn deterministic_labeled_optionals() {
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
        assert_deterministic(&Ast::ApplyOpt {
            func: Box::new(lam.clone()),
            opts: vec![("bias".to_string(), Ast::Int(40))],
            arg: Box::new(Ast::Int(2)),
        });
        // omitted (plain apply of an opt-closure) -> bias defaults None -> 2
        assert_deterministic(&app1(lam, Ast::Int(2)));
    }

    #[test]
    fn deterministic_literals_and_arithmetic() {
        assert_deterministic(&Ast::Int(42));
        assert_deterministic(&Ast::Str("hi".to_string()));
        assert_deterministic(&Ast::Bool(true));
        assert_deterministic(&app2("+", Ast::Int(2), Ast::Int(3)));
        assert_deterministic(&app2("*", Ast::Int(7), Ast::Int(6)));
        assert_deterministic(&app2("<", Ast::Int(2), Ast::Int(3)));
        assert_deterministic(&app2("^", Ast::Str("foo".into()), Ast::Str("bar".into())));
        // division by zero: both must error the same way.
        assert_deterministic(&app2("/", Ast::Int(1), Ast::Int(0)));
    }

    #[test]
    fn deterministic_let_lambda_and_capture() {
        // let const a b = a in const 1 2
        assert_deterministic(&Ast::LetIn(
            "id".into(),
            Box::new(Ast::Lambda("x".into(), Rc::new(var("x")))),
            Box::new(app1(var("id"), Ast::Int(7))),
        ));
        // capture of an outer let through a closure: let a = 5 in (fun x -> a + x) 3
        assert_deterministic(&Ast::LetIn(
            "a".into(),
            Box::new(Ast::Int(5)),
            Box::new(app1(
                Ast::Lambda("x".into(), Rc::new(app2("+", var("a"), var("x")))),
                Ast::Int(3),
            )),
        ));
    }

    #[test]
    fn deterministic_let_rec_fib_and_mutual() {
        assert_deterministic(&fib_program(15));
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
        assert_deterministic(&Ast::LetRecIn(
            bindings,
            Box::new(Ast::Tuple(vec![
                app1(var("even"), Ast::Int(10)),
                app1(var("odd"), Ast::Int(7)),
            ])),
        ));
        // a non-function let-rec binding errors identically in both paths.
        assert_deterministic(&Ast::LetRecIn(
            vec![("x".into(), Rc::new(Ast::Int(1)))],
            Box::new(var("x")),
        ));
    }

    #[test]
    fn deterministic_records_lists_tuples_and_fields() {
        assert_deterministic(&Ast::Record(vec![
            ("a".into(), Ast::Int(1)),
            ("b".into(), Ast::Str("x".into())),
        ]));
        assert_deterministic(&Ast::List(vec![Ast::Int(1), Ast::Int(2), Ast::Int(3)]));
        assert_deterministic(&Ast::Tuple(vec![Ast::Int(1), Ast::Bool(true)]));
        // field access, present and absent (absent => identical error)
        let rec = Ast::Record(vec![("a".into(), Ast::Int(9)), ("b".into(), Ast::Int(8))]);
        assert_deterministic(&Ast::AccessField(
            Box::new(rec.clone()),
            "a".into(),
            Span::default(),
        ));
        assert_deterministic(&Ast::AccessField(
            Box::new(rec.clone()),
            "zzz".into(),
            Span::default(),
        ));
        // functional update, present and absent
        assert_deterministic(&Ast::UpdateField(
            Box::new(rec.clone()),
            "a".into(),
            Box::new(Ast::Int(100)),
        ));
        assert_deterministic(&Ast::UpdateField(
            Box::new(rec),
            "nope".into(),
            Box::new(Ast::Int(1)),
        ));
    }

    #[test]
    fn deterministic_match_arms_guards_and_ctors() {
        // int literal + wildcard
        assert_deterministic(&Ast::Match(
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
        assert_deterministic(&Ast::Match(
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
        assert_deterministic(&Ast::Match(
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
        assert_deterministic(&Ast::Match(
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
        assert_deterministic(&Ast::Match(
            Box::new(Ast::Int(5)),
            vec![MatchArm {
                pat: Pattern::Int(1),
                guard: None,
                body: Ast::Int(0),
            }],
        ));
    }

    #[test]
    fn deterministic_mutable_while_and_sequential() {
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
                Box::new(Ast::Sequential(
                    Box::new(while_loop),
                    Box::new(deref("acc")),
                )),
            )),
        );
        assert_deterministic(&prog); // 0+1+2+3+4 = 10
    }

    // ---- document-level cross-check + shared prep for the doc benchmark ----

    /// Merge the `stdja-mini` prelude ahead of `src` (as the loader does),
    /// then elaborate + typecheck, returning `(base_env, elaborated body)`.
    fn prepare_document(src: &str) -> (BaseEnv, Ast) {
        let lib_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../lib-rustyfi/dist/packages/stdja-mini.satyh");
        let lib_src = std::fs::read_to_string(&lib_path).unwrap();
        let lib_file = rustyfi_syntax::parse_file(&lib_src).unwrap();
        let doc_file = rustyfi_syntax::parse_file(src).unwrap();
        let mut prelude = lib_file.prelude;
        prelude.extend(doc_file.prelude);
        let merged = rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: doc_file.in_kw,
            body: doc_file.body,
            eoi: doc_file.eoi,
        };
        let env = crate::primitives::base_env();
        let store = crate::symbol::SymbolStore::new();
        let scope = crate::elaborate::Scope::new(&store, env.names());
        let program = crate::elaborate::elaborate_program(&merged, &scope).unwrap();
        crate::typecheck::typecheck(&program).unwrap();
        // De-brand before returning: the store is local to this helper, and
        // the tests below drive the runtime, which is `Symbol`-free.
        (env, crate::ast::debrand(&program.body, &store))
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
    fn deterministic_document_many_paragraphs() {
        let (env_a, body_a) = prepare_document(&many_paragraph_src(12));
        let doc_a = eval_compiled(&env_a, &body_a).unwrap();
        let (env_b, body_b) = prepare_document(&many_paragraph_src(12));
        let doc_b = eval_compiled(&env_b, &body_b).unwrap();
        // The whole typeset document (pages/boxes) must come out identical
        // from two independent elaborate -> typecheck -> compile -> run runs.
        assert_eq!(
            format!("{doc_a:?}"),
            format!("{doc_b:?}"),
            "two independent runs produced different documents"
        );
        assert!(matches!(doc_a, Value::Document(_)));
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

        // Compilation is one-off; time it separately from repeated execution.
        let build = bench_ns(20, || {
            let _ = compile_program(&prog, &env);
        });
        let compiled = compile_program(&prog, &env);
        let mono = Mono;
        let mut interp = Interp::new(&mono);
        let root = Env::root();
        let run = bench_ns(20, || {
            let _ = compiled.run(&root, &mut interp).unwrap();
        });

        println!("\n== fib({N}) : {calls} calls/eval ==");
        println!("  compile   : {build:>9.0} ns  (one-off)");
        println!(
            "  run       : {:>9.0} ns/eval  ({:>5.1} ns/call)",
            run,
            run / calls as f64
        );
    }

    #[test]
    #[ignore = "benchmark; run with --release -- --ignored --nocapture"]
    fn bench_many_paragraph_document() {
        const PARAS: usize = 300;
        let (env, body) = prepare_document(&many_paragraph_src(PARAS));

        let build = bench_ns(20, || {
            let _ = compile_program(&body, &env);
        });
        let compiled = compile_program(&body, &env);
        let mono = Mono;
        let mut interp = Interp::new(&mono);
        let root = Env::root();
        let run = bench_ns(20, || {
            let _ = compiled.run(&root, &mut interp).unwrap();
        });

        println!("\n== document with {PARAS} paragraphs ==");
        println!("  compile   : {build:>10.0} ns  (one-off)");
        println!("  run       : {run:>10.0} ns/doc");
    }
}
