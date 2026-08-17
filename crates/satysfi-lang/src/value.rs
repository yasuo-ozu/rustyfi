//! Runtime values (a milestone-1 subset of `syntactic_value`).

use crate::ast::{Ast, BText, IText};
use crate::primitives::PrimDef;
use satysfi_backend::{Context, HorzBox, Length, Page, PageGeometry, VertBox};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Length(Length),
    Str(String),
    List(Vec<Value>),
    Record(BTreeMap<String, Value>),
    Context(Box<Context>),
    /// Quoted inline text with its captured environment
    /// (`InputHorzWithEnvironment`).
    InlineText { elems: Rc<Vec<IText>>, env: Env },
    /// Quoted block text with its captured environment.
    BlockText { elems: Rc<Vec<BText>>, env: Env },
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
    /// A (possibly partially applied) native primitive.
    Prim {
        def: &'static PrimDef,
        applied: Vec<Value>,
    },
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
            Value::Record(_) => "record",
            Value::Context(_) => "context",
            Value::InlineText { .. } => "inline-text",
            Value::BlockText { .. } => "block-text",
            Value::InlineBoxes(_) => "inline-boxes",
            Value::BlockBoxes(_) => "block-boxes",
            Value::Document(_) => "document",
            Value::Closure { .. } => "function",
            Value::Prim { .. } => "function",
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
