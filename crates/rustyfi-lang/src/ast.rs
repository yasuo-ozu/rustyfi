//! The elaborated abstract syntax tree (a milestone-1 subset of
//! `abstract_tree` in types.cppo.ml). Produced from the surface CST by
//! `elaborate`; consumed by the evaluator.

use rustyfi_backend::Length;
use rustyfi_syntax::{RustyfiVersion, Span};
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
    /// `let-math \cmd param* = expr in body` (`docs/plans/math-engine.md`
    /// §G) — a math-command binding. Evaluates identically to `LetIn` (a
    /// plain named binding; see `eval.rs`); the DISTINCT variant exists
    /// purely so the typechecker can tell it apart from an ordinary
    /// `\`-sigiled `LetIn` (a `let-inline` binding or a qualified-name
    /// alias of one) without re-deriving that from the shared `\` sigil —
    /// see `typecheck.rs`'s `Checker::math_command_scheme`.
    LetMathIn(String, Box<Ast>, Box<Ast>),
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
    /// `f ?(l = e, …) arg` — SATySFi 0.1 labeled-optional application
    /// (upstream `Apply(labmap, e1, e2)`). `opts` is non-empty by
    /// construction: a bundle-less 0.1 application lowers to plain
    /// [`Ast::Apply`]. Labels are deduplicated at elaboration. At
    /// beta-reduction a provided `?(l = e)` binds the closure's `l` binder to
    /// `Some e`; any of the closure's declared labels the call omits bind
    /// `None` (see `eval::Interp::apply_with_opts`).
    ApplyOpt {
        func: Box<Ast>,
        opts: Vec<(String, Ast)>,
        arg: Box<Ast>,
    },
    /// `fun ?(l = x, …) p -> body` — SATySFi 0.1 labeled-optional lambda
    /// (upstream `Function(evid_labmap, patbr)`). `opts` maps each label to
    /// the binder name that receives its `option`-typed value. Pattern
    /// params are pre-desugared (by `elaborate`) to a fresh var + `Match`,
    /// like `rec_clause_value`, so `param` here is always a plain binder.
    LambdaOpt {
        opts: Vec<(String, String)>,
        param: String,
        body: Rc<Ast>,
    },
    /// A version tag around one spliced cross-version dependency binding's
    /// RHS (Slice X2a, `docs/plans/design-cross-version-import.md`
    /// §"Slice X2 — per-group primitive environment", Option C).
    /// `elaborate.rs`'s cross-version splice wraps each binding contributed
    /// by a `LoadedCst::V0_0_6` dependency (`lib.rs`'s
    /// `compile_document_v1_with_trials` splice arm) in
    /// `VersionScope(V0_0_6, rhs)`, at RHS granularity (never the
    /// surrounding `LetIn`/`LetRecIn` node, and never the continuation that
    /// follows it — see `elaborate::elaborate_program_with_versions`'s doc
    /// comment). Three consumers read the tag, all pushing/popping a cursor
    /// around recursing into `body`:
    /// - `compile.rs`'s `Compiler::current_version` — which base
    ///   environment (`V0_1`'s or `V0_0_6`'s) an unshadowed `Ast::Var`
    ///   constant-folds against, so a version-forked primitive
    ///   (`page-break`, `math-*`, …) freezes to the RIGHT version's
    ///   `PrimDef` at compile time (`compile.rs:120-192`'s existing fold —
    ///   X2a's whole mechanism rides on that fold already being the only
    ///   version-sensitive resolution in the pipeline).
    /// - `eval.rs`'s `Interp::version` — the R2 fix: any runtime fork that
    ///   reads it (`primitives.rs`'s `reflect_math_elem`/
    ///   `coerce_graphics_result`/`make_paren_run`) sees `V0_0_6` while
    ///   evaluating on behalf of this subtree.
    /// - `typecheck.rs`'s base-type-env swap — the subtree's *internal*
    ///   forked-primitive-type use (e.g. constructing a `page` ADT to hand
    ///   to `page-break`) checks against `V0_0_6`'s primitive types.
    ///
    /// **Never emitted on a pure single-version load** — `elaborate_program`
    /// delegates to `elaborate_program_with_versions` with an empty index
    /// set, so this variant is structurally inert (no arm anywhere ever
    /// executes) on the pure-0.0.6 and pure-0.1 paths; the GOLDEN
    /// byte-identical invariant holds because the code producing/consuming
    /// it is simply never reached there, not because of any runtime check.
    VersionScope(RustyfiVersion, Box<Ast>),
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
    ModuleScope(Vec<String>, Box<Ast>),
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
pub struct CmdArg {
    pub opts: Vec<(String, Ast)>,
    pub arg: Ast,
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
    /// `\cmd args…` in math mode; sigil included. `args` is [`CmdArg`]-shaped
    /// for uniformity with `IText::Cmd`/`BText::Cmd` (`check_cmd_args`/the
    /// runtime command fold are shared across all three); the math-mode
    /// application grammar (`cst::ast::MathArg`) has no `?(l=e)` bundle form
    /// at all (math command *arguments* are always bracket groups — `{…}` /
    /// `!{…}` / `!<…>` / `!(…)`, upstream `narg`), so every `CmdArg` here
    /// has `opts: vec![]` by construction — see `elaborate::math_bot`.
    Cmd {
        name: String,
        span: Span,
        args: Vec<CmdArg>,
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
        args: Vec<CmdArg>,
    },
    /// `#expr;` — an embedded expression evaluating to inline-text, spliced
    /// in place (`UTInputHorzContent`).
    Embed { expr: Ast, span: Span },
    /// `${…}` embedded math (`UTInputHorzEmbeddedMath`). `read_inline`'s
    /// `EmbedMath` arm (`primitives.rs`) applies the context's installed
    /// `[math] inline-cmd` (`Context::math_command`, Gap 1 —
    /// `class-signature-lang-gaps.md`) to `(ctx, math value)`, exactly like
    /// upstream — `\cmd`/`#var` inside the literal go through
    /// `reflect_math_elem`/`as_math`. Contexts with no installed command
    /// (built by `Context::initial` directly, i.e. unit tests) fall back to
    /// reflecting + laying out through the same faithful engine directly.
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
        args: Vec<CmdArg>,
    },
    /// `#expr;` — an embedded expression evaluating to block-text
    /// (`UTInputVertContent`).
    Embed { expr: Ast, span: Span },
}
