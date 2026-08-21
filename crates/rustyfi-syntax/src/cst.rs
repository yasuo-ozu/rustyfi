//! The milestone-1+2a surface grammar (a subset of the v0.0.6 `parser.mly`),
//! parsed with syan derives over the SATySFi token atoms.
//!
//! Application and the binary-operator levels are left-recursive in the Menhir
//! grammar; here they are head-plus-arguments sequences (`Vec`), folded left
//! during elaboration — recursive descent must never see left recursion.
//!
//! **Operator-precedence flattening.** `parser.mly` spreads binary operators
//! across ten precedence levels (`nxlor`..`nxrtimes`, some left- some
//! right-associative). Reproducing that exactly would multiply every level
//! into its own non-left-recursive rule; instead all binops are flattened
//! into one `OpChain` (`head` `AppExpr` + a flat `Vec` of `(op, AppExpr)`
//! pairs), deferring precedence/associativity resolution to the elaborator
//! (not this crate's job). This is a deliberate deviation from `parser.mly`'s
//! *structure* (not its *token set* — every operator it accepts is accepted
//! here too).
//!
//! **`let-inline`/`let-block` are top-level-only**, matching `parser.mly`:
//! `LETHORZ`/`LETVERT` only appear in `nxtoplevel`/`nxstruct` (via
//! `nxhorzdec`/`nxvertdec`), never in `nxletsub`, so unlike `let`/`let-rec`
//! they have no local (`in`-bodied) form nested inside an arbitrary
//! expression — only as one of a file's leading [`TopBinding`]s.

use crate::leaf::*;
use crate::span::Span;
use newer_type::implement;
use syan::parse::{Parse, Unparse};

/// A whole `.saty`/`.satyh` file: headers, top-level bindings, `in`, the
/// document expression (`main`/`nxtoplevel`/`nxtopsubseq` in parser.mly).
///
/// **Library-file form.** A `.satyh` library is just headers + top-level
/// bindings + `EOI`, with no `in body` at all (`nxtopsubseq`'s bare `EOI`
/// alternative) — so unlike phase-1/2a, `body` is now optional: `in_kw`
/// present implies `body` present (checked at elaboration, not here), but a
/// file can also end right after `prelude` with neither.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct File {
    pub headers: Vec<Header>,
    pub prelude: Vec<TopBinding>,
    /// Required whenever `body` is present (checked at elaboration).
    pub in_kw: Option<KwIn>,
    /// Absent for a library file (`nxtopsubseq`'s bare `EOI` case).
    pub body: Option<ast::Expr>,
    pub eoi: EoiTok,
}

/// `@require:` / `@import:` / `@stage:` header element.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum Header {
    /// Accepted and currently ignored (driven by the loader crate).
    Require(HeaderRequireTok),
    /// Accepted and currently ignored (driven by the loader crate).
    Import(HeaderImportTok),
    /// `@stage: persistent` / `@stage: 0` / `@stage: 1` — the stage EVERY
    /// binding in this file is written at (0.0.6 declares it per file; 0.1
    /// dropped the header and says the same thing per binding, see
    /// [`TopStage`]). Honoured, not ignored: the loader's prelude merge
    /// records the entry range each file contributed
    /// (`rustyfi_lang::note_stage`) and `elaborate.rs` wraps every one of
    /// that file's bindings — `let`, `let-rec`, `let-inline`, `let-block`,
    /// `let-math` and `let-mutable` alike — in `Ast::StageScope`, so a
    /// stage-0 library may use `&(…)` and the document that requires it may
    /// not.
    Stage(HeaderStageTok),
}

/// A binding-position NAME: a plain variable, or `( ‹op› )` — a
/// parenthesized (possibly user-defined) operator name (`OpNameTok`), e.g.
/// `let (+++>) = ..` (`itemize.satyh`), `let (-->) t1 t2 = ..` / `val
/// (-->) : ty` (`progsynt.satyh`). Upstream's `var` nonterminal folds `VAR`
/// and `LPAREN binop RPAREN` into one production; this is that nonterminal,
/// reused by [`TopLet::name`], [`ast::Expr::LetIn`]'s `name`,
/// [`ast::RecBinding::name`], and [`SigItem::Val`]'s `name` — the four
/// binding positions upstream admits it in. `.name`/`.span` mirror
/// `VarTok`'s own public fields exactly (e.g. `"+++>"`/the whole `(..)`'s
/// span for `(+++>)`), so every existing `elaborate.rs`/`typecheck.rs`
/// callsite that reads `foo.name.name`/`foo.name.span` on one of the four
/// positions keeps compiling unchanged now that the field type there moves
/// from `VarTok` to this — an operator name binds/resolves exactly like an
/// ordinary variable of that string from here on. See also
/// [`ast::Atomic::OpRef`], the matching atomic-expression form (a bare
/// value reference, e.g. `(+++)`/`(-->)`).
#[derive(Debug, Clone, PartialEq)]
pub struct BindName {
    pub name: String,
    pub span: Span,
    repr: BindNameRepr,
}

/// [`BindName`]'s two surface forms, parsed with syan's ordinary
/// enum-variant backtracking (the same technique `Atomic`'s many
/// alternatives use below) — kept as a separate, non-`pub`-facing enum
/// purely so `#[derive(Parse, Unparse)]` can pick between them; [`BindName`]
/// itself is hand-written so it can additionally expose the precomputed
/// `name`/`span` fields. `Op` first: not a real ambiguity (a plain `VarTok`
/// never starts with `LParen`), just documents the intended priority.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
enum BindNameRepr {
    Op(OpNameTok),
    Var(VarTok),
}

impl Parse<crate::token::Atom> for BindName {
    type Error = syan::error::ParseError<crate::span::Span>;

    fn parse_stream<S: syan::parse::ParseStream<Atom = crate::token::Atom>>(
        stream: &mut S,
    ) -> Result<Self, Self::Error> {
        let repr = BindNameRepr::parse_stream(stream)?;
        let (name, span) = match &repr {
            BindNameRepr::Op(op) => (op.name.clone(), op.span),
            BindNameRepr::Var(v) => (v.name.clone(), v.span),
        };
        Ok(BindName { name, span, repr })
    }
}

impl Unparse<crate::token::Atom> for BindName {
    fn unparse<S: syan::parse::unparse::Emitter<crate::token::Atom>>(
        &self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        self.repr.unparse(sink)
    }
}

impl From<VarTok> for BindName {
    /// Synthesize a binding name from a bare variable token. Used only by
    /// the 0.1 lowering (`rustyfi-lang/src/v1/lower.rs`), which builds
    /// synthetic 0.0.6 CST out of parsed `cst_v1` nodes. Purely additive:
    /// no parse production changes, no existing behavior touched — the
    /// "frozen" contract on this file (`cst_v1.rs`'s module doc) is about
    /// the 0.0.6 grammar/behavior, which an inherent conversion cannot
    /// affect, same spirit as the plan's blessed visibility-only edits
    /// (Acceptance (b)).
    fn from(v: VarTok) -> BindName {
        BindName {
            name: v.name.clone(),
            span: v.span,
            repr: BindNameRepr::Var(v),
        }
    }
}

#[cfg(test)]
mod bind_name_tests {
    use super::*;

    #[test]
    fn from_var_tok_preserves_name_and_span() {
        let v = VarTok {
            name: "foo".to_string(),
            span: Span::default(),
        };
        let bn: BindName = v.clone().into();
        assert_eq!(bn.name, v.name);
        assert_eq!(bn.span, v.span);
    }
}

/// A top-level non-recursive binding: `let name param* = expr`. `params` is
/// `Vec<ast::PatBot>` (not merely `Vec<VarTok>`), matching `RecBinding`'s
/// field of the same name (`nxnonrecdec`'s `argpart` is `patbot*` upstream
/// too, e.g. the bundled `gr.satyh`'s `let rectangle (x1, y1) (x2, y2) =
/// ..` and `let circle (cx, cy) r = ..` — plain, non-`let-rec` top-level
/// functions with tuple-destructuring parameters). Elaborated by the same
/// `rec_clause_value` helper `RecBinding` uses (`elaborate.rs`), with no
/// multi-clause `extra`.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TopLet {
    pub let_kw: KwLet,
    /// The stage this ONE binding is written at, when it is not the default
    /// (see [`TopStage`]). 0.0.6 source never sets it — 0.0.6 declares a
    /// stage per FILE, with a `@stage:` header — so it parses as `None` for
    /// every 0.0.6 program; it exists because SATySFi **0.1** declares the
    /// stage per BINDING (`val ~x = e`, `parser_v1.mly:417-421`) and
    /// `v1/lower.rs` lowers 0.1 binds into this very node. Carrying the
    /// stage ON the binding (rather than in a side table keyed by prelude
    /// index, which is how the 0.0.6 `@stage:` header reaches
    /// `elaborate.rs`) is what lets it survive the loader's prelude merge,
    /// module nesting and cross-version splicing unharmed: an index-keyed
    /// map cannot name a binding *inside* a `module … = struct … end`, and
    /// every 0.1 `val` is inside one.
    pub stage: Option<TopStage>,
    pub name: BindName,
    /// Optional `: ty` type ascription (`let f : ty x = e`, `let x : ty = e`),
    /// upstream's `patbotwithann` — parse-and-ignore, exactly like
    /// [`ast::RecBinding::ascription`] (this untyped elaborator has nothing to
    /// check it against). Sits before `params`, matching `patbotwithann
    /// argpart`.
    pub ascription: Option<ast::RecAscription>,
    /// The `|` upstream's `nonrecdecargpart` allows between the name (or its
    /// ascription) and the argument list — `let f : τ | x = e` and
    /// `let f | x = e`, `parser.mly:610-614`. Unlike `let-rec`'s, a non-rec
    /// `|` introduces NO further clauses (`nonrecdecargpart` has no
    /// `nxrecdecpar` tail): it is purely a separator, so nothing downstream
    /// reads this field — it exists to make verbatim upstream source parse,
    /// exactly like [`ast::RecBinding::leading_bar`].
    ///
    /// Real source writes it: `azmath`'s `util.satyh` opens with
    /// `let math-in-math : math-class -> (context -> math) -> math`
    /// `| mcls embedf = ..`, and without this the whole file failed at its
    /// first binding — the package's ONLY blocker, in both the 0.0.6 and
    /// the cross-version arm.
    pub leading_bar: Option<BarTok>,
    pub params: Vec<ast::Param>,
    pub eq: DefEqTok,
    pub value: ast::Expr,
}

/// A binding's own stage qualifier: `~` (stage 0) or `persistent ~`
/// (persistent stage), the prefix SATySFi 0.1 writes between `val` and the
/// bound name (`parser_v1.mly:417-421`, `UTBindValue(Stage0 |
/// Persistent0, _)`; the absent prefix is `Stage1`, the document stage).
///
/// Spelled with tokens rather than a `rustyfi_lang::types::Stage` because
/// this crate is the syntax layer and knows nothing of the type layer;
/// `elaborate.rs` maps the pair to a `Stage` (`top_let_stage`).
///
/// `persistent` is a 0.1-only keyword (`lexer.rs`'s version-gated table), so
/// under 0.0.6 the `Some(_)` shape is unreachable through the `persistent`
/// spelling and reachable only as a bare `let ~x = e` — syntax upstream
/// 0.0.6 does not have (its `EXACT_TILDE` is a splice operand prefix,
/// `v0.0.6 parser.mly:797`, or macro syntax, `:608`/`:1199` — never a
/// binding qualifier). PARSING it under 0.0.6 is the usual additive-accept
/// latitude this shared cst takes; ELABORATING it is not — `elaborate.rs`'s
/// `binding_stage` refuses a stage qualifier on 0.0.6-authored input with a
/// version error, so no 0.0.6 file can quietly acquire a per-binding stage.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TopStage {
    pub persistent: Option<KwPersistent>,
    pub tilde: ExactTildeTok,
}

/// One top-level declaration (`nxtoplevel`/`nxstruct`'s per-declaration
/// alternatives). `LetInline`/`LetBlock` only exist here — see the module
/// doc comment.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum TopBinding {
    /// `let-rec name param* = expr (and name param* = expr)*`
    LetRec {
        kw: KwLetRec,
        /// See [`TopLet::stage`] — upstream 0.1 puts the qualifier before the
        /// WHOLE `bind_value`, and `bind_value` covers `rec`/`mutable`/
        /// `inline`/`block`/`math` as well as the plain non-recursive form
        /// (`dev-0-1-0 parser.mly:417-421` → `:581-593`), so every binding
        /// shape below carries one too. The stage applies to each `and`
        /// clause of this one `let-rec`, exactly as it does upstream (one
        /// `UTBindValue(stage, UTRec(binds))` for the whole chain).
        stage: Option<TopStage>,
        first: ast::RecBinding,
        ands: Vec<ast::AndBinding>,
    },
    /// `let name param* = expr`
    Let(TopLet),
    /// `let pat = expr` — a top-level (or `struct`-level) DESTRUCTURING `let`
    /// whose target is a general pattern, not a plain variable (e.g.
    /// `satysfi-xpath`'s `let (ulim1, ulim2) = (0. -. eps, 1. +. eps)`). The
    /// `struct`-body twin of [`ast::Expr::LetPatternIn`], and — for the same
    /// reason it sits after `Expr::LetIn` — **must stay after `Let`**: an
    /// ordinary `let x = e` parses through `Let` first (its `name: BindName`
    /// only accepts a bare var/op), leaving this to match only a non-variable
    /// pattern target. No `argpart` (curried params after the pattern), as
    /// upstream's `nxnonrecdec` never uses one here.
    LetPattern {
        let_kw: KwLet,
        pat: PatErased,
        eq: DefEqTok,
        value: ast::Expr,
    },
    /// `[ctxvar] let-inline \cmd param* = expr` (`nxhorzdec`; each `param`
    /// is upstream's `arg` — a full patbot, or a `?:`-marked variable, see
    /// [`ast::Param`]'s doc comment — `parser.mly:622-624`).
    LetInline {
        kw: KwLetHorz,
        /// See [`TopBinding::LetRec::stage`].
        stage: Option<TopStage>,
        ctx: Option<VarTok>,
        cmd: HorzCmdTok,
        params: Vec<ast::Param>,
        eq: DefEqTok,
        value: ast::Expr,
    },
    /// `[ctxvar] let-block +cmd param* = expr` (`nxvertdec`).
    LetBlock {
        kw: KwLetVert,
        /// See [`TopBinding::LetRec::stage`].
        stage: Option<TopStage>,
        ctx: Option<VarTok>,
        cmd: VertCmdTok,
        params: Vec<ast::Param>,
        eq: DefEqTok,
        value: ast::Expr,
    },
    /// `let-math \cmd param* = expr` (`nxmathdec`, `parser.mly:586-591`).
    /// **No leading context variable** — unlike `LetInline`/`LetBlock`,
    /// upstream's `nxmathdec` curries straight from the command name into
    /// `cmdarglst*` with no `ctxvar` slot at all (`UTLambdaMath`, not
    /// `UTLambdaHorz`/`UTLambdaVert`), since a math command's own type
    /// (`math-cmd`) carries no implicit `context` argument the way
    /// `inline-cmd`/`block-cmd` do. `cmd` reuses the plain `HorzCmdTok`
    /// token (upstream's `nxmathdec` also reuses `HORZCMD`, not a
    /// math-specific token — `\frac` here is lexed exactly like `\frac` in
    /// `let-inline`; the two forms are told apart only by which keyword
    /// introduced them).
    LetMath {
        kw: KwLetMath,
        /// See [`TopBinding::LetRec::stage`].
        stage: Option<TopStage>,
        cmd: HorzCmdTok,
        params: Vec<ast::Param>,
        eq: DefEqTok,
        value: ast::Expr,
    },
    /// `type name = [|] Ctor [of ty] (| Ctor [of ty])*` (a variant
    /// declaration) or `type name = ty` (a transparent type *synonym*) —
    /// `nxvariantdec`; see [`TypeDeclBody`] for how the two are told apart.
    Type(TypeDecl),
    /// `let-mutable name <- expr` (top-level; `nxtoplevel`/`nxstruct`'s
    /// `LETMUTABLE` case — the local, `in`-bodied form is
    /// [`ast::Expr::LetMutableIn`]).
    LetMutable {
        kw: KwLetMutable,
        /// See [`TopBinding::LetRec::stage`].
        stage: Option<TopStage>,
        name: VarTok,
        arrow: OverwriteEqTok,
        value: ast::Expr,
    },
    /// `module Name [: sig ... end] = struct ... end` (`nxtoplevel`'s
    /// `MODULE` case).
    Module {
        kw: KwModule,
        name: CtorTok,
        sig: Option<SigAnnot>,
        eq: DefEqTok,
        struct_kw: KwStruct,
        decls: Vec<StructDecl>,
        end_kw: KwEnd,
    },
    /// `open Name` (`nxtoplevel`'s `OPEN` case; the local, `in`-bodied form
    /// is [`ast::Expr::OpenIn`]).
    Open { kw: KwOpen, name: CtorTok },
}

/// One declaration inside a `module ... = struct ... end` body (`nxstruct`).
/// `nxstruct`'s alternatives are a strict subset of `nxtoplevel`'s (every
/// form it has, [`TopBinding`] also has, once `Module`/`Open` are added), so
/// this simply re-parses a [`TopBinding`] — but *not* by naming `TopBinding`
/// as a field type directly: `TopBinding` lives **outside** the
/// `#[recurse]` module, so `TopBinding -> Module -> Vec<StructDecl> ->
/// TopBinding` would be a self-recursive cycle through a plain
/// `#[derive(Parse)]`, which (without the `#[recurse]` engine to back it)
/// is an `E0275` hazard (an unbounded recursive trait-bound obligation).
/// Hand-writing `Parse`/`Unparse` here — the same trick as
/// [`ast::ExprErased`] et al. — sidesteps that: the impl has no recursive
/// where-bound for the compiler to try to satisfy, it just calls
/// `TopBinding::parse` through the stream-erasing adapter at runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl(pub Box<TopBinding>);

impl Parse<crate::token::Atom> for StructDecl {
    type Error = syan::error::ParseError<crate::span::Span>;

    fn parse_stream<S: syan::parse::ParseStream<Atom = crate::token::Atom>>(
        stream: &mut S,
    ) -> Result<Self, Self::Error> {
        let value = <TopBinding as Parse<_>>::parse_stream(stream)?;
        Ok(StructDecl(Box::new(value)))
    }
}

impl Unparse<crate::token::Atom> for StructDecl {
    fn unparse<S: syan::parse::unparse::Emitter<crate::token::Atom>>(
        &self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        self.0.unparse(sink)
    }
}

/// `: sig ... end` (`nxsigopt`/`nxsigelem`, drastically simplified — see
/// [`SigItem`]).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct SigAnnot {
    pub colon: ColonTok,
    pub sig_kw: KwSig,
    pub items: Vec<SigItem>,
    pub end_kw: KwEnd,
}

/// One `nxsigelem`. Type parameters/type synonyms on `type` items are not
/// supported — such input is rejected with a parse error. Each item may
/// carry a trailing `constrnts` (`parser.mly:526-530`) — see
/// [`SigConstraint`].
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum SigItem {
    /// `val \cmd : ty` / `val +cmd : ty`.
    ValHorzCmd {
        kw: KwVal,
        name: HorzCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
        constraints: Vec<SigConstraint>,
    },
    ValVertCmd {
        kw: KwVal,
        name: VertCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
        constraints: Vec<SigConstraint>,
    },
    /// `val name : ty` / `val ( ‹op› ) : ty`.
    Val {
        kw: KwVal,
        name: BindName,
        colon: ColonTok,
        ty: ast::TypeExpr,
        constraints: Vec<SigConstraint>,
    },
    /// `direct \cmd : ty` / `direct +cmd : ty`.
    DirectHorzCmd {
        kw: KwDirect,
        name: HorzCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
        constraints: Vec<SigConstraint>,
    },
    DirectVertCmd {
        kw: KwDirect,
        name: VertCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
        constraints: Vec<SigConstraint>,
    },
    /// `type tyvar* name` (no synonym).
    Type {
        kw: KwType,
        tyvars: Vec<TypeVarTok>,
        name: VarTok,
        constraints: Vec<SigConstraint>,
    },
}

/// One `constrnt`: `constraint 'a :: (| l1 : ty1; l2 : ty2; … |)`
/// (`parser.mly:526-530`), a per-item suffix binding *that item's* type
/// variable to a row-kind obligation — **not** a standalone `SigItem` (a
/// reader expecting the latter should see this doc: upstream attaches
/// `constrnts` to `SigValue`/`SigDirect`/`SigType` directly, so the suffix
/// form here is the faithful one and avoids an ambiguous "which item does
/// this constrain?").
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct SigConstraint {
    pub kw: ConstraintTok,
    pub tyvar: TypeVarTok,
    pub cons: ConsTok,
    pub kind: RecordKind,
}

/// `kxtop`: `(| l1 : ty1; … |)`, a record-kind bound — "the constrained
/// type variable must be a record containing at least these labels"
/// (upstream `MRecordKind`; lowers to this port's `Kind::Record` row
/// obligation, presence-only this milestone — see `typecheck.rs`).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct RecordKind {
    pub rec: RecordGroup<()>,
    #[group(self.rec)]
    pub fields: Vec<RecordKindField>,
}

/// One `l : ty;` field of a [`RecordKind`] (`txrecord`,
/// `parser.mly:962-965`). The field *type* is parsed but currently dropped
/// during lowering (only the label is kept, matching `Kind::Record`'s
/// label-only representation) — a documented Slice-1 limitation, not a
/// grammar gap.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct RecordKindField {
    pub name: VarTok,
    pub colon: ColonTok,
    pub ty: ast::TypeExpr,
    pub semi: Option<ListPunctTok>,
}

/// A `type` declaration, optionally with mutual (`and`) recursion between
/// several type declarations (`parser.mly`'s `nxvariantdec` `and`-chain, e.g.
/// `satysfi-base`'s `stream.satyg`: `type 'a state = … and 'a u = ('a state)
/// Promise.t`). The head clause is `kw`..`body`; every further `and`-clause is
/// an [`AndTypeClause`]. All clauses in one chain are mutually visible — they
/// lower to consecutive `UserTypeDecl`/`UserSynonymDecl`s, exactly the shape
/// the 0.1 lowering (`v1/lower.rs`) already produces for `type … and …`, which
/// the typechecker resolves with the same forward-reference tolerance.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub kw: KwType,
    pub tyvars: Vec<TypeVarTok>,
    pub name: VarTok,
    pub eq: DefEqTok,
    pub body: TypeDeclBody,
    pub ands: Vec<AndTypeClause>,
}

/// One `and 'a name = body` continuation of a [`TypeDecl`]'s mutual-recursion
/// chain (mirrors [`AndBinding`] for `let`-rec).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct AndTypeClause {
    pub and_kw: KwAnd,
    pub tyvars: Vec<TypeVarTok>,
    pub name: VarTok,
    pub eq: DefEqTok,
    pub body: TypeDeclBody,
}

/// The right-hand side of a `type` declaration: either a variant's
/// constructor list, or (transparently) a type-synonym body. Trying the
/// variant shape first is unambiguous: a type name is always a bare `VAR`
/// in this grammar (`txbot`), so no type expression can ever start with the
/// `BarTok`/`CtorTok` a variant list requires — any input that isn't a
/// variant list falls through to `Synonym` cleanly, exactly like upstream's
/// `nxvariantdec` telling `variants` (always `CONSTRUCTOR`-headed) apart
/// from `txfunc` by lookahead.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum TypeDeclBody {
    /// `[|] Ctor [of ty] (| Ctor [of ty])*`.
    Variant {
        leading_bar: Option<BarTok>,
        first: VariantDef,
        rest: Vec<BarVariantDef>,
    },
    /// `ty` — a transparent type synonym, e.g. `type point = length *
    /// length` (`typechecker.ml`'s `SynonymType`/`add_synonym`: the name is
    /// replaced by this body wherever it appears in type position, so it
    /// never reaches unification itself). `TypeDecl::tyvars` is parsed the
    /// same way for a synonym as for a variant (`type 'a foo = ..`), but
    /// only the zero-param case can actually be *referenced* anywhere today
    /// — this grammar has no applied-type-constructor syntax (`TypeAtom`'s
    /// doc comment) to spell a synonym's argument at a use site.
    Synonym(ast::TypeExpr),
}

/// One `Ctor [of ty]` variant definition.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct VariantDef {
    pub ctor: CtorTok,
    pub of_ty: Option<OfType>,
}

/// The `of ty` suffix of a variant definition.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct OfType {
    pub of_kw: KwOf,
    pub ty: ast::TypeExpr,
}

/// A `| Ctor [of ty]` continuation of a variant list.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct BarVariantDef {
    pub bar: BarTok,
    pub def: VariantDef,
}

/// Recursion-edge eraser types.
///
/// These exist purely for **compile-time sanity**. The `#[recurse]` engine's
/// generated code is monomorphized per concrete parse-stream type, and syan's
/// backtracking wraps the stream in a fresh `Dup<&mut _, _>` layer at every
/// enum/`Vec`/`Option` boundary — so an engine covering a large SCC gets
/// re-instantiated combinatorially (measured on the naive transcription of
/// this grammar: rustc >16 minutes, >10 GB, and >7 minutes for `cargo check`
/// alone). Routing every recursion edge *except the roots' own self-loops*
/// through these hand-written leaves keeps each SCC a singleton (`Expr`,
/// `PatBot`, `TypeExpr`), so each engine stays tiny, and parses the wrapped
/// grammar directly: `parse_stream` reborrows, so one stream type serves
/// the whole descent. Defined *outside* the `#[recurse]` module so the
/// macro treats them as opaque leaves (they never appear as cycle edges).
macro_rules! erased_leaf {
    ($($(#[$doc:meta])* $name:ident => $target:ty;)*) => {
        $(
            $(#[$doc])*
            #[implement(newer_type_std::ops::Deref)]
            #[derive(Debug, Clone, PartialEq)]
            pub struct $name(pub Box<$target>);

            impl Parse<crate::token::Atom> for $name {
                type Error = syan::error::ParseError<crate::span::Span>;

                // No erasure any more. `parse_stream` takes `&mut S` and
                // recursion REBORROWS, so `S` is a genuine fixed point and the
                // instantiation set is finite by construction — which is what
                // `EraseStream` used to buy by pinning everything to
                // `&mut dyn ParseStream`, at the price of a virtual call per
                // stream operation. The wrapper now only boxes the value.
                fn parse_stream<S: syan::parse::ParseStream<Atom = crate::token::Atom>>(
                    stream: &mut S,
                ) -> Result<Self, Self::Error> {
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

erased_leaf! {
    /// An [`ast::Expr`] behind a stream-erasing parse (see above).
    ExprErased => ast::Expr;
    /// An [`ast::Pattern`] behind a stream-erasing parse (see above).
    PatErased => ast::Pattern;
    /// An [`ast::PatBot`] behind a stream-erasing parse (see above). Kept
    /// separate from [`PatErased`] because a constructor pattern's argument
    /// is a `patbot`, *not* a full `patas` (`Some x as y` binds `y` to the
    /// whole value, not to `x`).
    PatBotErased => ast::PatBot;
    /// An [`ast::TypeExpr`] behind a stream-erasing parse (see above).
    TyErased => ast::TypeExpr;
    /// An [`ast::MathElemCst`] behind a stream-erasing parse (see above).
    /// Unlike the other three erasers this one isn't bridging a *self*-loop
    /// of its target's own SCC — `MathElemCst` turns out to have no direct
    /// self-loop at all (see its doc comment) — but every nested reference
    /// to "one math element" still goes through here, for the same
    /// monomorphize-once reason.
    MathErased => ast::MathElemCst;
    /// An [`ast::AppArg`] behind a stream-erasing parse (see above). Bridges
    /// a command tail's argument chain (`CmdTail::Args`, below) into
    /// `AppArg`'s own parser *without* a direct field reference: `CmdTail` is
    /// reached from `Expr`'s SCC via `Atomic::InlineText`/`BlockText` ->
    /// `InlineElem`/`BlockElem` -> `CmdTail`, so a *direct* `AppArg` field
    /// here would close a brand-new cycle back into `Atomic`
    /// (`AppArg::Atom.atom: Atomic`) entirely through non-root types — the
    /// exact "sub-cycle running entirely through non-root types" shape the
    /// `#[recurse]` engine rejects (see `PatCons`'s doc comment for the same
    /// hazard). Routing through this eraser keeps `CmdTail` a DAG leaf, same
    /// as every other cross-reference here.
    AppArgErased => ast::AppArg;
}

/// The recursive expression/pattern/type/text grammar. Program expressions
/// embed inline/block text (`{…}`, `'<…>`), text embeds commands, and
/// command arguments re-enter program expressions.
///
/// **Recursion structure.** Grammatically this is one big knot, but at the
/// type level every recursion edge except three self-loops is routed through
/// the stream-erasing leaf wrappers defined above ([`ExprErased`],
/// [`PatErased`], [`TyErased`]) — see their doc comment for the measured
/// compile-time blowup that forced this. The `#[recurse]` macro therefore
/// sees exactly three singleton SCCs, each a directly self-referential root:
///
/// * `Expr` (its own variants' `Box<Expr>` children — `nxlet` nesting like
///   `if … then if … else …` runs on the engine);
/// * `PatBot` (`CtorApplied`'s `Box<PatBot>` argument — `Some Some x`);
/// * `TypeExpr` (`Fun`'s right-recursive `Box<TypeExpr>` codomain).
///
/// Every sub-cycle trivially passes through its root. All other nesting
/// (command arguments, parenthesized/tuple bodies, record/list elements,
/// match-arm bodies, …) recurses at *runtime* through the erasers'
/// hand-written `Parse` impls, which is unbounded by construction.
///
/// A command's arguments are still represented as one application-chain
/// `Expr` (`CmdTail::Args`) rather than a dedicated argument list — faithful
/// to the OCaml AST, where command arguments are a curried `UTApply` chain
/// anyway. Elaboration flattens that chain back into the argument list.
#[syan::parse::recurse]
pub mod ast {
    use super::super::leaf::*;
    use syan::parse::{Parse, Unparse};

    /// `nxlet`: a let/if/match/lambda-headed expression, falling through to
    /// the flattened operator chain (`Ops`, `OpChain`) at the bottom.
    /// Variant order is parse priority; `Ops` has no distinguishing leading
    /// keyword, so it must stay last.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Expr {
        /// `let-rec name param* = expr (and name param* = expr)* in body`
        LetRecIn {
            kw: KwLetRec,
            first: RecBinding,
            ands: Vec<AndBinding>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let name param* = expr in body` (`nxletsub`'s `LETNONREC` case;
        /// the bound TARGET is a plain variable — a general pattern target
        /// is [`Expr::LetPatternIn`], below — but `param*` is a full
        /// `patbot*`, matching `parser.mly`'s `nxnonrecdec` (and this port's
        /// own `TopLet`/`Fun`/`RecBinding`, which already use `PatBot` here
        /// too): e.g. `hdecoset.satyh`'s `let deco _ _ _ _ = [] in ..`
        /// (Tier-2 decoration/graphics wave). Lowered by the same
        /// `elaborate::rec_clause_value` single-clause path `Fun`'s doc
        /// comment describes.
        LetIn {
            kw: KwLet,
            name: super::BindName,
            /// Optional `: ty` ascription (`let f : ty x = e in ..`) —
            /// parse-and-ignore, like [`RecBinding::ascription`].
            ascription: Option<RecAscription>,
            /// The `|` of `nonrecdecargpart` — see
            /// [`super::TopLet::leading_bar`], whose doc comment carries the
            /// whole story; this is the expression-level twin.
            leading_bar: Option<BarTok>,
            params: Vec<Param>,
            eq: DefEqTok,
            value: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let pat = value in body` (`nxnonrecdec`'s zero-additional-
        /// parameter case: the bound target is a general pattern, not
        /// merely a variable name — SATySFi's destructuring `let`, e.g.
        /// `let (_, acc) = pair in acc`, used by the bundled
        /// `list.satyg`'s `mapi-adjacent`). Kept as a SEPARATE variant from
        /// [`Expr::LetIn`] above (rather than widening `LetIn`'s `name:
        /// VarTok` field to a pattern) because `LetIn` additionally curries
        /// `params` for the ordinary `let f x y = ..` function-definition
        /// shape, which upstream keys off the bound target being a plain
        /// variable — the two shapes never overlap in real source (a
        /// destructuring target is never itself applied to further curried
        /// parameters). **Must stay after `LetIn`**: a bare-variable target
        /// like `let x = 1 in x` parses as this variant too (`PatBot::Var`),
        /// so `LetIn` (tried first, and not needing any pattern-lowering)
        /// wins for every ordinary `let`, leaving this variant to match only
        /// when the target isn't a plain variable. Only the no-`argpart`
        /// (no additional curried parameters after the pattern) form is
        /// implemented — `nxnonrecdec`'s `argpart` has no use in the
        /// bundled stdlib.
        LetPatternIn {
            kw: KwLet,
            pat: super::PatErased,
            eq: DefEqTok,
            value: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `if cond then a else b` (`nxif`; `else` is never optional in
        /// this grammar, so there is no dangling-else ambiguity).
        If {
            kw: KwIf,
            cond: Box<Expr>,
            then_kw: KwThen,
            then_branch: Box<Expr>,
            else_kw: KwElse,
            else_branch: Box<Expr>,
        },
        /// `fun x y -> body` (`nxlambda`'s `LAMBDA argpats ARROW nxlor`
        /// production, `parser.mly:713`). `argpats = list(patbot)`
        /// upstream — a lambda's parameters are full `patbot`s, not merely
        /// variables (e.g. the bundled `list.satyg`'s `mapi-adjacent`:
        /// `fun (i, acc) x leftopt rightopt -> ..`, a tuple-DESTRUCTURING
        /// first parameter), lowered by `curry_lambda_abstract_pattern` —
        /// this port's `elaborate::rec_clause_value` (shared with
        /// multi-clause `let-rec`, which faces the exact same
        /// arity-preserving pattern-currying problem) reproduces that
        /// directly, so this field is `PatBot`, matching `RecBinding`'s.
        Fun {
            kw: KwFun,
            params: Vec<PatBot>,
            arrow: ArrowTok,
            body: Box<Expr>,
        },
        /// `fun ?(l = x, …) p -> body` — a SATySFi 0.1 labeled-optional
        /// lambda unit (one `?(…)` bundle + one positional param). This is
        /// an **additive** 0.1 node: 0.0.6 has no `?(…)` param bundle, so it
        /// is reachable in a 0.0.6 parse only for input that used to be a
        /// parse error (a leading `?` cannot begin `Fun`'s `Vec<PatBot>`),
        /// where `elaborate` rejects it under a V0_0 [`crate::version`]
        /// gate. The V0_1 pipeline reaches it by lowering a `cst_v1` param
        /// bundle (multi-unit lambdas lower to a nested `FunRows`/`Fun`
        /// chain). Placed right after [`Expr::Fun`] so a plain `fun x -> …`
        /// still matches `Fun` first (its `?`-headed `opts` cannot begin a
        /// `PatBot`, so `Fun` cleanly backtracks here for a bundled unit).
        FunRows {
            kw: KwFun,
            opts: CstOptBinders,
            param: PatBot,
            arrow: ArrowTok,
            body: Box<Expr>,
        },
        /// `match scrutinee with [|] pat [when g] -> body (| pat [when g] -> body)*`
        Match {
            kw: KwMatch,
            scrutinee: Box<Expr>,
            with_kw: KwWith,
            leading_bar: Option<BarTok>,
            first: MatchArm,
            rest: Vec<BarArm>,
        },
        /// `let-mutable name <- init in body` (`nxletsub`'s `LETMUTABLE`
        /// case; `init`/`body` are both `nxlet` in `parser.mly`, simplified
        /// here to a direct `Expr` self-loop like `LetIn`).
        LetMutableIn {
            kw: KwLetMutable,
            name: VarTok,
            arrow: OverwriteEqTok,
            init: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let-math \cmd param* = expr in body` (`nxletsub`'s `LETMATH`
        /// case, `parser.mly:688` — upstream's ONLY command binding with an
        /// expression-level `in` form; `LETHORZ`/`LETVERT` stay
        /// top-level-only, see the module doc comment on
        /// [`super::TopBinding::LetInline`]/`LetBlock`). Same shape as
        /// [`super::TopBinding::LetMath`] — no leading context variable,
        /// `cmd` reuses the plain `HorzCmdTok` token — plus the `in body`
        /// suffix; `Box<Expr>` self-loops on the recurse root like `LetIn`.
        LetMathIn {
            kw: KwLetMath,
            cmd: HorzCmdTok,
            params: Vec<Param>,
            eq: DefEqTok,
            value: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `open Name in body` (`nxletsub`'s `OPEN` case).
        OpenIn {
            kw: KwOpen,
            name: CtorTok,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `while cond do body` (`nxwhl`; `body` is `nxwhl` itself in
        /// `parser.mly`, i.e. right-nested `while`s — simplified here to a
        /// plain `Expr`).
        WhileDo {
            kw: KwWhile,
            cond: Box<Expr>,
            do_kw: KwDo,
            body: Box<Expr>,
        },
        /// `name <- value` (`nxlambda`'s `OVERWRITEEQ` case). Starts with a
        /// bare `VarTok`, which is also how `Ops` can start (`x` alone) —
        /// **must** stay before `Ops` so backtracking tries the `<-` shape
        /// first. `value` is `nxlor` in `parser.mly`; routed through
        /// `ExprErased` here rather than mirrored precisely, both to keep
        /// `Expr` a singleton SCC and because this is already a `Var`-headed
        /// alternative sitting awkwardly among the keyword-headed ones.
        Overwrite {
            name: VarTok,
            arrow: OverwriteEqTok,
            value: super::ExprErased,
        },
        /// The flattened binary-operator chain — see the module doc comment
        /// on precedence flattening. Must stay last (no leading keyword).
        Ops(OpChain),
    }

    /// One `name [: ty] [|] patbot* = value [| patbot* = value]*` clause
    /// GROUP of a `let-rec` (also reused, from outside this module, by
    /// top-level `let-rec`). `ascription` is `parser.mly`'s rarer
    /// `COLON ty` type-annotated form (`recdecargpart`'s `COLON ty BAR`
    /// alternative), e.g. the bundled `itemize.satyh`'s `let-rec
    /// listing-item : context -> int -> bool -> bool -> itemize ->
    /// block-boxes | ctx depth is-first is-last (Item(...)) = ..`. Parsed
    /// but not enforced: this milestone has no signature-*enforcement* pass
    /// for value-level ascriptions (only module `val`/`direct` signature
    /// items reach `typecheck.rs`'s `command_scheme`/sig machinery), so the
    /// ascription is simply a documented parse-and-ignore stand-in — its
    /// only job is making verbatim upstream source parse. `params` is
    /// `patbot*` (`recdecargpart`'s plain `argpats` form, optionally
    /// preceded by a `leading_bar` — `recdecargpart`'s `BAR argpatlst`
    /// alternative, used both for the OCaml-style "every clause, including
    /// the first, gets a `|`" layout the bundled packages write, e.g.
    /// `list.satyg`'s `let-rec map\n  | f [] = []\n  | f (x :: xs) = ..`,
    /// and for the `COLON ty BAR` form above, whose single clause is *only*
    /// reachable via a leading `|`). `extra` holds any further
    /// `| patbot* = value` continuation clauses (`nxrecdecpar`) — SATySFi's
    /// multi-clause pattern-matching function-definition sugar. Every
    /// clause in the group must bind the same number of parameters (checked
    /// at elaboration — upstream's `IllegalArgumentLength` — not here); the
    /// (possibly plural) clauses desugar to one curried function that
    /// matches a tuple of fresh parameters against each clause's patterns
    /// in turn — see `elaborate.rs`'s `rec_clause_value`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecBinding {
        pub name: super::BindName,
        pub ascription: Option<RecAscription>,
        pub leading_bar: Option<BarTok>,
        pub params: Vec<PatBot>,
        pub eq: DefEqTok,
        pub value: super::ExprErased,
        pub extra: Vec<RecClause>,
    }

    /// A `let-rec` binding's optional `: ty` ascription (see [`RecBinding`]'s
    /// doc comment). A direct (non-erased) `TypeExpr` field: `RecBinding` is
    /// already inside this `#[recurse]` module (embedded directly by
    /// `Expr::LetRecIn`, not through an eraser), and connecting it straight
    /// to `TypeExpr` — one of the module's three self-recursive SCC roots —
    /// is exactly the same kind of cross-root DAG edge `RecBinding.params:
    /// Vec<PatBot>` already makes to the `PatBot` root; `TypeExpr` never
    /// refers back to `Expr`/`PatBot`/`RecBinding`, so no new cycle results.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecAscription {
        pub colon: ColonTok,
        pub ty: TypeExpr,
    }

    /// A `| patbot* = value` continuation clause of a multi-clause
    /// `let-rec` binding (see [`RecBinding`]'s doc comment).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecClause {
        pub bar: BarTok,
        pub params: Vec<PatBot>,
        pub eq: DefEqTok,
        pub value: super::ExprErased,
    }

    /// An `and name param* = value` continuation of a `let-rec`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AndBinding {
        pub and_kw: KwAnd,
        pub binding: RecBinding,
    }

    /// One `pat [when guard] -> body` match arm. The pattern and body sit
    /// behind the stream-erasing wrappers (deref to reach the inner nodes).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct MatchArm {
        pub pat: super::PatErased,
        pub guard: Option<Guard>,
        pub arrow: ArrowTok,
        pub body: super::ExprErased,
    }

    /// A match arm's `when cond` guard.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct Guard {
        pub when_kw: KwWhen,
        pub cond: super::ExprErased,
    }

    /// A `| pat [when guard] -> body` continuation of a match's arm list.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct BarArm {
        pub bar: BarTok,
        pub arm: MatchArm,
    }

    /// A flattened binary-operator chain: `head (op rhs)*`, left-folded
    /// (with correct per-operator precedence/associativity) during
    /// elaboration. `before` is `nxbfr`'s postfix (`e1 before e2`), attached
    /// here rather than modeled at its own precedence level: `nxbfr` sits
    /// between `nxif` and `nxlambda`, i.e. *above* `nxlor`/`OpChain`'s own
    /// level, so `parser.mly`'s left operand is actually `nxlambda` (which
    /// also covers `Fun`/`Overwrite`) — attaching to `OpChain` alone misses
    /// `(fun x -> e1) before e2`/`(x <- e1) before e2` as the left operand;
    /// such input is rejected here (a documented simplification, not a
    /// silent misparse). `body` is threaded through `ExprErased` (not
    /// boxed directly) to keep `Expr` a singleton SCC: a direct `Box<Expr>`
    /// field on `OpChain` would make `OpChain` itself part of `Expr`'s SCC
    /// (a second, non-`Expr`-variant self-loop edge), which is exactly the
    /// multi-type-cycle shape the module doc warns about.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OpChain {
        pub head: AppExpr,
        pub tail: Vec<OpRhs>,
        pub before: Option<BeforeTail>,
    }

    /// The `before body` suffix of an [`OpChain`] (`nxbfr`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct BeforeTail {
        pub kw: KwBefore,
        pub body: super::ExprErased,
    }

    /// One `op rhs` continuation of an [`OpChain`].
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OpRhs {
        pub op: BinOpTok,
        pub rhs: AppExpr,
    }

    /// `nxun`/`nxapp`/`nxunsub` flattened: an optional leading unary minus,
    /// an optional leading `!`/`!!`/... deref (`UNOP_EXCLAM`, `nxunsub`), an
    /// atomic head with any `#label` field accesses (`nxbot ACCESS var`,
    /// left-recursive in `parser.mly` — flattened to a postfix `Vec` here,
    /// the same technique as `PatCons`'s `::`), and an application-chain
    /// tail (`nxapp nxunsub` / `nxapp CONSTRUCTOR` / `nxapp OPTIONAL
    /// nxunsub` / `nxapp OMISSION`, left-folded during elaboration). `not`
    /// is not implemented yet; `EXACT_AMP`/`EXACT_TILDE` (`&`/`~`) are the
    /// staging prefixes, carried in `stage` (see [`StagePrefix`]). First-class command references (`command \cmd`,
    /// upstream's `nxapp: COMMAND hcmd`) are modeled one level down, as
    /// [`Atomic::Command`] — see its doc comment for the rationale.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AppExpr {
        pub minus: Option<ExactMinusTok>,
        pub stage: Option<StagePrefix>,
        pub excl: Option<UnopExclamTok>,
        pub head: Atomic,
        pub head_accesses: Vec<AccessSeg>,
        pub args: Vec<AppArg>,
    }

    /// A staging prefix on a `nxunsub` operand: `&e` builds code for the next
    /// stage, `~e` splices the result of a previous-stage computation
    /// (`parser.mly:796-797`, `UTNext`/`UTPrev`).
    ///
    /// Upstream spells these as alternatives of `nxunsub`, alongside the `!`
    /// deref; this grammar flattens that level, so like `minus`/`excl` they
    /// become an optional prefix field. The looser shape accepts a few
    /// combinations upstream's grammar does not (`&!x`), which is the same
    /// latitude `AppExpr` already takes for `-!x`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum StagePrefix {
        /// `&e` — quote: the value is `e`'s code, to run one stage later.
        Next(ExactAmpTok),
        /// `~e` — splice: run `e` now and drop its code in here.
        Prev(ExactTildeTok),
    }

    /// One `#label` field-access segment (`nxbot ACCESS var`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AccessSeg {
        pub hash: AccessTok,
        pub label: VarTok,
    }

    /// One application-chain argument: an optional-argument value (`?:
    /// arg`), an omitted optional argument (`?*`), a plain atomic value
    /// (with its own optional `!` prefix and `#access` suffixes, mirroring
    /// `AppExpr`'s head position — each `nxunsub`/`nxbot` in the `nxapp`
    /// chain is independent), or a bare constructor applied nullarily
    /// (`nxapp CONSTRUCTOR`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum AppArg {
        /// `?: arg` (`nxapp OPTIONAL nxunsub`, simplified to a bare
        /// `Atomic` operand rather than the full `nxunsub`).
        Optional { q: OptionalTok, value: Atomic },
        /// `?*` (`nxapp OMISSION`).
        Omission(OmissionTok),
        Atom {
            stage: Option<StagePrefix>,
            excl: Option<UnopExclamTok>,
            atom: Atomic,
            accesses: Vec<AccessSeg>,
        },
        Ctor(CtorTok),
        /// `?(l = e, …) atom` — a SATySFi 0.1 labeled-optional application
        /// bundle paired with the positional argument it precedes (pairing
        /// them in one arm rejects a dangling trailing bundle `f x ?(l=1)` at
        /// parse time, as upstream does). Additive 0.1 node; the `?(`-head is
        /// token-disjoint from every 0.0.6 `AppArg` arm (0.0.6's `Optional`
        /// is `?:`-headed, a distinct token), so no previously-parsing input
        /// changes shape. Elaboration rejects it under a V0_0 version gate.
        Bundled {
            opts: CstOptArgs,
            excl: Option<UnopExclamTok>,
            atom: Atomic,
            accesses: Vec<AccessSeg>,
        },
        /// `?(l = e, …) Ctor` — as [`AppArg::Bundled`] but the positional
        /// argument is a bare constructor.
        BundledCtor { opts: CstOptArgs, ctor: CtorTok },
    }

    /// A SATySFi 0.1 `?(l = x, …)` optional-parameter binder bundle (for
    /// [`Expr::FunRows`]): the `?` sigil, then a parenthesized `,`-separated
    /// list of `label = binder` entries.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstOptBinders {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub entries: Vec<CstOptBinderEntry>,
    }

    /// One `label = binder` entry of a [`CstOptBinders`] bundle (the last
    /// `,` is optional; `=` is upstream's `EXACT_EQ`, reusing [`DefEqTok`]).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstOptBinderEntry {
        pub label: VarTok,
        pub eq: DefEqTok,
        pub var: VarTok,
        pub comma: Option<CommaTok>,
    }

    /// A SATySFi 0.1 `?(l = e, …)` optional-argument bundle (for
    /// [`AppArg::Bundled`]/[`AppArg::BundledCtor`]): the `?` sigil, then a
    /// parenthesized `,`-separated list of `label = expr` entries.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstOptArgs {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub entries: Vec<CstOptArgEntry>,
    }

    /// One `label = expr` entry of a [`CstOptArgs`] bundle — a FULL
    /// expression (`?(bias = 1 + n)`), routed through [`super::ExprErased`]
    /// so this satellite never joins `Expr`'s SCC.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstOptArgEntry {
        pub label: VarTok,
        pub eq: DefEqTok,
        pub value: super::ExprErased,
        pub comma: Option<CommaTok>,
    }

    /// `nxbot` (plus the ctor-head case usually found in `nxun`): an atomic
    /// expression.
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
        /// `Mod.x` — a module-qualified variable (`VARWITHMOD`).
        VarWithMod(VarWithModTok),
        /// `( ‹op› )` — a bare reference to a (possibly user-defined)
        /// operator as a first-class value, e.g. `(+++)`, `(-->)`
        /// (`nxbot`'s `LPAREN binop RPAREN` alternative — the same syntax
        /// `super::BindName` accepts in binding position, here used as an
        /// ordinary atomic expression). Resolves via the same name
        /// `BinOpTok::op_text` yields, exactly like `Var` (`elaborate.rs`).
        OpRef(OpNameTok),
        /// `command \cmd` (upstream `nxapp: COMMAND hcmd` →
        /// `UTContentOf(mods, csnm)`): a first-class *value* that simply
        /// names an inline command's own binding — no argument tail, so
        /// modeling it as an atom (rather than upstream's `nxapp` level)
        /// is strictly simpler and covers every bundled usage (always
        /// parenthesized, e.g. `(command \math)`). Only the horizontal
        /// form is spelled upstream; if a package ever writes `command
        /// +cmd`/a math form, extend with an `AnyVertCmdTok`/math
        /// alternative then (see `class-signature-lang-gaps.md` R1).
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
        /// `Mod.(e)` ≡ `open Mod in e` (`nxbot`'s `OPENMODULE nxlet RPAREN`
        /// production). Reuses `ParenBody` exactly like `Atomic::Paren`
        /// above (so `Mod.(e, e, …)` would elaborate to a tuple the same
        /// way, though no bundled package writes it that way) — the `Mod.(`
        /// sigil is the open delimiter (`OpenModuleTok`, carrying the
        /// module name), closed by a plain `)`. Elaborated via the same
        /// machinery as `Expr::OpenIn` (`elaborate.rs`'s `open_module`
        /// helper).
        OpenModule {
            grp: OpenModuleGroup<()>,
            #[group(self.grp)]
            body: Box<ParenBody>,
        },
        /// `(| label = expr; … |)` or `(| base with label = expr; … |)`
        /// (`nxrecordsynt`; see [`RecordBody`]).
        Record {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            body: RecordBody,
        },
        /// `[ expr; … ]`
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
        /// `${ math }` (`nxbot`'s `BMATHGRP mathblock EMATHGRP` case).
        MathText {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErased>,
        },
    }

    /// `(| … |)`'s content: either a plain field list, or a *record update*
    /// `base with l = e; …` (`nxrecordsynt`'s third alternative). `Update`
    /// is tried first (it backtracks cleanly to `Fields` — parsing `base`
    /// as an expression stops right before a bare `label = expr`'s `=`,
    /// since `=` isn't a valid expression continuation, so the `with`
    /// keyword check fails fast and `Fields` picks it up). `base` is
    /// `nxbot` in `parser.mly` (an atomic expression); routed through
    /// `ExprErased` here instead, which is strictly more permissive
    /// (accepts any expression as the base, not just an atomic one) — a
    /// deliberate simplification, and also the only way to reference it
    /// without adding a second, non-`Group` recursion edge into `Atomic`
    /// (which — like `AppExpr`/`OpChain` — is *not* itself part of `Expr`'s
    /// SCC, and should stay that way).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum RecordBody {
        Update {
            base: super::ExprErased,
            with_kw: KwWith,
            fields: Vec<RecordField>,
        },
        Fields(Vec<RecordField>),
    }

    /// The parenthesized-expression group's content: one expression, plus
    /// any `, expr` continuations (present only for a tuple).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct ParenBody {
        pub first: super::ExprErased,
        pub rest: Vec<CommaExpr>,
    }

    /// A `, expr` continuation inside a parenthesized tuple.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CommaExpr {
        pub comma: CommaTok,
        pub value: super::ExprErased,
    }

    /// One record field `label = expr;` (the last `;` is optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecordField {
        pub name: VarTok,
        pub eq: DefEqTok,
        pub value: super::ExprErased,
        pub semi: Option<ListPunctTok>,
    }

    /// One list element `expr;` (the last `;` is optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct ListItem {
        pub value: super::ExprErased,
        pub semi: Option<ListPunctTok>,
    }

    /// One inline-text element (`ih`/`ihtext`/`ihcmd` in parser.mly).
    ///
    /// `ItemBullet`/`Sep` are flat markers rather than the nested tree
    /// `parser.mly` builds in-grammar (`sxsep`'s `nonempty_list(sxitem)` /
    /// `sxlist`): since `InlineText`'s content is already a flat
    /// `Vec<InlineElem>`, regrouping consecutive `ItemBullet`-headed runs
    /// into an itemize tree (and `Sep`-delimited runs into columns, for
    /// tabular/math use later) is deferred to the elaborator. Token-level
    /// round-tripping is unaffected either way.
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
        /// `${ math }` — embeds math content as inline text (`ihcmd`'s
        /// `BMATHGRP mathblock EMATHGRP` case).
        EmbedMath {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErased>,
        },
        /// `\cmd …` (`name` also accepts the module-qualified
        /// `\Mod.cmd` form).
        Cmd { name: AnyHorzCmdTok, tail: CmdTail },
        /// An itemize bullet (`*`+) marker — see the variant-group doc above.
        ItemBullet(ItemTok),
        /// A `|` separator marker — see the variant-group doc above.
        Sep(SepTok),
    }

    /// One block-text element (`vxbot`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum BlockElem {
        /// `#var;` — embeds a program variable's value as block content.
        Embed { var: VarInVertTok, semi: EndActiveTok },
        /// `+cmd …` (`name` also accepts the module-qualified
        /// `+Mod.cmd` form).
        Cmd { name: AnyVertCmdTok, tail: CmdTail },
    }

    /// A command's arguments (`narg* sargs` in parser.mly, upstream's own
    /// dedicated grammar — *not* a reuse of the general application chain
    /// like phase-2's `AppExpr`). Either a bare `;` (no arguments) or a flat,
    /// non-empty sequence of [`AppArg`]s: each is `?: value` (a supplied
    /// optional `narg`), `?*` (an omitted optional `narg`), or a plain
    /// (possibly `!`/`#access`-decorated) atomic value — `(expr)`,
    /// `(|record|)`, `[list]`, `{inline}`, `<block>`, a bare ctor, etc. —
    /// covering both `narg`'s mandatory forms and `sargs`'s group forms
    /// uniformly (this port's usual simplification: `AppArg::Atom`'s
    /// `Atomic` already spans every shape upstream splits across `narg`/
    /// `sargs`). Optional/omitted `narg`s may lead (`\ref?:(x){text}`,
    /// `\ref?*{text}`) since every element is independently one `AppArg`,
    /// unlike the old `Expr`-based encoding whose head could only ever be a
    /// plain atom.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum CmdTail {
        /// `;` — no arguments.
        Semi(EndActiveTok),
        /// The argument chain: at least one [`AppArg`], via [`super::AppArgErased`].
        Args {
            first: super::AppArgErased,
            rest: Vec<super::AppArgErased>,
            semi: Option<EndActiveTok>,
        },
    }

    /// `patas`: a pattern, plus an optional `as name` binding.
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

    /// One curried parameter of an ordinary (non-`let-rec`) `let`, or of a
    /// `let-inline`/`let-block`/`let-math` command binding — upstream's
    /// `arg` nonterminal (`nxnonrecdec`'s `argpart`/`cmdarglst`,
    /// `parser.mly:622-624`: `arg: patbot | OPTIONAL defedvar`): a full
    /// pattern, or the def-site optional-parameter marker `?:name`
    /// (`parser.mly`'s `OPTIONAL vartok`), e.g. `stdja.satyh`'s `let
    /// document record ?:configopt inner = ..` and `annot.satyh`'s
    /// `let-inline ctx \href ?:borderopt uri inner = ..`. Upstream's
    /// `let-rec`/`fun` argument grammar (`recdecargpart`/`argpats` —
    /// [`RecBinding`]/[`AndBinding`]/`Expr::Fun`) has no such alternative,
    /// only plain `let` and the three command-binding forms do — all four
    /// keep `Vec<Param>` ([`super::TopLet`], `Expr::LetIn`,
    /// [`super::TopBinding::LetInline`]/`LetBlock`/`LetMath`,
    /// [`Expr::LetMathIn`]). Elaborated (`elaborate.rs`) by widening
    /// `Optional` to `PatBot::Var` (`params_to_patbots`) before the ordinary
    /// pattern-currying machinery runs (plain `let`'s `rec_clause_value`, or
    /// a command binding's `curry_cmd_params`) — the `?:` marker carries no
    /// further semantics of its own in this port (`typecheck.rs`'s
    /// `command_scheme` doc comment: optionality is inferred structurally,
    /// not from this marker); for a command binding, the maximal *leading*
    /// run of `?:`-marked params is additionally counted by `elaborate.rs`'s
    /// `leading_optional_count` and recorded into the binding's
    /// `Scope::optional_arity`, so a marker-less call site can auto-omit
    /// those slots (see `cmd_args`/`math_bot`'s `Cmd` arm).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Param {
        Optional { q: OptionalTok, name: VarTok },
        Pat(PatBot),
        /// A SATySFi 0.1 `?(l = x, …)` labeled-optional command-parameter
        /// bundle (optional-arg-rows increment 3a), lowered from
        /// `cst_v1::Param { opts: Some(_), body }` by
        /// `v1/lower.rs::lower_command_params`. Reuses [`CstOptBinders`]
        /// verbatim (the same node a value-level `fun ?(l = x) p -> ..`
        /// bundle lowers to, increment 1) — `?(`-headed, so distinct from
        /// the 0.0.6 `?:`-headed [`Param::Optional`] above (no arm overlap,
        /// no grammar ambiguity: this variant is never PARSED directly by
        /// this 0.0.6-frozen `cst.rs`, only ever *constructed* by the 0.1
        /// lowering path). Consumed by `elaborate.rs`'s bundle-aware
        /// `curry_cmd_params_v1`, which emits `Ast::LambdaOpt` for it — see
        /// that function's doc comment.
        Bundled { opts: CstOptBinders, body: PatBot },
    }

    /// `pattr`: a `patbot`, followed by any number of `:: patbot` segments.
    /// `parser.mly` writes this as right recursion (`patbot :: pattr`, always
    /// fine, unlike left recursion) but it is flattened to a `Vec` here (the
    /// same right-fold-at-elaboration technique as `OpChain`, `::` being
    /// right-associative) so that `PatCons` need not be self-referential: a
    /// `PatCons`/`ConsRest` pair of mutually-referencing wrapper structs
    /// would form a 2-cycle with no self-loop of its own, which the
    /// `#[recurse]` depth engine rejects ("a sub-cycle running entirely
    /// through non-root types").
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

    /// `patbot`, plus the constructor-pattern forms `pattr` adds in
    /// `parser.mly` (folded in here to keep `PatCons` a plain struct).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum PatBot {
        /// `Ctor patbot` — a constructor applied to one argument pattern.
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
        /// pattern).
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: Box<PatternParenBody>,
        },
        /// `[ pat; … ]` (also matches `[]`).
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
        pub first: super::PatErased,
        pub rest: Vec<CommaPattern>,
    }

    /// A `, pat` continuation inside a parenthesized tuple pattern.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CommaPattern {
        pub comma: CommaTok,
        pub value: super::PatErased,
    }

    /// One list-pattern element `pat;` (the last `;` is optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct PatListItem {
        pub value: super::PatErased,
        pub semi: Option<ListPunctTok>,
    }

    /// A minimal type-expression grammar for `type` declarations and
    /// signature (`val .. : ty`) annotations (`txfunc`/`txprod`/`txapppre`/
    /// `txapp`/`txbot`, simplified). Function arrows (right-associative,
    /// with an optional-argument `?->` prefix chain — see [`OptArrowDom`]),
    /// 2+-way product types (`*`, [`TypeProd`]), a SINGLE-argument postfix
    /// type-constructor application (`'a option`, `'a list`; see
    /// [`TypeApp`]), command-argument-list types (`[ty; ty?; ..]
    /// inline-cmd`/`block-cmd`/`math-cmd`; see [`TypeAtom::Cmd`]),
    /// parenthesized grouping, closed record types (`(| l : ty; … |)`; see
    /// [`TypeAtom::Record`]), bare/qualified names, and type variables are
    /// supported; N-ary applied constructors are not — such input is
    /// rejected with a parse error. Self-recursive only through `Fun`'s
    /// codomain (right recursion); parenthesized nesting goes through the
    /// [`super::TyErased`] leaf.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeExpr {
        /// `opts?-> dom -> cod` (right-associative). `dom` is a [`TypeProd`]
        /// (not just [`TypeAtom`]) so e.g. `'a option -> 'b option` and `'a *
        /// 'b -> 'c` both parse at their expected precedence (application
        /// binds tighter than `*`, which binds tighter than `->`). `opts` is
        /// upstream's `txfuncopts` prefix (`parser.mly:880-882`): zero or
        /// more `ty ?->` domains greedily consumed *before* the final
        /// mandatory `dom -> cod`, e.g. `config ?-> block-text -> document`
        /// parses as `opts = [config]`, `dom = block-text`, `cod =
        /// document`. Lowered (`typecheck.rs`) to an `option`-wrapped
        /// mandatory domain per optional entry — see that module's doc
        /// comment on `lower_type_expr`.
        Fun {
            opts: Vec<OptArrowDom>,
            dom: TypeProd,
            arrow: ArrowTok,
            cod: Box<TypeExpr>,
        },
        /// The non-arrow fallthrough. Despite the name (kept stable — see
        /// the module's compile-time-blowup note on why every recursion
        /// edge here is deliberate), this holds a full [`TypeProd`], not a
        /// bare [`TypeAtom`]: a product/application with no enclosing arrow
        /// is still just "the whole type expression minus `->`".
        Atom(TypeProd),
        /// `?(l1 : ty1, …) dom -> cod` — a SATySFi 0.1 labeled-optional
        /// function TYPE domain (upstream `typ`'s second production,
        /// `parser_v1.mly:688-691`; "optional-arg-rows increment 2"). Lowered
        /// (`typecheck.rs`) to `MonoType::Func(Row::Cons(l1, ty1, …
        /// Row::Empty), dom, cod)` — a CLOSED row, matching what
        /// `Ast::LambdaOpt` infers (increment 1), so an explicit `?(l:τ)->`
        /// signature unifies against an actual `?(l=x)`-taking function.
        /// `?`-headed — token-disjoint from `Fun`/`Atom` (neither
        /// [`TypeProd`] nor [`TypeAtom`] can start with [`OptionalTypeTok`]),
        /// so declared order is safety-neutral; appended last, this file's
        /// convention for 0.1 additions. Additive 0.0.6 accept-surface
        /// widening (this whole track's established §7 pattern): a 0.0.6
        /// program containing `?(l : int) -> int` NOW parses (previously a
        /// parse error — no old `TypeExpr` arm starts with `?`) and reaches
        /// `typecheck.rs::lower_type_expr`'s version gate, which rejects it
        /// under `V0_0` with a version-error message (an improvement over
        /// the old parse error).
        OptRowFun {
            opt_dom: CstTypeOptDom,
            dom: TypeProd,
            arrow: ArrowTok,
            cod: Box<TypeExpr>,
        },
    }

    /// `?(l = ty, …)` — the closed labeled-optional-domain prefix of
    /// [`TypeExpr::OptRowFun`]. No row-variable-tail field: row-tailed
    /// optional domains need signature-level row quantification
    /// (`parser_v1.mly`'s `rowquant`/`quant`) — L4/2d territory, not this
    /// increment; `cst_v1`'s own `TypeOptDomInnerV1` models the tail at parse
    /// level and rejects it with a `LowerError` before ever reaching here
    /// (`v1/lower.rs`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstTypeOptDom {
        pub q: OptionalTypeTok,
        pub paren: ParenGroup<()>,
        #[group(self.paren)]
        pub entries: Vec<CstTypeOptEntry>,
    }

    /// One `label : ty,` entry of a [`CstTypeOptDom`] (last `,` optional —
    /// matching the 0.1 lowering convention this file's other additive nodes
    /// use, e.g. [`CstOptArgEntry`], rather than the frozen grammar's `;`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstTypeOptEntry {
        pub label: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErased,
        pub comma: Option<CommaTok>,
    }

    /// One `ty ?->` leading domain of a [`TypeExpr::Fun`]'s optional-argument
    /// prefix (`parser.mly`'s `txfuncopts`, 880-882).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct OptArrowDom {
        pub ty: TypeProd,
        pub arrow: OptionalArrowTok,
    }

    /// `txprod`: one or more `*`-separated [`TypeApp`]s (a product type),
    /// or just a single one if there's no `*` at all — flattened to a
    /// `Vec` (the same deferred-fold technique as `OpChain`/`PatCons`)
    /// rather than modeled as its own right-recursive rule, keeping
    /// `TypeExpr` a singleton SCC.
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

    /// `txapp` — a postfix type application `arg1 arg2 … argN ctor`, upstream's
    /// N-ary chain flattened into a greedy atom run (the way `OpChain`/`PatCons`
    /// flatten their own left-recursions). [`head`](TypeApp::head) is always
    /// present; when [`rest`](TypeApp::rest) is non-empty the LAST atom is the
    /// type constructor (a bare or `Mod.`-qualified name — `list`/`option`/
    /// `result`/`Eq.t`/`implicit`) and every atom before it (including `head`)
    /// is one of its arguments. This is unambiguous because SATySFi always
    /// parenthesizes a nested single-argument application (`('a list) list`,
    /// `('a t) implicit` — never `'a list list`), so a flat run of atoms can
    /// only be one constructor applied to the preceding arguments:
    ///
    /// - `int` → `head = int`, `rest = []` (a bare atom).
    /// - `'a option` → `head = 'a`, `rest = [option]` (one arg).
    /// - `'a 'e result` → `head = 'a`, `rest = ['e, result]` (`satysfi-base`'s
    ///   two-parameter `result`/`either`/`t`/`map`).
    /// - `('a Eq.t) implicit` → `head = ('a Eq.t)`, `rest = [implicit]`
    ///   (`satysfi-base`'s typeclass-dictionary marker).
    ///
    /// Elaboration (`typecheck::lower_type_app`) does the head/args/ctor split;
    /// the grammar itself is a plain, always-terminating greedy `Vec<TypeAtom>`
    /// (each atom consumes ≥1 token, stopping at the first non-atom — `->`,
    /// `*`, `)`, `;`, …). Both the unqualified and `Mod.`-qualified constructor
    /// forms fall out for free, since a `Mod.t` ctor is just a
    /// [`TypeAtom::NameMod`] like any other atom.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeApp {
        pub head: TypeAtom,
        pub rest: Vec<TypeAtom>,
    }

    /// An atomic type expression.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeAtom {
        /// `[ty; ty?; ..] inline-cmd` / `block-cmd` / `math-cmd`
        /// (`parser.mly`'s `txapppre` command-type productions, 903-919) —
        /// tried first: unambiguous, since no other `TypeAtom` starts with
        /// `[` (`BListTok`). Each `;`-separated element is a [`TypeCmdArgItem`].
        Cmd {
            list: ListGroup<()>,
            #[group(self.list)]
            args: Vec<TypeCmdArgItem>,
            kind: CmdTypeKind,
        },
        /// `( ty )`
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: super::TyErased,
        },
        /// `(| l1 : ty1; l2 : ty2; … |)` — a closed record type (`txbot`'s
        /// `txrecord` case, `parser.mly:955-961`), lowered to
        /// `MonoType::Record` (a `Row::Cons` chain ending in `Row::Empty` —
        /// see `typecheck.rs`'s `lower_type_atom`). Distinguished from
        /// [`TypeAtom::Paren`] (opens on plain `LParenTok`, i.e. `(`) and
        /// from a record-VALUE expression ([`Atomic::Record`], a different
        /// grammar position — only reachable where an `Expr` is expected,
        /// never in type position) purely by lexer-level delimiter token:
        /// `(|`/`|)` lex as the dedicated `BRecordTok`/`ERecordTok` pair
        /// (same as [`RecordKind`]'s use at `constraint 'a :: (|…|)`), so no
        /// backtracking between any of these three shapes is needed.
        Record {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            fields: Vec<TypeRecordField>,
        },
        /// A type variable, e.g. `'a`.
        Var(TypeVarTok),
        /// An unqualified type name, e.g. `int`, `string`.
        Name(VarTok),
        /// `Mod.t` — a bare module-qualified type name in atomic (0-ary,
        /// non-applied) position, e.g. `Eq.t` in `val eq : Eq.t -> Eq.t ->
        /// ordering`. Sibling of [`Name`](TypeAtom::Name) (not a widened
        /// field on it); as the last atom of a [`TypeApp`] it is a
        /// module-qualified type constructor (`int M.t`, `ordering Eq.t`).
        /// **Tried after `Name`** only by placement convention (the two are
        /// token-disjoint: `VarTok`/`VarWithModTok` are separate lexer
        /// tokens, so there is no real backtracking ambiguity between them).
        NameMod(VarWithModTok),
        /// `(| l1 : ty1, … | ?'r |)` — a SATySFi 0.1 OPEN record type: a
        /// row-variable tail after the fields (upstream `typ_bot`'s SECOND
        /// `L_RECORD`/`R_RECORD` production, `parser_v1.mly:748-749`;
        /// "optional-arg-rows increment 2"). Lowered (`typecheck.rs`) to
        /// `MonoType::Record(Row::Cons(l1, ty1, … Row::Var(fresh)))` — the
        /// row variable unifies structurally as an open record's tail
        /// (permitting additional fields at the unification site), reusing
        /// the existing generic `Row`/`RowVarRef`/`unify_row` machinery — no
        /// new type machinery needed. Genuinely a NEW shape, not a widening
        /// of the frozen [`TypeAtom::Record`] (0.0.6's `txrecord` grammar has
        /// no row-var tail at all, confirmed by grep of upstream
        /// `parser.mly`) — additive, appended after `Record`/`Var`/`Name`.
        /// Comma-separated fields, matching this file's other 0.1 additive
        /// nodes ([`CstOptArgEntry`], [`CstOptBinderEntry`]) rather than the
        /// frozen `Record`'s upstream-0.0.6 `;` separator. Unreachable from a
        /// `V0_0` token stream by construction: [`RowVarTok`] is only ever
        /// emitted by the lexer under [`crate::version::RustyfiVersion::
        /// V0_1`] (`lexer.rs`'s `'?'` arm), so no 0.0.6 parse can ever
        /// produce this variant — no elaborate/typecheck-time version gate
        /// is needed here (contrast [`TypeExpr::OptRowFun`], which IS
        /// reachable from 0.0.6 lexing and so DOES need one).
        // NOTE the group field is `orec`, not `rec`: syan names a group
        // substruct after (group-field name, ENUM name) with no variant
        // component, so a second `rec` group in `TypeAtom` collides with
        // `Record`'s (E0428 + E0119, and the survivor has the wrong fields).
        RecordOpen {
            orec: RecordGroup<()>,
            #[group(self.orec)]
            inner: CstRecordOpenInner,
        },
    }

    /// A [`TypeAtom::RecordOpen`]'s group content: one or more `,`-separated
    /// fields (nonempty enforced at lowering, matching the closed form),
    /// then a mandatory `| ?'r` row-variable tail.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstRecordOpenInner {
        pub fields: Vec<CstRecordOpenField>,
        pub bar: BarTok,
        pub var: RowVarTok,
    }

    /// One `l : ty,` field of a [`TypeAtom::RecordOpen`] (last `,` optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct CstRecordOpenField {
        pub name: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErased,
        pub comma: Option<CommaTok>,
    }

    /// One `l : ty;` field of a [`TypeAtom::Record`] (`txrecord`,
    /// `parser.mly:962-965`) — sibling of [`super::RecordKindField`], but
    /// (unlike that struct, defined *outside* the `#[recurse]` module and so
    /// free to hold a direct `ast::TypeExpr` field) this one lives inside
    /// `TypeAtom`'s own SCC, so the field type is routed through
    /// [`super::TyErased`] instead — a direct `ast::TypeExpr` field here
    /// would close a fresh cycle back through `TypeAtom` itself (the same
    /// hazard [`TypeCmdArgItem`]'s doc comment explains).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeRecordField {
        pub name: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErased,
        pub semi: Option<ListPunctTok>,
    }

    /// One `;`-separated element of a [`TypeAtom::Cmd`]'s bracketed argument
    /// list: a mandatory `ty`, or an optional `ty?` (`parser.mly`'s `txlist`,
    /// 955-960) — routed through [`super::TyErased`] rather than the
    /// narrower `TypeApp` upstream uses, both to stay a DAG leaf (a direct
    /// `TypeApp` field here would close `TypeAtom -> Cmd -> ... -> TypeApp ->
    /// TypeAtom`, a fresh cycle through non-root types — see
    /// `AppArgErased`'s doc comment for the identical hazard) and per this
    /// port's usual permissive-superset simplification.
    ///
    /// `opt_labels` (optional-arg-rows increment 3a) is the lowered
    /// `?(l:τ,…)` command-type row PREFIX on this slot (`TypeCmdOptDomV1` at
    /// the `cst_v1` side): a flat list of `label : ty` fields, no wrapping
    /// `?(` sigil/group of its own at this (already-lowered) target — purely
    /// a data carrier, populated by `v1/lower.rs::lower_type_cmd_args` and
    /// read by `typecheck.rs`'s `lower_type_atom` `Cmd` arm. Since no real
    /// 0.0.6 `TypeCmdArgItem` position can ever contain a bare `label :`
    /// shape (0.0.6's grammar has no colon here at all), every 0.0.6-parsed
    /// fixture yields `opt_labels == []`, byte-identical to before this
    /// field existed.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeCmdArgItem {
        pub opt_labels: Vec<TypeCmdOptField>,
        pub ty: super::TyErased,
        pub opt: Option<OptionalTypeTok>,
        pub semi: Option<ListPunctTok>,
    }

    /// One `label : ty,` field of a [`TypeCmdArgItem::opt_labels`] bundle
    /// (optional-arg-rows increment 3a; the last `,` is optional, matching
    /// this port's other 0.1-additive comma-separated satellite fields —
    /// [`CstOptBinderEntry`], [`CstTypeOptEntry`]).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct TypeCmdOptField {
        pub label: VarTok,
        pub colon: ColonTok,
        pub ty: super::TyErased,
        pub comma: Option<CommaTok>,
    }

    /// The command-type keyword closing a [`TypeAtom::Cmd`]'s bracketed list.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum CmdTypeKind {
        Inline(HorzCmdTypeTok),
        Block(VertCmdTypeTok),
        Math(MathCmdTypeTok),
    }

    /// `mathtop`: one math element, i.e. a `mathbot` base with any postfix
    /// `^`/`_`/`'` script combos (`mathtop`'s seven alternatives, flattened
    /// to a `Vec` in source order — the same `Ops`/`OpChain` deferred-
    /// precedence technique, since combos 3–6 interleave sub/superscript
    /// application order in a way elaboration is better placed to resolve).
    ///
    /// **No direct self-loop.** Unlike `Expr`/`PatBot`/`TypeExpr`, this
    /// grammar corner needs no fourth singleton SCC at all: `scripts` is a
    /// `Vec`, so an empty run already degenerates to plain `mathbot`, and
    /// `mathbot`'s only recursive spot (`{ … }` re-entering `mathmain`) is
    /// threaded through `MathErased` exactly like every *other* nested
    /// reference to "one math element" (`matharg`'s math-mode argument,
    /// `Atomic::MathText`'s program-mode embed, `InlineElem::EmbedMath`'s
    /// inline-text embed, `MathGroupArg`'s `{ … }` script operand). So
    /// `MathElemCst` ends up structurally acyclic within `#[recurse]`'s SCC
    /// analysis — like `OpChain`/`AppExpr`/`Atomic` — and is monomorphized
    /// exactly once (one stream type) regardless of how many distinct call
    /// sites embed math content. This is *safer* than carving out a real
    /// self-loop would have been, not a shortcut: every recursive edge is
    /// erased, so there is no bounded-depth engine to blow up in the first
    /// place.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct MathElemCst {
        pub base: MathBot,
        pub scripts: Vec<MathScript>,
    }

    /// `mathbot`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathBot {
        /// `\cmd matharg*` (`mcmd list(matharg)`), sigil-only or
        /// module-qualified (`\Mod.cmd matharg*`).
        Cmd { name: AnyMathCmdTok, args: Vec<MathArg> },
        Chars(MathCharTok),
        /// `#var` (`VARINMATH`; math mode never trails this with `;` —
        /// unlike `#var;` in inline/block text, the lexer doesn't switch to
        /// an active mode here).
        Embed(VarInMathTok),
        /// A `|` separator marker (flat; elaborator regroups, e.g. for
        /// tabular/matrix columns — `mathblock`'s `SEP mathlist` case).
        Sep(SepTok),
        /// `{ … }` — re-enters `mathmain` (`mathgroup`'s `BMATHGRP mathmain
        /// EMATHGRP` case, reached here via `mathbot`). Content is erased
        /// (see [`MathElemCst`]'s doc comment).
        Group {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErased>,
        },
    }

    /// One postfix script combo of a [`MathElemCst`] (`mathtop`'s
    /// `SUPERSCRIPT`/`SUBSCRIPT`/`PRIMES` suffixes, one at a time).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathScript {
        /// `^ group`
        Super { hat: SuperscriptTok, group: MathGroupArg },
        /// `_ group`
        Sub { under: SubscriptTok, group: MathGroupArg },
        /// A run of `'` marks — sugar for a superscript of primes
        /// characters; kept as its own token (not desugared here) since
        /// elaboration already special-cases it per `parser.mly`.
        Primes(PrimesTok),
    }

    /// `mathgroup`: a script's operand is either a bracketed math group or a
    /// bare `mathbot`.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathGroupArg {
        Group {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErased>,
        },
        Bot(Box<MathBot>),
    }

    /// `matharg` (parser.mly:1138-1146 + narg 1201-1210): one math-mode
    /// command argument — a mandatory body, a `?:`-supplied optional
    /// (UTOptionalArgument), or `?*` (UTOmission). The six body shapes live
    /// once in [`MathArgBody`]; Optional/Omission/Plain are first-token-disjoint.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathArg {
        Optional { q: OptionalTok, body: MathArgBody },
        Omission(OmissionTok),
        Plain(MathArgBody),
    }

    /// The six body shapes shared by mandatory and `?:`-optional math args:
    /// a math/inline/block group, or a `!`-escaped program-mode value. The
    /// lexer already switches mode on the escape sigil (`!(` / `![` / `!(|` /
    /// `!{` / `!<` all emit ordinary `LParen`/`BList`/`BRecord`/`BHorzGrp`/
    /// `BVertGrp` tokens — see `lexer.rs`'s `lex_math`), so at the token
    /// level the escapes are indistinguishable from `Atomic`'s own
    /// `Paren`/`List`/`Record` shapes; reusing those bodies directly here
    /// (rather than going through a full `ExprErased`, which would also
    /// happily swallow a *following* `matharg` bracket group as a trailing
    /// application argument) keeps each `matharg` exactly one bracket group.
    /// NOT `Box<MathArg>`: a direct self-loop on a non-root type is what
    /// `#[recurse]` rejects, and upstream's grammar is non-recursive here
    /// anyway.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathArgBody {
        /// `{ math }`.
        Math {
            mgrp: MathGroup<()>,
            #[group(self.mgrp)]
            elems: Vec<super::MathErased>,
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
        /// `![e; …]`.
        ListEscape {
            list: ListGroup<()>,
            #[group(self.list)]
            items: Vec<ListItem>,
        },
        /// `!(|l = e; …|)`.
        RecordEscape {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            body: RecordBody,
        },
    }
}

/// A parse failure with the source position recovered from the failing parse.
///
/// The span is whatever syan's span-carrying [`ParseError`](syan::error::ParseError)
/// reports for the failure (recovered via `span_of::<Span>`); with our
/// [`Span::migrate`](crate::span::Span) being a union it covers the attempted
/// region rather than pinpointing a single token.
#[derive(Debug, thiserror::Error)]
#[error("{span}: parse error: {message}")]
pub struct ParseFileError {
    pub span: Span,
    pub message: String,
}

/// Lex and parse a whole `.saty` source file.
pub fn parse_file(src: &str) -> Result<File, ParseFileError> {
    let atoms = crate::lexer::lex(src).map_err(|e| ParseFileError {
        span: e.span,
        message: e.msg,
    })?;
    // The error carries the position it failed at, so no high-water mark is
    // needed: syan's `ParseError` is span-generic and every variant holds one.
    let mut stream = crate::stream::AtomStream::new(atoms);
    <File as Parse<_>>::parse(&mut stream).map_err(|e| ParseFileError {
        span: *e.span(),
        message: render_parse_error(&e),
    })
}

/// Flatten syan's nested error tree into one readable line.
fn render_parse_error(err: &syan::error::ParseError<Span>) -> String {
    format!("{err:?}")
}
