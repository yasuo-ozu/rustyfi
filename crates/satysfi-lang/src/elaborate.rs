//! Surface CST → `Ast` elaboration. Milestone 1 does scope resolution and
//! structural checks only; this function's signature is the seam where the
//! phase-3 typechecker (typechecker.ml / unification.ml port) slots in.

use crate::ast::{Ast, BText, IText};
use satysfi_backend::Length;
use satysfi_syntax::cst::{self, ast as c};
use satysfi_syntax::span::Span;
use satysfi_syntax::Loc;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Debug, thiserror::Error)]
#[error("{span}: {msg}")]
pub struct ElabError {
    pub span: Span,
    pub msg: String,
}

fn err<T>(span: Span, msg: impl Into<String>) -> Result<T, ElabError> {
    Err(ElabError {
        span,
        msg: msg.into(),
    })
}

/// The names in scope (primitives plus, progressively, `let`-bound names).
#[derive(Clone, Debug, Default)]
pub struct Scope {
    names: HashSet<String>,
}

impl Scope {
    pub fn new(names: impl IntoIterator<Item = String>) -> Scope {
        Scope {
            names: names.into_iter().collect(),
        }
    }

    fn with(&self, name: &str) -> Scope {
        let mut s = self.clone();
        s.names.insert(name.to_string());
        s
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

/// Elaborate a whole file into one expression.
pub fn elaborate(file: &cst::File, prelude_scope: &Scope) -> Result<Ast, ElabError> {
    let start = Span {
        start: Loc::default(),
        end: Loc::default(),
    };
    if !file.prelude.is_empty() && file.in_kw.is_none() {
        return err(
            start,
            "top-level bindings must be followed by 'in' before the document expression",
        );
    }

    // Build the scope over the prelude, then fold the lets around the body.
    let mut scope = prelude_scope.clone();
    let mut elaborated: Vec<(String, Ast)> = Vec::new();
    for top in &file.prelude {
        // Parameters are in scope inside the binding's own body.
        let mut inner = scope.clone();
        for p in &top.params {
            inner = inner.with(&p.name);
        }
        let mut value = expr(&top.value, &inner)?;
        for p in top.params.iter().rev() {
            value = Ast::Lambda(p.name.clone(), Rc::new(value));
        }
        scope = scope.with(&top.name.name);
        elaborated.push((top.name.name.clone(), value));
    }

    let mut body = expr(&file.body, &scope)?;
    for (name, value) in elaborated.into_iter().rev() {
        body = Ast::LetIn(name, Box::new(value), Box::new(body));
    }
    Ok(body)
}

fn expr(e: &c::Expr, scope: &Scope) -> Result<Ast, ElabError> {
    match e {
        c::Expr::Fun {
            kw,
            params,
            body,
            ..
        } => {
            if params.is_empty() {
                return err(kw.0, "'fun' needs at least one parameter");
            }
            let mut inner = scope.clone();
            for p in params {
                inner = inner.with(&p.name);
            }
            let mut ast = expr(body, &inner)?;
            for p in params.iter().rev() {
                ast = Ast::Lambda(p.name.clone(), Rc::new(ast));
            }
            Ok(ast)
        }
        c::Expr::App { head, args } => {
            let mut ast = atomic(head, scope)?;
            for a in args {
                ast = Ast::Apply(Box::new(ast), Box::new(atomic(a, scope)?));
            }
            Ok(ast)
        }
    }
}

fn atomic(a: &c::Atomic, scope: &Scope) -> Result<Ast, ElabError> {
    match a {
        c::Atomic::Length(l) => match Length::from_unit(l.value, &l.unit) {
            Some(len) => Ok(Ast::Length(len)),
            None => err(l.span, format!("unknown length unit '{}'", l.unit)),
        },
        c::Atomic::Float(f) => Ok(Ast::Float(f.value)),
        c::Atomic::Int(i) => Ok(Ast::Int(i.value)),
        // TODO(phase 2): apply omit_spaces (leading-break/indentation
        // stripping) exactly as parser.mly's `omit_spaces pre post`.
        c::Atomic::Literal(l) => Ok(Ast::Str(l.body.clone())),
        c::Atomic::True(_) => Ok(Ast::Bool(true)),
        c::Atomic::False(_) => Ok(Ast::Bool(false)),
        c::Atomic::Var(v) => {
            if scope.contains(&v.name) {
                Ok(Ast::Var(v.name.clone(), v.span))
            } else {
                err(v.span, format!("unbound variable '{}'", v.name))
            }
        }
        c::Atomic::Unit { .. } => Ok(Ast::Unit),
        c::Atomic::Paren { inner, .. } => expr(inner, scope),
        c::Atomic::Record { fields, .. } => {
            let mut out = Vec::with_capacity(fields.len());
            for f in fields {
                out.push((f.name.name.clone(), expr(&f.value, scope)?));
            }
            Ok(Ast::Record(out))
        }
        c::Atomic::List { items, .. } => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(expr(&it.value, scope)?);
            }
            Ok(Ast::List(out))
        }
        c::Atomic::InlineText { elems, .. } => Ok(Ast::InlineText(Rc::new(inline_elems(
            elems, scope,
        )?))),
        c::Atomic::BlockText { elems, .. } => {
            Ok(Ast::BlockText(Rc::new(block_elems(elems, scope)?)))
        }
    }
}

/// Coalesce chars/spaces/breaks into text runs; commands become `IText::Cmd`.
fn inline_elems(elems: &[c::InlineElem], scope: &Scope) -> Result<Vec<IText>, ElabError> {
    let mut out = Vec::new();
    let mut text = String::new();
    for el in elems {
        match el {
            c::InlineElem::Char(c) => text.push_str(&c.text),
            c::InlineElem::Space(_) => text.push(' '),
            c::InlineElem::Break(_) => text.push('\n'),
            c::InlineElem::Cmd { name, tail } => {
                if !text.is_empty() {
                    out.push(IText::Text(std::mem::take(&mut text)));
                }
                if !scope.contains(&name.name) {
                    return err(name.span, format!("unbound inline command '{}'", name.name));
                }
                out.push(IText::Cmd {
                    name: name.name.clone(),
                    span: name.span,
                    args: cmd_args(tail, scope)?,
                });
            }
        }
    }
    if !text.is_empty() {
        out.push(IText::Text(text));
    }
    Ok(out)
}

fn block_elems(elems: &[c::BlockElem], scope: &Scope) -> Result<Vec<BText>, ElabError> {
    elems
        .iter()
        .map(|el| {
            let c::BlockElem::Cmd { name, tail } = el;
            if !scope.contains(&name.name) {
                return err(name.span, format!("unbound block command '{}'", name.name));
            }
            Ok(BText::Cmd {
                name: name.name.clone(),
                span: name.span,
                args: cmd_args(tail, scope)?,
            })
        })
        .collect()
}

/// Flatten a command tail back into its argument list: the parsed
/// application chain's head and arguments *are* the command's arguments.
fn cmd_args(tail: &c::CmdTail, scope: &Scope) -> Result<Vec<Ast>, ElabError> {
    match tail {
        c::CmdTail::Semi(_) => Ok(Vec::new()),
        c::CmdTail::Args { args, .. } => match args.as_ref() {
            c::Expr::App { head, args } => {
                let mut out = vec![atomic(head, scope)?];
                for a in args {
                    out.push(atomic(a, scope)?);
                }
                Ok(out)
            }
            c::Expr::Fun { kw, .. } => err(kw.0, "unexpected 'fun' as a command argument"),
        },
    }
}
