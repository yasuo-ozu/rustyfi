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
use crate::stream::TokenStream;
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

/// `@require:` / `@import:` header element. Accepted and currently ignored.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum Header {
    Require(HeaderRequireTok),
    Import(HeaderImportTok),
}

/// A top-level non-recursive binding: `let name param* = expr`.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TopLet {
    pub let_kw: KwLet,
    pub name: VarTok,
    pub params: Vec<VarTok>,
    pub eq: DefEqTok,
    pub value: ast::Expr,
}

/// One top-level declaration (`nxtoplevel`/`nxstruct`'s per-declaration
/// alternatives). `LetInline`/`LetBlock` only exist here — see the module
/// doc comment.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum TopBinding {
    /// `let-rec name param* = expr (and name param* = expr)*`
    LetRec {
        kw: KwLetRec,
        first: ast::RecBinding,
        ands: Vec<ast::AndBinding>,
    },
    /// `let name param* = expr`
    Let(TopLet),
    /// `[ctxvar] let-inline \cmd param* = expr` (`nxhorzdec`; only a plain
    /// variable-parameter list is supported, not full argument patterns).
    LetInline {
        kw: KwLetHorz,
        ctx: Option<VarTok>,
        cmd: HorzCmdTok,
        params: Vec<VarTok>,
        eq: DefEqTok,
        value: ast::Expr,
    },
    /// `[ctxvar] let-block +cmd param* = expr` (`nxvertdec`).
    LetBlock {
        kw: KwLetVert,
        ctx: Option<VarTok>,
        cmd: VertCmdTok,
        params: Vec<VarTok>,
        eq: DefEqTok,
        value: ast::Expr,
    },
    /// `type name = [|] Ctor [of ty] (| Ctor [of ty])*` (`nxvariantdec`).
    Type(TypeDecl),
    /// `let-mutable name <- expr` (top-level; `nxtoplevel`/`nxstruct`'s
    /// `LETMUTABLE` case — the local, `in`-bodied form is
    /// [`ast::Expr::LetMutableIn`]).
    LetMutable {
        kw: KwLetMutable,
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
    type Error = syan::error::ParseError;

    fn parse(
        stream: impl syan::parse::IntoParseStream<Atom = crate::token::Atom>,
    ) -> Result<Self, Self::Error> {
        let mut stream = crate::stream::InfallibleAdapter(stream.into_parse_stream());
        let mut erased = crate::stream::EraseStream::new(&mut stream);
        let value = <TopBinding as Parse<_>>::parse(&mut erased)?;
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

/// One `nxsigelem`. Only the plain, unconstrained forms are supported
/// (`constrnts`/type parameters/type synonyms are not) — such input is
/// rejected with a parse error.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum SigItem {
    /// `val \cmd : ty` / `val +cmd : ty`.
    ValHorzCmd {
        kw: KwVal,
        name: HorzCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
    },
    ValVertCmd {
        kw: KwVal,
        name: VertCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
    },
    /// `val name : ty`.
    Val {
        kw: KwVal,
        name: VarTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
    },
    /// `direct \cmd : ty` / `direct +cmd : ty`.
    DirectHorzCmd {
        kw: KwDirect,
        name: HorzCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
    },
    DirectVertCmd {
        kw: KwDirect,
        name: VertCmdTok,
        colon: ColonTok,
        ty: ast::TypeExpr,
    },
    /// `type tyvar* name` (no constraints, no synonym).
    Type {
        kw: KwType,
        tyvars: Vec<TypeVarTok>,
        name: VarTok,
    },
}

/// A minimal `type` declaration. `parser.mly`'s `nxvariantdec` additionally
/// supports type parameters used non-trivially, mutual (`and`) recursion
/// between type declarations, and type *synonyms* (`type t = ty` with no
/// variants at all) — none of those are implemented yet; such input is
/// rejected with a parse error.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub kw: KwType,
    pub tyvars: Vec<TypeVarTok>,
    pub name: VarTok,
    pub eq: DefEqTok,
    pub leading_bar: Option<BarTok>,
    pub first: VariantDef,
    pub rest: Vec<BarVariantDef>,
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
/// grammar behind [`crate::stream::EraseStream`] so it is monomorphized
/// exactly once crate-wide. Defined *outside* the `#[recurse]` module so the
/// macro treats them as opaque leaves (they never appear as cycle edges).
macro_rules! erased_leaf {
    ($($(#[$doc:meta])* $name:ident => $target:ty;)*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, PartialEq)]
            pub struct $name(pub Box<$target>);

            impl Parse<crate::token::Atom> for $name {
                type Error = syan::error::ParseError;

                fn parse(
                    stream: impl syan::parse::IntoParseStream<Atom = crate::token::Atom>,
                ) -> Result<Self, Self::Error> {
                    let mut stream =
                        crate::stream::InfallibleAdapter(stream.into_parse_stream());
                    let mut erased = crate::stream::EraseStream::new(&mut stream);
                    // From here on the stream type is fixed: exactly one
                    // monomorphization of the wrapped grammar, no matter how
                    // deep or varied the call sites.
                    let value = <$target as Parse<_>>::parse(&mut erased)?;
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

            impl std::ops::Deref for $name {
                type Target = $target;
                fn deref(&self) -> &$target {
                    &self.0
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
        /// only a plain variable target is supported, not a general
        /// pattern — `parser.mly`'s `nxnonrecdec` allows any `patbot`).
        LetIn {
            kw: KwLet,
            name: VarTok,
            params: Vec<VarTok>,
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
        /// `fun x y -> body`
        Fun {
            kw: KwFun,
            params: Vec<VarTok>,
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

    /// One `name param* = value` clause of a `let-rec` (also reused, from
    /// outside this module, by top-level `let-rec`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecBinding {
        pub name: VarTok,
        pub params: Vec<VarTok>,
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
    /// nxunsub` / `nxapp OMISSION`, left-folded during elaboration). `not`,
    /// `EXACT_AMP`/`EXACT_TILDE` (`&`/`~`-prefixed forms), and first-class
    /// command references (`command \cmd`) are not implemented yet.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct AppExpr {
        pub minus: Option<ExactMinusTok>,
        pub excl: Option<UnopExclamTok>,
        pub head: Atomic,
        pub head_accesses: Vec<AccessSeg>,
        pub args: Vec<AppArg>,
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
            excl: Option<UnopExclamTok>,
            atom: Atomic,
            accesses: Vec<AccessSeg>,
        },
        Ctor(CtorTok),
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
        /// `()`
        Unit { paren: ParenGroup<()> },
        /// `( expr )` or `( expr, expr, … )` (the latter elaborates to a
        /// tuple).
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: Box<ParenBody>,
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

    /// A command's arguments (`narg* sargs` in parser.mly). Either a bare `;`
    /// (no arguments) or the argument application chain — each argument is one
    /// `Atomic` of the chain: `(expr)`, `(|record|)`, `[list]`, `{inline}`,
    /// `<block>` — optionally `;`-terminated when the last argument is a
    /// program-mode one. The lexer's active-mode rules already reject
    /// everything else, so the chain shape is validated during elaboration.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum CmdTail {
        /// `;` — no arguments.
        Semi(EndActiveTok),
        /// The argument chain.
        Args {
            args: super::ExprErased,
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
        Unit { paren: ParenGroup<()> },
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

    /// A minimal type-expression grammar for `type` declarations
    /// (`txfunc`/`txapppre`/`txbot`, drastically simplified). Only function
    /// arrows, bare/qualified names, type variables, and parenthesized
    /// grouping are supported; product types (`*`), list/record/command
    /// types, optional-argument arrows (`?->`), and applied type
    /// constructors (`'a t`, `(int, int) t`) are not — such input is
    /// rejected with a parse error. Self-recursive only through `Fun`'s
    /// codomain (right recursion); parenthesized nesting goes through the
    /// [`super::TyErased`] leaf.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeExpr {
        /// `dom -> cod` (right-associative).
        Fun {
            dom: TypeAtom,
            arrow: ArrowTok,
            cod: Box<TypeExpr>,
        },
        Atom(TypeAtom),
    }

    /// An atomic type expression.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeAtom {
        /// `( ty )`
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: super::TyErased,
        },
        /// A type variable, e.g. `'a`.
        Var(TypeVarTok),
        /// A (possibly qualified) type name, e.g. `int`, `string`.
        Name(VarTok),
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
    /// exactly once (at `EraseStream`) regardless of how many distinct call
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
        /// `\cmd matharg*` (`mcmd list(matharg)`).
        Cmd { name: MathCmdTok, args: Vec<MathArg> },
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

    /// `matharg`: one command argument in math mode — a math/inline/block
    /// group, or a `!`-escaped program-mode value. The lexer already
    /// switches mode on the escape sigil (`!(` / `![` / `!(|` / `!{` /
    /// `!<` all emit ordinary `LParen`/`BList`/`BRecord`/`BHorzGrp`/
    /// `BVertGrp` tokens — see `lexer.rs`'s `lex_math`), so at the token
    /// level the escapes are indistinguishable from `Atomic`'s own
    /// `Paren`/`List`/`Record` shapes; reusing those bodies directly here
    /// (rather than going through a full `ExprErased`, which would also
    /// happily swallow a *following* `matharg` bracket group as a trailing
    /// application argument) keeps each `matharg` exactly one bracket group.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathArg {
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

/// A parse failure with the source position the parser got furthest to.
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
    let mut stream = TokenStream::new(atoms);
    <File as Parse<_>>::parse(&mut stream).map_err(|e| ParseFileError {
        span: stream.high_water_span(),
        message: render_parse_error(&e),
    })
}

/// Flatten syan's nested error tree into one readable line.
fn render_parse_error(err: &syan::error::ParseError) -> String {
    format!("{err:?}")
}
