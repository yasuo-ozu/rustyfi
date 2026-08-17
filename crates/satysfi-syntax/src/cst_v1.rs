//! The Slice-1 SATySFi **0.1.0** (`dev-0-1-0`) surface grammar — a *fork* of
//! [`crate::cst`], not a version-gate of it (gating one shared `cst.rs` would
//! mean hand-writing `Parse` for nearly every node, destroying the derive
//! idiom and risking 0.0.6 on every 0.1 edit; see
//! `docs/plans/satysfi-0-1-0-support.md` §1.5). [`crate::cst`] stays frozen:
//! this module imports only its token-`Atom`-generic, non-recursive helpers
//! ([`crate::cst::Header`], [`crate::cst::ParseFileError`]) and re-declares
//! everything else — including its own `*ErasedV1` eraser leaves and its own
//! copy of `render_parse_error` — so that touching `cst_v1.rs` never touches
//! `cst.rs`.
//!
//! **Scope (Slice 1 only).** SATySFi 0.1's grammar adds a whole ML-style
//! module system (`bind`/`modexpr`/`sigexpr`/`decl`) on top of an expr/
//! pattern/type layer that is structurally close to 0.0.6's, plus five
//! surface deltas: `,`-separated lists/records (not `;`), `EXACT_EQ` for
//! both definitional and record `=` (reusing [`DefEqTok`] — no new `=`
//! leaf), mandatory `match … with … end`, per-binding staging instead of a
//! whole-file `@stage:` header, and no `when`/`while`/`before` at all. This
//! module builds *only* what the transliterated `stdja-mini.satyh` needs to
//! parse under 0.1 syntax:
//!
//! * [`FileV1`] — `header* expr EOI` or `header* module Name = struct
//!   bind* end EOI` (no signature annotation yet — see below).
//! * [`BindV1`] — the three `val` forms `stdja-mini` uses (`val`, `val
//!   inline`, `val block`); the full `Bind` (adding `rec`/`mutable`/`math`/
//!   `type`/`module`/`signature`/`include`) is deferred to roadmap phase 3.
//! * A copy of [`crate::cst::ast`]'s expr/pattern/type layer with the 0.1
//!   deltas applied (see each type's doc comment for the exact delta).
//!
//! **Deliberately NOT built yet** (roadmap phase 3, `docs/plans/satysfi-0-1-0
//! -support.md` §4.2): the `ModExpr`/`SigExpr` recursive roots, `Decl`, the
//! `StructBind`/`StructDeclV1` connector types, the `BindErased`/`DeclErased`
//! erasers, and `FileV1::Library`'s `sig_annot` field (a `module Name =
//! struct … end` library therefore never carries a `: sig … end`
//! constraint in Slice 1 — such input is rejected with a parse error, a
//! documented simplification). `Bind`'s "bound name is `LOWER | ( binop )`"
//! generality (upstream `bound_identifier`) is likewise simplified to a bare
//! [`VarTok`] here — none of Slice 1's binds need an operator-bound name.
//!
//! **The `#[recurse]` SCC story (Slice 1).** Exactly three singleton,
//! directly self-referential roots — the same shape [`crate::cst::ast`]
//! uses, for the same reason (see its module doc comment for the measured
//! compile-time blowup a naive transcription hits):
//!
//! * [`ast::Expr`] (its variants' own `Box<Expr>` children);
//! * [`ast::PatBot`] (`CtorApplied`'s `Box<PatBot>` argument);
//! * [`ast::TypeExpr`] (`Fun`'s right-recursive `Box<TypeExpr>` codomain).
//!
//! Every other recursion edge is routed through the erasers declared below
//! ([`ExprErasedV1`], [`PatErasedV1`], [`PatBotErasedV1`], [`TyErasedV1`],
//! [`MathErasedV1`]), keeping each SCC a singleton and the wrapped grammar
//! monomorphized exactly once behind [`crate::stream::EraseStream`]. When
//! phase 3 adds the module/sig layer, `ModExpr`/`SigExpr` join this same
//! `#[recurse]` module as two more self-loop roots (their cross-layer edges
//! into `Expr`/`TypeExpr` are already plain DAG edges through the existing
//! erasers), per the full design in the S5 spec.

use crate::leaf::*;
use crate::span::Span;
use newer_type::implement;
use syan::parse::{Parse, Unparse};

/// `@require:` / `@import:` header element — byte-identical between 0.0.6
/// and `dev-0-1-0` (this port's own confirmation; see
/// `docs/plans/satysfi-0-1-0-support.md` §1.1), so 0.1 simply reuses
/// [`crate::cst`]'s definition rather than re-declaring an identical enum.
/// 0.1 has no `@stage:` header at all (the shared lexer's `V0_1` path
/// rejects it outright — see `lexer.rs`'s `lex_header`), so
/// [`crate::cst::Header`]'s absence of a `Stage` variant costs nothing here.
pub use crate::cst::Header;

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
        headers: Vec<Header>,
        body: ast::Expr,
        eoi: EoiTok,
    },
    /// `header* MODULE UPPER EXACT_EQ STRUCT bind* END EOI`
    /// (`parser_v1.mly:372-375`, `main_lib`). No signature annotation in
    /// Slice 1 — see the module doc comment.
    Library {
        headers: Vec<Header>,
        module_kw: KwModule,
        name: CtorTok,
        eq: DefEqTok,
        struct_kw: KwStruct,
        binds: Vec<BindV1>,
        end_kw: KwEnd,
        eoi: EoiTok,
    },
}

/// One parameter of a Slice-1 [`BindV1`] (`param_unit`,
/// `parser_v1.mly:635-646`, Slice-1 subset). The full form additionally
/// allows a `?(l = e, …)` optional-argument bundle and a `( pat : typ )`
/// ascription; both are deferred (roadmap phase 2/4) — `stdja-mini`'s
/// binds only ever take plain patterns.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum Param {
    Pat(ast::PatBot),
}

/// Slice-1 subset of `bind` (`parser_v1.mly:415-440`): the three `val`
/// forms `stdja-mini` uses. The full `Bind` (adding `rec`/`mutable`/`math`
/// binds, `type`/`module`/`signature`/`include` declarations, and
/// per-binding staging) is deferred to roadmap phase 3 — see the module doc
/// comment. Every arm's `=` is `EXACT_EQ` ([`DefEqTok`]) and body is an
/// [`ast::Expr`]; `name`/`ctx` are plain [`VarTok`]s (see the module doc
/// comment on the `bound_identifier` simplification).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum BindV1 {
    /// `VAL bind_value_nonrec` (`parser_v1.mly:416,442,459-465`): `val
    /// <name> <param>* = <expr>`.
    Value {
        kw: KwVal,
        name: VarTok,
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
                type Error = syan::error::ParseError;

                fn parse(
                    stream: impl syan::parse::IntoParseStream<Atom = crate::token::Atom>,
                ) -> Result<Self, Self::Error> {
                    let mut stream =
                        crate::stream::InfallibleAdapter(stream.into_parse_stream());
                    let mut erased = crate::stream::EraseStream::new(&mut stream);
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
}

/// The recursive expression/pattern/type/text grammar for SATySFi 0.1,
/// Slice 1. A copy of [`crate::cst::ast`] with the deltas documented on each
/// type; see the module doc comment for the SCC/root story.
#[syan::parse::recurse]
pub mod ast {
    use crate::leaf::*;
    use syan::parse::{Parse, Unparse};

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
    /// keyword) and, in Slice 1, is single-clause only (no `and`-chain —
    /// full mutual recursion is deferred to roadmap phase 3); a new
    /// `LetPatternIn` form covers `let pat = value in body` for any
    /// non-bare-variable pattern (`parser_v1.mly:796`, `pattern_non_var`);
    /// `open` requires a leading `let` (`parser_v1.mly:798`, `LET OPEN
    /// UPPER IN`) where 0.0.6 allows a bare `open Name in body`; and
    /// `WhileDo`, the `Guard`/`when` match-arm suffix, and `OpChain`'s
    /// `before` postfix are dropped entirely — SATySFi 0.1's grammar has no
    /// `WHEN`/`WHILE`/`BEFORE` tokens at all (confirmed by grep of
    /// `parser_v1.mly`). `Overwrite` (`name <- value`) is kept unchanged
    /// (`parser_v1.mly:810-812`, `REVERSED_ARROW`) — it is unrelated to the
    /// removed `before` postfix.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Expr {
        /// `let rec name param* = value in body` (Slice-1 single clause;
        /// `parser_v1.mly:794` dispatching to `bind_value_rec`,
        /// `:455-458`).
        LetRecIn {
            let_kw: KwLet,
            rec_kw: KwRec,
            name: VarTok,
            params: Vec<VarTok>,
            eq: DefEqTok,
            value: Box<Expr>,
            in_kw: KwIn,
            body: Box<Expr>,
        },
        /// `let name param* = value in body` (only a plain variable target
        /// is supported here — a general pattern falls through to
        /// [`Expr::LetPatternIn`]).
        LetIn {
            kw: KwLet,
            name: VarTok,
            params: Vec<VarTok>,
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
        /// `fun x y -> body`.
        Fun {
            kw: KwFun,
            params: Vec<VarTok>,
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

    /// One application-chain argument — identical shape to
    /// [`crate::cst::ast::AppArg`] (no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum AppArg {
        /// `?: arg`.
        Optional { q: OptionalTok, value: Atomic },
        /// `?*`.
        Omission(OmissionTok),
        Atom {
            excl: Option<UnopExclamTok>,
            atom: Atomic,
            accesses: Vec<AccessSeg>,
        },
        Ctor(CtorTok),
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
        Unit { paren: ParenGroup<()> },
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
    /// [`crate::cst::ast::CmdTail`] (no 0.1 delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum CmdTail {
        /// `;` — no arguments.
        Semi(EndActiveTok),
        /// The argument chain.
        Args {
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
        Unit { paren: ParenGroup<()> },
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

    /// A minimal type-expression grammar (`typ`/`typ_prod`/`typ_app`/
    /// `typ_bot`, `parser_v1.mly:685-752`, drastically simplified — same
    /// scope as [`crate::cst::ast::TypeExpr`], "basic" per the S5 spec's
    /// acceptance table: row polymorphism (`?'r`, open records) is
    /// deferred). Self-recursive only through `Fun`'s codomain (right
    /// recursion); parenthesized nesting goes through [`super::TyErasedV1`].
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum TypeExpr {
        /// `dom -> cod` (right-associative). This field is `TypeExpr`'s own
        /// self-loop (the root SCC).
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
            inner: super::TyErasedV1,
        },
        /// A type variable, e.g. `'a`.
        Var(TypeVarTok),
        /// A (possibly qualified) type name, e.g. `int`, `string`.
        Name(VarTok),
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
    /// delta).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum MathBot {
        /// `\cmd matharg*`.
        Cmd { name: MathCmdTok, args: Vec<MathArg> },
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
}

/// Lex ([`crate::lexer::lex_with_version`] under [`crate::version::SatysfiVersion::V0_1`])
/// and parse a whole 0.1 `.saty`/`.satyh` source file. Mirrors
/// [`crate::cst::parse_file`]'s two-step shape exactly, sharing its
/// [`crate::cst::ParseFileError`] (no new error type).
pub fn parse_file_v1(src: &str) -> Result<FileV1, crate::cst::ParseFileError> {
    let atoms = crate::lexer::lex_with_version(src, crate::version::SatysfiVersion::V0_1)
        .map_err(|e| crate::cst::ParseFileError {
            span: e.span,
            message: e.msg,
        })?;
    <FileV1 as Parse<_>>::parse(atoms).map_err(|e| crate::cst::ParseFileError {
        span: e.span_of::<Span>().unwrap_or_default(),
        message: render_parse_error(&e),
    })
}

/// Flatten syan's nested error tree into one readable line. A private copy
/// of [`crate::cst`]'s identical helper (not `pub(crate)` there, and
/// `cst.rs` stays untouched — see the module doc comment).
fn render_parse_error(err: &syan::error::ParseError) -> String {
    format!("{err:?}")
}
