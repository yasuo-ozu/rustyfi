//! The elaborated abstract syntax tree (a milestone-1 subset of
//! `abstract_tree` in types.cppo.ml). Produced from the surface CST by
//! `elaborate`; consumed by the evaluator.
//!
//! # The identifier parameter `I`
//!
//! Every node type here is generic over `I`, the representation of a
//! **lexical identifier** — precisely those names that become keys in the
//! runtime environment ([`crate::value::Env`]). Two instantiations exist
//! (Phase 1):
//!
//! * `Ast<Symbol<'s>>` — the *compile-side* tree, produced by
//!   [`crate::elaborate`] and consumed by [`crate::typecheck`]. Identifiers
//!   are interned [`crate::symbol::Symbol`]s: `Copy`, 4 bytes, compared and
//!   hashed as integers, and branded to the [`crate::symbol::SymbolStore`]
//!   that minted them.
//! * `Ast<String>` — the *runtime* tree, and the *default* (so `Ast`
//!   unadorned still means exactly what it always did). This is what
//!   [`crate::compile`] lowers and what [`crate::value::Value`] embeds in its
//!   quoted-text and closure payloads. It has **no lifetime**, which is the
//!   whole point: a branded `Symbol<'s>` reaching `Value` would cascade `'s`
//!   through all 172 `prim_*` functions for zero speed (design doc §1).
//!
//! [`crate::compile::compile_program`] is the membrane between the two: it
//! [de-brands](Ast::map_idents) the branded tree to `Ast<String>` once, at
//! compile time, so the compiler and the entire runtime stay untouched.
//!
//! ## What is *not* parameterised
//!
//! `I` marks environment keys and nothing else. String literals
//! ([`Ast::Str`], [`Pattern::Str`], [`MathElem::Chars`], [`IText::Text`]) are
//! char data. Record field labels, constructor tags, and labeled-optional
//! *labels* are separate data namespaces that reach the runtime as
//! `BTreeMap` keys and value tags — they stay `String` end-to-end and must
//! never be symbolized (design doc §7). Note the asymmetry in
//! [`Ast::LambdaOpt`]: an optional argument's **label** is data (`String`),
//! its **binder** is a lexical variable (`I`).

use rustyfi_backend::Length;
use rustyfi_syntax::{RustyfiVersion, Span};
use std::rc::Rc;

/// The **branded** instantiation of every node type in this module: what
/// [`crate::elaborate`] produces and [`crate::typecheck`] consumes, with each
/// lexical identifier interned into a [`crate::symbol::SymbolStore`].
///
/// The names deliberately shadow the unparameterised ones, so the front half
/// of the pipeline reads exactly as it did before interning — `use
/// crate::ast::branded::{Ast, Pattern, ..}` and every `Ast::Var(..)` /
/// `match .. { Ast::LetIn(..) => }` site is unchanged apart from the `<'s>`
/// its enclosing signature now carries.
pub mod branded {
    use crate::symbol::Symbol;

    pub type Ast<'s> = super::Ast<Symbol<'s>>;
    pub type BText<'s> = super::BText<Symbol<'s>>;
    pub type CmdArg<'s> = super::CmdArg<Symbol<'s>>;
    pub type IText<'s> = super::IText<Symbol<'s>>;
    pub type MatchArm<'s> = super::MatchArm<Symbol<'s>>;
    pub type MathElem<'s> = super::MathElem<Symbol<'s>>;
    pub type Pattern<'s> = super::Pattern<Symbol<'s>>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ast<I = String> {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Length(Length),
    Str(String),
    Var(I, Span),
    Apply(Box<Ast<I>>, Box<Ast<I>>),
    Lambda(I, Rc<Ast<I>>),
    LetIn(I, Box<Ast<I>>, Box<Ast<I>>),
    /// Mutually recursive bindings (`let-rec … and …`); every body must be a
    /// `Lambda`, all names are in scope in all bodies.
    LetRecIn(Vec<(I, Rc<Ast<I>>)>, Box<Ast<I>>),
    /// `let-math \cmd param* = expr in body` — a math-command binding.
    /// Evaluates identically to `LetIn` (a plain named binding; see
    /// `eval.rs`); the DISTINCT variant exists purely so the typechecker
    /// can tell it apart from an ordinary `\`-sigiled `LetIn` (a
    /// `let-inline` binding or a qualified-name alias of one) without
    /// re-deriving that from the shared `\` sigil — see `typecheck.rs`'s
    /// `Checker::math_command_scheme`.
    LetMathIn(I, Box<Ast<I>>, Box<Ast<I>>),
    IfThenElse(Box<Ast<I>>, Box<Ast<I>>, Box<Ast<I>>),
    Match(Box<Ast<I>>, Vec<MatchArm<I>>),
    Tuple(Vec<Ast<I>>),
    /// A variant constructor, optionally applied (`None` / `Some 3`).
    /// The tag is a data-level name, not an environment key — see the module
    /// doc comment on what `I` does and does not cover.
    Ctor(String, Option<Box<Ast<I>>>),
    Record(Vec<(String, Ast<I>)>),
    List(Vec<Ast<I>>),
    /// Quoted inline text: evaluated only when `read-inline` runs it.
    InlineText(Rc<Vec<IText<I>>>),
    /// Quoted block text: evaluated only when `read-block` runs it.
    BlockText(Rc<Vec<BText<I>>>),
    /// Quoted math text (`${…}`); typesetting is deferred to phase 7, the
    /// value is carried opaquely until then.
    MathText(Rc<Vec<MathElem<I>>>),
    /// `let-mutable x <- init in body` — binds `x` to a mutable cell.
    LetMutableIn(I, Box<Ast<I>>, Box<Ast<I>>),
    /// `x <- e` — overwrite a mutable cell; evaluates to unit.
    Overwrite(I, Span, Box<Ast<I>>),
    /// `while cond do body` — evaluates to unit.
    WhileDo(Box<Ast<I>>, Box<Ast<I>>),
    /// `e1 before e2` (`UTSequential`) — evaluate `e1` for effect, then `e2`.
    Sequential(Box<Ast<I>>, Box<Ast<I>>),
    /// `e#label` (`UTAccessField`). The label is a record field, not an
    /// environment key — see the module doc comment.
    AccessField(Box<Ast<I>>, String, Span),
    /// `(| e with label = v |)` (`UTUpdateField`) — functional record update.
    UpdateField(Box<Ast<I>>, String, Box<Ast<I>>),
    /// `f ?(l = e, …) arg` — SATySFi 0.1 labeled-optional application
    /// (upstream `Apply(labmap, e1, e2)`). `opts` is non-empty by
    /// construction: a bundle-less 0.1 application lowers to plain
    /// [`Ast::Apply`]. Labels are deduplicated at elaboration. At
    /// beta-reduction a provided `?(l = e)` binds the closure's `l` binder to
    /// `Some e`; any of the closure's declared labels the call omits bind
    /// `None` (see `eval::Interp::apply_with_opts`).
    ApplyOpt {
        func: Box<Ast<I>>,
        opts: Vec<(String, Ast<I>)>,
        arg: Box<Ast<I>>,
    },
    /// `fun ?(l = x, …) p -> body` — SATySFi 0.1 labeled-optional lambda
    /// (upstream `Function(evid_labmap, patbr)`). `opts` maps each label to
    /// the binder name that receives its `option`-typed value. Pattern
    /// params are pre-desugared (by `elaborate`) to a fresh var + `Match`,
    /// like `rec_clause_value`, so `param` here is always a plain binder.
    ///
    /// Note the mixed pair: each label is *data* (`String` — it is matched
    /// against a call site's `?(l = e)` labels), while each binder is a
    /// *lexical variable* (`I` — it becomes an environment key).
    LambdaOpt {
        opts: Vec<(String, I)>,
        param: I,
        body: Rc<Ast<I>>,
    },
    /// A version tag around one spliced cross-version dependency binding's
    /// RHS (Slice X2a,, Option C). `elaborate.rs`'s cross-version splice
    /// wraps each binding contributed by a `LoadedCst::V0_0` dependency
    /// (`lib.rs`'s `compile_document_v1_with_trials` splice arm) in
    /// `VersionScope(V0_0, rhs)`, at RHS granularity (never the surrounding
    /// `LetIn`/`LetRecIn` node, and never the continuation that follows it
    /// — see `elaborate::elaborate_program_with_versions`'s doc comment).
    /// Three consumers read the tag, all pushing/popping a cursor around
    /// recursing into `body`:
    /// - `compile.rs`'s `Compiler::current_version` — which base
    ///   environment (`V0_1`'s or `V0_0`'s) an unshadowed `Ast::Var`
    ///   constant-folds against, so a version-forked primitive
    ///   (`page-break`, `math-*`, …) freezes to the RIGHT version's
    ///   `PrimDef` at compile time (`compile.rs:120-192`'s existing fold —
    ///   X2a's whole mechanism rides on that fold already being the only
    ///   version-sensitive resolution in the pipeline).
    /// - `eval.rs`'s `Interp::version` — the R2 fix: any runtime fork that
    ///   reads it (`primitives.rs`'s `reflect_math_elem`/
    ///   `coerce_graphics_result`/`make_paren_run`) sees `V0_0` while
    ///   evaluating on behalf of this subtree.
    /// - `typecheck.rs`'s base-type-env swap — the subtree's *internal*
    ///   forked-primitive-type use (e.g. constructing a `page` ADT to hand
    ///   to `page-break`) checks against `V0_0`'s primitive types.
    ///
    /// **Never emitted on a pure single-version load** — `elaborate_program`
    /// delegates to `elaborate_program_with_versions` with an empty index
    /// set, so this variant is structurally inert (no arm anywhere ever
    /// executes) on the pure-0.0.6 and pure-0.1 paths; the GOLDEN
    /// byte-identical invariant holds because the code producing/consuming
    /// it is simply never reached there, not because of any runtime check.
    VersionScope(RustyfiVersion, Box<Ast<I>>),
    /// `ModuleScope(["M", "N"], rhs)`: marks that `rhs` is the body of a
    /// member of module `M.N`, so a BARE constructor reference inside it
    /// resolves against that module's constructors first (the type/ctor analog
    /// of `push_named_binding`'s value `Scope::rename`). Transparent
    /// everywhere except `Checker::infer`/`bind_pattern`, which push the path
    /// onto `Checker::ctor_scope` and try qualified ctor keys before the bare
    /// fallback — no constructor NAME string ever changes (so eval,
    /// exhaustiveness, and error/warning text stay byte-identical). Wraps a
    /// module member's RHS only (never the spine `LetIn`/`LetRecIn` node),
    /// exactly like `VersionScope`.
    ModuleScope(Vec<String>, Box<Ast<I>>),
}

/// One command-application argument (SATySFi 0.1 optional-arg-rows increment
/// 3b-β): `arg` is the ordinary positional argument value (exactly what
/// `Vec<Ast>` used to carry directly), `opts` is this argument's supplied
/// `?(l = e, …)` labeled-optional bundle — upstream's `UTCommandArg of
/// (label * expr) list * expr` (`types.cppo.ml:583-584`). Every producer
/// this port has BEFORE increment 3b (all of 0.0.6, and every V0_1 command
/// call with no bundle — the only kind increment 3a's demand census found)
/// emits `opts: vec![]`, so this is additive: an empty bundle behaves
/// exactly like the old bare `Ast` did. At runtime, a non-empty `opts` folds
/// through `eval::Interp::apply_with_opts` (like `Ast::ApplyOpt` does for a
/// plain-value application) instead of a plain `apply`; each label the
/// command declares but this call omits still defaults to `None` there.
#[derive(Clone, Debug, PartialEq)]
pub struct CmdArg<I = String> {
    pub opts: Vec<(String, Ast<I>)>,
    pub arg: Ast<I>,
}

/// One quoted math element (structure mirrors the `mathmain`/`mathtop`/
/// `mathbot` rules; only carried, not typeset, until phase 7).
#[derive(Clone, Debug, PartialEq)]
pub enum MathElem<I = String> {
    /// A run of math characters/symbols (`MATHCHAR`).
    Chars(String),
    /// `{ … }` grouping.
    Group(Vec<MathElem<I>>),
    /// `base _ script`
    Sub(Box<MathElem<I>>, Vec<MathElem<I>>),
    /// `base ^ script`
    Sup(Box<MathElem<I>>, Vec<MathElem<I>>),
    /// `base '`+ (primes count as a superscript)
    Primes(Box<MathElem<I>>, usize),
    /// `\cmd args…` in math mode; sigil included. `args` is [`CmdArg`]-shaped
    /// for uniformity with `IText::Cmd`/`BText::Cmd` (`check_cmd_args`/the
    /// runtime command fold are shared across all three); the math-mode
    /// application grammar (`cst::ast::MathArg`) has no `?(l=e)` bundle form
    /// at all (math command *arguments* are always bracket groups — `{…}` /
    /// `!{…}` / `!<…>` / `!(…)`, upstream `narg`), so every `CmdArg` here
    /// has `opts: vec![]` by construction — see `elaborate::math_bot`.
    Cmd {
        name: I,
        span: Span,
        args: Vec<CmdArg<I>>,
    },
    /// `#x` in math mode.
    Embed { expr: Ast<I>, span: Span },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm<I = String> {
    pub pat: Pattern<I>,
    /// `when` guard, if any.
    pub guard: Option<Ast<I>>,
    pub body: Ast<I>,
}

/// Match patterns (`untyped_pattern_tree`).
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern<I = String> {
    Wild,
    Var(I),
    Unit,
    Bool(bool),
    Int(i64),
    Str(String),
    Tuple(Vec<Pattern<I>>),
    EmptyList,
    /// `head :: tail`
    Cons(Box<Pattern<I>>, Box<Pattern<I>>),
    Ctor(String, Option<Box<Pattern<I>>>),
    /// `pat as name`
    As(Box<Pattern<I>>, I),
}

/// One inline-text element (`input_horz_element`).
#[derive(Clone, Debug, PartialEq)]
pub enum IText<I = String> {
    Text(String),
    /// A backtick literal inside inline text (`` `…` ``;
    /// `UTInputHorzEmbeddedCodeText`). Kept apart from `Text` because the
    /// context's installed code-text command decides how it is set — see
    /// `Context::code_text_command` and `read_inline`'s arm.
    CodeText(String),
    Cmd {
        /// Sigil included (`\emph`), matching the environment entry.
        name: I,
        span: Span,
        args: Vec<CmdArg<I>>,
    },
    /// `#expr;` — an embedded expression evaluating to inline-text, spliced
    /// in place (`UTInputHorzContent`).
    Embed {
        expr: Ast<I>,
        span: Span,
    },
    /// `${…}` embedded math (`UTInputHorzEmbeddedMath`). `read_inline`'s
    /// `EmbedMath` arm (`primitives.rs`) applies the context's installed
    /// `[math] inline-cmd` (`Context::math_command`, Gap 1 —
    /// `class-signature-lang-gaps.md`) to `(ctx, math value)`, exactly like
    /// upstream — `\cmd`/`#var` inside the literal go through
    /// `reflect_math_elem`/`as_math`. Contexts with no installed command
    /// (built by `Context::initial` directly, i.e. unit tests) fall back to
    /// reflecting + laying out through the same faithful engine directly.
    EmbedMath {
        elems: Rc<Vec<MathElem<I>>>,
        span: Span,
    },
}

/// One block-text element (`input_vert_element`).
#[derive(Clone, Debug, PartialEq)]
pub enum BText<I = String> {
    Cmd {
        /// Sigil included (`+p`).
        name: I,
        span: Span,
        args: Vec<CmdArg<I>>,
    },
    /// `#expr;` — an embedded expression evaluating to block-text
    /// (`UTInputVertContent`).
    Embed { expr: Ast<I>, span: Span },
}

// ---------------------------------------------------------------------------
// Identifier remapping — the compile membrane's de-branding
// ---------------------------------------------------------------------------
//
// `map_idents` rebuilds a tree with every *lexical identifier* (and nothing
// else — see the module doc comment) passed through `f`. Its one production
// use is `compile::compile_program`, which turns the branded compile-side
// `Ast<Symbol<'s>>` into the lifetime-free runtime `Ast<String>` by resolving
// each symbol back to its text. Everything else — literals, record labels,
// constructor tags, optional-argument labels, spans — is cloned verbatim, so
// the result is *structurally identical* to the tree elaboration produced
// before identifiers were interned. That is what makes the interning phase
// byte-identical by construction.
//
// This is a deep copy: `Rc` payloads are rebuilt rather than shared. The
// elaborated tree is a pure tree (no DAG sharing), so nothing that was
// previously shared gets duplicated, and the copy happens exactly once per
// compile — not once per fixpoint trial.

impl<I> Ast<I> {
    /// Rebuild this tree with every lexical identifier mapped through `f`.
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> Ast<J> {
        // A local alias keeps the arms below readable.
        let go = |a: &Ast<I>| a.map_idents(f);
        match self {
            Ast::Unit => Ast::Unit,
            Ast::Bool(b) => Ast::Bool(*b),
            Ast::Int(n) => Ast::Int(*n),
            Ast::Float(x) => Ast::Float(*x),
            Ast::Length(l) => Ast::Length(*l),
            Ast::Str(s) => Ast::Str(s.clone()),
            Ast::Var(n, sp) => Ast::Var(f(n), *sp),
            Ast::Apply(g, a) => Ast::Apply(Box::new(go(g)), Box::new(go(a))),
            Ast::Lambda(p, b) => Ast::Lambda(f(p), Rc::new(go(b))),
            Ast::LetIn(n, v, r) => Ast::LetIn(f(n), Box::new(go(v)), Box::new(go(r))),
            Ast::LetRecIn(bs, body) => Ast::LetRecIn(
                bs.iter().map(|(n, v)| (f(n), Rc::new(go(v)))).collect(),
                Box::new(go(body)),
            ),
            Ast::LetMathIn(n, v, r) => Ast::LetMathIn(f(n), Box::new(go(v)), Box::new(go(r))),
            Ast::IfThenElse(c, t, e) => {
                Ast::IfThenElse(Box::new(go(c)), Box::new(go(t)), Box::new(go(e)))
            }
            Ast::Match(s, arms) => Ast::Match(
                Box::new(go(s)),
                arms.iter().map(|a| a.map_idents(f)).collect(),
            ),
            Ast::Tuple(items) => Ast::Tuple(items.iter().map(go).collect()),
            Ast::Ctor(tag, arg) => Ast::Ctor(tag.clone(), arg.as_ref().map(|a| Box::new(go(a)))),
            Ast::Record(fields) => {
                Ast::Record(fields.iter().map(|(l, e)| (l.clone(), go(e))).collect())
            }
            Ast::List(items) => Ast::List(items.iter().map(go).collect()),
            Ast::InlineText(elems) => {
                Ast::InlineText(Rc::new(elems.iter().map(|e| e.map_idents(f)).collect()))
            }
            Ast::BlockText(elems) => {
                Ast::BlockText(Rc::new(elems.iter().map(|e| e.map_idents(f)).collect()))
            }
            Ast::MathText(elems) => {
                Ast::MathText(Rc::new(elems.iter().map(|e| e.map_idents(f)).collect()))
            }
            Ast::LetMutableIn(n, i, b) => Ast::LetMutableIn(f(n), Box::new(go(i)), Box::new(go(b))),
            Ast::Overwrite(n, sp, v) => Ast::Overwrite(f(n), *sp, Box::new(go(v))),
            Ast::WhileDo(c, b) => Ast::WhileDo(Box::new(go(c)), Box::new(go(b))),
            Ast::Sequential(a, b) => Ast::Sequential(Box::new(go(a)), Box::new(go(b))),
            Ast::AccessField(e, l, sp) => Ast::AccessField(Box::new(go(e)), l.clone(), *sp),
            Ast::UpdateField(e, l, v) => {
                Ast::UpdateField(Box::new(go(e)), l.clone(), Box::new(go(v)))
            }
            Ast::ApplyOpt { func, opts, arg } => Ast::ApplyOpt {
                func: Box::new(go(func)),
                opts: opts.iter().map(|(l, e)| (l.clone(), go(e))).collect(),
                arg: Box::new(go(arg)),
            },
            Ast::LambdaOpt { opts, param, body } => Ast::LambdaOpt {
                // Label stays data, binder is a lexical variable.
                opts: opts.iter().map(|(l, b)| (l.clone(), f(b))).collect(),
                param: f(param),
                body: Rc::new(go(body)),
            },
            Ast::VersionScope(v, b) => Ast::VersionScope(*v, Box::new(go(b))),
            Ast::ModuleScope(path, b) => Ast::ModuleScope(path.clone(), Box::new(go(b))),
        }
    }
}

impl<I> MatchArm<I> {
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> MatchArm<J> {
        MatchArm {
            pat: self.pat.map_idents(f),
            guard: self.guard.as_ref().map(|g| g.map_idents(f)),
            body: self.body.map_idents(f),
        }
    }
}

impl<I> Pattern<I> {
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> Pattern<J> {
        match self {
            Pattern::Wild => Pattern::Wild,
            Pattern::Var(n) => Pattern::Var(f(n)),
            Pattern::Unit => Pattern::Unit,
            Pattern::Bool(b) => Pattern::Bool(*b),
            Pattern::Int(n) => Pattern::Int(*n),
            Pattern::Str(s) => Pattern::Str(s.clone()),
            Pattern::Tuple(ps) => Pattern::Tuple(ps.iter().map(|p| p.map_idents(f)).collect()),
            Pattern::EmptyList => Pattern::EmptyList,
            Pattern::Cons(h, t) => {
                Pattern::Cons(Box::new(h.map_idents(f)), Box::new(t.map_idents(f)))
            }
            Pattern::Ctor(tag, p) => {
                Pattern::Ctor(tag.clone(), p.as_ref().map(|p| Box::new(p.map_idents(f))))
            }
            Pattern::As(p, n) => Pattern::As(Box::new(p.map_idents(f)), f(n)),
        }
    }
}

impl<I> CmdArg<I> {
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> CmdArg<J> {
        CmdArg {
            opts: self
                .opts
                .iter()
                .map(|(l, e)| (l.clone(), e.map_idents(f)))
                .collect(),
            arg: self.arg.map_idents(f),
        }
    }
}

impl<I> IText<I> {
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> IText<J> {
        match self {
            IText::Text(s) => IText::Text(s.clone()),
            IText::CodeText(s) => IText::CodeText(s.clone()),
            IText::Cmd { name, span, args } => IText::Cmd {
                name: f(name),
                span: *span,
                args: args.iter().map(|a| a.map_idents(f)).collect(),
            },
            IText::Embed { expr, span } => IText::Embed {
                expr: expr.map_idents(f),
                span: *span,
            },
            IText::EmbedMath { elems, span } => IText::EmbedMath {
                elems: Rc::new(elems.iter().map(|e| e.map_idents(f)).collect()),
                span: *span,
            },
        }
    }
}

impl<I> BText<I> {
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> BText<J> {
        match self {
            BText::Cmd { name, span, args } => BText::Cmd {
                name: f(name),
                span: *span,
                args: args.iter().map(|a| a.map_idents(f)).collect(),
            },
            BText::Embed { expr, span } => BText::Embed {
                expr: expr.map_idents(f),
                span: *span,
            },
        }
    }
}

impl<I> MathElem<I> {
    pub fn map_idents<J>(&self, f: &impl Fn(&I) -> J) -> MathElem<J> {
        match self {
            MathElem::Chars(s) => MathElem::Chars(s.clone()),
            MathElem::Group(es) => MathElem::Group(es.iter().map(|e| e.map_idents(f)).collect()),
            MathElem::Sub(b, s) => MathElem::Sub(
                Box::new(b.map_idents(f)),
                s.iter().map(|e| e.map_idents(f)).collect(),
            ),
            MathElem::Sup(b, s) => MathElem::Sup(
                Box::new(b.map_idents(f)),
                s.iter().map(|e| e.map_idents(f)).collect(),
            ),
            MathElem::Primes(b, n) => MathElem::Primes(Box::new(b.map_idents(f)), *n),
            MathElem::Cmd { name, span, args } => MathElem::Cmd {
                name: f(name),
                span: *span,
                args: args.iter().map(|a| a.map_idents(f)).collect(),
            },
            MathElem::Embed { expr, span } => MathElem::Embed {
                expr: expr.map_idents(f),
                span: *span,
            },
        }
    }
}

/// **The compile membrane**: resolve every interned `Symbol` in a branded,
/// elaborated tree back to its text, producing the lifetime-free
/// `Ast<String>` that [`crate::compile`] lowers and the whole runtime works
/// on.
///
/// This is where the identifier brand is discharged. Nothing downstream —
/// the compiler, `CompiledExpr` (whose boxed closure is implicitly
/// `'static`), [`crate::value::Value`]'s quoted-text and closure payloads,
/// `Interp`, the 172 `prim_*` functions — ever sees a `Symbol` or the `'s`
/// it carries. That is exactly what keeps the brand from cascading a
/// lifetime through the entire runtime for zero speed (design doc §1).
///
/// Byte-identical by construction: [`Ast::map_idents`] rebuilds the same tree
/// with each symbol replaced by precisely the string it was interned from, so
/// the compiler receives the tree elaboration used to hand it before
/// identifiers were interned. Cost is one deep copy per compile — not per
/// fixpoint trial, since the trials re-run the resulting compiled closure.
pub fn debrand(ast: &branded::Ast<'_>, store: &crate::symbol::SymbolStore) -> Ast {
    ast.map_idents(&|sym| store.resolve(*sym).to_string())
}
