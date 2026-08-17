//! Runtime values (a milestone-1 subset of `syntactic_value`).

use crate::ast::{Ast, BText, IText, MathElem};
use crate::compile::CompiledExpr;
use crate::primitives::PrimDef;
use satysfi_backend::{Context, HorzBox, Length, Page, PageGeometry, VertBox};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

// `Value::CompiledClosure` carries a crate-internal `CompiledExpr` body (an
// opaque compiled-closure handle). External code can obtain such a value but
// cannot name, construct, or inspect its body, which is the intent — so the
// `private_interfaces` lint for that one field is deliberately allowed.
#[allow(private_interfaces)]
#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Length(Length),
    Str(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    /// A variant constructor value, optionally carrying a payload
    /// (`None` / `Some 3`).
    Ctor(String, Option<Box<Value>>),
    Record(BTreeMap<String, Value>),
    Context(Box<Context>),
    /// Quoted inline text with its captured environment
    /// (`InputHorzWithEnvironment`).
    InlineText { elems: Rc<Vec<IText>>, env: Env },
    /// Quoted block text with its captured environment.
    BlockText { elems: Rc<Vec<BText>>, env: Env },
    /// Quoted math text with its captured environment (mirrors
    /// `InlineText`/`BlockText`); typesetting is deferred to phase 7, so this
    /// is carried opaquely for now.
    MathText { elems: Rc<Vec<MathElem>>, env: Env },
    /// A mutable cell (`let-mutable`'s binding; v0.0.6's `Location`/store
    /// entry). This port uses a directly-shared `RefCell` instead of an
    /// indirection through a separate store table.
    Ref(Rc<RefCell<Value>>),
    /// `inline-boxes` (the `Horz` base constant).
    InlineBoxes(Vec<HorzBox>),
    /// `block-boxes` (the `Vert` base constant).
    BlockBoxes(Vec<VertBox>),
    Document(Rc<DocumentValue>),
    Closure {
        param: String,
        body: Rc<Ast>,
        env: Env,
    },
    /// A closure produced by the closure-compiling evaluator
    /// ([`crate::compile`]). Semantically identical to [`Value::Closure`] —
    /// same captured `Env`, same "function" type name — but its body is an
    /// already-compiled [`CompiledExpr`] run directly by
    /// [`crate::eval::Interp::apply`] rather than re-tree-walked. The
    /// tree-walking `eval` never produces this; the compiled path never
    /// produces `Closure`.
    CompiledClosure {
        param: String,
        body: CompiledExpr,
        env: Env,
    },
    /// A (possibly partially applied) native primitive.
    Prim {
        def: &'static PrimDef,
        applied: Vec<Value>,
    },
    /// `pre-path` (Slice 1 graphics; `start-path`/`line-to`'s result — see
    /// `docs/plans/graphics-subsystem.md` §1).
    PrePath(satysfi_backend::PrePath),
    /// `path` (`terminate-path`/`close-with-line`'s result).
    Path(satysfi_backend::Path),
    /// `graphics` — one resolved drawing element (`fill`/`stroke`'s result);
    /// a `graphics list` is just `Value::List` of these, same as upstream.
    Graphics(satysfi_backend::GraphicsElem),
}

impl Value {
    /// A short type name for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "unit",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Length(_) => "length",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Ctor(_, _) => "variant",
            Value::Record(_) => "record",
            Value::Context(_) => "context",
            Value::InlineText { .. } => "inline-text",
            Value::BlockText { .. } => "block-text",
            Value::MathText { .. } => "math",
            Value::Ref(_) => "mutable",
            Value::InlineBoxes(_) => "inline-boxes",
            Value::BlockBoxes(_) => "block-boxes",
            Value::Document(_) => "document",
            Value::Closure { .. } => "function",
            Value::CompiledClosure { .. } => "function",
            Value::Prim { .. } => "function",
            Value::PrePath(_) => "pre-path",
            Value::Path(_) => "path",
            Value::Graphics(_) => "graphics",
        }
    }
}

/// The final result of evaluating a document.
#[derive(Clone, Debug)]
pub struct DocumentValue {
    pub geometry: PageGeometry,
    pub pages: Vec<Page>,
}

/// A lexical environment: a frame chain (`environment` in the OCaml).
#[derive(Clone, Debug)]
pub struct Env(Rc<Frame>);

#[derive(Debug)]
struct Frame {
    vars: RefCell<HashMap<String, Value>>,
    parent: Option<Env>,
}

impl Env {
    pub fn root() -> Env {
        Env(Rc::new(Frame {
            vars: RefCell::new(HashMap::new()),
            parent: None,
        }))
    }

    pub fn child(&self) -> Env {
        Env(Rc::new(Frame {
            vars: RefCell::new(HashMap::new()),
            parent: Some(self.clone()),
        }))
    }

    pub fn define(&self, name: impl Into<String>, value: Value) {
        self.0.vars.borrow_mut().insert(name.into(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.0.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.0.parent.as_ref()?.lookup(name)
    }

    /// All names bound anywhere in the chain (feeds the elaborator's scope).
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.0.vars.borrow().keys().cloned().collect();
        if let Some(p) = &self.0.parent {
            out.extend(p.names());
        }
        out
    }
}
