//! Structural CST-to-CST transcription: `cst_v1::ast` -> `cst::ast`
//! (`docs/plans/satysfi-0-1-0-support.md` §3, the finale spec's §1-§3).
//!
//! **Strategy (§1 of the finale spec).** Rather than widening
//! `elaborate.rs`'s ~30 expression-lowering helpers to also walk
//! `cst_v1::ast` nodes (which would re-implement the entire recursive walk —
//! operator-precedence climbing, itemize regrouping, pattern currying — over
//! a second node type), this module converts a parsed [`cst_v1::FileV1`]
//! into ordinary [`cst::TopBinding`]s / [`cst::ast::Expr`], and the caller
//! (`compile_document_v1` in `lib.rs`) assembles one synthetic [`cst::File`]
//! — exactly the shape the CLI's `merge_program` already produces for
//! 0.0.6 — and hands it to the **untouched** `elaborate::elaborate_program`
//! -> `typecheck_with_version(V0_1)` -> `compile`/`eval` pipeline.
//!
//! **Why this is near-mechanical.** `cst_v1::ast` imports the very same
//! `satysfi_syntax::leaf` token types `cst::ast` uses — `VarTok`, `DefEqTok`,
//! `ParenGroup<()>`, `LengthTok`, keyword leaves, … are literally identical
//! types on both sides, so most of the transcription below *moves/clones
//! tokens*, it does not re-encode them. The one genuinely non-1:1 seam is
//! the `CmdTail` bridge (§3.3, [`lower_cmd_tail`]): `cst_v1` kept the older
//! "one application-chain `Expr`" argument encoding, while `cst.rs` has
//! since moved to a flat `AppArg` list.
//!
//! **What this module deliberately does NOT lower** (§3.4 of the finale
//! spec — a real user-facing [`LowerError`], never a panic):
//!
//! - `let rec` (`Expr::LetRecIn`) — needs `RecBinding` reshaping and
//!   Slice-1's single-clause-only semantics; roadmap phase 3.
//! - `?:`/`?*` application arguments (`AppArg::Optional`/`Omission`) — 0.1's
//!   optional arguments are *labeled rows* (`?(l = e)`), semantically
//!   different from 0.0.6's positional `?:` marker; transcribing would bake
//!   in the wrong semantics. Roadmap phase 4.
//! - 0.1 math (`Atomic::MathText`, `InlineElem::EmbedMath`, and the whole
//!   math-element layer) — **not** mechanical:
//!   [`satysfi_syntax::SatysfiVersion::math_is_split`] is `true` for `V0_1`,
//!   so 0.1's `${…}` produces a `math-text` value, not 0.0.6's unsplit
//!   `math` — a structural transcription would be semantically wrong.
//!   Roadmap phase 2.
//! - A module-qualified command name (`\Mod.cmd`/`+Mod.cmd`) in *binding*
//!   position — the cst target field (`TopBinding::LetInline::cmd` /
//!   `LetBlock::cmd`) is a bare `HorzCmdTok`/`VertCmdTok`.
//!
//! **`TypeExpr`/`TypeAtom`: no lowering function exists at all.** Unlike
//! every other node kind, `cst_v1::ast::TypeExpr` is *unreachable* from
//! `FileV1` in Slice 1 — `BindV1` has no type ascriptions and `Param` is
//! `Pat`-only — so there is nothing to lower it from.
//!
//! **Module-wrapper erasure (§3.2).** [`lower_file_v1`] on
//! `FileV1::Library { name, binds, .. }` drops the module name entirely and
//! lowers `binds` to FLAT, unqualified top-level [`cst::TopBinding`]s —
//! byte-for-byte the scoping the CLI's `merge_program` already gives every
//! 0.0.6 package prelude. The natural upgrade path once roadmap phase 3
//! makes modules real: wrap the binds in one `cst::TopBinding::Module`
//! instead of splicing them (a ~5-line change).

use satysfi_syntax::cst;
use satysfi_syntax::cst_v1::{self, ast as ast_v1};
use satysfi_syntax::leaf::*;
use satysfi_syntax::Span;

/// A 0.1 construct Slice 1 deliberately does not lower yet. A real user
/// error (not a panic): points at the construct and the roadmap.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{span}: SATySFi 0.1 construct not supported yet in this port's Slice 1: {construct} ({hint})")]
pub struct LowerError {
    pub construct: &'static str,
    pub hint: &'static str,
    pub span: Span,
}

fn unsupported(construct: &'static str, hint: &'static str, span: Span) -> LowerError {
    LowerError {
        construct,
        hint,
        span,
    }
}

fn unsupported_math(span: Span) -> LowerError {
    unsupported(
        "0.1 math (`${...}` / math-mode content)",
        "0.1 math is semantically split (`math-text`/`math-boxes`) — roadmap \
         phase 2 (the `math-text`/`math-boxes` split)",
        span,
    )
}

/// Lower one dependency library (`module Name = struct binds end`) to a
/// flat prelude fragment — Slice-1 module ERASURE, names unqualified (§3.2
/// of the finale spec). `FileV1::Document` input is a caller bug (the
/// loader's `DocumentAsDependency` check already rejects it before this is
/// ever reached): a `LowerError`, not a panic.
pub fn lower_file_v1(file: &cst_v1::FileV1) -> Result<Vec<cst::TopBinding>, LowerError> {
    match file {
        cst_v1::FileV1::Library { binds, .. } => {
            binds.iter().map(lower_bind_v1).collect::<Result<Vec<_>, _>>()
        }
        cst_v1::FileV1::Document { eoi, .. } => Err(unsupported(
            "a document file used as a dependency library",
            "the loader's DocumentAsDependency check should have rejected \
             this before lowering ever ran",
            eoi.0,
        )),
    }
}

/// Lower the entry document's body expression. `FileV1::Library` input is
/// the mirror-image caller bug (the loader's `LibraryAsEntry` check already
/// rejects it): a `LowerError`, not a panic.
pub fn lower_document_v1(file: &cst_v1::FileV1) -> Result<cst::ast::Expr, LowerError> {
    match file {
        cst_v1::FileV1::Document { body, .. } => lower_expr(body),
        cst_v1::FileV1::Library { end_kw, .. } => Err(unsupported(
            "a library file used as the entry document",
            "the loader's LibraryAsEntry check should have rejected this \
             before lowering ever ran",
            end_kw.0,
        )),
    }
}

// ---- BindV1 -----------------------------------------------------------

fn lower_bind_v1(b: &cst_v1::BindV1) -> Result<cst::TopBinding, LowerError> {
    match b {
        cst_v1::BindV1::Value {
            kw,
            name,
            params,
            eq,
            body,
        } => Ok(cst::TopBinding::Let(cst::TopLet {
            let_kw: KwLet(kw.0),
            name: name.clone().into(),
            params: lower_params(params)?,
            eq: eq.clone(),
            value: lower_expr(body)?,
        })),
        cst_v1::BindV1::ValueInline {
            kw,
            ctx,
            cmd,
            params,
            eq,
            body,
            ..
        } => Ok(cst::TopBinding::LetInline {
            kw: KwLetHorz(kw.0),
            ctx: ctx.clone(),
            cmd: plain_horz(cmd)?,
            params: lower_params(params)?,
            eq: eq.clone(),
            value: lower_expr(body)?,
        }),
        cst_v1::BindV1::ValueBlock {
            kw,
            ctx,
            cmd,
            params,
            eq,
            body,
            ..
        } => Ok(cst::TopBinding::LetBlock {
            kw: KwLetVert(kw.0),
            ctx: ctx.clone(),
            cmd: plain_vert(cmd)?,
            params: lower_params(params)?,
            eq: eq.clone(),
            value: lower_expr(body)?,
        }),
    }
}

fn plain_horz(name: &AnyHorzCmdTok) -> Result<HorzCmdTok, LowerError> {
    match name {
        AnyHorzCmdTok::Plain(t) => Ok(t.clone()),
        AnyHorzCmdTok::Mod(t) => Err(unsupported(
            "a module-qualified command name in binding position",
            "the cst target field (`LetInline::cmd`) is a bare `HorzCmdTok` \
             — roadmap phase 3 (real modules)",
            t.span,
        )),
    }
}

fn plain_vert(name: &AnyVertCmdTok) -> Result<VertCmdTok, LowerError> {
    match name {
        AnyVertCmdTok::Plain(t) => Ok(t.clone()),
        AnyVertCmdTok::Mod(t) => Err(unsupported(
            "a module-qualified command name in binding position",
            "the cst target field (`LetBlock::cmd`) is a bare `VertCmdTok` \
             — roadmap phase 3 (real modules)",
            t.span,
        )),
    }
}

fn lower_params(params: &[cst_v1::Param]) -> Result<Vec<cst::ast::Param>, LowerError> {
    params
        .iter()
        .map(|p| match p {
            cst_v1::Param::Pat(pb) => Ok(cst::ast::Param::Pat(lower_pat_bot(pb)?)),
        })
        .collect()
}

// ---- Expr ---------------------------------------------------------------

fn lower_expr(e: &ast_v1::Expr) -> Result<cst::ast::Expr, LowerError> {
    match e {
        ast_v1::Expr::LetRecIn { rec_kw, .. } => Err(unsupported(
            "`let rec`",
            "roadmap phase 3 (full `Bind`, mutual recursion)",
            rec_kw.0,
        )),
        ast_v1::Expr::LetIn {
            kw,
            name,
            params,
            eq,
            value,
            in_kw,
            body,
        } => Ok(cst::ast::Expr::LetIn {
            kw: kw.clone(),
            name: name.clone().into(),
            params: params
                .iter()
                .map(|v| cst::ast::Param::Pat(cst::ast::PatBot::Var(v.clone())))
                .collect(),
            eq: eq.clone(),
            value: Box::new(lower_expr(value)?),
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::LetPatternIn {
            kw,
            pat,
            eq,
            value,
            in_kw,
            body,
        } => Ok(cst::ast::Expr::LetPatternIn {
            kw: kw.clone(),
            pat: erase_pat(lower_pattern(pat)?),
            eq: eq.clone(),
            value: Box::new(lower_expr(value)?),
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::OpenIn {
            open_kw,
            name,
            in_kw,
            body,
            ..
        } => Ok(cst::ast::Expr::OpenIn {
            kw: open_kw.clone(),
            name: name.clone(),
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::If {
            kw,
            cond,
            then_kw,
            then_branch,
            else_kw,
            else_branch,
        } => Ok(cst::ast::Expr::If {
            kw: kw.clone(),
            cond: Box::new(lower_expr(cond)?),
            then_kw: then_kw.clone(),
            then_branch: Box::new(lower_expr(then_branch)?),
            else_kw: else_kw.clone(),
            else_branch: Box::new(lower_expr(else_branch)?),
        }),
        ast_v1::Expr::Fun {
            kw,
            params,
            arrow,
            body,
        } => Ok(cst::ast::Expr::Fun {
            kw: kw.clone(),
            params: params.iter().map(|v| cst::ast::PatBot::Var(v.clone())).collect(),
            arrow: arrow.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::Match {
            kw,
            scrutinee,
            with_kw,
            leading_bar,
            first,
            rest,
            ..
        } => Ok(cst::ast::Expr::Match {
            kw: kw.clone(),
            scrutinee: Box::new(lower_expr(scrutinee)?),
            with_kw: with_kw.clone(),
            leading_bar: leading_bar.clone(),
            first: lower_match_arm(first)?,
            rest: rest.iter().map(lower_bar_arm).collect::<Result<_, _>>()?,
        }),
        ast_v1::Expr::Overwrite { name, arrow, value } => Ok(cst::ast::Expr::Overwrite {
            name: name.clone(),
            arrow: arrow.clone(),
            value: erase_expr(lower_expr(value)?),
        }),
        ast_v1::Expr::Ops(chain) => Ok(cst::ast::Expr::Ops(lower_op_chain(chain)?)),
    }
}

fn lower_match_arm(a: &ast_v1::MatchArm) -> Result<cst::ast::MatchArm, LowerError> {
    Ok(cst::ast::MatchArm {
        pat: erase_pat(lower_pattern(&a.pat)?),
        guard: None,
        arrow: a.arrow.clone(),
        body: erase_expr(lower_expr(&a.body)?),
    })
}

fn lower_bar_arm(a: &ast_v1::BarArm) -> Result<cst::ast::BarArm, LowerError> {
    Ok(cst::ast::BarArm {
        bar: a.bar.clone(),
        arm: lower_match_arm(&a.arm)?,
    })
}

fn lower_op_chain(c: &ast_v1::OpChain) -> Result<cst::ast::OpChain, LowerError> {
    Ok(cst::ast::OpChain {
        head: lower_app_expr(&c.head)?,
        tail: c.tail.iter().map(lower_op_rhs).collect::<Result<_, _>>()?,
        before: None,
    })
}

fn lower_op_rhs(r: &ast_v1::OpRhs) -> Result<cst::ast::OpRhs, LowerError> {
    Ok(cst::ast::OpRhs {
        op: r.op.clone(),
        rhs: lower_app_expr(&r.rhs)?,
    })
}

fn lower_app_expr(e: &ast_v1::AppExpr) -> Result<cst::ast::AppExpr, LowerError> {
    Ok(cst::ast::AppExpr {
        minus: e.minus.clone(),
        excl: e.excl.clone(),
        head: lower_atomic(&e.head)?,
        head_accesses: e.head_accesses.iter().map(lower_access_seg).collect(),
        args: e.args.iter().map(lower_app_arg).collect::<Result<_, _>>()?,
    })
}

fn lower_access_seg(a: &ast_v1::AccessSeg) -> cst::ast::AccessSeg {
    cst::ast::AccessSeg {
        hash: a.hash.clone(),
        label: a.label.clone(),
    }
}

fn lower_app_arg(a: &ast_v1::AppArg) -> Result<cst::ast::AppArg, LowerError> {
    match a {
        ast_v1::AppArg::Optional { q, .. } => Err(unsupported(
            "an optional application argument (`?:`)",
            "0.1's optional arguments are labeled rows (`?(l = e)`), \
             semantically different from 0.0.6's `?:` marker — roadmap \
             phase 4 (labeled-optional rows)",
            q.0,
        )),
        ast_v1::AppArg::Omission(t) => Err(unsupported(
            "an omitted optional application argument (`?*`)",
            "0.1's optional arguments are labeled rows (`?(l = e)`), \
             semantically different from 0.0.6's `?*` marker — roadmap \
             phase 4 (labeled-optional rows)",
            t.0,
        )),
        ast_v1::AppArg::Atom {
            excl,
            atom,
            accesses,
        } => Ok(cst::ast::AppArg::Atom {
            excl: excl.clone(),
            atom: lower_atomic(atom)?,
            accesses: accesses.iter().map(lower_access_seg).collect(),
        }),
        ast_v1::AppArg::Ctor(t) => Ok(cst::ast::AppArg::Ctor(t.clone())),
    }
}

fn lower_atomic(a: &ast_v1::Atomic) -> Result<cst::ast::Atomic, LowerError> {
    match a {
        ast_v1::Atomic::Length(t) => Ok(cst::ast::Atomic::Length(t.clone())),
        ast_v1::Atomic::Float(t) => Ok(cst::ast::Atomic::Float(t.clone())),
        ast_v1::Atomic::Int(t) => Ok(cst::ast::Atomic::Int(t.clone())),
        ast_v1::Atomic::Literal(t) => Ok(cst::ast::Atomic::Literal(t.clone())),
        ast_v1::Atomic::True(t) => Ok(cst::ast::Atomic::True(t.clone())),
        ast_v1::Atomic::False(t) => Ok(cst::ast::Atomic::False(t.clone())),
        ast_v1::Atomic::Ctor(t) => Ok(cst::ast::Atomic::Ctor(t.clone())),
        ast_v1::Atomic::Var(t) => Ok(cst::ast::Atomic::Var(t.clone())),
        ast_v1::Atomic::VarWithMod(t) => Ok(cst::ast::Atomic::VarWithMod(t.clone())),
        ast_v1::Atomic::Command { kw, name } => Ok(cst::ast::Atomic::Command {
            kw: kw.clone(),
            name: name.clone(),
        }),
        ast_v1::Atomic::Unit { paren } => Ok(cst::ast::Atomic::Unit { paren: paren.clone() }),
        ast_v1::Atomic::Paren { paren, inner } => Ok(cst::ast::Atomic::Paren {
            paren: paren.clone(),
            inner: Box::new(lower_paren_body(inner)?),
        }),
        ast_v1::Atomic::Record { rec, body } => Ok(cst::ast::Atomic::Record {
            rec: rec.clone(),
            body: lower_record_body(body)?,
        }),
        ast_v1::Atomic::List { list, items } => Ok(cst::ast::Atomic::List {
            list: list.clone(),
            items: items.iter().map(lower_list_item).collect::<Result<_, _>>()?,
        }),
        ast_v1::Atomic::InlineText { igrp, elems } => Ok(cst::ast::Atomic::InlineText {
            igrp: igrp.clone(),
            elems: elems
                .iter()
                .map(lower_inline_elem)
                .collect::<Result<_, _>>()?,
        }),
        ast_v1::Atomic::BlockText { bgrp, elems } => Ok(cst::ast::Atomic::BlockText {
            bgrp: bgrp.clone(),
            elems: elems
                .iter()
                .map(lower_block_elem)
                .collect::<Result<_, _>>()?,
        }),
        ast_v1::Atomic::MathText { mgrp, .. } => Err(unsupported_math(mgrp.open.0)),
    }
}

fn lower_record_body(b: &ast_v1::RecordBody) -> Result<cst::ast::RecordBody, LowerError> {
    match b {
        ast_v1::RecordBody::Update {
            base,
            with_kw,
            fields,
        } => Ok(cst::ast::RecordBody::Update {
            base: erase_expr(lower_expr(base)?),
            with_kw: with_kw.clone(),
            fields: fields.iter().map(lower_record_field).collect::<Result<_, _>>()?,
        }),
        ast_v1::RecordBody::Fields(fields) => Ok(cst::ast::RecordBody::Fields(
            fields.iter().map(lower_record_field).collect::<Result<_, _>>()?,
        )),
    }
}

fn lower_record_field(f: &ast_v1::RecordField) -> Result<cst::ast::RecordField, LowerError> {
    Ok(cst::ast::RecordField {
        name: f.name.clone(),
        eq: f.eq.clone(),
        value: erase_expr(lower_expr(&f.value)?),
        // The `,` separator is dropped (`semi: None`) — harmless: the
        // synthetic tree this module builds is never unparsed. See the
        // finale spec §11's "Separator-token loss" risk note.
        semi: None,
    })
}

fn lower_paren_body(b: &ast_v1::ParenBody) -> Result<cst::ast::ParenBody, LowerError> {
    Ok(cst::ast::ParenBody {
        first: erase_expr(lower_expr(&b.first)?),
        rest: b.rest.iter().map(lower_comma_expr).collect::<Result<_, _>>()?,
    })
}

fn lower_comma_expr(c: &ast_v1::CommaExpr) -> Result<cst::ast::CommaExpr, LowerError> {
    Ok(cst::ast::CommaExpr {
        comma: c.comma.clone(),
        value: erase_expr(lower_expr(&c.value)?),
    })
}

fn lower_list_item(i: &ast_v1::ListItem) -> Result<cst::ast::ListItem, LowerError> {
    Ok(cst::ast::ListItem {
        value: erase_expr(lower_expr(&i.value)?),
        semi: None,
    })
}

fn lower_inline_elem(e: &ast_v1::InlineElem) -> Result<cst::ast::InlineElem, LowerError> {
    match e {
        ast_v1::InlineElem::Char(t) => Ok(cst::ast::InlineElem::Char(t.clone())),
        ast_v1::InlineElem::Space(t) => Ok(cst::ast::InlineElem::Space(t.clone())),
        ast_v1::InlineElem::Break(t) => Ok(cst::ast::InlineElem::Break(t.clone())),
        ast_v1::InlineElem::Embed { var, semi } => Ok(cst::ast::InlineElem::Embed {
            var: var.clone(),
            semi: semi.clone(),
        }),
        ast_v1::InlineElem::EmbedMath { mgrp, .. } => Err(unsupported_math(mgrp.open.0)),
        ast_v1::InlineElem::Cmd { name, tail } => Ok(cst::ast::InlineElem::Cmd {
            name: name.clone(),
            tail: lower_cmd_tail(tail)?,
        }),
        ast_v1::InlineElem::ItemBullet(t) => Ok(cst::ast::InlineElem::ItemBullet(t.clone())),
        ast_v1::InlineElem::Sep(t) => Ok(cst::ast::InlineElem::Sep(t.clone())),
    }
}

fn lower_block_elem(e: &ast_v1::BlockElem) -> Result<cst::ast::BlockElem, LowerError> {
    match e {
        ast_v1::BlockElem::Embed { var, semi } => Ok(cst::ast::BlockElem::Embed {
            var: var.clone(),
            semi: semi.clone(),
        }),
        ast_v1::BlockElem::Cmd { name, tail } => Ok(cst::ast::BlockElem::Cmd {
            name: name.clone(),
            tail: lower_cmd_tail(tail)?,
        }),
    }
}

/// The `CmdTail` bridge (§3.3 of the finale spec) — the one non-1:1
/// transcription. `cst_v1` kept the OLD "one application-chain `Expr`"
/// argument encoding (`Args { args: ExprErasedV1, semi }`), while `cst.rs`
/// has since moved to the flat `AppArg` list (`Args { first, rest, semi }`).
/// Semantics-preserving by construction: `cst_v1` parses `\cmd{a}{b}` as
/// `AppExpr { head: {a}, args: [{b}] }`, and `cst` parses the same surface
/// as `first = {a}, rest = [{b}]` — this bridge maps one onto the other
/// exactly. The two error arms are unreachable from token streams the
/// `cst_v1` grammar can actually produce in command-tail position (a bare
/// application chain with no operator/negation) — `LowerError` (not
/// `unreachable!()`) keeps a future grammar-drift bug user-visible rather
/// than silently mis-nesting arguments.
fn lower_cmd_tail(t: &ast_v1::CmdTail) -> Result<cst::ast::CmdTail, LowerError> {
    match t {
        ast_v1::CmdTail::Semi(s) => Ok(cst::ast::CmdTail::Semi(s.clone())),
        ast_v1::CmdTail::Args { args, semi } => {
            let ast_v1::Expr::Ops(chain) = &*args.0 else {
                return Err(unsupported(
                    "command arguments that are not a plain application chain",
                    "grammar-drift guard — the cst_v1 grammar cannot actually \
                     produce this shape in command-tail position",
                    Span::default(),
                ));
            };
            if !chain.tail.is_empty() || chain.head.minus.is_some() {
                return Err(unsupported(
                    "an operator or unary negation inside a command argument chain",
                    "grammar-drift guard — the cst_v1 grammar cannot actually \
                     produce this shape in command-tail position",
                    Span::default(),
                ));
            }
            let a = &chain.head;
            let first = cst::AppArgErased(Box::new(cst::ast::AppArg::Atom {
                excl: a.excl.clone(),
                atom: lower_atomic(&a.head)?,
                accesses: a.head_accesses.iter().map(lower_access_seg).collect(),
            }));
            let rest = a
                .args
                .iter()
                .map(|arg| Ok(cst::AppArgErased(Box::new(lower_app_arg(arg)?))))
                .collect::<Result<Vec<_>, LowerError>>()?;
            Ok(cst::ast::CmdTail::Args {
                first,
                rest,
                semi: semi.clone(),
            })
        }
    }
}

// ---- Pattern layer --------------------------------------------------------

fn lower_pattern(p: &ast_v1::Pattern) -> Result<cst::ast::Pattern, LowerError> {
    Ok(cst::ast::Pattern {
        head: lower_pat_cons(&p.head)?,
        as_clause: p.as_clause.as_ref().map(lower_as_clause),
    })
}

fn lower_as_clause(a: &ast_v1::AsClause) -> cst::ast::AsClause {
    cst::ast::AsClause {
        as_kw: a.as_kw.clone(),
        name: a.name.clone(),
    }
}

fn lower_pat_cons(c: &ast_v1::PatCons) -> Result<cst::ast::PatCons, LowerError> {
    Ok(cst::ast::PatCons {
        head: lower_pat_bot(&c.head)?,
        tail: c.tail.iter().map(lower_cons_seg).collect::<Result<_, _>>()?,
    })
}

fn lower_cons_seg(s: &ast_v1::ConsSeg) -> Result<cst::ast::ConsSeg, LowerError> {
    Ok(cst::ast::ConsSeg {
        cons: s.cons.clone(),
        tail: lower_pat_bot(&s.tail)?,
    })
}

fn lower_pat_bot(p: &ast_v1::PatBot) -> Result<cst::ast::PatBot, LowerError> {
    match p {
        ast_v1::PatBot::CtorApplied { ctor, arg } => Ok(cst::ast::PatBot::CtorApplied {
            ctor: ctor.clone(),
            arg: Box::new(lower_pat_bot(arg)?),
        }),
        ast_v1::PatBot::Ctor(t) => Ok(cst::ast::PatBot::Ctor(t.clone())),
        ast_v1::PatBot::Int(t) => Ok(cst::ast::PatBot::Int(t.clone())),
        ast_v1::PatBot::True(t) => Ok(cst::ast::PatBot::True(t.clone())),
        ast_v1::PatBot::False(t) => Ok(cst::ast::PatBot::False(t.clone())),
        ast_v1::PatBot::Str(t) => Ok(cst::ast::PatBot::Str(t.clone())),
        ast_v1::PatBot::Wild(t) => Ok(cst::ast::PatBot::Wild(t.clone())),
        ast_v1::PatBot::Var(t) => Ok(cst::ast::PatBot::Var(t.clone())),
        ast_v1::PatBot::Unit { paren } => Ok(cst::ast::PatBot::Unit { paren: paren.clone() }),
        ast_v1::PatBot::Paren { paren, inner } => Ok(cst::ast::PatBot::Paren {
            paren: paren.clone(),
            inner: Box::new(lower_pattern_paren_body(inner)?),
        }),
        ast_v1::PatBot::List { plist, items } => Ok(cst::ast::PatBot::List {
            plist: plist.clone(),
            items: items
                .iter()
                .map(lower_pat_list_item)
                .collect::<Result<_, _>>()?,
        }),
    }
}

fn lower_pattern_paren_body(
    b: &ast_v1::PatternParenBody,
) -> Result<cst::ast::PatternParenBody, LowerError> {
    Ok(cst::ast::PatternParenBody {
        first: erase_pat(lower_pattern(&b.first)?),
        rest: b
            .rest
            .iter()
            .map(lower_comma_pattern)
            .collect::<Result<_, _>>()?,
    })
}

fn lower_comma_pattern(c: &ast_v1::CommaPattern) -> Result<cst::ast::CommaPattern, LowerError> {
    Ok(cst::ast::CommaPattern {
        comma: c.comma.clone(),
        value: erase_pat(lower_pattern(&c.value)?),
    })
}

fn lower_pat_list_item(i: &ast_v1::PatListItem) -> Result<cst::ast::PatListItem, LowerError> {
    Ok(cst::ast::PatListItem {
        value: erase_pat(lower_pattern(&i.value)?),
        semi: None,
    })
}

// ---- erasure helpers --------------------------------------------------

fn erase_expr(e: cst::ast::Expr) -> cst::ExprErased {
    cst::ExprErased(Box::new(e))
}

fn erase_pat(p: cst::ast::Pattern) -> cst::PatErased {
    cst::PatErased(Box::new(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_v1(src: &str) -> cst_v1::FileV1 {
        satysfi_syntax::parse_file_v1(src).unwrap_or_else(|e| panic!("v1 parse failed: {e}"))
    }

    #[test]
    fn let_rec_is_a_lower_error() {
        let file = parse_v1("let rec f n = f n in f");
        let err = lower_document_v1(&file).unwrap_err();
        assert!(err.to_string().contains("let rec"), "{err}");
    }

    #[test]
    fn optional_app_arg_is_a_lower_error() {
        let file = parse_v1("f ?:1");
        let err = lower_document_v1(&file).unwrap_err();
        assert!(err.to_string().contains("optional"), "{err}");
    }

    #[test]
    fn omission_app_arg_is_a_lower_error() {
        let file = parse_v1("f ?*");
        let err = lower_document_v1(&file).unwrap_err();
        assert!(err.to_string().contains("optional"), "{err}");
    }

    #[test]
    fn math_text_is_a_lower_error() {
        let file = parse_v1("${x}");
        let err = lower_document_v1(&file).unwrap_err();
        assert!(err.to_string().contains("math"), "{err}");
    }

    /// The shared lexer never actually emits a module-qualified command
    /// token (`HorzCmdWithMod`/`VertCmdWithMod`) in program-mode binding
    /// position — only inline/block-text and math areas dotted-scan a
    /// backslash/plus command name (`lexer.rs`'s `lex_program` vs.
    /// `lex_horz`/`lex_vert`/`lex_math`) — so `AnyHorzCmdTok::Mod`/
    /// `AnyVertCmdTok::Mod` are unreachable from any real `parse_file_v1`
    /// input at a `val inline`/`val block` binding's command-name position.
    /// This exercises `plain_horz`/`plain_vert` directly (a hand-built
    /// token, not a parse) so the `LowerError` arm itself is still proven,
    /// same rationale as the `CmdTail` bridge's own unreachable-in-practice
    /// guards (§3.3's doc comment).
    #[test]
    fn mod_qualified_command_name_in_bind_is_a_lower_error() {
        let tok = HorzCmdWithModTok {
            mods: vec!["Mod".to_string()],
            name: "\\emph".to_string(),
            span: Span::default(),
        };
        let err = plain_horz(&AnyHorzCmdTok::Mod(tok)).unwrap_err();
        assert!(err.to_string().contains("module-qualified"), "{err}");

        let tok = VertCmdWithModTok {
            mods: vec!["Mod".to_string()],
            name: "+p".to_string(),
            span: Span::default(),
        };
        let err = plain_vert(&AnyVertCmdTok::Mod(tok)).unwrap_err();
        assert!(err.to_string().contains("module-qualified"), "{err}");
    }

    #[test]
    fn lower_file_v1_on_a_document_is_an_error_not_a_panic() {
        let file = parse_v1("3");
        assert!(lower_file_v1(&file).is_err());
    }

    #[test]
    fn lower_document_v1_on_a_library_is_an_error_not_a_panic() {
        let file = parse_v1("module M = struct\nval x = 1\nend");
        assert!(lower_document_v1(&file).is_err());
    }

    /// The CmdTail bridge (§3.3): `\cmd{a}{b}` parses under `cst_v1` as one
    /// application chain (`AppExpr { head: {a}, args: [{b}] }`) and must
    /// lower to the same shape `cst.rs`'s own `\cmd{a}{b}` parse produces
    /// (`CmdTail::Args { first: {a}, rest: [{b}] }`).
    #[test]
    fn cmd_tail_bridge_matches_flat_app_arg_shape() {
        let file = parse_v1(r"{\cmd{a}{b}}");
        let cst_v1::FileV1::Document { body, .. } = file else {
            panic!("expected a document file");
        };
        let ast_v1::Expr::Ops(chain) = body else {
            panic!("expected an operator-chain expression");
        };
        let ast_v1::Atomic::InlineText { elems, .. } = chain.head.head else {
            panic!("expected inline text");
        };
        let ast_v1::InlineElem::Cmd { tail, .. } = &elems[0] else {
            panic!("expected the first element to be a command");
        };
        let lowered = lower_cmd_tail(tail).unwrap();
        let cst::ast::CmdTail::Args { first, rest, .. } = lowered else {
            panic!("expected CmdTail::Args");
        };
        assert_eq!(rest.len(), 1, "\\cmd{{a}}{{b}} has exactly one trailing arg");
        assert!(matches!(
            &*first.0,
            cst::ast::AppArg::Atom {
                atom: cst::ast::Atomic::InlineText { .. },
                ..
            }
        ));
        assert!(matches!(
            &*rest[0].0,
            cst::ast::AppArg::Atom {
                atom: cst::ast::Atomic::InlineText { .. },
                ..
            }
        ));
    }
}
