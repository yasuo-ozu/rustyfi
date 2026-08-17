//! The tree-walking evaluator (the naive-interpreter shape of
//! evaluator.cppo.ml; the bytecode VM is intentionally not ported).

use crate::ast::Ast;
use crate::value::{Env, Value};
use satysfi_backend::FontMetrics;
use satysfi_syntax::Span;

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

/// Evaluation state: the font-metrics seam (and later: cross references,
/// image tables, mutable stores).
pub struct Interp<'a> {
    pub metrics: &'a dyn FontMetrics,
}

impl<'a> Interp<'a> {
    pub fn new(metrics: &'a dyn FontMetrics) -> Self {
        Interp { metrics }
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
        }
    }

    pub fn apply(&mut self, func: Value, arg: Value) -> Result<Value, EvalError> {
        match func {
            Value::Closure { param, body, env } => {
                let inner = env.child();
                inner.define(param, arg);
                self.eval(&inner, &body)
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
