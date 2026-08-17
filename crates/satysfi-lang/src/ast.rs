//! The elaborated abstract syntax tree (a milestone-1 subset of
//! `abstract_tree` in types.cppo.ml). Produced from the surface CST by
//! `elaborate`; consumed by the evaluator.

use satysfi_backend::Length;
use satysfi_syntax::Span;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Length(Length),
    Str(String),
    Var(String, Span),
    Apply(Box<Ast>, Box<Ast>),
    Lambda(String, Rc<Ast>),
    LetIn(String, Box<Ast>, Box<Ast>),
    Record(Vec<(String, Ast)>),
    List(Vec<Ast>),
    /// Quoted inline text: evaluated only when `read-inline` runs it.
    InlineText(Rc<Vec<IText>>),
    /// Quoted block text: evaluated only when `read-block` runs it.
    BlockText(Rc<Vec<BText>>),
}

/// One inline-text element (`input_horz_element`).
#[derive(Clone, Debug, PartialEq)]
pub enum IText {
    Text(String),
    Cmd {
        /// Sigil included (`\emph`), matching the environment entry.
        name: String,
        span: Span,
        args: Vec<Ast>,
    },
}

/// One block-text element (`input_vert_element`).
#[derive(Clone, Debug, PartialEq)]
pub enum BText {
    Cmd {
        /// Sigil included (`+p`).
        name: String,
        span: Span,
        args: Vec<Ast>,
    },
}
