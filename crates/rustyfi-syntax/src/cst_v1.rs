//! The Slice-1 SATySFi **0.1.0** (`dev-0-1-0`) surface grammar — a *fork* of
//! [`crate::cst`], not a version-gate of it (gating one shared `cst.rs` would
//! mean hand-writing `Parse` for nearly every node, destroying the derive
//! idiom and risking 0.0.6 on every 0.1 edit). [`crate::cst`] stays frozen:
//! this module imports only its token-`Atom`-generic, non-recursive helpers
//! ([`crate::cst::Header`], [`crate::cst::ParseFileError`]) and re-declares
//! everything else — including its own `*ErasedV1` eraser leaves and its own
//! copy of `render_parse_error` — so that touching `cst_v1.rs` never touches
//! `cst.rs`.
//!
//! **Scope (through Sub-slice 2c).** SATySFi 0.1's grammar adds a whole
//! ML-style module system (`bind`/`modexpr`/`sigexpr`/`decl`) on top of an
//! expr/pattern/type layer that is structurally close to 0.0.6's, plus five
//! surface deltas: `,`-separated lists/records (not `;`), `EXACT_EQ` for
//! both definitional and record `=` (reusing [`DefEqTok`] — no new `=`
//! leaf), mandatory `match … with … end`, per-binding staging instead of a
//! whole-file `@stage:` header, and no `when`/`while`/`before` at all. This
//! module builds:
//!
//! * [`FileV1`] — `header* expr EOI` or `header* module Name
//!   option(sig_annot) = struct bind* end EOI`.
//! * [`Bind`] — every arm of upstream `bind`: `val`/`val inline`/`val
//!   block` (Slice 1), `val rec … and …`/`val mutable`/`type … and …`
//!   (Sub-slice 2b), `module … = modexpr`/`signature … = sigexpr`/`include
//!   modexpr` (Sub-slice 2c).
//! * [`ast::ModExpr`]/[`ast::SigExpr`]/[`ast::Decl`] — the full
//!   module/signature grammar (Sub-slice 2c): functor literals/
//!   application, module paths/aliases, `:>` coercion, `sig … end` with
//!   every `decl` form, `with type` refinement, `include`.
//! * A copy of [`crate::cst::ast`]'s expr/pattern/type layer with the 0.1
//!   deltas applied (see each type's doc comment for the exact delta),
//!   including the full `let rec … and … in`/`let mutable … in` expression
//!   forms and a widened `TypeExpr` grammar (products, prefix application).
//!
//! **Grammar shipped with placeholder semantics only (Sub-slice 2c).** Every
//! module/signature construct beyond Sub-slice 2a's struct-literal
//! `ModExpr::Struct` body PARSES and round-trips, but lowers to a precise
//! `LowerError` (`v1/lower.rs`) rather than real semantics — see that
//! module's doc comment for the placeholder set and the seal rule. Real
//! signature checking is Sub-slice 2d (2e for `include`, 2f for functors).
//!
//! **Deliberately NOT built yet** (post-2c deferrals): staged `val ~x`/`val
//! persistent ~x` and macro binds/decls (phase 5, no `KwPersistent` token
//! yet); `?(l = e)` optional parameter bundles and `( pat : typ )` ascribed
//! params; type-level records/row vars; row quantifiers (`rowquant`, no
//! `ROWVAR` token). `val math` (the math-text/math-boxes split) DOES parse
//! now — see [`Bind::ValueMath`] (math-split spec, L6). Sub-slice 2d-2 lands
//! the other two post-2c grammar gaps: `inline […]`/`block […]` command
//! types (`parser_v1.mly:730-735`; `math […]` stays deferred to the
//! math-split phase — `math` is not a V0_1 keyword, `TypeApp::InlineCmdTy`/
//! `BlockCmdTy`'s doc comment) and `LONG_LOWER` qualified type paths
//! (`:720-728,742-743`; `TypeApp::AppliedLong`/`TypeAtom::LongName`).
//!
//! **The `#[recurse]` SCC story (Sub-slice 2c: five roots).** Five
//! singleton, directly self-referential roots — grown from three in
//! Sub-slice 2b to five in Sub-slice 2c — the same shape [`crate::cst::ast`]
//! uses, for the same reason (see its module doc comment for the measured
//! compile-time blowup a naive transcription hits):
//!
//! * [`ast::Expr`] (its variants' own `Box<Expr>` children);
//! * [`ast::PatBot`] (`CtorApplied`'s `Box<PatBot>` argument);
//! * [`ast::TypeExpr`] (`Fun`'s right-recursive `Box<TypeExpr>` codomain);
//! * [`ast::ModExpr`] (`Functor.body`'s `Box<ModExpr>` self-loop; Sub-slice
//!   2c);
//! * [`ast::SigExpr`] (`Functor.dom`/`Functor.cod`'s `Box<SigExpr>`
//!   self-loop; Sub-slice 2c — encoded left-recursion-safe, `with` is
//!   bot+suffix, never `With { base: Box<SigExpr> }`; see [`ast::SigExpr`]'s
//!   own doc comment).
//!
//! Every other recursion edge is routed through the erasers declared below
//! ([`ExprErasedV1`], [`PatErasedV1`], [`PatBotErasedV1`], [`TyErasedV1`],
//! [`MathErasedV1`], [`ModExprErasedV1`], [`SigExprErasedV1`],
//! [`TypeBindsErasedV1`]), keeping each SCC a singleton and the wrapped
//! grammar's recursion reborrowing one stream type (syan pins it, not us).
//! [`ast::Decl`] is a satellite, not a root: it has no `Box<Self>` anywhere
//! and no type inside the `#[recurse]` module ever names it — it is reached
//! only through the hand-written [`StructDeclV1`] connector (an opaque leaf
//! to the SCC analysis, mirroring [`StructBindV1`]), so `SigExpr ↔ Decl`
//! never forms a rootless static sub-cycle. Full edge-by-edge audit lives in
//! the Sub-slice 2c spec §1.6.

use crate::leaf::*;
use crate::span::Span;
use newer_type::implement;
use syan::parse::{Parse, Unparse};

/// `@require:` / `@import:` header element — byte-identical between 0.0.6
/// and `dev-0-1-0` (this port's own confirmation), so 0.1 simply reuses
/// [`crate::cst`]'s definition rather than re-declaring an identical enum.
/// 0.1 has no `@stage:` header at all (the shared lexer's `V0_1` path
/// rejects it outright — see `lexer.rs`'s `lex_header`), so
/// [`crate::cst::Header`]'s absence of a `Stage` variant costs nothing here.
pub use crate::cst::Header;

/// A 0.1 header element — the UNION of BOTH packaging generations' header
/// forms (Axis B; see the Ld3a spec §4.1 and). `Legacy` is `dev-0-1-0`'s
/// `@require:`/`@import:` (byte-identical to 0.0.6's, reusing [`Header`] —
/// the previous type of `FileV1::*::headers` wholesale); the three `Use*`
/// forms are `saphe-split`'s `headerelem` (`parser.mly:371-380 @ b836d512`).
/// Which family is *legal* is a `LoadMode` question the loader answers
/// ([`rustyfi_loader`]), not a grammar question — this ONE `V0_1` grammar
/// accepts both so the mode error can be raised at load time with a better
/// message than a lex error would give.
///
/// Variant order is parse priority (syan ordered-alternatives, most-specific
/// first): `UsePackage` (the `package` keyword disambiguates) precedes the
/// `of`-suffixed `UseOf`, which precedes bare `Use` (longest-match: `use M of
/// …` must claim its `of` before a bare `use M` matches), which precedes the
/// token-disjoint `Legacy` (`@`-headers lex to distinct tokens).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum HeaderV1 {
    /// `USE PACKAGE optional_open mod_chain` — depend on an installed package
    /// by its consumer-chosen alias (`used_as`). Header attributes
    /// (`#[test-only]` etc., upstream `list(attribute)`) are DEFERRED (Ld3b+).
    UsePackage {
        use_kw: KwUse,
        package_kw: KwPackage,
        open_kw: Option<KwOpen>,
        path: ast::ModChainV1,
    },
    /// `USE optional_open mod_chain OF STRING` — load a local file by
    /// backtick-quoted relative path (the `@import:` analog).
    UseOf {
        use_kw: KwUse,
        open_kw: Option<KwOpen>,
        path: ast::ModChainV1,
        of_kw: KwOf,
        relpath: LiteralTok,
    },
    /// `USE optional_open mod_chain` — sibling module inside the same package
    /// (closed resolution; only legal inside envelope source trees, enforced
    /// by the loader).
    Use {
        use_kw: KwUse,
        open_kw: Option<KwOpen>,
        path: ast::ModChainV1,
    },
    /// `@require:`/`@import:` — Legacy packaging, unchanged shape.
    Legacy(Header),
}

impl HeaderV1 {
    /// A short human-readable name for this header, for loader diagnostics
    /// (e.g. `use package Stdlib`, `@require: foo`).
    pub fn display_name(&self) -> String {
        match self {
            Self::UsePackage { path, .. } => format!("use package {}", path.render()),
            Self::UseOf { path, relpath, .. } => {
                format!("use {} of `{}`", path.render(), relpath.body)
            }
            Self::Use { path, .. } => format!("use {}", path.render()),
            Self::Legacy(Header::Require(t)) => format!("@require: {}", t.content),
            Self::Legacy(Header::Import(t)) => format!("@import: {}", t.content),
            Self::Legacy(Header::Stage(_)) => "@stage:".to_string(),
        }
    }
}

impl ast::ModChainV1 {
    /// The dotted path as source text, e.g. `Stdlib.Logo` or `Local`.
    pub fn render(&self) -> String {
        match self {
            Self::Long(t) => {
                let mut parts = t.mods.clone();
                parts.push(t.name.clone());
                parts.join(".")
            }
            Self::Single(t) => t.name.clone(),
        }
    }

    /// The HEAD component — the module/envelope identifier the loader keys
    /// dependency resolution off (upstream's `used_as` map is keyed by it;
    /// the tail is submodule access, a typecheck-time concern). For `A.B.C`
    /// that is `A`; for a bare `A` it is `A`.
    pub fn head_name(&self) -> String {
        match self {
            Self::Long(t) => t.mods.first().cloned().unwrap_or_else(|| t.name.clone()),
            Self::Single(t) => t.name.clone(),
        }
    }
}

/// A binding-position NAME: `LOWER | ( binop )` — upstream 0.1's
/// `bound_identifier` (`parser_v1.mly:358-363`) is the same nonterminal
/// 0.0.6's `var` folds ([`crate::cst::BindName`]'s doc comment), and the
/// leaf-level parse (`VarTok` | `OpNameTok`) is identical in both
/// generations, so 0.1 reuses the type rather than re-declaring it —
/// another token-generic, non-recursive import like [`Header`] above.
/// Retires the Slice-1 "bound name is a bare `VarTok`" simplification
/// (this module's doc comment).
pub use crate::cst::BindName;

/// A whole 0.1 `.saty`/`.satyh` file (`main`, upstream `parser_v1.mly:364-
/// 368`): a header list followed by either a library (`main_lib`) or a
/// document expression. Unlike 0.0.6's [`crate::cst::File`] (a flat prelude
/// of top-level `let`s with an optional trailing `in body`), 0.1 has no flat
/// top-level binding sequence at all: a document body is *just* an
/// [`ast::Expr`] (every `let` chains its own `in`), and a library is exactly
/// one `module … = struct … end`.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum FileV1 {
    /// `header* expr EOI` (`parser_v1.mly:367`).
    Document {
        headers: Vec<HeaderV1>,
        body: ast::Expr,
        eoi: EoiTok,
    },
    /// `header* MODULE UPPER option(sig_annot) EXACT_EQ STRUCT bind* END
    /// EOI` (`parser_v1.mly:372-375`, `main_lib`; `sig_annot = COERCE
    /// sigexpr`, `:555-557` — Sub-slice 2c). Note 0.1's annotation sigil is
    /// `:>` (COERCE), never 0.0.6's `: sig … end`.
    Library {
        headers: Vec<HeaderV1>,
        module_kw: KwModule,
        name: CtorTok,
        sig_annot: Option<SigAnnotV1>,
        eq: DefEqTok,
        struct_kw: KwStruct,
        binds: Vec<Bind>,
        end_kw: KwEnd,
        eoi: EoiTok,
    },
}

/// `COERCE sigexpr` — a signature annotation `:> S` (`sig_annot`,
/// `parser_v1.mly:555-557`). 0.1's annotation sigil is `:>` (COERCE,
/// `lexer_v1.mll:280`), NOT 0.0.6's `: sig … end` ([`crate::cst::SigAnnot`],
/// `cst.rs:295-303`) — `module M : S = …` is a 0.1 parse error (pinned in
/// tests). The signature body goes through [`SigExprErasedV1`]: `SigAnnotV1`
/// lives outside the `#[recurse]` module, so this is a cross-boundary edge
/// into the `SigExpr` root (see the module doc comment's SCC story).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct SigAnnotV1 {
    pub coerce: CoerceTok,
    pub sig_: SigExprErasedV1,
}

/// One parameter of a [`Bind`] (`param_unit`, `parser_v1.mly:635-646`): an
/// optional `?(l = x, …)` labeled-optional binder bundle, then either a
/// plain `patbot` or a `( pat : τ )` ascribed pattern (optional-arg-rows
/// increment 2 — [`ast::ParamBody::Ascribed`]). Defined INSIDE [`mod@ast`]
/// and re-exported here so that [`ast::Expr::Fun`]/[`ast::Expr::LetIn`]/
/// [`ast::RecClauseV1`] can reference it without a boundary-crossing
/// Parse-trait cycle (the `TypeBindsErasedV1` E0275 hazard).
pub use ast::{AscribedInnerV1, OptParamEntryV1, OptParamsV1, Param, ParamBody};

/// Every arm of `bind` (`parser_v1.mly:415-440`) — upstream's own
/// nonterminal name (Sub-slice 2c retires the prior stopgap `V1`-suffixed
/// name now that this carries the full arm set; helper types like
/// [`StructBindV1`]/[`TypeBindSingleV1`] keep their `V1` suffix). Every
/// value arm's `=` is
/// `EXACT_EQ` ([`DefEqTok`]) and body is an [`ast::Expr`]. `name` is a
/// [`super::BindName`] wherever upstream's `bound_identifier` reaches it
/// (`Value`, and the rec clauses inside [`ast::RecClauseV1`]); `ValueMutable`
/// and `ValueInline`/`ValueBlock`'s `ctx` stay plain [`VarTok`]s — upstream's
/// `MUTABLE LOWER …`/ctx-variable productions are a plain `LOWER`, not
/// `bound_identifier` (see [`super::BindName`]'s doc comment for the
/// ordered-choice-safety argument).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum Bind {
    /// `VAL bind_value_nonrec` (`parser_v1.mly:416,442,459-465`): `val
    /// <name> <param>* = <expr>`.
    Value {
        kw: KwVal,
        name: BindName,
        params: Vec<Param>,
        eq: DefEqTok,
        body: ast::Expr,
    },
    /// `VAL INLINE bind_inline` (`parser_v1.mly:422-431` dispatch → `448` →
    /// `466-491`): `val inline <ctx> \cmd <param>* = <expr>` (the
    /// heavyweight, ctx-explicit form — the only one `stdja-mini` uses;
    /// `ctx` stays `Option` so the lightweight, ctx-synthesized form parses
    /// too, for free).
    ValueInline {
        kw: KwVal,
        inline_kw: KwInline,
        ctx: Option<VarTok>,
        cmd: AnyHorzCmdTok,
        params: Vec<Param>,
        eq: DefEqTok,
        body: ast::Expr,
    },
    /// `VAL BLOCK bind_block` (`parser_v1.mly:450` → `493-518`): `val block
    /// <ctx> +cmd <param>* = <expr>`.
    ValueBlock {
        kw: KwVal,
        block_kw: KwBlock,
        ctx: Option<VarTok>,
        cmd: AnyVertCmdTok,
        params: Vec<Param>,
        eq: DefEqTok,
        body: ast::Expr,
    },
    /// `VAL MATH bind_math` (`parser_v1.mly:452-453` dispatch → `520-531`):
    /// `val math <ctx> \cmd <param>* [with <sub> <sup>] = <expr>`
    /// (math-split spec §4.1). Unlike `ValueInline`/`ValueBlock`, `ctx` is
    /// MANDATORY — upstream has no lightweight ctx-less form (contrast
    /// `bind_inline`'s two productions, :466-491). Placed after
    /// `ValueBlock`, ordered-choice-safe for the same reason as `Value`
    /// above: `math`/`with` both lex as keyword tokens under V0_1, so no
    /// arm can steal another's input.
    ValueMath {
        kw: KwVal,
        math_kw: KwMath,
        ctx: VarTok,
        /// `\cmd` — math commands share the `\` sigil with inline commands
        /// (there is no separate math-command token; see `elaborate.rs`'s
        /// `command_scheme` doc comment, which notes the same sharing on
        /// the eval side).
        cmd: AnyHorzCmdTok,
        params: Vec<Param>,
        scripts: Option<ScriptsParamV1>,
        eq: DefEqTok,
        body: ast::Expr,
    },
    /// `VAL REC bind_value_nonrec (AND bind_value_nonrec)*`
    /// (`parser_v1.mly:444-445,455-465`): `val rec f p* = e (and g p* = e)*`.
    /// With `rec`/`mutable`/`inline`/`block` all lexed as keyword tokens
    /// under V0_1 (§2 of the sub-slice 2b spec), no arm can steal another's
    /// input — `Value.name: BindName` cannot match a keyword token — so
    /// declared order is a documentation/perf choice; `Value` stays first
    /// because it is the overwhelmingly common arm.
    ValueRec {
        kw: KwVal,
        rec_kw: KwRec,
        first: ast::RecClauseV1,
        ands: Vec<ast::AndClauseV1>,
    },
    /// `VAL MUTABLE LOWER REVERSED_ARROW expr` (`parser_v1.mly:446-447`):
    /// `val mutable x <- e`. The name is a plain `LOWER` upstream (not
    /// `bound_identifier`), hence `VarTok`, matching the cst target
    /// (`cst::TopBinding::LetMutable.name`, `cst.rs:237`).
    ValueMutable {
        kw: KwVal,
        mutable_kw: KwMutable,
        name: VarTok,
        arrow: OverwriteEqTok,
        value: ast::Expr,
    },
    /// `TYPE bind_type_single (AND bind_type_single)*`
    /// (`parser_v1.mly:432-433,535-544`): `type t 'a* = body (and u 'a* =
    /// body)*` — variant and synonym forms, mutually recursive across the
    /// `and` chain.
    Type {
        kw: KwType,
        first: TypeBindSingleV1,
        ands: Vec<TypeAndV1>,
    },
    /// `MODULE UPPER option(sig_annot) EXACT_EQ modexpr` — upstream
    /// `bind`'s MODULE arm (`parser_v1.mly:434-435`), with the FULL
    /// `modexpr` body and the optional `:>` annotation (Sub-slice 2c;
    /// retires 2a's struct-literal-only restriction). The body goes
    /// through [`ModExprErasedV1`]: `Bind` is outside the `#[recurse]`
    /// module, and `Bind → ModExpr → StructBindV1 → Bind` is the runtime
    /// cycle both connectors erase (one break per direction).
    Module {
        module_kw: KwModule,
        name: CtorTok,
        sig_annot: Option<SigAnnotV1>,
        eq: DefEqTok,
        body: ModExprErasedV1,
    },
    /// `SIGNATURE UPPER EXACT_EQ sigexpr` (`parser_v1.mly:436-437`).
    Signature {
        kw: KwSignature,
        name: CtorTok,
        eq: DefEqTok,
        sig_: SigExprErasedV1,
    },
    /// `INCLUDE modexpr` (`:438-439`) — a bind-include includes a MODULE
    /// (contrast [`ast::Decl::Include`], which includes a signature).
    Include { kw: KwInclude, body: ModExprErasedV1 },
}

/// `scripts_param` (`parser_v1.mly:532-534`): `WITH sub=LOWER sup=LOWER` —
/// `val math`'s optional `with sub sup` suffix (math-split spec §4.1),
/// binding the two script-callback parameters directly rather than
/// synthesizing the hidden `%math-attach-scripts` wrapper (§4.2/§4.3).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct ScriptsParamV1 {
    pub with_kw: KwWith,
    pub sub: VarTok,
    pub sup: VarTok,
}

/// One `bind_type_single` (`parser_v1.mly:539-544`). **0.1 delta from
/// [`crate::cst::TypeDecl`]:** the type parameters come AFTER the name
/// (`type t 'a = …`, `tyident LOWER; tyvars list(TYPEVAR)`), where 0.0.6
/// writes them before (`type 'a t = …`, `cst.rs:401-408`) — the lowering
/// reorders the fields. No `constraint` suffix exists in 0.1's production.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TypeBindSingleV1 {
    pub name: VarTok,
    pub tyvars: Vec<TypeVarTok>,
    pub eq: DefEqTok,
    pub body: TypeBodyV1,
}

/// An `and bind_type_single` continuation (`bind_type`'s
/// `separated_nonempty_list(AND, …)`, `parser_v1.mly:535-537`).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TypeAndV1 {
    pub and_kw: KwAnd,
    pub bind: TypeBindSingleV1,
}

/// One whole `bind_type` chain — `bind_type_single (AND bind_type_single)*`
/// (`parser_v1.mly:535-537`) — grouped into a single struct so the sig
/// layer ([`ast::SigExpr::WithType`], [`ast::Decl::Type`]) can reference the
/// chain through ONE eraser ([`TypeBindsErasedV1`]). [`Bind::Type`] keeps
/// its flattened `first`/`ands` fields unchanged (avoiding 2b call-site
/// churn); the two spellings are the same grammar.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TypeBindsV1 {
    pub first: TypeBindSingleV1,
    pub ands: Vec<TypeAndV1>,
}

/// The right-hand side of one type bind: a variant's constructor list
/// (`EXACT_EQ BAR? variants`, `parser_v1.mly:540-541,545-553`) or a
/// transparent synonym (`EXACT_EQ typ`, `:542-543`). Variant-first is
/// unambiguous for the same reason as [`crate::cst::TypeDeclBody`]
/// (`cst.rs:410-418`): a variant list is `BarTok`/`CtorTok`-headed and no
/// [`ast::TypeExpr`] can start with either.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum TypeBodyV1 {
    Variant {
        leading_bar: Option<BarTok>,
        first: VariantDefV1,
        rest: Vec<BarVariantDefV1>,
    },
    Synonym(ast::TypeExpr),
}

/// One `UPPER [OF typ]` variant (`parser_v1.mly:549-553`).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct VariantDefV1 {
    pub ctor: CtorTok,
    pub of_ty: Option<OfTypeV1>,
}

/// The `of typ` payload suffix.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct OfTypeV1 {
    pub of_kw: KwOf,
    pub ty: ast::TypeExpr,
}

/// A `| UPPER [OF typ]` continuation (`variants`' `separated_nonempty_
/// list(BAR, variant)`, `parser_v1.mly:545-548`).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct BarVariantDefV1 {
    pub bar: BarTok,
    pub def: VariantDefV1,
}

/// One declaration inside a `module … = struct … end` body
/// (Sub-slice 2a). [`Bind`]'s own alternatives are exactly what a struct
/// body may contain (`bind*`), so this simply re-parses a [`Bind`] — but
/// *not* by naming `Bind` as a field type directly: [`Bind`] lives
/// **outside** the `#[recurse]` module (below), so `Bind -> ModExpr ->
/// Vec<StructBindV1> -> Bind` would be a self-recursive cycle through a
/// plain `#[derive(Parse)]`, which (without the `#[recurse]` engine to back
/// it) is an `E0275` hazard (an unbounded recursive trait-bound
/// obligation) — exactly [`crate::cst::StructDecl`]'s own rationale
/// (`cst.rs:262-269`). Hand-writing `Parse`/`Unparse` here — the same trick
/// as the `erased_leaf_v1!` macro below — sidesteps that: the impl has no
/// recursive where-bound for the compiler to try to satisfy, it just calls
/// `Bind::parse` through the stream-erasing adapter at runtime. A
/// byte-for-byte analogue of [`crate::cst::StructDecl`] (`cst.rs:270-293`).
#[derive(Debug, Clone, PartialEq)]
pub struct StructBindV1(pub Box<Bind>);

impl Parse<crate::token::Atom> for StructBindV1 {
    type Error = syan::error::ParseError<crate::span::Span>;

    fn parse_stream<S: syan::parse::ParseStream<Atom = crate::token::Atom>>(
        stream: &mut S,
    ) -> Result<Self, Self::Error> {
        let value = <Bind as Parse<_>>::parse_stream(stream)?;
        Ok(StructBindV1(Box::new(value)))
    }
}

impl Unparse<crate::token::Atom> for StructBindV1 {
    fn unparse<S: syan::parse::unparse::Emitter<crate::token::Atom>>(
        &self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        self.0.unparse(sink)
    }
}

/// One declaration inside a `sig … end` body (`list(decl)`,
/// `parser_v1.mly:591`). [`ast::Decl`] lives INSIDE the `#[recurse]` module
/// and `SigExpr → SigBotV1 → StructDeclV1 → Decl → SigExpr` is a runtime
/// cycle; naming `ast::Decl` as a plain derived field of [`ast::SigBotV1`]
/// would re-enter the module's own SCC analysis. Hand-writing
/// `Parse`/`Unparse` here — the [`crate::cst::StructDecl`] trick
/// (`cst.rs:262-269`, the `E0275` rationale) — makes this an opaque leaf:
/// the impl has no recursive where-bound, it just calls `Decl::parse`
/// through the stream-erasing adapter at runtime, closing the
/// `SigExpr`↔`Decl` cycle at RUNTIME while keeping both SCCs singletons.
/// NOTE: deliberately named after [`crate::cst::StructDecl`] (the
/// mechanism), even though it carries a sig-`decl`, not a struct binding —
/// the struct-side twin is [`StructBindV1`].
#[derive(Debug, Clone, PartialEq)]
pub struct StructDeclV1(pub Box<ast::Decl>);

impl Parse<crate::token::Atom> for StructDeclV1 {
    type Error = syan::error::ParseError<crate::span::Span>;

    fn parse_stream<S: syan::parse::ParseStream<Atom = crate::token::Atom>>(
        stream: &mut S,
    ) -> Result<Self, Self::Error> {
        let value = <ast::Decl as Parse<_>>::parse_stream(stream)?;
        Ok(StructDeclV1(Box::new(value)))
    }
}

impl Unparse<crate::token::Atom> for StructDeclV1 {
    fn unparse<S: syan::parse::unparse::Emitter<crate::token::Atom>>(
        &self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        self.0.unparse(sink)
    }
}

/// Recursion-edge eraser types for the Slice-1 expr/pattern/type layer —
/// the `cst_v1` analogue of [`crate::cst`]'s `erased_leaf!` macro (see its
/// doc comment for the measured compile-time blowup that makes this
/// mandatory). Suffixed `V1` throughout so these never collide with
/// [`crate::cst`]'s own erasers, even though the two live in sibling
/// modules and could not actually name-clash. Defined *outside* the
/// `#[recurse]` module so the macro treats them as opaque leaves.
macro_rules! erased_leaf_v1 {
    ($($(#[$doc:meta])* $name:ident => $target:ty;)*) => {
        $(
            $(#[$doc])*
            #[implement(newer_type_std::ops::Deref)]
            #[derive(Debug, Clone, PartialEq)]
            pub struct $name(pub Box<$target>);

            impl Parse<crate::token::Atom> for $name {
                type Error = syan::error::ParseError<crate::span::Span>;

                fn parse_stream<S: syan::parse::ParseStream<Atom = crate::token::Atom>>(
                    stream: &mut S,
                ) -> Result<Self, Self::Error> {
                    // No erasure any more — see `cst.rs`'s `erased_leaf!`.
                    let value = <$target as Parse<_>>::parse_stream(stream)?;
                    Ok($name(Box::new(value)))
                }
            }

            impl Unparse<crate::token::Atom> for $name {
                fn unparse<S: syan::parse::unparse::Emitter<crate::token::Atom>>(
                    &self,
                    sink: &mut S,
                ) -> Result<(), S::Error> {
                    self.0.unparse(sink)
                }
            }
        )*
    };
}

erased_leaf_v1! {
    /// An [`ast::Expr`] behind a stream-erasing parse (see above).
    ExprErasedV1 => ast::Expr;
    /// An [`ast::Pattern`] behind a stream-erasing parse (see above).
    PatErasedV1 => ast::Pattern;
    /// An [`ast::PatBot`] behind a stream-erasing parse (see above). Kept
    /// separate from [`PatErasedV1`] for the same reason
    /// [`crate::cst`]'s `PatErased`/`PatBotErased` split exists: a
    /// constructor pattern's argument is a `patbot`, not a full `patas`.
    PatBotErasedV1 => ast::PatBot;
    /// An [`ast::TypeExpr`] behind a stream-erasing parse (see above).
    TyErasedV1 => ast::TypeExpr;
    /// An [`ast::MathElemCst`] behind a stream-erasing parse (see above).
    MathErasedV1 => ast::MathElemCst;
    /// An [`ast::ModExpr`] behind a stream-erasing parse. Carries the
    /// OUTSIDE→INSIDE edge `Bind::Module.body → ModExpr` — the one edge of
    /// the `Bind → ModExpr → StructBindV1 → Bind` runtime cycle not already
    /// erased by the connector (the erasers are for the INSIDE types, not
    /// for `Bind`).
    ModExprErasedV1 => ast::ModExpr;
    /// An [`ast::SigExpr`] behind a stream-erasing parse. Used by
    /// [`SigAnnotV1`] and [`Bind::Signature`] (outside → the SigExpr root).
    SigExprErasedV1 => ast::SigExpr;
    /// A [`TypeBindsV1`] behind a stream-erasing parse. Unlike every other
    /// eraser this one targets an OUTSIDE type: `SigExpr::WithType` /
    /// `Decl::Type` (inside) must reach `bind_type`, whose
    /// `TypeBindSingleV1` re-enters the module through plain-derived
    /// `ast::TypeExpr` fields — an inside→outside-plain-derive→inside-root
    /// chain with no precedent in `cst.rs`'s discipline. Erasing at the
    /// boundary keeps the re-entry cheap (one stream type, monomorphized
    /// once), exactly like every other cross-boundary edge.
    TypeBindsErasedV1 => TypeBindsV1;
}

/// The recursive expression/pattern/type/text grammar for SATySFi 0.1,
/// Slice 1. A copy of [`crate::cst::ast`] with the deltas documented on each
/// type; see the module doc comment for the SCC/root story.
#[syan::parse::recurse]
pub mod ast {
    use crate::leaf::*;
    use syan::parse::{Parse, Unparse};

    /// One `param_unit` (`parser_v1.mly:635-646`): an optional `?(l = x, …)`
    /// labeled-optional binder bundle, then a [`ParamBody`] (a plain
    /// `patbot`, or a `( pat : τ )` ascribed pattern). Held inside `mod ast`
    /// (re-exported at [`super::Param`]) so the roots that carry param lists
    /// (`Expr::Fun`/`Expr::LetIn`/`RecClauseV1`) reference it without a
    /// boundary Parse-trait cycle. A bare `patbot` param parses `Param {
    /// opts: None, body: ParamBody::Pat(_) }` directly — the `?`-headed
    /// `opts` `Option` is tried first, failing on a non-`?` head with no
    /// token stolen, so every existing all-plain fixture parses
    /// byte-identically.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct Param {
        pub opts: Option<OptParamsV1>,
        pub body: ParamBody,
    }

    /// A `param_unit`'s trailing shape (`parser_v1.mly:635-646`): either a
    /// plain `patbot`, or a `( pattern : typ )` ascribed pattern
    /// (optional-arg-rows increment 2; `parser_v1.mly:641-645`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum ParamBody {
        /// Tried FIRST — preserves today's parse for every existing fixture:
        /// `( x : int )` fails `patbot`'s own paren body at the `:` (a
        /// `patbot` paren group expects only more patterns/`,`/`)`) and
        /// backtracks here cleanly (ordered choice, no token stolen).
        Pat(PatBot),
        /// `( pattern : typ )` — a FULL `pattern` (not `patbot`) ascribed
        /// with a full `typ`, both via erasers (same cycle-avoidance
        /// discipline as every other satellite in this module).
        Ascribed {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: AscribedInnerV1,
        },
    }

    /// An ascribed param's group content: `pattern : typ` (`parser_v1.mly`'s
    /// `param_unit`, `:641-645`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AscribedInnerV1 {
        pub pat: super::PatErasedV1,
        pub colon: ColonTok,
        pub ty: super::TyErasedV1,
    }

    /// A `?(l = x, …)` labeled-optional parameter bundle (`parser_v1.mly`'s
    /// optional-`param_unit` head). The `?` reuses [`OptionalTypeTok`]
    /// (SATySFi 0.1 dropped the fused `?:` sigil); the `(…)` is a paren
    /// group of `,`-separated `label = binder` entries (non-empty enforced
    /// at lowering).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OptParamsV1 {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub entries: Vec<OptParamEntryV1>,
    }

    /// One `label = binder` entry of an [`OptParamsV1`] bundle (the last `,`
    /// is optional; `=` is upstream's `EXACT_EQ`, reusing [`DefEqTok`]).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OptParamEntryV1 {
        pub label: VarTok,
        pub eq: DefEqTok,
        pub var: VarTok,
        pub comma: Option<CommaTok>,
    }

    /// `nxlet`-analogue: a let/if/match/lambda-headed expression, falling
    /// through to the flattened operator chain ([`Ops`], [`OpChain`]) at the
    /// bottom. Variant order is parse priority (ordered-choice
    /// backtracking): every `let`-headed form is tried before the fallback
    /// [`Expr::Overwrite`]/[`Expr::Ops`] (which may also start with a bare
    /// variable), and `Ops` — having no distinguishing leading keyword —
    /// must stay last.
    ///
    /// **0.1 deltas from [`crate::cst::ast::Expr`]:** `Match` gains a
    /// mandatory trailing `end` (`parser_v1.mly:792`); `let-rec` becomes
    /// `let rec … in …` (a plain `let` followed by the new [`KwRec`]
    /// keyword) with full `and`-chained mutual recursion (Sub-slice 2b —
    /// Slice 1's single-clause restriction is retired, see
    /// [`Expr::LetRecIn`]); a new `LetMutableIn` form covers `let mutable x
    /// <- init in body` (Sub-slice 2b); a new `LetPatternIn` form covers
    /// `let pat = value in body` for any non-bare-variable pattern
    /// (`parser_v1.mly:796`, `pattern_non_var`); `open` requires a leading
    /// `let` (`parser_v1.mly:798`, `LET OPEN UPPER IN`) where 0.0.6 allows a
    /// bare `open Name in body`; and `WhileDo`, the `Guard`/`when` match-arm
    /// suffix, and `OpChain`'s `before` postfix are dropped entirely —
    /// SATySFi 0.1's grammar has no `WHEN`/`WHILE`/`BEFORE` tokens at all
    /// (confirmed by grep of `parser_v1.mly`). `Overwrite` (`name <- value`)
    /// is kept unchanged (`parser_v1.mly:810-812`, `REVERSED_ARROW`) — it is
    /// unrelated to the removed `before` postfix.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Expr {
        /// `let rec clause (and clause)* in body` (`parser_v1.mly:794-795`
        /// dispatching to `bind_value_rec`, `:455-458`) — full mutual
        /// recursion; Slice 1's single-clause restriction is retired
        /// (Sub-slice 2b).
        LetRecIn {
            let_kw: KwLet,
            rec_kw: KwRec,
            first: RecClauseV1,
            ands: Vec<AndClauseV1>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let mutable x <- init in body` (`parser_v1.mly:794-795`
        /// dispatching to `bind_value`'s MUTABLE arm, `:446-447`). Same
        /// shape as [`crate::cst::ast::Expr::LetMutableIn`] minus the fused
        /// keyword: 0.1 spells it `let mutable` (two tokens), 0.0.6
        /// `let-mutable` (one). Disambiguated one token after `let` by the
        /// V0_1-gated `mutable` keyword, so declared order relative to the
        /// other `let`-headed arms is correctness-irrelevant.
        LetMutableIn {
            let_kw: KwLet,
            mutable_kw: KwMutable,
            name: VarTok,
            arrow: OverwriteEqTok,
            init: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let name param* = value in body` (only a plain variable target
        /// is supported here — a general pattern falls through to
        /// [`Expr::LetPatternIn`]). `name` is a [`super::BindName`]
        /// (Sub-slice 2b): upstream's expression-level `let` reaches the
        /// same `bind_value_nonrec` (`:794-795` → `:459-465`) `val`/`val
        /// rec` do, so `let (+++) a b = … in` is valid 0.1 here too.
        LetIn {
            kw: KwLet,
            name: super::BindName,
            params: Vec<Param>,
            eq: DefEqTok,
            value: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let pat = value in body` (`parser_v1.mly:796`,
        /// `pattern_non_var`) — any pattern shape, tried only after the
        /// bare-variable form [`Expr::LetIn`] fails to match.
        LetPatternIn {
            kw: KwLet,
            pat: super::PatErasedV1,
            eq: DefEqTok,
            value: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let open Name in body` (`parser_v1.mly:798`; unlike 0.0.6's
        /// bare `open Name in body`, 0.1 requires the leading `let`).
        OpenIn {
            let_kw: KwLet,
            open_kw: KwOpen,
            name: CtorTok,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `if cond then a else b` (`else` is never optional, so there is
        /// no dangling-else ambiguity).
        If {
            kw: KwIf,
            cond: Box<Expr>,
            then_kw: KwThen,
            then_branch: Box<Expr>,
            else_kw: KwElse,
            else_branch: Box<Expr>,
        },
        /// `fun x y -> body`. **Delta from Slice 1:** `params` widened from
        /// `Vec<VarTok>` to `Vec<PatBot>` (gaps 2+3 of the V0_1-only
        /// language-completeness sweep) — upstream `parser_v1.mly:849-863`'s
        /// `fun` genuinely binds a `patbot` per parameter (`ELambda(patbot,
        /// e)` in `types.cppo.ml`), not a bare variable, so `fun _ -> …`
        /// (wildcard) and `fun (a, b) -> …` (tuple-destructuring) are legal
        /// upstream syntax this port was rejecting. Same cross-root DAG edge
        /// [`RecClauseV1::params`] already makes (both `Expr` and `PatBot`
        /// are roots inside this `#[recurse]` module — see the module doc
        /// comment) — no new SCC edge, `PatBot` was already reachable from
        /// `Expr` via [`super::PatErasedV1`] elsewhere.
        Fun {
            kw: KwFun,
            params: Vec<Param>,
            arrow: ArrowTok,
            body: Box<Expr>,
        },
        /// `match scrutinee with [|] pat -> body (| pat -> body)* end`
        /// (`parser_v1.mly:792`). Mandatorily closed with `end`
        /// (`tokR=END`); no `when` guards (0.1 has no `WHEN` token).
        Match {
            kw: KwMatch,
            scrutinee: Box<Expr>,
            with_kw: KwWith,
            leading_bar: Option<BarTok>,
            first: MatchArm,
            rest: Vec<BarArm>,
            end_kw: KwEnd,
        },
        /// `name <- value` (`expr_overwrite`, `parser_v1.mly:810-812`,
        /// `REVERSED_ARROW`). Starts with a bare [`VarTok`], which is also
        /// how [`Expr::Ops`] can start — must stay before `Ops` so
        /// backtracking tries the `<-` shape first.
        Overwrite {
            name: VarTok,
            arrow: OverwriteEqTok,
            value: super::ExprErasedV1,
        },
        /// The flattened binary-operator chain — see
        /// [`crate::cst::ast::Expr`]'s module doc comment on precedence
        /// flattening (unchanged approach here). Must stay last (no leading
        /// keyword).
        Ops(OpChain),
    }

    /// One `name param* = value` clause of a `val rec`/`let rec` group —
    /// upstream `bind_value_nonrec` (`parser_v1.mly:459-465`) as reached
    /// from `bind_value_rec` (`:455-458`). **0.1 deltas from
    /// [`crate::cst::ast::RecBinding`]:** no `: ty` ascription and no
    /// multi-clause `| patbot* = value` sugar exist in 0.1 at all
    /// (`bind_value_nonrec` has neither a `COLON ty` nor a `BAR`
    /// alternative — 0.0.6's `recdecargpart` machinery has no 0.1
    /// counterpart), so there are no `ascription`/`leading_bar`/`extra`
    /// fields to mirror. `params` is upstream's `list(param_unit)`
    /// restricted to the Slice-1 plain-pattern subset (see [`super::Param`]'s
    /// doc comment) — held as `Vec<PatBot>` directly, the same proven
    /// cross-root DAG edge `cst::ast::RecBinding.params` makes
    /// (`cst.rs:753-762`); `value` goes through [`super::ExprErasedV1`] (not
    /// `Box<Expr>`) so this struct never joins `Expr`'s SCC — byte-for-byte
    /// `cst.rs`'s own `RecBinding.value: ExprErased` discipline.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecClauseV1 {
        pub name: super::BindName,
        pub params: Vec<Param>,
        pub eq: DefEqTok,
        pub value: super::ExprErasedV1,
    }

    /// An `and name param* = value` continuation of a `val rec`/`let rec`
    /// group (`bind_value_rec`'s `separated_nonempty_list(AND, …)`,
    /// `parser_v1.mly:455-458`). `and` lexes as `Token::LetAnd` → [`KwAnd`]
    /// in both generations (`lexer.rs:141`), so no new token is needed.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AndClauseV1 {
        pub and_kw: KwAnd,
        pub clause: RecClauseV1,
    }

    /// One `pat -> body` match arm (`parser_v1.mly:959`). Unlike
    /// [`crate::cst::ast::MatchArm`], has no `when` guard — 0.1's grammar
    /// has none.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct MatchArm {
        pub pat: super::PatErasedV1,
        pub arrow: ArrowTok,
        pub body: super::ExprErasedV1,
    }

    /// A `| pat -> body` continuation of a match's arm list.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct BarArm {
        pub bar: BarTok,
        pub arm: MatchArm,
    }

    /// A flattened binary-operator chain: `head (op rhs)*`, left-folded
    /// (with correct per-operator precedence/associativity) during
    /// elaboration — see [`crate::cst::ast::OpChain`]'s doc comment.
    /// **Delta:** no `before` postfix field — 0.1 has no `BEFORE` token at
    /// all (confirmed by grep of `parser_v1.mly`), so
    /// [`crate::cst::ast::OpChain::before`] simply has no 0.1 counterpart.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OpChain {
        pub head: AppExpr,
        pub tail: Vec<OpRhs>,
    }

    /// One `op rhs` continuation of an [`OpChain`].
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OpRhs {
        pub op: BinOpTok,
        pub rhs: AppExpr,
    }

    /// `nxun`/`nxapp`/`nxunsub`-analogue flattened: an optional leading
    /// unary minus, an optional leading `!`/`!!`/... deref, an atomic head
    /// with any `#label` field accesses, and an application-chain tail —
    /// structurally identical to [`crate::cst::ast::AppExpr`] (`expr_app`,
    /// `parser_v1.mly:849-863`, no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AppExpr {
        pub minus: Option<ExactMinusTok>,
        pub excl: Option<UnopExclamTok>,
        pub head: Atomic,
        pub head_accesses: Vec<AccessSeg>,
        pub args: Vec<AppArg>,
    }

    /// One `#label` field-access segment (`expr_bot ACCESS`,
    /// `parser_v1.mly:878`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AccessSeg {
        pub hash: AccessTok,
        pub label: VarTok,
    }

    /// One application-chain argument. **0.1 delta:** the 0.0.6 `?:`/`?*`
    /// (`Optional`/`Omission`) forms are gone — SATySFi 0.1 dropped the fused
    /// `?:` sigil (`?:`/`?*` now lex as `?` + `:`/`*`, a downstream parse
    /// error), replaced by the labeled `?(l = e, …)` bundle
    /// ([`AppArg::Bundled`]).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum AppArg {
        /// `?(l = e, …) atom` — a labeled-optional bundle paired with the
        /// positional argument it precedes (pairing them rejects a dangling
        /// trailing bundle at parse time). `?(`-headed, token-disjoint from
        /// the `Atom`/`Ctor` arms.
        Bundled {
            opts: OptArgsV1,
            excl: Option<UnopExclamTok>,
            atom: Atomic,
            accesses: Vec<AccessSeg>,
        },
        /// `?(l = e, …) Ctor` — as [`AppArg::Bundled`] but the positional
        /// argument is a bare constructor.
        BundledCtor { opts: OptArgsV1, ctor: CtorTok },
        Atom {
            excl: Option<UnopExclamTok>,
            atom: Atomic,
            accesses: Vec<AccessSeg>,
        },
        Ctor(CtorTok),
    }

    /// A `?(l = e, …)` labeled-optional application bundle: the `?` sigil
    /// (reusing [`OptionalTypeTok`]), then a paren group of `,`-separated
    /// `label = expr` entries (non-empty enforced at lowering).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OptArgsV1 {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub entries: Vec<OptArgEntryV1>,
    }

    /// One `label = expr` entry of an [`OptArgsV1`] bundle — a FULL
    /// expression (`?(bias = 1 + n)`), routed through [`super::ExprErasedV1`]
    /// so this satellite never joins `Expr`'s SCC.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OptArgEntryV1 {
        pub label: VarTok,
        pub eq: DefEqTok,
        pub value: super::ExprErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// `expr_bot`-analogue: an atomic expression. **Delta:** [`Atomic::List`]
    /// and [`Atomic::Record`] are now `,`-separated
    /// (`optterm_list(COMMA, …)`, `parser_v1.mly:935,942`) rather than
    /// 0.0.6's `;`-separated forms — see [`ListItem`]/[`RecordField`].
    /// Parenthesized/tuple bodies were already `,`-separated in 0.0.6 and
    /// are unchanged (`parser_v1.mly:914`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Atomic {
        Length(LengthTok),
        Float(FloatTok),
        Int(IntTok),
        Literal(LiteralTok),
        True(KwTrue),
        False(KwFalse),
        /// A bare constructor, e.g. `None`, or the head of `Some 1`.
        Ctor(CtorTok),
        Var(VarTok),
        /// `Mod.x` — a module-qualified variable.
        VarWithMod(VarWithModTok),
        /// `command \cmd` (upstream `parser_v1.mly:906`, `L_PAREN COMMAND
        /// backslash_cmd R_PAREN` — the parens arrive via [`Atomic::Paren`]
        /// here, exactly like [`crate::cst::ast::Atomic::Command`], which
        /// this reproduces verbatim; the `plus_cmd` alternative at :908 is
        /// deferred with the same rationale as the 0.0.6 comment). Needed by
        /// the transliterated `v01-mini.satyh`'s `(command \math)`.
        Command { kw: CommandTok, name: AnyHorzCmdTok },
        /// `()`
        Unit { paren: UnitParen },
        /// `( expr )` or `( expr, expr, … )` (the latter elaborates to a
        /// tuple).
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: Box<ParenBody>,
        },
        /// `(| label = expr, … |)` or `(| base with label = expr, … |)`
        /// (`,`-separated — see [`RecordBody`]).
        Record {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            body: RecordBody,
        },
        /// `[ expr, … ]` (`,`-separated — see [`ListItem`]).
        List {
            list: ListGroup<()>,
            #[group(self.list)]
            items: Vec<ListItem>,
        },
        /// `{ inline text }`
        InlineText {
            igrp: InlineGroup<()>,
            #[group(self.igrp)]
            elems: Vec<InlineElem>,
        },
        /// `'< block text >`
        BlockText {
            bgrp: BlockGroup<()>,
            #[group(self.bgrp)]
            elems: Vec<BlockElem>,
        },
        /// `${ math }`. Parses the same math grammar as 0.0.6; the
        /// `math-text`/`math-boxes` value split is a lowering/typing
        /// concern, not a cst_v1 shape change.
        MathText {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErasedV1>,
        },
    }

    /// `(| … |)`'s content: either a plain field list, or a *record
    /// update* `base with l = e, …` (`parser_v1.mly:942-957`). `Update` is
    /// tried first (backtracks cleanly to `Fields`, same rationale as
    /// [`crate::cst::ast::RecordBody`]).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum RecordBody {
        Update {
            base: super::ExprErasedV1,
            with_kw: KwWith,
            fields: Vec<RecordField>,
        },
        Fields(Vec<RecordField>),
    }

    /// The parenthesized-expression group's content: one expression, plus
    /// any `, expr` continuations (present only for a tuple) — unchanged
    /// from 0.0.6 (already `,`-separated, `parser_v1.mly:914`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct ParenBody {
        pub first: super::ExprErasedV1,
        pub rest: Vec<CommaExpr>,
    }

    /// A `, expr` continuation inside a parenthesized tuple.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CommaExpr {
        pub comma: CommaTok,
        pub value: super::ExprErasedV1,
    }

    /// One record field `label = expr,` (the last `,` is optional; `=` is
    /// upstream's `EXACT_EQ`, reusing [`DefEqTok`]). **Delta from
    /// [`crate::cst::ast::RecordField`]:** `,` separator, not `;`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecordField {
        pub name: VarTok,
        pub eq: DefEqTok,
        pub value: super::ExprErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// One list element `expr,` (the last `,` is optional). **Delta from
    /// [`crate::cst::ast::ListItem`]:** `,` separator, not `;`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct ListItem {
        pub value: super::ExprErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// One inline-text element — identical shape to
    /// [`crate::cst::ast::InlineElem`] (no 0.1 delta; text-mode content is
    /// untouched by the comma/`end` deltas).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum InlineElem {
        Char(CharTok),
        /// A backtick literal written inside inline text (`` `…` ``). Its own
        /// arm, not a `Char` run, so the elaborator can dispatch it through the
        /// context's code-text command — see `Token::CodeText`.
        CodeText(CodeTextTok),
        Space(SpaceTok),
        Break(BreakTok),
        /// `#var;` — embeds a program variable's value as inline content.
        Embed { var: VarInHorzTok, semi: EndActiveTok },
        /// `${ math }` — embeds math content as inline text.
        EmbedMath {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErasedV1>,
        },
        /// `\cmd …` (`name` also accepts the module-qualified `\Mod.cmd`
        /// form).
        Cmd { name: AnyHorzCmdTok, tail: CmdTail },
        /// An itemize bullet (`*`+) marker.
        ItemBullet(ItemTok),
        /// A `|` separator marker.
        Sep(SepTok),
    }

    /// One block-text element — identical shape to
    /// [`crate::cst::ast::BlockElem`] (no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum BlockElem {
        /// `#var;` — embeds a program variable's value as block content.
        Embed { var: VarInVertTok, semi: EndActiveTok },
        /// `+cmd …` (`name` also accepts the module-qualified `+Mod.cmd`
        /// form).
        Cmd { name: AnyVertCmdTok, tail: CmdTail },
    }

    /// A command's arguments — identical shape to
    /// [`crate::cst::ast::CmdTail`] — **0.1 delta:** an optional LEADING
    /// `?(l = e, …)` bundle (optional-arg-rows increment 3b-β). A command
    /// applied with an optional on its FIRST argument (`\cmd ?(l = e){arg}`,
    /// `+sec ?(label = t){title}<body>` — the ONLY shape the capstone census
    /// finds) can't ride inside `args` (an `expr_app` application chain whose
    /// *head* must be a bare `Atomic`, never a `?`-headed bundle — the head
    /// slot has no place for a leading bundle), so it is peeled off here as
    /// `lead_opts` and re-attached to the first argument at lowering
    /// (`v1::lower::lower_cmd_tail`). A bundle on a LATER argument
    /// (`\cmd{a} ?(l = e){b}`) still rides inside `args` as an ordinary
    /// [`AppArg::Bundled`] (inc1). `?(`-headed, token-disjoint from every
    /// `args` head shape, so a bundle-less tail parses `lead_opts: None`
    /// exactly as before.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum CmdTail {
        /// `;` — no arguments.
        Semi(EndActiveTok),
        /// The argument chain, optionally prefixed by a leading `?(l = e, …)`
        /// bundle on the first argument.
        Args {
            lead_opts: Option<OptArgsV1>,
            args: super::ExprErasedV1,
            semi: Option<EndActiveTok>,
        },
    }

    /// `patas`-analogue: a pattern, plus an optional `as name` binding —
    /// identical shape to [`crate::cst::ast::Pattern`] (no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct Pattern {
        pub head: PatCons,
        pub as_clause: Option<AsClause>,
    }

    /// The `as name` suffix of a pattern.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AsClause {
        pub as_kw: KwAs,
        pub name: VarTok,
    }

    /// `pattr`-analogue: a `patbot`, followed by any number of `:: patbot`
    /// segments — identical shape to [`crate::cst::ast::PatCons`] (see its
    /// doc comment for why this is a flattened `Vec` rather than right
    /// recursion; no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct PatCons {
        pub head: PatBot,
        pub tail: Vec<ConsSeg>,
    }

    /// One `:: patbot` continuation of a cons pattern.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct ConsSeg {
        pub cons: ConsTok,
        pub tail: PatBot,
    }

    /// `patbot`, plus the constructor-pattern forms `pattr` adds —
    /// identical shape to [`crate::cst::ast::PatBot`], except
    /// [`PatBot::List`] is now `,`-separated (`parser_v1.mly:990-1015`,
    /// comma-sep list/tuple patterns per the S5 spec's acceptance table).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum PatBot {
        /// `Ctor patbot` — a constructor applied to one argument pattern.
        /// This field is `PatBot`'s own self-loop (the root SCC).
        CtorApplied { ctor: CtorTok, arg: Box<PatBot> },
        /// A bare (nullary) constructor pattern.
        Ctor(CtorTok),
        Int(IntTok),
        True(KwTrue),
        False(KwFalse),
        Str(LiteralTok),
        Wild(WildcardTok),
        Var(VarTok),
        /// `()`
        Unit { paren: UnitParen },
        /// `( pat )` or `( pat, pat, … )` (the latter elaborates to a tuple
        /// pattern; already `,`-separated in 0.0.6, unchanged).
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: Box<PatternParenBody>,
        },
        /// `[ pat, … ]` (also matches `[]`). **Delta:** `,` separator, not
        /// `;`.
        List {
            plist: ListGroup<()>,
            #[group(self.plist)]
            items: Vec<PatListItem>,
        },
    }

    /// The parenthesized-pattern group's content: one pattern, plus any
    /// `, pat` continuations (present only for a tuple pattern).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct PatternParenBody {
        pub first: super::PatErasedV1,
        pub rest: Vec<CommaPattern>,
    }

    /// A `, pat` continuation inside a parenthesized tuple pattern.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CommaPattern {
        pub comma: CommaTok,
        pub value: super::PatErasedV1,
    }

    /// One list-pattern element `pat,` (the last `,` is optional). **Delta
    /// from [`crate::cst::ast::PatListItem`]:** `,` separator, not `;`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct PatListItem {
        pub value: super::PatErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// A type-expression grammar (`typ`/`typ_prod`/`typ_app`/`typ_bot`,
    /// `parser_v1.mly:685-752`, simplified — same scope as
    /// [`crate::cst::ast::TypeExpr`]). Widened (Sub-slice 2b) from Slice 1's
    /// `Fun{dom: TypeAtom, ..} | Atom(TypeAtom)` to also spell products
    /// (`length * length`, [`TypeProd`]) and prefix type application (`list
    /// int`, [`TypeApp`]) — without them a `type` bind could declare almost
    /// nothing. Optional-arg-rows increment 2 adds the `?(…)` labeled-optional
    /// domain prefix ([`TypeExpr::OptRowFun`]) — a row-variable TAIL inside
    /// that prefix (`?(… | ?'r) ->`) parses but is rejected at lowering
    /// (needs signature-level row quantification, L4/2d territory, not this
    /// increment): `dom -> cod` right-recursive through `cod` only (the
    /// root's unchanged self-loop). Self-recursive only through `Fun`'s/
    /// `OptRowFun`'s codomain (right recursion); parenthesized nesting goes
    /// through [`super::TyErasedV1`]. `TypeExpr` was unreachable from
    /// `FileV1` before 2b (`Bind` had no type ascriptions and `Param` was
    /// `Pat`-only); `Type` binds are its first reachable use.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeExpr {
        /// `?(l : ty, … [| ?'r]) dom -> cod` (`typ` `:688-693`, `typ_opt_dom`
        /// `:753-758`; optional-arg-rows increment 2). `?`-headed — neither
        /// `Fun`/`Atom` (headed by `TypeProd`) can start with
        /// `OptionalTypeTok`, so declared order relative to them is
        /// safety-neutral; declared first to mirror the upstream `typ`
        /// production order. Lowered (`v1/lower.rs`) to
        /// `cst::ast::TypeExpr::OptRowFun`, thence (`typecheck.rs`) to
        /// `MonoType::Func(Row::Cons(l1, ty1, … Row::Empty), dom, cod)` — a
        /// CLOSED row, matching what `Ast::LambdaOpt` infers (increment 1),
        /// so an explicit `?(l:τ)->` signature unifies against an actual
        /// `?(l=x)`-taking function.
        OptRowFun {
            opt_dom: TypeOptDomV1,
            dom: TypeProd,
            arrow: ArrowTok,
            cod: Box<TypeExpr>,
        },
        /// `dom -> cod` (right-associative). This field is `TypeExpr`'s own
        /// self-loop (the root SCC). `dom` widened from [`TypeAtom`] to
        /// [`TypeProd`] (Sub-slice 2b) so e.g. `'a option -> 'b option` and
        /// `'a * 'b -> 'c` both parse at their expected precedence.
        Fun {
            dom: TypeProd,
            arrow: ArrowTok,
            cod: Box<TypeExpr>,
        },
        /// The non-arrow fallthrough — widened from [`TypeAtom`] to
        /// [`TypeProd`] (Sub-slice 2b): a product/application with no
        /// enclosing arrow is still just "the whole type expression minus
        /// `->`".
        Atom(TypeProd),
    }

    /// `?(l : ty, … [| ?'r])` — the (possibly row-tailed) labeled-optional
    /// domain prefix of a [`TypeExpr::OptRowFun`] (`typ_opt_dom`,
    /// `parser_v1.mly:753-758`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeOptDomV1 {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub inner: TypeOptDomInnerV1,
    }

    /// A [`TypeOptDomV1`]'s group content: one or more `label : typ` entries
    /// (nonempty enforced at lowering), then an optional `| ?'r` row-variable
    /// tail (`typ_opt_dom` `:756-757`) — parsed, but rejected with a
    /// `LowerError` (needs signature-level row quantification — L4/2d
    /// territory, not this increment; contrast [`TypeRecordInnerV1`]'s own
    /// `row_tail`, which the SAME increment DOES complete, since a bare
    /// record-typed value has no `quant`-list obligation to satisfy).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeOptDomInnerV1 {
        pub entries: Vec<TypeOptEntryV1>,
        pub row_tail: Option<RowTailV1>,
    }

    /// One `label : typ,` entry of a [`TypeOptDomV1`] (last `,` optional;
    /// `typ_opt_dom_entry`, `parser_v1.mly:759-762` — COLON, unlike the
    /// value-level `?(l = e)` bundle's `=`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeOptEntryV1 {
        pub label: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// `| ?'r` — a row-variable tail (shared by [`TypeOptDomInnerV1`] and
    /// [`TypeRecordInnerV1`]; `parser_v1.mly:748-749`/`:756-757`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RowTailV1 {
        pub bar: BarTok,
        pub var: RowVarTok,
    }

    /// `typ_prod` (`parser_v1.mly:696-709`): one or more `*`-separated
    /// [`TypeApp`]s, flattened to head+`Vec` exactly like
    /// [`crate::cst::ast::TypeProd`] (`cst.rs:1284-1295`) — the same
    /// deferred-fold technique as `OpChain`/`PatCons`, keeping `TypeExpr` a
    /// singleton SCC.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeProd {
        pub first: TypeApp,
        pub rest: Vec<StarType>,
    }

    /// A `* ty` continuation of a [`TypeProd`].
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct StarType {
        pub star: ExactTimesTok,
        pub ty: TypeApp,
    }

    /// `typ_app` (`parser_v1.mly:711-739`). **0.1 delta from
    /// [`crate::cst::ast::TypeApp`] (`cst.rs:1297-1312`):** application is
    /// PREFIX and n-ary (`list int`, `pair int bool`), not 0.0.6's postfix
    /// single-argument (`int list`) — the prefix→postfix bridge (with an
    /// arity-1 guard: arity ≥ 2 is a `LowerError`, not a parse error) lives
    /// in `v1/lower.rs`. `Applied`/`AppliedLong` (needing at least one
    /// argument atom) are tried before `Atom` — a bare name has no argument
    /// atom to consume and falls through cleanly (a following
    /// keyword/`=`/`and`/`->`/`*` never parses as a [`TypeAtom`]).
    ///
    /// Sub-slice 2d-2 adds three arms, all keyword- or token-headed and so
    /// disjoint from `Applied`/`Atom`'s `VarTok`-headed shapes (ordered
    /// BEFORE them is cosmetic, not load-bearing):
    ///
    /// - [`InlineCmdTy`](TypeApp::InlineCmdTy)/[`BlockCmdTy`](TypeApp::
    ///   BlockCmdTy)/[`MathCmdTy`](TypeApp::MathCmdTy): `inline [τ, …]`/
    ///   `block [τ, …]`/`math [τ, …]` command types (`parser.mly:730-735`,
    ///   `typ_cmd_arg` `:763-774`; `math […]`: `parser.mly:830-831`),
    ///   `KwInline`/`KwBlock`/`KwMath`-headed (already V0_1 keywords —
    ///   `val inline`/`val block`/`val math` binds, 2b; `math`'s keyword
    ///   status since the math-split, `token.rs`'s `KwMath`, `lexer.rs`'s
    ///   `"math"` arm — zero lexer work for any of the three). One
    ///   deliberate superset of upstream remains: each bracketed slot is a
    ///   full [`TyErasedV1`] (`TypeExpr`), not upstream's narrower
    ///   `typ_prod`. The `?(label: τ, …)` optional-labeled-slot prefix
    ///   (roadmap phase 4) now IS modeled — see [`TypeCmdArgItemV1::opts`]
    ///   (optional-arg-rows increment 3a); `MathCmdTy` reuses
    ///   `TypeCmdArgItemV1` as-is, so `math [?(l : τ) …]` sig rows work with
    ///   zero extra grammar (math-completion M1).
    /// - [`AppliedLong`](TypeApp::AppliedLong): `M.t τ…` — the `LONG_LOWER`
    ///   qualified-head twin of `Applied` (`parser.mly:720-728`,
    ///   `LONG_LOWER` `lexer.mll:318`), `VarWithModTok`-headed (already
    ///   lexed by the program-mode capital-head scan, `lexer.rs:753-777` —
    ///   zero lexer work). Needed to NAME an abstract type from outside its
    ///   sealing module (2d-2 spec §2.4) — without it, an opaque `M.t`
    ///   could never appear in another module's signature at all.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeApp {
        /// `inline [τ, …]` (Sub-slice 2d-2) — see the enum doc comment.
        //
        // NOTE the group fields are `ilist`/`blist`/`mlist`, not three `list`s:
        // syan names a group substruct after (group-field name, ENUM name) with
        // no variant component, so same-named groups in one enum collide
        // (E0428 + E0119).
        InlineCmdTy {
            kw: KwInline,
            ilist: ListGroup<()>,
            #[group(self.ilist)]
            args: Vec<TypeCmdArgItemV1>,
        },
        /// `block [τ, …]` (Sub-slice 2d-2) — see the enum doc comment.
        BlockCmdTy {
            kw: KwBlock,
            blist: ListGroup<()>,
            #[group(self.blist)]
            args: Vec<TypeCmdArgItemV1>,
        },
        /// `math [τ, …]` (math-package completion M1; upstream
        /// `parser.mly:830-831` `MATH L_SQUARE optterm_list(COMMA,
        /// typ_cmd_arg) R_SQUARE → MMathCommandType(mncmdargtys)` — same
        /// `typ_cmd_arg` as inline/block). `KwMath`-headed — `math` has been
        /// a V0_1 lexer keyword since the math-split (`token.rs`'s
        /// `KwMath`, `lexer.rs`'s `"math"` arm), so this arm is disjoint
        /// from `Applied`/`Atom` (a bare `math` can never lex as a `VarTok`
        /// under V0_1 at all) and ambiguity-free. Reuses
        /// `TypeCmdArgItemV1`, so `?(l : τ, …)` optional-label prefixes
        /// (inc3a) work in `math […]` rows with zero extra grammar.
        MathCmdTy {
            kw: KwMath,
            mlist: ListGroup<()>,
            #[group(self.mlist)]
            args: Vec<TypeCmdArgItemV1>,
        },
        /// `M.t τ…` (Sub-slice 2d-2) — see the enum doc comment. Mirrors
        /// `Applied`'s n-ary shape (`v1/lower.rs`'s prefix→postfix bridge
        /// rejects arity ≥ 2 identically for both).
        AppliedLong {
            ctor: VarWithModTok,
            first: TypeAtom,
            rest: Vec<TypeAtom>,
        },
        Applied {
            ctor: VarTok,
            first: TypeAtom,
            rest: Vec<TypeAtom>,
        },
        Atom(TypeAtom),
    }

    /// One `[…]`-bracketed command-type argument slot: an optional
    /// `?(l : τ, …)` labeled-optional bundle PREFIX (optional-arg-rows
    /// increment 3a — upstream `typ_cmd_arg : option(typ_opt_dom) typ_prod`,
    /// `parser.mly:753-773`; roadmap phase 4, now landed), then the
    /// mandatory `τ,` (`,`-separated, last `,` optional — the [`ListItem`]
    /// pattern). A full [`TyErasedV1`] per slot (permissive superset of
    /// upstream's narrower `typ_prod`). `opts` is `Option`-tried first: a
    /// non-`?`-headed slot (every existing command-type element) fails the
    /// `?` head with no token stolen, so `opts: None` parses exactly as
    /// before this increment.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeCmdArgItemV1 {
        pub opts: Option<TypeCmdOptDomV1>,
        pub ty: super::TyErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// `?(l : τ, …)` — a CLOSED command-type optional bundle (optional-arg-
    /// rows increment 3a; upstream `typ_opt_dom`, `parser.mly:755-761`,
    /// minus the `| ?'r` row-variable tail: command optional-argument types
    /// are closed maps, never rows — upstream itself silently DISCARDS a
    /// written row variable here, `parser.mly:859-869`'s literal `TODO
    /// (error)` — so this port doesn't model one either; a stray `?'r` inside
    /// a command-type bracket is a parse error, faithfully matching
    /// upstream's "never actually usable" treatment of it). Mirrors
    /// [`ast::CstTypeOptDom`]-shaped satellites elsewhere in this file.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeCmdOptDomV1 {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub entries: Vec<TypeCmdOptEntryV1>,
    }

    /// One `label : τ,` entry of a [`TypeCmdOptDomV1`] bundle (the last `,`
    /// is optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeCmdOptEntryV1 {
        pub label: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// An atomic type expression. `parser_v1.mly:740-752`'s record forms are
    /// fully modeled: both the closed form and the open (row-var-tailed) form
    /// share [`TypeAtom::Record`], distinguished by
    /// [`TypeRecordInnerV1::row_tail`] (optional-arg-rows increment 2).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeAtom {
        /// `( ty )`
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: super::TyErasedV1,
        },
        /// `(| l1 : ty1, l2 : ty2, … |)` (closed) or `(| l1 : ty1, … | ?'r |)`
        /// (open — a row-variable tail; optional-arg-rows increment 2)
        /// (`typ_bot`'s two `L_RECORD` arms, `parser_v1.mly:746-749`;
        /// `typ_record_elem` `:775-777` — COLON fields, unlike record
        /// EXPRESSIONS' `l = e`). Lowered (`v1/lower.rs`): the closed form to
        /// the existing `cst::ast::TypeAtom::Record` (`cst.rs:1344`) and
        /// thence to a closed `MonoType::Record` row (`typecheck.rs:512`);
        /// the open form to the additive `cst::ast::TypeAtom::RecordOpen`
        /// and thence to an OPEN `MonoType::Record(Row::Var(…))` — a fresh
        /// row variable, using the existing generic `Row`/`RowVarRef`/
        /// `unify_row` machinery (no new type machinery needed).
        Record {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            inner: TypeRecordInnerV1,
        },
        /// A type variable, e.g. `'a`.
        Var(TypeVarTok),
        /// `M.t` — a qualified type name (Sub-slice 2d-2; upstream
        /// `LONG_LOWER`, `parser.mly:742-743`). `VarWithModTok`-headed,
        /// token-disjoint from `Var`/`Name`/`Paren` (`TypeVarTok`/`VarTok`/
        /// `LParenTok`) — see [`TypeApp::AppliedLong`]'s doc comment.
        LongName(VarWithModTok),
        /// A (possibly qualified) type name, e.g. `int`, `string`.
        Name(VarTok),
    }

    /// A [`TypeAtom::Record`]'s group content: the field list, plus an
    /// optional `| ?'r` row-variable tail (optional-arg-rows increment 2 —
    /// present ⇒ an OPEN record type; absent ⇒ closed, byte-identical to
    /// before this increment).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeRecordInnerV1 {
        pub fields: Vec<TypeRecordFieldV1>,
        pub row_tail: Option<RowTailV1>,
    }

    /// One `l : ty,` field (last `,` optional — the [`ListItem`] pattern).
    /// **Deltas from [`crate::cst::ast::TypeRecordField`] (`cst.rs:1363`):**
    /// `,` separator, not `;` (same delta as [`RecordField`]); field type is
    /// a full [`super::TyErasedV1`] (upstream `typ_record_elem :776` takes a
    /// full `typ`) — erased, not a direct `TypeExpr`, for the same
    /// cycle-avoidance reason `cst.rs:1355-1362` documents.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeRecordFieldV1 {
        pub name: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErasedV1,
        pub comma: Option<CommaTok>,
    }

    /// `mathtop`-analogue: one math element — identical shape to
    /// [`crate::cst::ast::MathElemCst`] (no 0.1 delta; see its doc comment
    /// for why this needs no direct self-loop of its own).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct MathElemCst {
        pub base: MathBot,
        pub scripts: Vec<MathScript>,
    }

    /// `mathbot` — identical shape to [`crate::cst::ast::MathBot`] (no 0.1
    /// delta; `name` accepts a module-qualified `\Mod.cmd` math command
    /// too, `AnyMathCmdTok::Mod` — the lexer already emits
    /// `Token::MathCmdWithMod` for one, `lexer.rs`'s `\\` arm in `Mode::
    /// Math`, math-package completion M4's `${\Math.paren{…}}`-shaped
    /// qualified references need it to parse at all).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathBot {
        /// `\cmd matharg*`, sigil-only or module-qualified (`\Mod.cmd
        /// matharg*`).
        Cmd { name: AnyMathCmdTok, args: Vec<MathArg> },
        Chars(MathCharTok),
        /// `#var` (math mode never trails this with `;`).
        Embed(VarInMathTok),
        /// A `|` separator marker (flat; elaborator regroups).
        Sep(SepTok),
        /// `{ … }` — re-enters the math grammar.
        Group {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErasedV1>,
        },
    }

    /// One postfix script combo of a [`MathElemCst`] — identical shape to
    /// [`crate::cst::ast::MathScript`] (no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathScript {
        /// `^ group`
        Super { hat: SuperscriptTok, group: MathGroupArg },
        /// `_ group`
        Sub { under: SubscriptTok, group: MathGroupArg },
        /// A run of `'` marks — sugar for a superscript of primes
        /// characters.
        Primes(PrimesTok),
    }

    /// `mathgroup`-analogue: a script's operand is either a bracketed math
    /// group or a bare `mathbot`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathGroupArg {
        Group {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErasedV1>,
        },
        Bot(Box<MathBot>),
    }

    /// `matharg`-analogue: one command argument in math mode — identical
    /// shape to [`crate::cst::ast::MathArg`] (no 0.1 delta; the escape
    /// bodies reuse the now-comma-separated [`ParenBody`]/[`ListItem`]/
    /// [`RecordBody`] defined above).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathArg {
        /// `{ math }`.
        Math {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErasedV1>,
        },
        /// `!{ inline text }`.
        Inline {
            igrp: InlineGroup<()>,
            #[group(self.igrp)]
            elems: Vec<InlineElem>,
        },
        /// `!<block text>`.
        Block {
            bgrp: BlockGroup<()>,
            #[group(self.bgrp)]
            elems: Vec<BlockElem>,
        },
        /// `!(e)` / `!(e, e, …)`.
        ParenEscape {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: Box<ParenBody>,
        },
        /// `![e, …]`.
        ListEscape {
            list: ListGroup<()>,
            #[group(self.list)]
            items: Vec<ListItem>,
        },
        /// `!(|l = e, …|)`.
        RecordEscape {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            body: RecordBody,
        },
    }

    // ---- Sub-slice 2c: the module/signature layer -----------------------

    /// `mod_chain`: `UPPER | LONG_UPPER` (`parser_v1.mly:404-414`). `M.N.P`
    /// arrives as ONE [`LongUpperTok`] (the V0_1 lexer branch), so a chain is
    /// always exactly one token — which is what makes [`ModExpr::App`]'s two-
    /// chain juxtaposition (`F X`, `F.G X.Y`) unambiguous at the token level.
    /// Token-disjoint arms; order is cosmetic.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum ModChainV1 {
        Long(LongUpperTok),
        Single(CtorTok),
    }

    /// `modexpr` (`parser_v1.mly:380-403`). SELF-LOOP ROOT: `Functor.body:
    /// Box<ModExpr>` (`:381-382`). Variant order is parse priority:
    /// `Functor` (`fun`-headed) and `Struct` (`struct`-headed) are
    /// keyword-disjoint from everything; `Coerce` (`UPPER :>`) must precede
    /// `App`/`Var` so the `:>` suffix is claimed before a bare chain matches;
    /// `App` (two chains, `modexpr_app` `:388-394`) precedes `Var` (one
    /// chain, `modexpr_bot` `:398-400`) for longest-match. Struct bodies go
    /// through [`super::StructBindV1`] (the 2a connector, erased), so
    /// `ModExpr` never statically references [`super::Bind`] — see the
    /// module doc comment's SCC story.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum ModExpr {
        /// `FUN ( UPPER : sigexpr ) ARROW modexpr` (`parser_v1.mly:381-382`).
        Functor {
            fun_kw: KwFun,
            lp: LParenTok,
            param: CtorTok,
            colon: ColonTok,
            dom: Box<SigExpr>,
            rp: RParenTok,
            arrow: ArrowTok,
            body: Box<ModExpr>,
        },
        /// `UPPER COERCE sigexpr` (`:383-384`) — coercion applies to a BARE
        /// module name only, upstream-faithfully (`A.B :> S` is a parse
        /// error there too).
        Coerce {
            name: CtorTok,
            coerce: CoerceTok,
            sig_: Box<SigExpr>,
        },
        /// `mod_chain mod_chain` — functor application (`:389-394`).
        App { func: ModChainV1, arg: ModChainV1 },
        /// `mod_chain` — a (possibly long) module path (`:399-400`).
        Var(ModChainV1),
        /// `STRUCT list(bind) END` (`:401-402`) — the (only) form 2a already
        /// lowered; reuses 2a's connector.
        Struct {
            struct_kw: KwStruct,
            binds: Vec<super::StructBindV1>,
            end_kw: KwEnd,
        },
    }

    /// `sigexpr` (`parser_v1.mly:558-573`). SELF-LOOP ROOT: `Functor.dom`/
    /// `Functor.cod: Box<SigExpr>` (`:570-571`).
    ///
    /// **Left-recursion note (load-bearing).** The naive sketch would write
    /// `With { base: Box<SigExpr>, … }` — as a syan2 ordered-choice
    /// production that is LEFT RECURSION (`SigExpr` would begin by parsing
    /// `SigExpr`; syan2 gives no diagnostic, it just recurses/fails at parse
    /// time — a known consumer hazard). Upstream is *not* left-recursive:
    /// the `with` base is `sigexpr_bot` (`:559,564`) and `with` cannot chain
    /// (the result of a `with` is never itself a valid `with` base). So the
    /// faithful encoding is bot + one optional-shaped suffix arm, tried
    /// before the bare-bot fallthrough: `S with type t = int with type u =
    /// bool` is a parse error here exactly as upstream (pinned in tests).
    /// NO arm of this enum may ever begin with `Box<SigExpr>`/`SigExpr` as
    /// its first field — reviewer checklist item.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum SigExpr {
        /// `( UPPER : sigexpr ) ARROW sigexpr` (`:570-571`) — the functor
        /// signature. `(`-headed; no [`SigBotV1`] starts with `(`, so this
        /// is token-disjoint from the other arms.
        Functor {
            lp: LParenTok,
            param: CtorTok,
            colon: ColonTok,
            dom: Box<SigExpr>,
            rp: RParenTok,
            arrow: ArrowTok,
            cod: Box<SigExpr>,
        },
        /// `sigexpr_bot WITH TYPE bind_type` (`:559-563`) /
        /// `sigexpr_bot WITH mod_chain TYPE bind_type` (`:564-569`). The
        /// `Option<ModChainV1>` is greedy-then-backtrack: on `with type` the
        /// chain fails (`type` is a keyword token, not `UPPER`/`LONG_UPPER`)
        /// and collapses to `None`. `binds` goes through
        /// [`super::TypeBindsErasedV1`].
        WithType {
            base: SigBotV1,
            with_kw: KwWith,
            path: Option<ModChainV1>,
            type_kw: KwType,
            binds: super::TypeBindsErasedV1,
        },
        /// A bare `sigexpr_bot` (`:572-573`). Must come after [`WithType`]
        /// (maximal munch of the `with` suffix).
        Bot(SigBotV1),
    }

    /// `sigexpr_bot` (`parser_v1.mly:575-595`) — a satellite (no self-loop;
    /// no edge back to `SigExpr`). Sig bodies go through
    /// [`super::StructDeclV1`] (opaque hand-written connector), so
    /// `SigBotV1` never statically references [`Decl`].
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum SigBotV1 {
        /// `LONG_UPPER` — a signature path `M.N.S` (`:581-590`).
        Path(LongUpperTok),
        /// `UPPER` — a signature name (`:576-580`).
        Var(CtorTok),
        /// `SIG list(decl) END` (`:591-595`). `sig` is already a
        /// version-independent keyword (`lexer.rs`) — zero lexer work.
        Sig {
            sig_kw: KwSig,
            decls: Vec<super::StructDeclV1>,
            end_kw: KwEnd,
        },
    }

    /// `decl` (`parser_v1.mly:597-621`) — one item of a `sig … end` body.
    /// NOT a root: no arm contains `Decl`; reached only through
    /// [`super::StructDeclV1`], so `SigExpr ↔ Decl` never forms a rootless
    /// static sub-cycle (the shape `cst.rs`'s `AppArgErased` doc warns the
    /// engine rejects). Its recursion-bearing edges are plain DAG edges INTO
    /// roots: `ty: TypeExpr` (the same satellite→root shape as
    /// `RecClauseV1.params: Vec<PatBot>`) and `sig_: Box<SigExpr>`.
    ///
    /// Deferred arms (parse errors): staged `val ~x`/`val persistent ~x`
    /// (`:600-603`, phase 5 — `persistent` has no keyword token yet), macro
    /// decls `val \m : macro-type` (`:608-611`, phase 5), row quantifiers
    /// (`rowquant`, `:631-633` — no `ROWVAR` token until phase 4).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Decl {
        /// `VAL bound_identifier quant COLON typ` (`:598-599`; `quant`'s
        /// tyvar list `:623-630` — `val map 'a 'b : ('a -> 'b) -> …`).
        Val {
            kw: KwVal,
            name: super::BindName,
            quant: Vec<TypeVarTok>,
            colon: ColonTok,
            ty: TypeExpr,
        },
        /// `VAL BACKSLASH_CMD quant COLON typ` (`:604-605`). Plain
        /// [`HorzCmdTok`] — upstream uses the bare token, and program mode
        /// already lexes `\cmd`. Naming mirrors
        /// [`crate::cst::SigItem::ValHorzCmd`].
        ValHorzCmd {
            kw: KwVal,
            cmd: HorzCmdTok,
            quant: Vec<TypeVarTok>,
            colon: ColonTok,
            ty: TypeExpr,
        },
        /// `VAL PLUS_CMD quant COLON typ` (`:606-607`).
        ValVertCmd {
            kw: KwVal,
            cmd: VertCmdTok,
            quant: Vec<TypeVarTok>,
            colon: ColonTok,
            ty: TypeExpr,
        },
        /// `TYPE LOWER CONS kind` — an OPAQUE type (`:612-613`). Tried
        /// before the transparent [`Decl::Type`]: the two share the `type
        /// name` prefix and are told apart by `::` vs `=`/tyvars
        /// (backtracking is two tokens deep, cheap).
        TypeOpaque {
            kw: KwType,
            name: VarTok,
            cons: ConsTok,
            kind: KindV1,
        },
        /// `TYPE bind_type` — transparent type(s) (`:614-615`), sharing the
        /// grouped chain with [`SigExpr::WithType`].
        Type {
            kw: KwType,
            binds: super::TypeBindsErasedV1,
        },
        /// `MODULE UPPER COLON sigexpr` (`:616-617`) — note `:` here (a
        /// decl constrains), vs `:>` on binds (a bind seals).
        Module {
            kw: KwModule,
            name: CtorTok,
            colon: ColonTok,
            sig_: Box<SigExpr>,
        },
        /// `SIGNATURE UPPER EXACT_EQ sigexpr` (`:618-619`).
        Signature {
            kw: KwSignature,
            name: CtorTok,
            eq: DefEqTok,
            sig_: Box<SigExpr>,
        },
        /// `INCLUDE sigexpr` (`:620-621`) — a decl-include includes a
        /// SIGNATURE (contrast [`super::Bind::Include`], which includes a
        /// MODULE).
        Include { kw: KwInclude, sig_: Box<SigExpr> },
    }

    // Ordered-choice safety of `Decl`: all arms are keyword-headed
    // (`val`/`type`/`module`/`signature`/`include`); within `val`, the
    // second token (`BindName`'s `Var`-or-`LParen` vs `HorzCmdTok` vs
    // `VertCmdTok`) is disjoint; within `type`, `TypeOpaque`-before-`Type`
    // as documented.

    /// `kind` (`parser_v1.mly:672-677`): `kind_base (ARROW kind_base)*`
    /// flattened head+`Vec` — the same deferred-fold shape as
    /// [`TypeProd`]/[`PatCons`], keeping the type acyclic. `kind_base` is a
    /// bare LOWER (`:678-681`, `MKindName`), so the whole kind grammar is
    /// token-only. (`kind_row`, `:682-683`, arrives with row quantifiers —
    /// phase 4.)
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct KindV1 {
        pub first: VarTok,
        pub rest: Vec<KindArrowV1>,
    }

    /// An `-> kind_base` continuation of a [`KindV1`].
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct KindArrowV1 {
        pub arrow: ArrowTok,
        pub base: VarTok,
    }

    // (`Quant` needs no struct: upstream `quant = list(tyquant)
    // list(rowquant)` (`:623-625`), and with rowquants deferred it is
    // exactly `Vec<TypeVarTok>` — inlined into `Decl::Val*` above.)
}

/// Lex ([`crate::lexer::lex_with_version`] under [`crate::version::RustyfiVersion::V0_1`])
/// and parse a whole 0.1 `.saty`/`.satyh` source file. Mirrors
/// [`crate::cst::parse_file`]'s two-step shape exactly, sharing its
/// [`crate::cst::ParseFileError`] (no new error type).
pub fn parse_file_v1(src: &str) -> Result<FileV1, crate::cst::ParseFileError> {
    let atoms = crate::lexer::lex_with_version(src, crate::version::RustyfiVersion::V0_1)
        .map_err(|e| crate::cst::ParseFileError {
            span: e.span,
            message: e.msg,
        })?;
    let mut stream = crate::stream::AtomStream::new(atoms);
    <FileV1 as Parse<_>>::parse(&mut stream).map_err(|e| crate::cst::ParseFileError {
        span: *e.span(),
        message: render_parse_error(&e),
    })
}

/// Flatten syan's nested error tree into one readable line. A private copy
/// of [`crate::cst`]'s identical helper (not `pub(crate)` there, and
/// `cst.rs` stays untouched — see the module doc comment).
fn render_parse_error(err: &syan::error::ParseError<Span>) -> String {
    format!("{err:?}")
}
