//! Quoted text in its **compiled** form — what `{ … }` / `'< … >` / `${ … }`
//! become once [`crate::compile`] has lowered them, and what
//! [`crate::value::Value`]'s `InlineText`/`BlockText`/`MathText` variants
//! carry.
//!
//! Phase 3 of (§4, "THE CRUX").
//!
//! # Why these exist
//!
//! Quoted text used to be carried as raw [`crate::ast`] nodes plus a captured
//! `Env`, and resolved **lazily, by string, at layout time**: a `\emph`'s
//! command name went through `env.lookup` on every occurrence, and every
//! embedded expression re-entered the compiler mid-evaluation (memoized by AST
//! pointer address) with global constant-folding switched *off*, because the
//! captured environment's local frames were statically unknown.
//!
//! None of that is necessary. The compiler *does* know the lexical scope at
//! the quote site — it is exactly the scope stack at that point — so command
//! names and embedded expressions are resolved there, once, like any other
//! expression. What survives into the runtime is this name-free tree: every
//! `Cmd` already holds the [`CompiledExpr`] that yields its command value, and
//! every argument is already compiled.
//!
//! The captured `Env` is still needed (a compiled node resolves its *locals*
//! against the environment it runs in), but nothing here is ever looked up by
//! name any more.
//!
//! # Shape
//!
//! Deliberately mirrors [`crate::ast`]'s `IText`/`BText`/`MathElem`/`CmdArg`
//! one-for-one, so the structural walks in `primitives.rs` (`read_inline`,
//! `read_block`, `reflect_math_elem*`, `layout_math_elem`, …) are unchanged
//! apart from how a `Cmd` obtains its command and how an argument is
//! evaluated. Only two fields differ: a `Cmd`'s `name: String` became a
//! resolved `cmd: CompiledExpr`, and an `Embed`'s `expr: Ast` became a
//! compiled one.

use crate::ast::Ast;
use crate::compile::CompiledExpr;
use crate::value::BaseEnv;
use rustyfi_syntax::Span;
use std::rc::Rc;

/// One command-application argument: the positional argument plus its
/// (usually empty) `?(l = e, …)` labeled-optional bundle. Labels stay text —
/// they are matched against a closure's declared labels, not looked up in an
/// environment.
#[allow(private_interfaces)]
#[derive(Clone, Debug)]
pub struct CmdArg {
    pub opts: Vec<(String, CompiledExpr)>,
    pub arg: CompiledExpr,
}

/// One inline-text element (the compiled mirror of [`crate::ast::IText`]).
#[allow(private_interfaces)]
#[derive(Clone, Debug)]
pub enum IText {
    Text(String),
    /// A backtick literal, dispatched at box-building time through the
    /// context's `code_text_command` (see `read_inline`).
    CodeText(String),
    Cmd {
        /// Yields the command's value — the `\emph` binding, already resolved
        /// against the quote site's lexical scope. Running it can still fail
        /// with the same "unbound inline command '…' at run time" error the
        /// `env.lookup` used to produce, for the defensive case where the name
        /// was in no compile-time scope.
        cmd: CompiledExpr,
        args: Vec<CmdArg>,
    },
    /// `#expr;` — an embedded expression evaluating to inline-text.
    Embed {
        expr: CompiledExpr,
        span: Span,
    },
    /// `${…}` embedded math.
    EmbedMath {
        elems: Rc<Vec<MathElem>>,
        span: Span,
    },
}

/// One block-text element (the compiled mirror of [`crate::ast::BText`]).
#[allow(private_interfaces)]
#[derive(Clone, Debug)]
pub enum BText {
    Cmd {
        /// See [`IText::Cmd::cmd`].
        cmd: CompiledExpr,
        args: Vec<CmdArg>,
    },
    Embed {
        expr: CompiledExpr,
        span: Span,
    },
}

/// One quoted-math element (the compiled mirror of [`crate::ast::MathElem`]).
#[allow(private_interfaces)]
#[derive(Clone, Debug)]
pub enum MathElem {
    Chars(String),
    Group(Vec<MathElem>),
    Sub(Box<MathElem>, Vec<MathElem>),
    Sup(Box<MathElem>, Vec<MathElem>),
    Primes(Box<MathElem>, usize),
    Cmd {
        /// See [`IText::Cmd::cmd`].
        cmd: CompiledExpr,
        /// Kept purely for diagnostics: two `reflect_*` arms name the command
        /// in a "not valid here" error. Never used to look anything up.
        name: Rc<str>,
        span: Span,
        args: Vec<CmdArg>,
    },
    Embed {
        expr: CompiledExpr,
        span: Span,
    },
}

impl IText {
    /// Build an `#expr;` embed element, compiling `expr` against `env`.
    ///
    /// [`crate::primitives::read_inline`] is public and takes these elements,
    /// so there has to be a way to build one from outside the crate — but
    /// `CompiledExpr` is deliberately crate-internal, so the compile step
    /// happens here rather than in the caller. `env` plays the role the
    /// enclosing compiler's lexical scope plays for a real quote site: it is
    /// what free names in `expr` are resolved against.
    pub fn embed(expr: &Ast, env: &BaseEnv, span: Span) -> IText {
        IText::Embed {
            expr: crate::compile::compile_program(expr, env),
            span,
        }
    }
}
