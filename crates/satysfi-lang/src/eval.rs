//! The tree-walking evaluator (the naive-interpreter shape of
//! evaluator.cppo.ml; the bytecode VM is intentionally not ported).

use crate::ast::{Ast, Pattern};
use crate::crossref::CrossRefs;
use crate::value::{Env, Value};
use satysfi_backend::{FontMetrics, ImageResource, MathCmdId};
use satysfi_syntax::Span;
use std::cell::RefCell;
use std::rc::Rc;

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
                param: param.clone(),
                body: body.clone(),
                env: env.clone(),
            }),
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

    pub fn apply(&mut self, func: Value, arg: Value) -> Result<Value, EvalError> {
        match func {
            Value::Closure { param, body, env } => {
                let inner = env.child();
                inner.define(param, arg);
                self.eval(&inner, &body)
            }
            Value::CompiledClosure { param, body, env } => {
                let inner = env.child();
                inner.define(param, arg);
                body.run(&inner, self)
            }
            Value::Prim { def, mut applied } => {
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
