//! Sub-slice 2f-1 (`…/tmp/slice2f-functors.md` §2.1, §4.A): the functor-
//! instantiation substitution pre-pass — a self-contained `cst_v1 -> cst_v1`
//! clone+rewrite, consumed by `v1/lower.rs` (and, for the surface-only body-
//! shape check, `v1/surface.rs`) at every functor application.
//!
//! **The model (spec §2.1).** A functor `Make = fun (X : S) -> body` is
//! stored SYNTACTICALLY (never lowered to a runtime value — like a named
//! `signature`). An application `Make Arg` is instantiated by rewriting the
//! body: every reference whose leading module segment is the parameter name
//! is replaced by the ARGUMENT's already-resolved ABSOLUTE path
//! (`Key.compare` -> `Int.compare`) — because a relative dotted sibling
//! reference does not resolve at the elaborator layer (`elaborate.rs`'s
//! `qualify_key`/`push_named_binding`, verified in the spec's §0.4), while an
//! absolute path (a dependency's export, or a sibling nested module's own
//! fully-qualified export) always does. [`substitute_binds`] performs this
//! rewrite; the caller then re-lowers the substituted binds with the
//! ordinary, UNCHANGED `v1/lower.rs` helpers (`lower_module_bind`/
//! `lower_bind_v1`) — this module never itself lowers anything, it only
//! produces another `cst_v1` tree.
//!
//! **Exactly three (plus one deliberately-deferred) reference sites carry a
//! parameter reference** (spec §2.1):
//!
//! 1. A value reference: [`ast_v1::Atomic::VarWithMod`] (`Key.compare`).
//! 2. A type atom: [`ast_v1::TypeAtom::LongName`] (`Key.t`).
//! 3. A type-application head: [`ast_v1::TypeApp::AppliedLong`] (`Key.t
//!    int`).
//! 4. (found beyond the spec's own list, same treatment as 1) an embedded
//!    text-mode variable reference: `VarInHorzTok`/`VarInVertTok`/
//!    `VarInMathTok` all carry an optional `mods` prefix too (`#Mod.var`,
//!    `leaf.rs`'s `qualified_name_tokens!` macro) — substituted identically
//!    to site 1.
//! 5. A module PATH reference (`ast_v1::ModChainV1`/`ast_v1::SigBotV1::Path`)
//!    whose head is the parameter — substituted the same way, needed for a
//!    nested `module N = Key.Sub` or `signature S = Key.SomeSig` inside a
//!    functor body (no demand package needs this, but it costs nothing extra
//!    once the head-splice helper exists).
//!
//! **Deliberately NOT substituted — a precise [`LowerError`], never a silent
//! pass-through (spec §2.1 risk 3):** a parameter-qualified COMMAND name
//! (`\Key.cmd`/`+Key.cmd`, `HorzCmdWithModTok`/`VertCmdWithModTok`/
//! `MathCmdWithModTok`) — no demand package's functor parameter ever carries
//! a command member (`Ord`/`Settings` declare only `val`s), so rewriting
//! these is Sub-slice 2f-2. A bare `Key` used where only a SINGLE module
//! segment fits (`let open Key in …`, `Key :> S`) is substituted when the
//! argument itself is single-segment, and a precise `LowerError` (again 2f-2)
//! when the argument is multi-segment (neither shape needed by any demand
//! package).
//!
//! **No `_` wildcard in any match arm below** (spec §4.A) — a future
//! `ast_v1` grammar arm must break THIS module's build, not silently drop a
//! parameter reference on the floor. The one exception: subtrees PROVEN to
//! carry no qualified-name/command-name site at all
//! ([`ast_v1::Pattern`]/[`ast_v1::PatBot`] and everything reachable only
//! through them, plus [`ast_v1::KindV1`]) are cloned WHOLESALE rather than
//! walked field-by-field — sound today (no `Pattern` arm anywhere names a
//! `VarWithModTok`/qualified command, confirmed by inspection of
//! `cst_v1.rs`), but flagged here as the one place a future grammar addition
//! to the pattern layer would need re-review (spec §8 risk 3's family).

use crate::v1::lower::LowerError;
use rustyfi_syntax::cst_v1::{self, ast as ast_v1};
use rustyfi_syntax::leaf::*;
use rustyfi_syntax::Span;

/// The functor body's OWN struct binds, if it is (as every 2f-1 demand
/// package's functor is) a literal `struct … end` — `None` for any other
/// `ModExpr` shape (`Var`/`Coerce`/`App`/nested `Functor`), which 2f-1 does
/// not instantiate (an alias-bodied, applied-bodied, or curried functor body
/// is Sub-slice 2f-2 territory; the caller turns a `None` here into a
/// precise `LowerError`/a `None` frozen resolution, never a panic).
pub(crate) fn functor_body_binds(body: &ast_v1::ModExpr) -> Option<&[cst_v1::StructBindV1]> {
    match body {
        ast_v1::ModExpr::Struct { binds, .. } => Some(binds.as_slice()),
        ast_v1::ModExpr::Functor { .. }
        | ast_v1::ModExpr::Coerce { .. }
        | ast_v1::ModExpr::App { .. }
        | ast_v1::ModExpr::Var(_) => None,
    }
}

/// Instantiate a functor's body (`binds`, from [`functor_body_binds`]) for
/// one application: clone the whole subtree, rewriting every parameter
/// reference's leading segment to `arg_path` (the argument's own resolved
/// absolute path, e.g. `["Code", "DefaultSettings"]`). The result is an
/// ordinary, freshly-owned `cst_v1` tree with NO reference to `param` left in
/// it anywhere it could resolve as a module name — the caller re-lowers it
/// with the unchanged `v1/lower.rs` helpers.
pub(crate) fn substitute_binds(
    binds: &[cst_v1::StructBindV1],
    param: &str,
    arg_path: &[String],
) -> Result<Vec<cst_v1::StructBindV1>, LowerError> {
    let rw = ParamSubstRewrite { param, arg_path };
    binds
        .iter()
        .map(|sb| subst_struct_bind_v1(sb, &rw, &[]))
        .collect()
}

/// Sub-slice 2f-2b (`…/tmp/slice2d3b-2f2-sigmembers.md` §5.2-2): the
/// codomain-substitution twin of [`substitute_binds`] — `cod[param :=
/// arg_path]`, reusing the SAME [`ParamSubstRewrite`] this whole module's
/// walker already drives for body substitution (one head-splice
/// implementation, module doc comment's risk-6 guard). `module_check.rs`'s
/// per-application abstract-result sealing calls this to compute the
/// DECLARED codomain at the application site before sealing the
/// instantiated result against it.
pub(crate) fn subst_sig_expr_for_param(
    cod: &ast_v1::SigExpr,
    param: &str,
    arg_path: &[String],
) -> Result<ast_v1::SigExpr, LowerError> {
    let rw = ParamSubstRewrite { param, arg_path };
    subst_sig_expr(cod, &rw, &[])
}

/// Sub-slice 2f-2a (`…/tmp/slice2d3b-2f2-sigmembers.md` §4.2): the reusable
/// head-splice rule this whole module's walker consults at every leaf/module-
/// path site, generalizing what was a hard-coded `(param, arg_path)` pair in
/// 2f-1. Two implementations:
///
/// - [`ParamSubstRewrite`]: 2f-1's functor-instantiation substitution,
///   reproduced byte-for-byte — splice the argument's absolute path in place
///   of a leading parameter-name segment; `path` (the reference site's own
///   module path) is irrelevant here and always ignored.
/// - `v1/lower.rs`'s `AbsolutizeRewrite` (defined there, alongside
///   [`SurfaceEnv`](crate::v1::surface::SurfaceEnv) access — this module
///   deliberately has no direct dependency on `v1/surface.rs` beyond the
///   `resolve_module` call `AbsolutizeRewrite` itself makes): the lowering
///   pre-pass that absolutizes a RELATIVE SIBLING module head (`Impl.add` ->
///   `Doc.S.Impl.add`) by consulting `path` for an outward search.
///
/// `mods` is the dotted head found at a reference site (`["Key"]`/
/// `["Impl"]`/`["A","B"]`); `path` is the module path of the reference SITE
/// itself (threaded by the walker, extended at every `Bind::Module`);
/// `Ok(None)` means "leave `mods` unchanged".
pub(crate) trait HeadRewrite {
    fn rewrite(
        &self,
        mods: &[String],
        path: &[String],
        span: Span,
    ) -> Result<Option<Vec<String>>, LowerError>;

    /// Command-name leaf sites (`\Mod.cmd`/`+Mod.cmd`/…, spec §2.1's
    /// deliberate non-rewrite): default = same rule as [`Self::rewrite`]
    /// (an absolutized command head is a landed, working shape, spec
    /// §4.2-2); [`ParamSubstRewrite`] overrides this to REJECT instead — no
    /// demand package's functor parameter carries a command member.
    fn rewrite_command(
        &self,
        mods: &[String],
        path: &[String],
        span: Span,
    ) -> Result<Option<Vec<String>>, LowerError> {
        self.rewrite(mods, path, span)
    }

    /// Whether a BARE single-segment reference slot (`let open Key in …`,
    /// `Key :> S` — grammatically a lone `CtorTok`, never a dotted chain)
    /// may be rewritten at all. [`ParamSubstRewrite`] = true (the parameter
    /// itself, used bare, must still substitute); the absolutizer = false
    /// (no demand needs it, and a bare slot cannot grammatically hold a
    /// multi-segment absolutized path — spec §4.2 leaves this un-rewritten
    /// rather than risk a spurious width error on a working sealed alias).
    fn rewrite_bare_names(&self) -> bool {
        true
    }

    /// Whether to descend into signature bodies (`sig … end` decls, `:>`
    /// annotations, `with type` refinements) at all. [`ParamSubstRewrite`]
    /// = true (a parameter reference inside a nested signature must still
    /// substitute, unchanged 2f-1 behavior); the absolutizer = false (spec
    /// §4.2-4: signature bodies are deliberately NOT absolutized — no
    /// demand sig references a sibling module relatively).
    fn walk_signatures(&self) -> bool {
        true
    }

    /// Whether encountering a `ModExpr::Functor` NODE while walking is
    /// necessarily a curried/nested functor literal (an error). Only true
    /// for [`ParamSubstRewrite`], which walks EXCLUSIVELY inside an already-
    /// known functor's own body (`substitute_binds`'s whole job) — any
    /// `Functor` node found there truly is nested. The absolutizer runs
    /// over ORDINARY, un-nested binds (every `lower_module_bind` call), so
    /// a `Functor` node there is an everyday sibling-level definition —
    /// left completely untouched instead (spec §4.2's scope: only an
    /// APPLICATION's substituted-then-absolutized body ever needs this
    /// walker to look inside a functor's contents at all).
    fn reject_nested_functor_literals(&self) -> bool {
        true
    }
}

/// 2f-1's original substitution rule, now expressed as a [`HeadRewrite`]
/// impl instead of a hard-coded `(param, arg_path)` pair threaded by hand —
/// behavior-identical to the pre-2f-2a code.
pub(crate) struct ParamSubstRewrite<'a> {
    param: &'a str,
    arg_path: &'a [String],
}

impl HeadRewrite for ParamSubstRewrite<'_> {
    fn rewrite(
        &self,
        mods: &[String],
        _path: &[String],
        _span: Span,
    ) -> Result<Option<Vec<String>>, LowerError> {
        if mods.first().map(String::as_str) == Some(self.param) {
            let mut out = self.arg_path.to_vec();
            out.extend(mods[1..].iter().cloned());
            Ok(Some(out))
        } else {
            Ok(None)
        }
    }

    fn rewrite_command(
        &self,
        mods: &[String],
        _path: &[String],
        span: Span,
    ) -> Result<Option<Vec<String>>, LowerError> {
        if mods.first().map(String::as_str) == Some(self.param) {
            Err(LowerError {
                construct: "a parameter-qualified command name inside a functor body",
                hint: "rewriting `\\Param.cmd`/`+Param.cmd` command references through a \
                       functor instantiation is Sub-slice 2f-2 — no demand package's \
                       functor parameter carries a command member",
                span,
            })
        } else {
            Ok(None)
        }
    }
}

/// Every qualified-name leaf site (`VarWithModTok`/`TypeAtom::LongName`/
/// `TypeApp::AppliedLong`/the `VarInHorz`/`VarInVert`/`VarInMathTok` family/
/// `SigBotV1::Path`) reduces to this one formula: consult `rw`, and splice
/// its answer in place of `mods` (keeping any further segments beyond the
/// rewritten head), or leave `mods` unchanged on `Ok(None)`.
fn rewrite_mods(
    mods: &[String],
    rw: &dyn HeadRewrite,
    path: &[String],
    span: Span,
) -> Result<Vec<String>, LowerError> {
    Ok(rw
        .rewrite(mods, path, span)?
        .unwrap_or_else(|| mods.to_vec()))
}

/// A bare single-segment reference slot (`let open Key in …`, `Key :> S`, or
/// `ModChainV1::Single("Key")`'s OWN name, grammatically a lone `CtorTok`) —
/// gated by [`HeadRewrite::rewrite_bare_names`]; a `rw.rewrite` answer with
/// more than one segment cannot be spliced into a slot that grammatically
/// holds exactly one segment — a precise `LowerError`, not a silent
/// truncation (2f-1's `subst_bare_param_name`, generalized).
fn rewrite_bare_name(
    name: &str,
    span: Span,
    rw: &dyn HeadRewrite,
    path: &[String],
    construct: &'static str,
    hint: &'static str,
) -> Result<Option<String>, LowerError> {
    if !rw.rewrite_bare_names() {
        return Ok(None);
    }
    let mods = vec![name.to_string()];
    match rw.rewrite(&mods, path, span)? {
        None => Ok(None),
        Some(v) if v.len() == 1 => Ok(Some(v.into_iter().next().unwrap())),
        Some(_) => Err(LowerError {
            construct,
            hint,
            span,
        }),
    }
}

fn rewrite_mod_chain(
    c: &ast_v1::ModChainV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::ModChainV1, LowerError> {
    Ok(match c {
        ast_v1::ModChainV1::Long(t) => ast_v1::ModChainV1::Long(LongUpperTok {
            mods: rewrite_mods(&t.mods, rw, path, t.span)?,
            name: t.name.clone(),
            span: t.span,
        }),
        ast_v1::ModChainV1::Single(t) => {
            match rw.rewrite(std::slice::from_ref(&t.name), path, t.span)? {
                None => ast_v1::ModChainV1::Single(t.clone()),
                Some(v) if v.len() == 1 => ast_v1::ModChainV1::Single(CtorTok {
                    name: v.into_iter().next().unwrap(),
                    span: t.span,
                }),
                Some(v) => {
                    let (last, init) = v.split_last().expect("a HeadRewrite answer is never empty");
                    ast_v1::ModChainV1::Long(LongUpperTok {
                        mods: init.to_vec(),
                        name: last.clone(),
                        span: t.span,
                    })
                }
            }
        }
    })
}

// ---- StructBindV1 / Bind ---------------------------------------------------

fn subst_struct_bind_v1(
    sb: &cst_v1::StructBindV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::StructBindV1, LowerError> {
    Ok(cst_v1::StructBindV1(Box::new(subst_bind(&sb.0, rw, path)?)))
}

/// Sub-slice 2f-2a (spec §4.2 step 3): the `Bind`-level (rather than
/// `StructBindV1`-level) twin of [`substitute_binds`] — `v1/lower.rs::
/// lower_module_bind`'s own `binds` shape is `impl IntoIterator<Item =
/// &cst_v1::Bind>` (its doc comment explains why a top-level library's
/// `Vec<Bind>` and a nested module's `Vec<StructBindV1>` differ), so this
/// is the entry point that module reaches for, driven by ANY [`HeadRewrite`]
/// — today only `v1/lower.rs`'s `AbsolutizeRewrite`, run as a pre-pass
/// before every `lower_module_bind` invocation.
pub(crate) fn rewrite_binds<'a>(
    binds: impl IntoIterator<Item = &'a cst_v1::Bind>,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<Vec<cst_v1::Bind>, LowerError> {
    binds.into_iter().map(|b| subst_bind(b, rw, path)).collect()
}

fn subst_bind(
    b: &cst_v1::Bind,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::Bind, LowerError> {
    Ok(match b {
        cst_v1::Bind::Value {
            kw,
            stage,
            name,
            params,
            eq,
            body,
        } => cst_v1::Bind::Value {
            kw: kw.clone(),
            stage: stage.clone(),
            name: name.clone(),
            params: subst_params(params, rw, path)?,
            eq: eq.clone(),
            body: subst_expr(body, rw, path)?,
        },
        cst_v1::Bind::ValueInline {
            kw,
            inline_kw,
            ctx,
            cmd,
            params,
            eq,
            body,
        } => cst_v1::Bind::ValueInline {
            kw: kw.clone(),
            inline_kw: inline_kw.clone(),
            ctx: ctx.clone(),
            cmd: subst_any_horz_cmd(cmd, rw, path)?,
            params: subst_params(params, rw, path)?,
            eq: eq.clone(),
            body: subst_expr(body, rw, path)?,
        },
        cst_v1::Bind::ValueBlock {
            kw,
            block_kw,
            ctx,
            cmd,
            params,
            eq,
            body,
        } => cst_v1::Bind::ValueBlock {
            kw: kw.clone(),
            block_kw: block_kw.clone(),
            ctx: ctx.clone(),
            cmd: subst_any_vert_cmd(cmd, rw, path)?,
            params: subst_params(params, rw, path)?,
            eq: eq.clone(),
            body: subst_expr(body, rw, path)?,
        },
        cst_v1::Bind::ValueMath {
            kw,
            math_kw,
            ctx,
            cmd,
            params,
            scripts,
            eq,
            body,
        } => cst_v1::Bind::ValueMath {
            kw: kw.clone(),
            math_kw: math_kw.clone(),
            ctx: ctx.clone(),
            cmd: subst_any_horz_cmd(cmd, rw, path)?,
            params: subst_params(params, rw, path)?,
            scripts: scripts.clone(),
            eq: eq.clone(),
            body: subst_expr(body, rw, path)?,
        },
        cst_v1::Bind::ValueRec {
            kw,
            rec_kw,
            first,
            ands,
        } => cst_v1::Bind::ValueRec {
            kw: kw.clone(),
            rec_kw: rec_kw.clone(),
            first: subst_rec_clause(first, rw, path)?,
            ands: ands
                .iter()
                .map(|a| {
                    Ok(ast_v1::AndClauseV1 {
                        and_kw: a.and_kw.clone(),
                        clause: subst_rec_clause(&a.clause, rw, path)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
        },
        cst_v1::Bind::ValueMutable {
            kw,
            mutable_kw,
            name,
            arrow,
            value,
        } => cst_v1::Bind::ValueMutable {
            kw: kw.clone(),
            mutable_kw: mutable_kw.clone(),
            name: name.clone(),
            arrow: arrow.clone(),
            value: subst_expr(value, rw, path)?,
        },
        cst_v1::Bind::Type { kw, first, ands } => cst_v1::Bind::Type {
            kw: kw.clone(),
            first: subst_type_bind_single(first, rw, path)?,
            ands: ands
                .iter()
                .map(|a| {
                    Ok(cst_v1::TypeAndV1 {
                        and_kw: a.and_kw.clone(),
                        bind: subst_type_bind_single(&a.bind, rw, path)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
        },
        cst_v1::Bind::Module {
            module_kw,
            name,
            sig_annot,
            eq,
            body,
        } => {
            // 2f-2a (spec §4.2): the walker threads `path`, extending it
            // here — the ONE place a nested module is introduced — so a
            // reference site inside `body` sees the correct (deeper)
            // enclosing module path for outward resolution
            // (`AbsolutizeRewrite`; `ParamSubstRewrite` ignores `path`
            // entirely, so this is a no-op for 2f-1's own behavior).
            let mut child_path = path.to_vec();
            child_path.push(name.name.clone());
            cst_v1::Bind::Module {
                module_kw: module_kw.clone(),
                name: name.clone(),
                sig_annot: sig_annot
                    .as_ref()
                    .map(|sa| subst_sig_annot(sa, rw, path))
                    .transpose()?,
                eq: eq.clone(),
                body: cst_v1::ModExprErasedV1(Box::new(subst_mod_expr(&body.0, rw, &child_path)?)),
            }
        }
        cst_v1::Bind::Signature { kw, name, eq, sig_ } => cst_v1::Bind::Signature {
            kw: kw.clone(),
            name: name.clone(),
            eq: eq.clone(),
            sig_: cst_v1::SigExprErasedV1(Box::new(subst_sig_expr(&sig_.0, rw, path)?)),
        },
        cst_v1::Bind::Include { kw, body } => cst_v1::Bind::Include {
            kw: kw.clone(),
            body: cst_v1::ModExprErasedV1(Box::new(subst_mod_expr(&body.0, rw, path)?)),
        },
    })
}

fn subst_sig_annot(
    sa: &cst_v1::SigAnnotV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::SigAnnotV1, LowerError> {
    Ok(cst_v1::SigAnnotV1 {
        coerce: sa.coerce.clone(),
        sig_: cst_v1::SigExprErasedV1(Box::new(subst_sig_expr(&sa.sig_.0, rw, path)?)),
    })
}

// ---- Param / Expr -----------------------------------------------------

fn subst_params(
    params: &[ast_v1::Param],
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<Vec<ast_v1::Param>, LowerError> {
    params.iter().map(|p| subst_param(p, rw, path)).collect()
}

fn subst_param(
    p: &ast_v1::Param,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::Param, LowerError> {
    Ok(ast_v1::Param {
        // `OptParamsV1`'s entries are plain `label = var` binder pairs (no
        // `Expr`/`TypeExpr` inside) — no parameter-reference site exists
        // here, so cloning wholesale is exact.
        opts: p.opts.clone(),
        body: subst_param_body(&p.body, rw, path)?,
    })
}

fn subst_param_body(
    pb: &ast_v1::ParamBody,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::ParamBody, LowerError> {
    Ok(match pb {
        // `PatBot` (and everything reachable only through it) is proven
        // reference-free (module doc comment) — clone wholesale.
        ast_v1::ParamBody::Pat(p) => ast_v1::ParamBody::Pat(p.clone()),
        ast_v1::ParamBody::Ascribed { paren, inner } => ast_v1::ParamBody::Ascribed {
            paren: paren.clone(),
            inner: ast_v1::AscribedInnerV1 {
                pat: inner.pat.clone(),
                colon: inner.colon.clone(),
                ty: cst_v1::TyErasedV1(Box::new(subst_type_expr(&inner.ty.0, rw, path)?)),
            },
        },
    })
}

fn subst_rec_clause(
    rc: &ast_v1::RecClauseV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::RecClauseV1, LowerError> {
    Ok(ast_v1::RecClauseV1 {
        name: rc.name.clone(),
        params: subst_params(&rc.params, rw, path)?,
        eq: rc.eq.clone(),
        value: cst_v1::ExprErasedV1(Box::new(subst_expr(&rc.value.0, rw, path)?)),
    })
}

fn subst_expr(
    e: &ast_v1::Expr,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::Expr, LowerError> {
    Ok(match e {
        ast_v1::Expr::LetRecIn {
            let_kw,
            rec_kw,
            first,
            ands,
            in_kw,
            body,
        } => ast_v1::Expr::LetRecIn {
            let_kw: let_kw.clone(),
            rec_kw: rec_kw.clone(),
            first: subst_rec_clause(first, rw, path)?,
            ands: ands
                .iter()
                .map(|a| {
                    Ok(ast_v1::AndClauseV1 {
                        and_kw: a.and_kw.clone(),
                        clause: subst_rec_clause(&a.clause, rw, path)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
            in_kw: in_kw.clone(),
            body: Box::new(subst_expr(body, rw, path)?),
        },
        ast_v1::Expr::LetMutableIn {
            let_kw,
            mutable_kw,
            name,
            arrow,
            init,
            in_kw,
            body,
        } => ast_v1::Expr::LetMutableIn {
            let_kw: let_kw.clone(),
            mutable_kw: mutable_kw.clone(),
            name: name.clone(),
            arrow: arrow.clone(),
            init: Box::new(subst_expr(init, rw, path)?),
            in_kw: in_kw.clone(),
            body: Box::new(subst_expr(body, rw, path)?),
        },
        ast_v1::Expr::LetIn {
            kw,
            name,
            params,
            eq,
            value,
            in_kw,
            body,
        } => ast_v1::Expr::LetIn {
            kw: kw.clone(),
            name: name.clone(),
            params: subst_params(params, rw, path)?,
            eq: eq.clone(),
            value: Box::new(subst_expr(value, rw, path)?),
            in_kw: in_kw.clone(),
            body: Box::new(subst_expr(body, rw, path)?),
        },
        ast_v1::Expr::LetPatternIn {
            kw,
            pat,
            eq,
            value,
            in_kw,
            body,
        } => ast_v1::Expr::LetPatternIn {
            kw: kw.clone(),
            pat: pat.clone(),
            eq: eq.clone(),
            value: Box::new(subst_expr(value, rw, path)?),
            in_kw: in_kw.clone(),
            body: Box::new(subst_expr(body, rw, path)?),
        },
        ast_v1::Expr::OpenIn {
            let_kw,
            open_kw,
            name,
            in_kw,
            body,
        } => {
            let name = match rewrite_bare_name(
                &name.name,
                name.span,
                rw,
                path,
                "`let open Param in …` where the functor argument is a \
                 multi-segment module path",
                "a single-module-segment slot cannot hold a multi-segment path — \
                 Sub-slice 2f-2",
            )? {
                Some(new_name) => CtorTok {
                    name: new_name,
                    span: name.span,
                },
                None => name.clone(),
            };
            ast_v1::Expr::OpenIn {
                let_kw: let_kw.clone(),
                open_kw: open_kw.clone(),
                name,
                in_kw: in_kw.clone(),
                body: Box::new(subst_expr(body, rw, path)?),
            }
        }
        ast_v1::Expr::If {
            kw,
            cond,
            then_kw,
            then_branch,
            else_kw,
            else_branch,
        } => ast_v1::Expr::If {
            kw: kw.clone(),
            cond: Box::new(subst_expr(cond, rw, path)?),
            then_kw: then_kw.clone(),
            then_branch: Box::new(subst_expr(then_branch, rw, path)?),
            else_kw: else_kw.clone(),
            else_branch: Box::new(subst_expr(else_branch, rw, path)?),
        },
        ast_v1::Expr::Fun {
            kw,
            params,
            arrow,
            body,
        } => ast_v1::Expr::Fun {
            kw: kw.clone(),
            params: subst_params(params, rw, path)?,
            arrow: arrow.clone(),
            body: Box::new(subst_expr(body, rw, path)?),
        },
        ast_v1::Expr::Match {
            kw,
            scrutinee,
            with_kw,
            leading_bar,
            first,
            rest,
            end_kw,
        } => ast_v1::Expr::Match {
            kw: kw.clone(),
            scrutinee: Box::new(subst_expr(scrutinee, rw, path)?),
            with_kw: with_kw.clone(),
            leading_bar: leading_bar.clone(),
            first: subst_match_arm(first, rw, path)?,
            rest: rest
                .iter()
                .map(|r| {
                    Ok(ast_v1::BarArm {
                        bar: r.bar.clone(),
                        arm: subst_match_arm(&r.arm, rw, path)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
            end_kw: end_kw.clone(),
        },
        ast_v1::Expr::Overwrite { name, arrow, value } => ast_v1::Expr::Overwrite {
            name: name.clone(),
            arrow: arrow.clone(),
            value: cst_v1::ExprErasedV1(Box::new(subst_expr(&value.0, rw, path)?)),
        },
        ast_v1::Expr::Ops(chain) => ast_v1::Expr::Ops(subst_op_chain(chain, rw, path)?),
    })
}

fn subst_match_arm(
    m: &ast_v1::MatchArm,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::MatchArm, LowerError> {
    Ok(ast_v1::MatchArm {
        pat: m.pat.clone(),
        arrow: m.arrow.clone(),
        body: cst_v1::ExprErasedV1(Box::new(subst_expr(&m.body.0, rw, path)?)),
    })
}

fn subst_op_chain(
    c: &ast_v1::OpChain,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::OpChain, LowerError> {
    Ok(ast_v1::OpChain {
        head: subst_app_expr(&c.head, rw, path)?,
        tail: c
            .tail
            .iter()
            .map(|r| {
                Ok(ast_v1::OpRhs {
                    op: r.op.clone(),
                    rhs: subst_app_expr(&r.rhs, rw, path)?,
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_app_expr(
    a: &ast_v1::AppExpr,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::AppExpr, LowerError> {
    Ok(ast_v1::AppExpr {
        minus: a.minus.clone(),
        stage: a.stage.clone(),
        excl: a.excl.clone(),
        head: subst_atomic(&a.head, rw, path)?,
        // `AccessSeg.label` is a record-field name, never module-qualified.
        head_accesses: a.head_accesses.clone(),
        args: a
            .args
            .iter()
            .map(|x| subst_app_arg(x, rw, path))
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_app_arg(
    x: &ast_v1::AppArg,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::AppArg, LowerError> {
    Ok(match x {
        ast_v1::AppArg::Bundled {
            opts,
            excl,
            atom,
            accesses,
        } => ast_v1::AppArg::Bundled {
            opts: subst_opt_args(opts, rw, path)?,
            excl: excl.clone(),
            atom: subst_atomic(atom, rw, path)?,
            accesses: accesses.clone(),
        },
        ast_v1::AppArg::BundledCtor { opts, ctor } => ast_v1::AppArg::BundledCtor {
            opts: subst_opt_args(opts, rw, path)?,
            ctor: ctor.clone(),
        },
        ast_v1::AppArg::Atom {
            stage,
            excl,
            atom,
            accesses,
        } => ast_v1::AppArg::Atom {
            stage: stage.clone(),
            excl: excl.clone(),
            atom: subst_atomic(atom, rw, path)?,
            accesses: accesses.clone(),
        },
        ast_v1::AppArg::Ctor(c) => ast_v1::AppArg::Ctor(c.clone()),
    })
}

fn subst_opt_args(
    o: &ast_v1::OptArgsV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::OptArgsV1, LowerError> {
    Ok(ast_v1::OptArgsV1 {
        q: o.q.clone(),
        paren: o.paren.clone(),
        entries: o
            .entries
            .iter()
            .map(|e| {
                Ok(ast_v1::OptArgEntryV1 {
                    label: e.label.clone(),
                    eq: e.eq.clone(),
                    value: cst_v1::ExprErasedV1(Box::new(subst_expr(&e.value.0, rw, path)?)),
                    comma: e.comma.clone(),
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_atomic(
    a: &ast_v1::Atomic,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::Atomic, LowerError> {
    Ok(match a {
        ast_v1::Atomic::Length(t) => ast_v1::Atomic::Length(t.clone()),
        ast_v1::Atomic::Float(t) => ast_v1::Atomic::Float(t.clone()),
        ast_v1::Atomic::Int(t) => ast_v1::Atomic::Int(t.clone()),
        ast_v1::Atomic::Literal(t) => ast_v1::Atomic::Literal(t.clone()),
        ast_v1::Atomic::True(t) => ast_v1::Atomic::True(t.clone()),
        ast_v1::Atomic::False(t) => ast_v1::Atomic::False(t.clone()),
        ast_v1::Atomic::Ctor(t) => ast_v1::Atomic::Ctor(t.clone()),
        ast_v1::Atomic::Var(t) => ast_v1::Atomic::Var(t.clone()),
        // Site 1 (module doc comment).
        ast_v1::Atomic::VarWithMod(t) => ast_v1::Atomic::VarWithMod(VarWithModTok {
            mods: rewrite_mods(&t.mods, rw, path, t.span)?,
            name: t.name.clone(),
            span: t.span,
        }),
        ast_v1::Atomic::Command { kw, name } => ast_v1::Atomic::Command {
            kw: kw.clone(),
            name: subst_any_horz_cmd(name, rw, path)?,
        },
        ast_v1::Atomic::Unit { paren } => ast_v1::Atomic::Unit {
            paren: paren.clone(),
        },
        ast_v1::Atomic::Paren { paren, inner } => ast_v1::Atomic::Paren {
            paren: paren.clone(),
            inner: Box::new(subst_paren_body(inner, rw, path)?),
        },
        ast_v1::Atomic::Record { rec, body } => ast_v1::Atomic::Record {
            rec: rec.clone(),
            body: subst_record_body(body, rw, path)?,
        },
        ast_v1::Atomic::List { list, items } => ast_v1::Atomic::List {
            list: list.clone(),
            items: items
                .iter()
                .map(|i| subst_list_item(i, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::Atomic::InlineText { igrp, elems } => ast_v1::Atomic::InlineText {
            igrp: igrp.clone(),
            elems: elems
                .iter()
                .map(|el| subst_inline_elem(el, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::Atomic::BlockText { bgrp, elems } => ast_v1::Atomic::BlockText {
            bgrp: bgrp.clone(),
            elems: elems
                .iter()
                .map(|el| subst_block_elem(el, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::Atomic::MathText { mgrp, elems } => ast_v1::Atomic::MathText {
            mgrp: mgrp.clone(),
            elems: elems
                .iter()
                .map(|el| subst_math_erased(el, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
    })
}

fn subst_math_erased(
    m: &cst_v1::MathErasedV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::MathErasedV1, LowerError> {
    Ok(cst_v1::MathErasedV1(Box::new(subst_math_elem(
        &m.0, rw, path,
    )?)))
}

fn subst_paren_body(
    p: &ast_v1::ParenBody,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::ParenBody, LowerError> {
    Ok(ast_v1::ParenBody {
        first: cst_v1::ExprErasedV1(Box::new(subst_expr(&p.first.0, rw, path)?)),
        rest: p
            .rest
            .iter()
            .map(|c| {
                Ok(ast_v1::CommaExpr {
                    comma: c.comma.clone(),
                    value: cst_v1::ExprErasedV1(Box::new(subst_expr(&c.value.0, rw, path)?)),
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_record_body(
    b: &ast_v1::RecordBody,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::RecordBody, LowerError> {
    Ok(match b {
        ast_v1::RecordBody::Update {
            base,
            with_kw,
            fields,
        } => ast_v1::RecordBody::Update {
            base: cst_v1::ExprErasedV1(Box::new(subst_expr(&base.0, rw, path)?)),
            with_kw: with_kw.clone(),
            fields: fields
                .iter()
                .map(|f| subst_record_field(f, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::RecordBody::Fields(fs) => ast_v1::RecordBody::Fields(
            fs.iter()
                .map(|f| subst_record_field(f, rw, path))
                .collect::<Result<_, LowerError>>()?,
        ),
    })
}

fn subst_record_field(
    f: &ast_v1::RecordField,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::RecordField, LowerError> {
    Ok(ast_v1::RecordField {
        name: f.name.clone(),
        eq: f.eq.clone(),
        value: cst_v1::ExprErasedV1(Box::new(subst_expr(&f.value.0, rw, path)?)),
        comma: f.comma.clone(),
    })
}

fn subst_list_item(
    i: &ast_v1::ListItem,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::ListItem, LowerError> {
    Ok(ast_v1::ListItem {
        value: cst_v1::ExprErasedV1(Box::new(subst_expr(&i.value.0, rw, path)?)),
        comma: i.comma.clone(),
    })
}

fn subst_inline_elem(
    e: &ast_v1::InlineElem,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::InlineElem, LowerError> {
    Ok(match e {
        ast_v1::InlineElem::Char(t) => ast_v1::InlineElem::Char(t.clone()),
        ast_v1::InlineElem::CodeText(t) => ast_v1::InlineElem::CodeText(t.clone()),
        ast_v1::InlineElem::Space(t) => ast_v1::InlineElem::Space(t.clone()),
        ast_v1::InlineElem::Break(t) => ast_v1::InlineElem::Break(t.clone()),
        // Site 4 ("found beyond the spec", module doc comment).
        ast_v1::InlineElem::Embed { var, semi } => ast_v1::InlineElem::Embed {
            var: VarInHorzTok {
                mods: rewrite_mods(&var.mods, rw, path, var.span)?,
                name: var.name.clone(),
                span: var.span,
            },
            semi: semi.clone(),
        },
        ast_v1::InlineElem::EmbedMath { mgrp, elems } => ast_v1::InlineElem::EmbedMath {
            mgrp: mgrp.clone(),
            elems: elems
                .iter()
                .map(|m| subst_math_erased(m, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::InlineElem::Cmd { name, tail } => ast_v1::InlineElem::Cmd {
            name: subst_any_horz_cmd(name, rw, path)?,
            tail: subst_cmd_tail(tail, rw, path)?,
        },
        ast_v1::InlineElem::ItemBullet(t) => ast_v1::InlineElem::ItemBullet(t.clone()),
        ast_v1::InlineElem::Sep(t) => ast_v1::InlineElem::Sep(t.clone()),
    })
}

fn subst_block_elem(
    e: &ast_v1::BlockElem,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::BlockElem, LowerError> {
    Ok(match e {
        ast_v1::BlockElem::Embed { var, semi } => ast_v1::BlockElem::Embed {
            var: VarInVertTok {
                mods: rewrite_mods(&var.mods, rw, path, var.span)?,
                name: var.name.clone(),
                span: var.span,
            },
            semi: semi.clone(),
        },
        ast_v1::BlockElem::Cmd { name, tail } => ast_v1::BlockElem::Cmd {
            name: subst_any_vert_cmd(name, rw, path)?,
            tail: subst_cmd_tail(tail, rw, path)?,
        },
    })
}

fn subst_cmd_tail(
    t: &ast_v1::CmdTail,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::CmdTail, LowerError> {
    Ok(match t {
        ast_v1::CmdTail::Semi(s) => ast_v1::CmdTail::Semi(s.clone()),
        ast_v1::CmdTail::Args {
            lead_opts,
            args,
            semi,
        } => ast_v1::CmdTail::Args {
            lead_opts: lead_opts
                .as_ref()
                .map(|o| subst_opt_args(o, rw, path))
                .transpose()?,
            args: cst_v1::ExprErasedV1(Box::new(subst_expr(&args.0, rw, path)?)),
            semi: semi.clone(),
        },
    })
}

fn subst_any_horz_cmd(
    c: &AnyHorzCmdTok,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<AnyHorzCmdTok, LowerError> {
    Ok(match c {
        AnyHorzCmdTok::Plain(t) => AnyHorzCmdTok::Plain(t.clone()),
        AnyHorzCmdTok::Mod(t) => {
            let mods = rw
                .rewrite_command(&t.mods, path, t.span)?
                .unwrap_or_else(|| t.mods.clone());
            AnyHorzCmdTok::Mod(HorzCmdWithModTok {
                mods,
                name: t.name.clone(),
                span: t.span,
            })
        }
    })
}

fn subst_any_vert_cmd(
    c: &AnyVertCmdTok,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<AnyVertCmdTok, LowerError> {
    Ok(match c {
        AnyVertCmdTok::Plain(t) => AnyVertCmdTok::Plain(t.clone()),
        AnyVertCmdTok::Mod(t) => {
            let mods = rw
                .rewrite_command(&t.mods, path, t.span)?
                .unwrap_or_else(|| t.mods.clone());
            AnyVertCmdTok::Mod(VertCmdWithModTok {
                mods,
                name: t.name.clone(),
                span: t.span,
            })
        }
    })
}

fn subst_any_math_cmd(
    c: &AnyMathCmdTok,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<AnyMathCmdTok, LowerError> {
    Ok(match c {
        AnyMathCmdTok::Plain(t) => AnyMathCmdTok::Plain(t.clone()),
        AnyMathCmdTok::Mod(t) => {
            let mods = rw
                .rewrite_command(&t.mods, path, t.span)?
                .unwrap_or_else(|| t.mods.clone());
            AnyMathCmdTok::Mod(MathCmdWithModTok {
                mods,
                name: t.name.clone(),
                span: t.span,
            })
        }
    })
}

// ---- math grammar ----------------------------------------------------------

fn subst_math_elem(
    m: &ast_v1::MathElemCst,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::MathElemCst, LowerError> {
    Ok(ast_v1::MathElemCst {
        base: subst_math_bot(&m.base, rw, path)?,
        scripts: m
            .scripts
            .iter()
            .map(|s| subst_math_script(s, rw, path))
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_math_bot(
    m: &ast_v1::MathBot,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::MathBot, LowerError> {
    Ok(match m {
        ast_v1::MathBot::Cmd { name, args } => ast_v1::MathBot::Cmd {
            name: subst_any_math_cmd(name, rw, path)?,
            args: args
                .iter()
                .map(|a| subst_math_arg(a, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::MathBot::Chars(t) => ast_v1::MathBot::Chars(t.clone()),
        // Site 4 ("found beyond the spec", module doc comment).
        ast_v1::MathBot::Embed(t) => ast_v1::MathBot::Embed(VarInMathTok {
            mods: rewrite_mods(&t.mods, rw, path, t.span)?,
            name: t.name.clone(),
            span: t.span,
        }),
        ast_v1::MathBot::Sep(t) => ast_v1::MathBot::Sep(t.clone()),
        ast_v1::MathBot::Group { mgrp, elems } => ast_v1::MathBot::Group {
            mgrp: mgrp.clone(),
            elems: elems
                .iter()
                .map(|e| subst_math_erased(e, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
    })
}

fn subst_math_script(
    s: &ast_v1::MathScript,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::MathScript, LowerError> {
    Ok(match s {
        ast_v1::MathScript::Super { hat, group } => ast_v1::MathScript::Super {
            hat: hat.clone(),
            group: subst_math_group_arg(group, rw, path)?,
        },
        ast_v1::MathScript::Sub { under, group } => ast_v1::MathScript::Sub {
            under: under.clone(),
            group: subst_math_group_arg(group, rw, path)?,
        },
        ast_v1::MathScript::Primes(t) => ast_v1::MathScript::Primes(t.clone()),
    })
}

fn subst_math_group_arg(
    g: &ast_v1::MathGroupArg,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::MathGroupArg, LowerError> {
    Ok(match g {
        ast_v1::MathGroupArg::Group { mgrp, elems } => ast_v1::MathGroupArg::Group {
            mgrp: mgrp.clone(),
            elems: elems
                .iter()
                .map(|e| subst_math_erased(e, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::MathGroupArg::Bot(b) => {
            ast_v1::MathGroupArg::Bot(Box::new(subst_math_bot(b, rw, path)?))
        }
    })
}

fn subst_math_arg(
    a: &ast_v1::MathArg,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::MathArg, LowerError> {
    Ok(match a {
        ast_v1::MathArg::Math { mgrp, elems } => ast_v1::MathArg::Math {
            mgrp: mgrp.clone(),
            elems: elems
                .iter()
                .map(|e| subst_math_erased(e, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::MathArg::Inline { igrp, elems } => ast_v1::MathArg::Inline {
            igrp: igrp.clone(),
            elems: elems
                .iter()
                .map(|e| subst_inline_elem(e, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::MathArg::Block { bgrp, elems } => ast_v1::MathArg::Block {
            bgrp: bgrp.clone(),
            elems: elems
                .iter()
                .map(|e| subst_block_elem(e, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::MathArg::ParenEscape { paren, inner } => ast_v1::MathArg::ParenEscape {
            paren: paren.clone(),
            inner: Box::new(subst_paren_body(inner, rw, path)?),
        },
        ast_v1::MathArg::ListEscape { list, items } => ast_v1::MathArg::ListEscape {
            list: list.clone(),
            items: items
                .iter()
                .map(|i| subst_list_item(i, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::MathArg::RecordEscape { rec, body } => ast_v1::MathArg::RecordEscape {
            rec: rec.clone(),
            body: subst_record_body(body, rw, path)?,
        },
    })
}

// ---- type grammar -----------------------------------------------------

fn subst_type_expr(
    t: &ast_v1::TypeExpr,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeExpr, LowerError> {
    Ok(match t {
        ast_v1::TypeExpr::OptRowFun {
            opt_dom,
            dom,
            arrow,
            cod,
        } => ast_v1::TypeExpr::OptRowFun {
            opt_dom: subst_type_opt_dom(opt_dom, rw, path)?,
            dom: subst_type_prod(dom, rw, path)?,
            arrow: arrow.clone(),
            cod: Box::new(subst_type_expr(cod, rw, path)?),
        },
        ast_v1::TypeExpr::Fun { dom, arrow, cod } => ast_v1::TypeExpr::Fun {
            dom: subst_type_prod(dom, rw, path)?,
            arrow: arrow.clone(),
            cod: Box::new(subst_type_expr(cod, rw, path)?),
        },
        ast_v1::TypeExpr::Atom(p) => ast_v1::TypeExpr::Atom(subst_type_prod(p, rw, path)?),
    })
}

fn subst_type_opt_dom(
    o: &ast_v1::TypeOptDomV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeOptDomV1, LowerError> {
    Ok(ast_v1::TypeOptDomV1 {
        q: o.q.clone(),
        paren: o.paren.clone(),
        inner: subst_type_opt_dom_inner(&o.inner, rw, path)?,
    })
}

fn subst_type_opt_dom_inner(
    i: &ast_v1::TypeOptDomInnerV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeOptDomInnerV1, LowerError> {
    Ok(ast_v1::TypeOptDomInnerV1 {
        entries: i
            .entries
            .iter()
            .map(|e| {
                Ok(ast_v1::TypeOptEntryV1 {
                    label: e.label.clone(),
                    colon: e.colon.clone(),
                    ty: cst_v1::TyErasedV1(Box::new(subst_type_expr(&e.ty.0, rw, path)?)),
                    comma: e.comma.clone(),
                })
            })
            .collect::<Result<_, LowerError>>()?,
        row_tail: i.row_tail.clone(),
    })
}

fn subst_type_prod(
    p: &ast_v1::TypeProd,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeProd, LowerError> {
    Ok(ast_v1::TypeProd {
        first: subst_type_app(&p.first, rw, path)?,
        rest: p
            .rest
            .iter()
            .map(|s| {
                Ok(ast_v1::StarType {
                    star: s.star.clone(),
                    ty: subst_type_app(&s.ty, rw, path)?,
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_type_app(
    a: &ast_v1::TypeApp,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeApp, LowerError> {
    Ok(match a {
        ast_v1::TypeApp::InlineCmdTy { kw, ilist: list, args } => ast_v1::TypeApp::InlineCmdTy {
            kw: kw.clone(),
            ilist: list.clone(),
            args: args
                .iter()
                .map(|x| subst_type_cmd_arg_item(x, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::TypeApp::BlockCmdTy { kw, blist: list, args } => ast_v1::TypeApp::BlockCmdTy {
            kw: kw.clone(),
            blist: list.clone(),
            args: args
                .iter()
                .map(|x| subst_type_cmd_arg_item(x, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::TypeApp::MathCmdTy { kw, mlist: list, args } => ast_v1::TypeApp::MathCmdTy {
            kw: kw.clone(),
            mlist: list.clone(),
            args: args
                .iter()
                .map(|x| subst_type_cmd_arg_item(x, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        // Site 3 (module doc comment).
        ast_v1::TypeApp::AppliedLong { ctor, first, rest } => ast_v1::TypeApp::AppliedLong {
            ctor: VarWithModTok {
                mods: rewrite_mods(&ctor.mods, rw, path, ctor.span)?,
                name: ctor.name.clone(),
                span: ctor.span,
            },
            first: subst_type_atom(first, rw, path)?,
            rest: rest
                .iter()
                .map(|x| subst_type_atom(x, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::TypeApp::Applied { ctor, first, rest } => ast_v1::TypeApp::Applied {
            ctor: ctor.clone(),
            first: subst_type_atom(first, rw, path)?,
            rest: rest
                .iter()
                .map(|x| subst_type_atom(x, rw, path))
                .collect::<Result<_, LowerError>>()?,
        },
        ast_v1::TypeApp::Atom(at) => ast_v1::TypeApp::Atom(subst_type_atom(at, rw, path)?),
    })
}

fn subst_type_cmd_arg_item(
    x: &ast_v1::TypeCmdArgItemV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeCmdArgItemV1, LowerError> {
    Ok(ast_v1::TypeCmdArgItemV1 {
        opts: x
            .opts
            .as_ref()
            .map(|o| subst_type_cmd_opt_dom(o, rw, path))
            .transpose()?,
        ty: cst_v1::TyErasedV1(Box::new(subst_type_expr(&x.ty.0, rw, path)?)),
        comma: x.comma.clone(),
    })
}

fn subst_type_cmd_opt_dom(
    o: &ast_v1::TypeCmdOptDomV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeCmdOptDomV1, LowerError> {
    Ok(ast_v1::TypeCmdOptDomV1 {
        q: o.q.clone(),
        paren: o.paren.clone(),
        entries: o
            .entries
            .iter()
            .map(|e| {
                Ok(ast_v1::TypeCmdOptEntryV1 {
                    label: e.label.clone(),
                    colon: e.colon.clone(),
                    ty: cst_v1::TyErasedV1(Box::new(subst_type_expr(&e.ty.0, rw, path)?)),
                    comma: e.comma.clone(),
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

fn subst_type_atom(
    t: &ast_v1::TypeAtom,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeAtom, LowerError> {
    Ok(match t {
        ast_v1::TypeAtom::Paren { paren, inner } => ast_v1::TypeAtom::Paren {
            paren: paren.clone(),
            inner: cst_v1::TyErasedV1(Box::new(subst_type_expr(&inner.0, rw, path)?)),
        },
        ast_v1::TypeAtom::Record { rec, inner } => ast_v1::TypeAtom::Record {
            rec: rec.clone(),
            inner: subst_type_record_inner(inner, rw, path)?,
        },
        ast_v1::TypeAtom::Var(v) => ast_v1::TypeAtom::Var(v.clone()),
        // Site 2 (module doc comment).
        ast_v1::TypeAtom::LongName(v) => ast_v1::TypeAtom::LongName(VarWithModTok {
            mods: rewrite_mods(&v.mods, rw, path, v.span)?,
            name: v.name.clone(),
            span: v.span,
        }),
        ast_v1::TypeAtom::Name(n) => ast_v1::TypeAtom::Name(n.clone()),
    })
}

fn subst_type_record_inner(
    i: &ast_v1::TypeRecordInnerV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::TypeRecordInnerV1, LowerError> {
    Ok(ast_v1::TypeRecordInnerV1 {
        fields: i
            .fields
            .iter()
            .map(|f| {
                Ok(ast_v1::TypeRecordFieldV1 {
                    name: f.name.clone(),
                    colon: f.colon.clone(),
                    ty: cst_v1::TyErasedV1(Box::new(subst_type_expr(&f.ty.0, rw, path)?)),
                    comma: f.comma.clone(),
                })
            })
            .collect::<Result<_, LowerError>>()?,
        row_tail: i.row_tail.clone(),
    })
}

fn subst_type_bind_single(
    s: &cst_v1::TypeBindSingleV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::TypeBindSingleV1, LowerError> {
    Ok(cst_v1::TypeBindSingleV1 {
        name: s.name.clone(),
        tyvars: s.tyvars.clone(),
        eq: s.eq.clone(),
        body: subst_type_body(&s.body, rw, path)?,
    })
}

fn subst_type_body(
    b: &cst_v1::TypeBodyV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::TypeBodyV1, LowerError> {
    Ok(match b {
        cst_v1::TypeBodyV1::Variant {
            leading_bar,
            first,
            rest,
        } => cst_v1::TypeBodyV1::Variant {
            leading_bar: leading_bar.clone(),
            first: subst_variant_def(first, rw, path)?,
            rest: rest
                .iter()
                .map(|r| {
                    Ok(cst_v1::BarVariantDefV1 {
                        bar: r.bar.clone(),
                        def: subst_variant_def(&r.def, rw, path)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
        },
        cst_v1::TypeBodyV1::Synonym(ty) => {
            cst_v1::TypeBodyV1::Synonym(subst_type_expr(ty, rw, path)?)
        }
    })
}

fn subst_variant_def(
    v: &cst_v1::VariantDefV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::VariantDefV1, LowerError> {
    Ok(cst_v1::VariantDefV1 {
        ctor: v.ctor.clone(),
        of_ty: v
            .of_ty
            .as_ref()
            .map(|o| {
                Ok(cst_v1::OfTypeV1 {
                    of_kw: o.of_kw.clone(),
                    ty: subst_type_expr(&o.ty, rw, path)?,
                })
            })
            .transpose()?,
    })
}

fn subst_type_binds(
    b: &cst_v1::TypeBindsV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::TypeBindsV1, LowerError> {
    Ok(cst_v1::TypeBindsV1 {
        first: subst_type_bind_single(&b.first, rw, path)?,
        ands: b
            .ands
            .iter()
            .map(|a| {
                Ok(cst_v1::TypeAndV1 {
                    and_kw: a.and_kw.clone(),
                    bind: subst_type_bind_single(&a.bind, rw, path)?,
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

// ---- module / signature grammar (nested modules inside a functor body) ----

fn subst_mod_expr(
    m: &ast_v1::ModExpr,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::ModExpr, LowerError> {
    Ok(match m {
        // `ParamSubstRewrite` (2f-1): this walker is ONLY ever run over the
        // CONTENTS of an already-known functor body (`substitute_binds`'s
        // whole job), so a `ModExpr::Functor` encountered HERE necessarily
        // means a functor DEFINED inside another functor's body (curried/
        // nested) — out of the first-order 2f-1 slice (§0.6 of the spec: no
        // demand package is higher-order) — a precise error, not a silent
        // identity pass-through.
        //
        // `AbsolutizeRewrite` (2f-2a, spec §4.2): this walker runs over
        // ORDINARY binds before every `lower_module_bind` — a `ModExpr::
        // Functor` here is an everyday, SIBLING-level functor DEFINITION
        // (never itself lowered — `lower_bind_v1`'s own `Functor` arm emits
        // zero bindings for it), not a nested one. Its body is absolutized
        // fresh, from scratch, only at APPLICATION time (after parameter
        // substitution, at the instantiated site) — so it is left
        // completely untouched here, never descended into.
        ast_v1::ModExpr::Functor {
            fun_kw,
            lp,
            param,
            colon,
            dom,
            rp,
            arrow,
            body,
        } if rw.reject_nested_functor_literals() => {
            let _ = (lp, param, colon, dom, rp, arrow, body);
            return Err(LowerError {
                construct: "a nested functor literal inside another functor's body",
                hint: "curried/higher-order functors are Sub-slice 2f-2",
                span: fun_kw.0,
            });
        }
        ast_v1::ModExpr::Functor { .. } => m.clone(),
        ast_v1::ModExpr::Coerce { name, coerce, sig_ } => {
            let new_name = rewrite_bare_name(
                &name.name,
                name.span,
                rw,
                path,
                "`Param :> S` coercion where the functor argument is a \
                 multi-segment module path",
                "`:>` coercion applies to a bare module name only — \
                 coercing a multi-segment argument through a functor \
                 parameter is Sub-slice 2f-2",
            )?;
            let name = match new_name {
                Some(n) => CtorTok {
                    name: n,
                    span: name.span,
                },
                None => name.clone(),
            };
            ast_v1::ModExpr::Coerce {
                name,
                coerce: coerce.clone(),
                sig_: Box::new(subst_sig_expr(sig_, rw, path)?),
            }
        }
        ast_v1::ModExpr::App { func, arg } => ast_v1::ModExpr::App {
            func: rewrite_mod_chain(func, rw, path)?,
            arg: rewrite_mod_chain(arg, rw, path)?,
        },
        ast_v1::ModExpr::Var(chain) => ast_v1::ModExpr::Var(rewrite_mod_chain(chain, rw, path)?),
        ast_v1::ModExpr::Struct {
            struct_kw,
            binds,
            end_kw,
        } => ast_v1::ModExpr::Struct {
            struct_kw: struct_kw.clone(),
            binds: binds
                .iter()
                .map(|sb| subst_struct_bind_v1(sb, rw, path))
                .collect::<Result<_, LowerError>>()?,
            end_kw: end_kw.clone(),
        },
    })
}

fn subst_sig_expr(
    s: &ast_v1::SigExpr,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::SigExpr, LowerError> {
    // Spec §4.2-4: signature bodies are deliberately NOT absolutized (no
    // demand sig references a sibling module relatively); `ParamSubstRewrite`
    // still needs to descend here (a parameter reference inside a nested
    // signature must still substitute, unchanged 2f-1 behavior).
    if !rw.walk_signatures() {
        return Ok(s.clone());
    }
    Ok(match s {
        ast_v1::SigExpr::Functor {
            lp,
            param: p2,
            colon,
            dom,
            rp,
            arrow,
            cod,
        } => ast_v1::SigExpr::Functor {
            lp: lp.clone(),
            param: p2.clone(),
            colon: colon.clone(),
            dom: Box::new(subst_sig_expr(dom, rw, path)?),
            rp: rp.clone(),
            arrow: arrow.clone(),
            cod: Box::new(subst_sig_expr(cod, rw, path)?),
        },
        ast_v1::SigExpr::WithType {
            base,
            with_kw,
            path: type_path,
            type_kw,
            binds,
        } => ast_v1::SigExpr::WithType {
            base: subst_sig_bot(base, rw, path)?,
            with_kw: with_kw.clone(),
            path: type_path
                .as_ref()
                .map(|c| rewrite_mod_chain(c, rw, path))
                .transpose()?,
            type_kw: type_kw.clone(),
            binds: cst_v1::TypeBindsErasedV1(Box::new(subst_type_binds(&binds.0, rw, path)?)),
        },
        ast_v1::SigExpr::Bot(b) => ast_v1::SigExpr::Bot(subst_sig_bot(b, rw, path)?),
    })
}

fn subst_sig_bot(
    b: &ast_v1::SigBotV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::SigBotV1, LowerError> {
    Ok(match b {
        // Site 5 (module doc comment).
        ast_v1::SigBotV1::Path(t) => ast_v1::SigBotV1::Path(LongUpperTok {
            mods: rewrite_mods(&t.mods, rw, path, t.span)?,
            name: t.name.clone(),
            span: t.span,
        }),
        ast_v1::SigBotV1::Var(t) => ast_v1::SigBotV1::Var(t.clone()),
        ast_v1::SigBotV1::Sig {
            sig_kw,
            decls,
            end_kw,
        } => ast_v1::SigBotV1::Sig {
            sig_kw: sig_kw.clone(),
            decls: decls
                .iter()
                .map(|d| subst_struct_decl_v1(d, rw, path))
                .collect::<Result<_, LowerError>>()?,
            end_kw: end_kw.clone(),
        },
    })
}

fn subst_struct_decl_v1(
    d: &cst_v1::StructDeclV1,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<cst_v1::StructDeclV1, LowerError> {
    Ok(cst_v1::StructDeclV1(Box::new(subst_decl(&d.0, rw, path)?)))
}

fn subst_decl(
    d: &ast_v1::Decl,
    rw: &dyn HeadRewrite,
    path: &[String],
) -> Result<ast_v1::Decl, LowerError> {
    Ok(match d {
        ast_v1::Decl::Val {
            kw,
            stage,
            name,
            quant,
            colon,
            ty,
        } => ast_v1::Decl::Val {
            kw: kw.clone(),
            stage: stage.clone(),
            name: name.clone(),
            quant: quant.clone(),
            colon: colon.clone(),
            ty: subst_type_expr(ty, rw, path)?,
        },
        ast_v1::Decl::ValHorzCmd {
            kw,
            cmd,
            quant,
            colon,
            ty,
        } => ast_v1::Decl::ValHorzCmd {
            kw: kw.clone(),
            cmd: cmd.clone(),
            quant: quant.clone(),
            colon: colon.clone(),
            ty: subst_type_expr(ty, rw, path)?,
        },
        ast_v1::Decl::ValVertCmd {
            kw,
            cmd,
            quant,
            colon,
            ty,
        } => ast_v1::Decl::ValVertCmd {
            kw: kw.clone(),
            cmd: cmd.clone(),
            quant: quant.clone(),
            colon: colon.clone(),
            ty: subst_type_expr(ty, rw, path)?,
        },
        ast_v1::Decl::TypeOpaque {
            kw,
            name,
            cons,
            kind,
        } => ast_v1::Decl::TypeOpaque {
            kw: kw.clone(),
            name: name.clone(),
            cons: cons.clone(),
            kind: kind.clone(),
        },
        ast_v1::Decl::Type { kw, binds } => ast_v1::Decl::Type {
            kw: kw.clone(),
            binds: cst_v1::TypeBindsErasedV1(Box::new(subst_type_binds(&binds.0, rw, path)?)),
        },
        ast_v1::Decl::Module {
            kw,
            name,
            colon,
            sig_,
        } => ast_v1::Decl::Module {
            kw: kw.clone(),
            name: name.clone(),
            colon: colon.clone(),
            sig_: Box::new(subst_sig_expr(sig_, rw, path)?),
        },
        ast_v1::Decl::Signature { kw, name, eq, sig_ } => ast_v1::Decl::Signature {
            kw: kw.clone(),
            name: name.clone(),
            eq: eq.clone(),
            sig_: Box::new(subst_sig_expr(sig_, rw, path)?),
        },
        ast_v1::Decl::Include { kw, sig_ } => ast_v1::Decl::Include {
            kw: kw.clone(),
            sig_: Box::new(subst_sig_expr(sig_, rw, path)?),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_syntax::parse_file_v1;

    fn parse(src: &str) -> cst_v1::FileV1 {
        parse_file_v1(src).unwrap_or_else(|e| panic!("v1 parse failed: {e}"))
    }

    fn make_body_binds(file: &cst_v1::FileV1) -> &[cst_v1::StructBindV1] {
        let cst_v1::FileV1::Library { binds, .. } = file else {
            panic!("expected a library file")
        };
        let cst_v1::Bind::Module { body, .. } = &binds[0] else {
            panic!("expected `module Make = ..` as the first bind")
        };
        let ast_v1::ModExpr::Functor { body: fbody, .. } = &*body.0 else {
            panic!("expected a functor literal body")
        };
        functor_body_binds(fbody).expect("a struct-literal functor body")
    }

    /// T-fn1 (spec §5): `Key.compare`/`Key.t`/`Key.t int` all become
    /// `Int.*`; the control reference `Local.x` is untouched.
    #[test]
    fn substitute_param_rewrites_the_three_leaf_sites_and_leaves_others_alone() {
        let file = parse(
            "module Lib = struct\n\
             module Make = fun (Key : Ord) -> struct\n\
             val cmp x y = Key.compare x y\n\
             type t = Key.t\n\
             type u = Key.t int\n\
             val same x = Local.x\n\
             end\n\
             end",
        );
        let body_binds = make_body_binds(&file);
        let substituted = substitute_binds(body_binds, "Key", &["Int".to_string()])
            .expect("substitution should succeed");
        assert_eq!(substituted.len(), 4);

        let cst_v1::Bind::Value { body, .. } = &*substituted[0].0 else {
            panic!("expected `val cmp`")
        };
        let ast_v1::Expr::Ops(chain) = body else {
            panic!("expected an op chain")
        };
        let ast_v1::Atomic::VarWithMod(v) = &chain.head.head else {
            panic!("expected a qualified variable reference")
        };
        assert_eq!(v.mods, vec!["Int".to_string()]);
        assert_eq!(v.name, "compare");

        let cst_v1::Bind::Type { first, .. } = &*substituted[1].0 else {
            panic!("expected `type t`")
        };
        let cst_v1::TypeBodyV1::Synonym(ty) = &first.body else {
            panic!("expected a synonym body")
        };
        let ast_v1::TypeExpr::Atom(prod) = ty else {
            panic!("expected an atom type")
        };
        let ast_v1::TypeApp::Atom(ast_v1::TypeAtom::LongName(v)) = &prod.first else {
            panic!("expected a qualified type atom")
        };
        assert_eq!(v.mods, vec!["Int".to_string()]);
        assert_eq!(v.name, "t");

        let cst_v1::Bind::Type { first, .. } = &*substituted[2].0 else {
            panic!("expected `type u`")
        };
        let cst_v1::TypeBodyV1::Synonym(ty) = &first.body else {
            panic!("expected a synonym body")
        };
        let ast_v1::TypeExpr::Atom(prod) = ty else {
            panic!("expected an atom type")
        };
        let ast_v1::TypeApp::AppliedLong { ctor, .. } = &prod.first else {
            panic!("expected a qualified type-application head")
        };
        assert_eq!(ctor.mods, vec!["Int".to_string()]);
        assert_eq!(ctor.name, "t");

        let cst_v1::Bind::Value { body, .. } = &*substituted[3].0 else {
            panic!("expected `val same` (the control)")
        };
        let ast_v1::Expr::Ops(chain) = body else {
            panic!("expected an op chain")
        };
        let ast_v1::Atomic::VarWithMod(v) = &chain.head.head else {
            panic!("expected a qualified variable reference")
        };
        assert_eq!(
            v.mods,
            vec!["Local".to_string()],
            "an unrelated qualified reference must stay untouched"
        );
    }

    /// A parameter-qualified command name (`\Key.cmd`) inside a functor body
    /// is a precise, live error — never a silent pass-through (spec §2.1).
    #[test]
    fn substitute_param_rejects_a_parameter_qualified_command_name() {
        let file = parse(
            "module Lib = struct\n\
             module Make = fun (Key : Ord) -> struct\n\
             val f ctx = { \\Key.cmd; }\n\
             end\n\
             end",
        );
        let body_binds = make_body_binds(&file);
        let err = substitute_binds(body_binds, "Key", &["Int".to_string()])
            .expect_err("a parameter-qualified command name must be rejected");
        assert!(err.hint.contains("2f-2"), "{err:?}");
    }
}
