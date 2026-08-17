//! The tree-walking evaluator (the naive-interpreter shape of
//! evaluator.cppo.ml; the bytecode VM is intentionally not ported).

use crate::ast::{Ast, Pattern};
use crate::crossref::CrossRefs;
use crate::value::{Env, Value};
use satysfi_backend::{DocInfo, FontMetrics, ImageResource, MathCmdId};
use satysfi_syntax::{SatysfiVersion, Span};
use std::cell::RefCell;
use std::rc::Rc;

/// See [`Interp::decos`].
#[derive(Clone, Debug)]
pub enum DecoEntry {
    Inline {
        deco: Value,
    },
    Block {
        pads: satysfi_backend::Paddings,
        /// The frame's OUTER width (the wrapping context's paragraph_width).
        width: satysfi_backend::Length,
        /// `(decoS, decoH, decoM, decoT)` — evalUtil.ml:169 `get_decoset`.
        decoset: [Value; 4],
    },
}

#[derive(Debug, thiserror::Error)]
#[error("{}{msg}", .span.map(|s| format!("{s}: ")).unwrap_or_default())]
pub struct EvalError {
    pub span: Option<Span>,
    pub msg: String,
}

pub fn eval_error<T>(msg: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError {
        span: None,
        msg: msg.into(),
    })
}

/// Comma-separated, sorted field names of a record — the "(available fields:
/// …)" hint shared by the field-access and field-update error messages.
pub(crate) fn available_fields(map: &std::collections::BTreeMap<String, Value>) -> String {
    let mut keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
    keys.sort();
    keys.join(", ")
}

/// Evaluation state: the font-metrics seam (and later: cross references,
/// mutable stores).
pub struct Interp<'a> {
    pub metrics: &'a dyn FontMetrics,
    /// The document-wide image table (`docs/plans/math-images.md` §Slice
    /// 1): `load-image` (`primitives::prim_load_image`) decodes eagerly and
    /// pushes here, returning the new entry's index as `Value::Image`;
    /// `use-image-by-width` (`primitives::prim_use_image_by_width`) looks
    /// the resource back up by that index. `page-break`
    /// (`primitives::prim_page_break`) clones this out into
    /// `DocumentValue::images` when it packages the final document, so the
    /// PDF writer sees every image ever decoded while evaluating (a superset
    /// of what actually ends up placed on a page — the writer itself filters
    /// down to the ones a placed line actually references).
    pub images: Vec<ImageResource>,
    /// The document-wide page-break-hook closure table
    /// (`docs/plans/hooks-annotations-crossref.md` §Slice 1): `hook-page-break`
    /// pushes its closure argument here and returns a `HookId` index (via
    /// `PureHorzBox::HookPageBreak`) — the same `ImageId`-style seam as
    /// `images` above, but for a deferred *computation* rather than a
    /// resource. Reset every trial (see `crossrefs`, which is the one
    /// exception), read back by `fire_hooks` once `break_pages` has placed
    /// every hook and its final geometry is known.
    pub hooks: Vec<Value>,
    /// Installed-math-command table (`get-initial-context`/
    /// `set-math-command` push here; `Context::math_command` holds the
    /// index) — the `ImageId`/`HookId`-style seam, because the backend
    /// `Context` cannot hold a lang-side `Value`. Read back by
    /// `read_inline`'s `EmbedMath` arm.
    pub math_commands: Vec<Value>,
    /// The cross-reference table, shared with the compile driver
    /// (`lib.rs::compile_document_cst`) across every trial of the fixpoint
    /// loop — unlike `hooks`/`images`, this must *not* reset per trial, so
    /// the driver constructs one `Rc<RefCell<CrossRefs>>` and clones the
    /// handle into each trial's fresh `Interp`. Defaults to a fresh empty
    /// table so existing single-run call sites/unit tests compile unchanged.
    pub crossrefs: Rc<RefCell<CrossRefs>>,
    /// Memoized compilations of command-argument / embed `Ast`s reached
    /// through `read_inline`/`read_block` (see [`Interp::eval_arg`]). Keyed by
    /// the argument node's address: these nodes live inside the program's
    /// `Rc<Vec<IText>>`/`Rc<Vec<BText>>`, which are kept alive for the whole
    /// interpreter run, so the addresses are stable and re-reading the same
    /// quoted text reuses its already-compiled argument closures.
    arg_cache: std::collections::HashMap<usize, crate::compile::CompiledExpr>,
    /// §B/§C accumulators (docs/plans/hooks-annotations-crossref.md):
    /// link annotations / named destinations / outline entries, plus the
    /// per-page deco-graphics overlays (§D). All reset per trial (fresh
    /// `Interp`); the FINAL trial's contents are moved into
    /// `DocumentValue::extras` by `compile_document_cst_with_trials`.
    pub annotations: Vec<satysfi_backend::Annot>,
    pub destinations: Vec<satysfi_backend::NamedDest>,
    pub outline: Vec<satysfi_backend::OutlineEntry>,
    pub page_graphics: Vec<Vec<satysfi_backend::GraphicsElem>>,
    /// `register-document-information`'s accumulator (prim-retype-sweep
    /// §2.4) — LAST WRITE WINS, same reset-per-trial policy as
    /// `outline`/`annotations`/`destinations` above (a fresh `Interp` per
    /// trial resets this to `None`; the final trial's value is drained into
    /// `DocExtras::doc_info` by `lib.rs`'s `eval_document_trials`).
    pub doc_info: Option<DocInfo>,
    /// `Some(0-based page)` only while `fire_hooks` is walking that page —
    /// the port of upstream's `State.during_page_break` + "current page"
    /// (`annotation.ml:15`, `namedDest.ml`'s `notify_pagebreak`).
    pub current_page: Option<usize>,
    /// `namedDest.ml`'s key -> "nameddest{N}" sanitizer table (`name_from_
    /// hash_table`): arbitrary user keys become stable PDF name strings,
    /// shared by register-destination / register-link-to-location /
    /// register-outline within one trial.
    dest_names: std::collections::HashMap<String, String>,
    /// §D deco-closure table (`DecoId` indexes here) — `hooks`' twin for
    /// decorations. `Inline` holds one `deco` closure
    /// (`point -> length -> length -> length -> graphics list`); `Block`
    /// holds a block frame's four-closure deco-set + the geometry the
    /// markers can't carry. Reset per trial.
    pub decos: Vec<DecoEntry>,
    /// Deferred `inline-graphics-outer` callbacks (`length -> point ->
    /// graphics list`), indexed by `GraphicsFnId` — the `hooks` pattern.
    /// Reset per trial like `hooks`/`images`.
    pub outer_graphics: Vec<Value>,
    /// The target language version this evaluation run is checking against
    /// (math-split spec §3.4) — consulted only by `read_inline`'s
    /// `IText::EmbedMath` FALLBACK arm (no installed math command; unit-test
    /// contexts only — the installed-command path is version-blind already).
    /// Default `V0_0_6`; `lib.rs`'s `eval_document_trials` (the shared tail
    /// both `compile_document_cst_with_trials` and
    /// `compile_document_v1_with_trials` fall into) sets this to the real
    /// target version on every `Interp` it constructs.
    pub version: SatysfiVersion,
}

impl<'a> Interp<'a> {
    pub fn new(metrics: &'a dyn FontMetrics) -> Self {
        Interp {
            metrics,
            images: Vec::new(),
            hooks: Vec::new(),
            math_commands: Vec::new(),
            crossrefs: Rc::new(RefCell::new(CrossRefs::new())),
            arg_cache: std::collections::HashMap::new(),
            annotations: Vec::new(),
            destinations: Vec::new(),
            outline: Vec::new(),
            page_graphics: Vec::new(),
            doc_info: None,
            current_page: None,
            dest_names: std::collections::HashMap::new(),
            decos: Vec::new(),
            outer_graphics: Vec::new(),
            version: SatysfiVersion::V0_0_6,
        }
    }

    /// Evaluate a command-argument / embedded expression reached from
    /// `read_inline`/`read_block`, compiling it once (on first sight) and
    /// running the cached compiled closure thereafter. Behavior is identical
    /// to [`Interp::eval`] — the compiled argument uses no global
    /// constant-folding, so every free name resolves through `env.lookup`
    /// exactly as tree-walking would.
    pub(crate) fn eval_arg(&mut self, env: &Env, arg: &Ast) -> Result<Value, EvalError> {
        let key = arg as *const Ast as usize;
        let compiled = match self.arg_cache.get(&key) {
            Some(c) => c.clone(),
            None => {
                let c = crate::compile::compile_arg(arg);
                self.arg_cache.insert(key, c.clone());
                c
            }
        };
        compiled.run(env, self)
    }

    pub fn eval(&mut self, env: &Env, ast: &Ast) -> Result<Value, EvalError> {
        match ast {
            Ast::Unit => Ok(Value::Unit),
            Ast::Bool(b) => Ok(Value::Bool(*b)),
            Ast::Int(n) => Ok(Value::Int(*n)),
            Ast::Float(x) => Ok(Value::Float(*x)),
            Ast::Length(l) => Ok(Value::Length(*l)),
            Ast::Str(s) => Ok(Value::Str(s.clone())),
            Ast::Var(name, span) => env.lookup(name).ok_or_else(|| EvalError {
                span: Some(*span),
                msg: format!("unbound variable '{name}' at run time"),
            }),
            Ast::Apply(f, arg) => {
                let func = self.eval(env, f)?;
                let arg = self.eval(env, arg)?;
                self.apply(func, arg)
            }
            Ast::Lambda(param, body) => Ok(Value::Closure {
                opt_params: Vec::new(),
                param: param.clone(),
                body: body.clone(),
                env: env.clone(),
            }),
            // `fun ?(l = x, …) p -> body` (SATySFi 0.1). Builds a closure
            // that additionally binds each labeled-optional param at
            // application time (`Some`/`None` defaulting in
            // `apply_with_opts`).
            Ast::LambdaOpt { opts, param, body } => Ok(Value::Closure {
                opt_params: opts.clone(),
                param: param.clone(),
                body: body.clone(),
                env: env.clone(),
            }),
            // `f ?(l = e, …) arg` (SATySFi 0.1) — evaluate the function, each
            // labeled optional value, and the positional argument, then
            // beta-reduce with the optional bundle.
            Ast::ApplyOpt { func, opts, arg } => {
                let f = self.eval(env, func)?;
                let mut opt_vals = Vec::with_capacity(opts.len());
                for (label, e) in opts {
                    opt_vals.push((label.clone(), self.eval(env, e)?));
                }
                let a = self.eval(env, arg)?;
                self.apply_with_opts(f, opt_vals, a)
            }
            Ast::LetIn(name, value, rest) => {
                let v = self.eval(env, value)?;
                let inner = env.child();
                inner.define(name.clone(), v);
                self.eval(&inner, rest)
            }
            // `let-math` binds identically to `LetIn` at run time (see
            // `ast.rs`'s doc comment — the distinct variant is purely a
            // typecheck-time signal).
            Ast::LetMathIn(name, value, rest) => {
                let v = self.eval(env, value)?;
                let inner = env.child();
                inner.define(name.clone(), v);
                self.eval(&inner, rest)
            }
            Ast::Record(fields) => {
                let mut map = std::collections::BTreeMap::new();
                for (name, e) in fields {
                    map.insert(name.clone(), self.eval(env, e)?);
                }
                Ok(Value::Record(map))
            }
            Ast::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for e in items {
                    out.push(self.eval(env, e)?);
                }
                Ok(Value::List(out))
            }
            Ast::InlineText(elems) => Ok(Value::InlineText {
                elems: elems.clone(),
                env: env.clone(),
            }),
            Ast::BlockText(elems) => Ok(Value::BlockText {
                elems: elems.clone(),
                env: env.clone(),
            }),
            Ast::LetRecIn(bindings, body) => {
                let inner = env.child();
                for (name, value_ast) in bindings {
                    let v = self.eval(&inner, value_ast)?;
                    if !matches!(v, Value::Closure { .. }) {
                        return eval_error(format!(
                            "let-rec binding '{name}' must be a function, got {}",
                            v.type_name()
                        ));
                    }
                    inner.define(name.clone(), v);
                }
                self.eval(&inner, body)
            }
            Ast::IfThenElse(cond, then_e, else_e) => match self.eval(env, cond)? {
                Value::Bool(true) => self.eval(env, then_e),
                Value::Bool(false) => self.eval(env, else_e),
                other => eval_error(format!(
                    "if-then-else condition must be bool, got {}",
                    other.type_name()
                )),
            },
            Ast::Tuple(items) => {
                let mut out = Vec::with_capacity(items.len());
                for e in items {
                    out.push(self.eval(env, e)?);
                }
                Ok(Value::Tuple(out))
            }
            Ast::Ctor(name, arg) => {
                let payload = match arg {
                    Some(a) => Some(Box::new(self.eval(env, a)?)),
                    None => None,
                };
                Ok(Value::Ctor(name.clone(), payload))
            }
            // Quoted math text (`${…}`); like InlineText/BlockText, this only
            // captures the environment — typesetting is phase 7's job.
            Ast::MathText(elems) => Ok(Value::MathText {
                elems: elems.clone(),
                env: env.clone(),
            }),
            // `let-mutable x <- init in body` (`UTLetMutableIn` /
            // evaluator.cppo.ml's `LetMutableIn`). v0.0.6 evaluates the
            // initializer in the *outer* environment, allocates a fresh
            // store location for it, then binds `x` to that location in a
            // child environment for `body`. We have no separate store table
            // (see `Value::Ref`'s doc comment), so `x` is bound directly to
            // a shared `RefCell` cell.
            Ast::LetMutableIn(name, init, body) => {
                let v = self.eval(env, init)?;
                let inner = env.child();
                inner.define(name.clone(), Value::Ref(Rc::new(RefCell::new(v))));
                self.eval(&inner, body)
            }
            // `x <- e` (`UTOverwrite` / evaluator.cppo.ml's `Overwrite`).
            // v0.0.6 looks up `x`, requires it to hold a `Location`
            // (otherwise a "bug" — the type checker is supposed to rule
            // this out ahead of time), evaluates `e`, and destructively
            // updates the store; the result is `unit`.
            Ast::Overwrite(name, span, value) => {
                let cell = env.lookup(name).ok_or_else(|| EvalError {
                    span: Some(*span),
                    msg: format!("unbound mutable variable '{name}' at run time"),
                })?;
                match cell {
                    Value::Ref(cell) => {
                        let v = self.eval(env, value)?;
                        *cell.borrow_mut() = v;
                        Ok(Value::Unit)
                    }
                    other => Err(EvalError {
                        span: Some(*span),
                        msg: format!(
                            "cannot overwrite an immutable variable '{name}' (got a value of type {})",
                            other.type_name()
                        ),
                    }),
                }
            }
            // `while cond do body` (`UTWhileDo` / evaluator.cppo.ml's
            // `WhileDo`) — v0.0.6 recurses with no iteration cap (it's a
            // faithful loop, not a bounded one), so we do the same.
            Ast::WhileDo(cond, body) => loop {
                match self.eval(env, cond)? {
                    Value::Bool(true) => {
                        self.eval(env, body)?;
                    }
                    Value::Bool(false) => break Ok(Value::Unit),
                    other => {
                        return eval_error(format!(
                            "while-do condition must be bool, got {}",
                            other.type_name()
                        ))
                    }
                }
            },
            // `e1 before e2` (`UTSequential` / evaluator.cppo.ml's
            // `Sequential`). v0.0.6 asserts at runtime that `e1` evaluated
            // to `unit` (`BaseConstant(BCUnit)`), but that assertion only
            // exists because the *type checker* already guarantees `e1 :
            // unit` — the runtime check is a "this should be impossible"
            // bug trap, not a feature. This port has no separate type
            // checker yet, so we simply discard `e1`'s value regardless of
            // its runtime type.
            Ast::Sequential(e1, e2) => {
                self.eval(env, e1)?;
                self.eval(env, e2)
            }
            // `e#label` (`UTAccessField` / evaluator.cppo.ml's
            // `AccessField`).
            Ast::AccessField(e, label, span) => {
                let v = self.eval(env, e)?;
                match v {
                    Value::Record(map) => map.get(label).cloned().ok_or_else(|| EvalError {
                        span: Some(*span),
                        msg: format!(
                            "record has no field '{label}' (available fields: {})",
                            available_fields(&map)
                        ),
                    }),
                    other => Err(EvalError {
                        span: Some(*span),
                        msg: format!(
                            "cannot access field '{label}' of a non-record value (got {})",
                            other.type_name()
                        ),
                    }),
                }
            }
            // `(| e with label = v |)` (`UTUpdateField` / evaluator.cppo.ml's
            // `UpdateField`) — functional record update. v0.0.6 requires the
            // field to already exist (`Assoc.find_opt asc1 fldnm` is matched
            // against `None -> report_bug_reduction "... not found" | Some(_)
            // -> Assoc.add ...`): updating an absent label is a bug, not a
            // way to add a new field. We mirror that: absent label is a
            // runtime error. v0.0.6 also evaluates the replacement value
            // *before* checking whether the field exists (both `ast1` and
            // `ast2` are interpreted up front), which we replicate here.
            Ast::UpdateField(e, label, value) => {
                let v = self.eval(env, e)?;
                let new_v = self.eval(env, value)?;
                match v {
                    Value::Record(mut map) => {
                        if !map.contains_key(label) {
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
            }
            Ast::Match(scrutinee, arms) => {
                let v = self.eval(env, scrutinee)?;
                for arm in arms {
                    let mut bindings = Vec::new();
                    if !match_pattern(&arm.pat, &v, &mut bindings) {
                        continue;
                    }
                    let inner = env.child();
                    for (name, val) in bindings {
                        inner.define(name, val);
                    }
                    if let Some(guard) = &arm.guard {
                        match self.eval(&inner, guard)? {
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
                    return self.eval(&inner, &arm.body);
                }
                eval_error(format!(
                    "non-exhaustive match: no arm matched a value of type {}",
                    v.type_name()
                ))
            }
        }
    }

    /// Intern an installed math command, returning the handle a `Context`
    /// carries (`Context::math_command`).
    pub fn register_math_command(&mut self, cmd: Value) -> MathCmdId {
        self.math_commands.push(cmd);
        MathCmdId(self.math_commands.len() - 1)
    }

    /// `namedDest.ml:name_from_hash_table` — the stable PDF name for `key`,
    /// minting `nameddest{N}` on first sight. Also used by `register-outline`
    /// (upstream `Outline.make_entry` calls `NamedDest.get`, which mints too).
    pub fn dest_name(&mut self, key: &str) -> String {
        if let Some(n) = self.dest_names.get(key) {
            return n.clone();
        }
        let n = format!("nameddest{}", self.dest_names.len());
        self.dest_names.insert(key.to_string(), n.clone());
        n
    }

    pub fn apply(&mut self, func: Value, arg: Value) -> Result<Value, EvalError> {
        // A plain (0.0.6-shaped) application supplies no optional bundle; a
        // closure that *does* declare optional params (reached this way from
        // e.g. a higher-order caller) then defaults every one to `None`,
        // faithful to upstream's `reduce_beta_list`.
        self.apply_with_opts(func, Vec::new(), arg)
    }

    /// Beta-reduce `func` against a positional argument plus a SATySFi 0.1
    /// labeled-optional bundle. For a closure, each of the closure's declared
    /// optional params binds `Some v` when the bundle carries its label, else
    /// `None`; a supplied label the closure does not declare is ignored
    /// (upstream `reduce_beta` folds over the *closure's* map — the
    /// typechecker rejects genuinely-wrong labels first). This
    /// unknown-label-ignore is only sound because typecheck runs first.
    pub fn apply_with_opts(
        &mut self,
        func: Value,
        opt_vals: Vec<(String, Value)>,
        arg: Value,
    ) -> Result<Value, EvalError> {
        match func {
            Value::Closure {
                opt_params,
                param,
                body,
                env,
            } => {
                let inner = env.child();
                bind_opt_params(&inner, &opt_params, &opt_vals);
                inner.define(param, arg);
                self.eval(&inner, &body)
            }
            Value::CompiledClosure {
                opt_params,
                param,
                body,
                env,
            } => {
                let inner = env.child();
                bind_opt_params(&inner, &opt_params, &opt_vals);
                inner.define(param, arg);
                body.run(&inner, self)
            }
            Value::Prim { def, mut applied } => {
                if !opt_vals.is_empty() {
                    return eval_error(
                        "labeled optional arguments to a primitive are roadmap phase 5",
                    );
                }
                applied.push(arg);
                if applied.len() == def.arity {
                    (def.run)(self, applied)
                } else {
                    Ok(Value::Prim { def, applied })
                }
            }
            other => eval_error(format!(
                "cannot apply a value of type {} as a function",
                other.type_name()
            )),
        }
    }
}

/// Bind each of a closure's SATySFi 0.1 labeled-optional params into `env`:
/// `Some v` when `opt_vals` supplies the label, `None` otherwise (upstream
/// `reduce_beta`'s fold over the closure's own label map). Supplied labels a
/// closure does not declare are silently ignored — safe only because
/// typecheck rejects wrong labels first.
fn bind_opt_params(env: &Env, opt_params: &[(String, String)], opt_vals: &[(String, Value)]) {
    for (label, binder) in opt_params {
        let value = match opt_vals.iter().find(|(l, _)| l == label) {
            Some((_, v)) => Value::Ctor("Some".to_string(), Some(Box::new(v.clone()))),
            None => Value::Ctor("None".to_string(), None),
        };
        env.define(binder.clone(), value);
    }
}

/// Structural pattern matching against an already-evaluated scrutinee.
/// Returns `true` (and appends every binding introduced along the way) on a
/// structural match; returns `false` (leaving `bindings` for this attempt
/// unusable — callers must use a fresh `Vec` per arm) otherwise. A pattern
/// and a value of mismatched shape is simply "no match", never an error:
/// this untyped evaluator relies on the (separate, not-yet-ported)
/// exhaustiveness/type checker to rule out ill-typed matches ahead of time.
pub fn match_pattern(pat: &Pattern, value: &Value, bindings: &mut Vec<(String, Value)>) -> bool {
    match pat {
        Pattern::Wild => true,
        Pattern::Var(name) => {
            bindings.push((name.clone(), value.clone()));
            true
        }
        Pattern::As(inner_pat, name) => {
            if match_pattern(inner_pat, value, bindings) {
                bindings.push((name.clone(), value.clone()));
                true
            } else {
                false
            }
        }
        Pattern::Unit => matches!(value, Value::Unit),
        Pattern::Bool(b) => matches!(value, Value::Bool(v) if v == b),
        Pattern::Int(n) => matches!(value, Value::Int(v) if v == n),
        Pattern::Str(s) => matches!(value, Value::Str(v) if v == s),
        Pattern::Tuple(ps) => match value {
            Value::Tuple(vs) if ps.len() == vs.len() => ps
                .iter()
                .zip(vs.iter())
                .all(|(p, v)| match_pattern(p, v, bindings)),
            _ => false,
        },
        Pattern::EmptyList => matches!(value, Value::List(vs) if vs.is_empty()),
        Pattern::Cons(head_pat, tail_pat) => match value {
            Value::List(vs) if !vs.is_empty() => {
                if !match_pattern(head_pat, &vs[0], bindings) {
                    return false;
                }
                let tail = Value::List(vs[1..].to_vec());
                match_pattern(tail_pat, &tail, bindings)
            }
            _ => false,
        },
        Pattern::Ctor(name, parg) => match value {
            Value::Ctor(vname, vpayload) if name == vname => match (parg, vpayload) {
                (None, None) => true,
                (Some(p), Some(v)) => match_pattern(p, v, bindings),
                _ => false,
            },
            _ => false,
        },
    }
}
