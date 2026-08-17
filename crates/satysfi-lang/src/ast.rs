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
    /// Mutually recursive bindings (`let-rec … and …`); every body must be a
    /// `Lambda`, all names are in scope in all bodies.
    LetRecIn(Vec<(String, Rc<Ast>)>, Box<Ast>),
    IfThenElse(Box<Ast>, Box<Ast>, Box<Ast>),
    Match(Box<Ast>, Vec<MatchArm>),
    Tuple(Vec<Ast>),
    /// A variant constructor, optionally applied (`None` / `Some 3`).
    Ctor(String, Option<Box<Ast>>),
    Record(Vec<(String, Ast)>),
    List(Vec<Ast>),
    /// Quoted inline text: evaluated only when `read-inline` runs it.
    InlineText(Rc<Vec<IText>>),
    /// Quoted block text: evaluated only when `read-block` runs it.
    BlockText(Rc<Vec<BText>>),
    /// Quoted math text (`${…}`); typesetting is deferred to phase 7, the
    /// value is carried opaquely until then.
    MathText(Rc<Vec<MathElem>>),
    /// `let-mutable x <- init in body` — binds `x` to a mutable cell.
    LetMutableIn(String, Box<Ast>, Box<Ast>),
    /// `x <- e` — overwrite a mutable cell; evaluates to unit.
    Overwrite(String, Span, Box<Ast>),
    /// `while cond do body` — evaluates to unit.
    WhileDo(Box<Ast>, Box<Ast>),
    /// `e1 before e2` (`UTSequential`) — evaluate `e1` for effect, then `e2`.
    Sequential(Box<Ast>, Box<Ast>),
    /// `e#label` (`UTAccessField`).
    AccessField(Box<Ast>, String, Span),
    /// `(| e with label = v |)` (`UTUpdateField`) — functional record update.
    UpdateField(Box<Ast>, String, Box<Ast>),
}

/// One quoted math element (structure mirrors the `mathmain`/`mathtop`/
/// `mathbot` rules; only carried, not typeset, until phase 7).
#[derive(Clone, Debug, PartialEq)]
pub enum MathElem {
    /// A run of math characters/symbols (`MATHCHAR`).
    Chars(String),
    /// `{ … }` grouping.
    Group(Vec<MathElem>),
    /// `base _ script`
    Sub(Box<MathElem>, Vec<MathElem>),
    /// `base ^ script`
    Sup(Box<MathElem>, Vec<MathElem>),
    /// `base '`+ (primes count as a superscript)
    Primes(Box<MathElem>, usize),
    /// `\cmd args…` in math mode; sigil included.
    Cmd {
        name: String,
        span: Span,
        args: Vec<Ast>,
    },
    /// `#x` in math mode.
    Embed { expr: Ast, span: Span },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pat: Pattern,
    /// `when` guard, if any.
    pub guard: Option<Ast>,
    pub body: Ast,
}

/// Match patterns (`untyped_pattern_tree`).
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern {
    Wild,
    Var(String),
    Unit,
    Bool(bool),
    Int(i64),
    Str(String),
    Tuple(Vec<Pattern>),
    EmptyList,
    /// `head :: tail`
    Cons(Box<Pattern>, Box<Pattern>),
    Ctor(String, Option<Box<Pattern>>),
    /// `pat as name`
    As(Box<Pattern>, String),
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
    /// `#expr;` — an embedded expression evaluating to inline-text, spliced
    /// in place (`UTInputHorzContent`).
    Embed { expr: Ast, span: Span },
    /// `${…}` embedded math (`UTInputHorzEmbeddedMath`); errors at read time
    /// until phase 7.
    EmbedMath {
        elems: Rc<Vec<MathElem>>,
        span: Span,
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
    /// `#expr;` — an embedded expression evaluating to block-text
    /// (`UTInputVertContent`).
    Embed { expr: Ast, span: Span },
}
