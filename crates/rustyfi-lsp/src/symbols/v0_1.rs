//! The SATySFi **0.1.0** outline.
//!
//! 0.1 splits sharply by file kind, and the split is what shapes this walk:
//!
//! - A **library** is *one* top-level `module M [:> S] = struct … end`
//!   (`FileV1::Library`). Everything it declares is a `Bind` inside that
//!   `struct`, possibly nested several modules deep.
//! - A **document** has no top-level binding sequence at all
//!   (`FileV1::Document` is `header* expr EOI`): every `let` chains its own
//!   `in`, so the whole outline lives in the body's spine.
//!
//! Both are reached from the same place: after the headers, `Vec<Bind>` is
//! tried. Every `Bind` arm is keyword-headed (`val`/`type`/`module`/
//! `signature`/`include`) and no 0.1 expression can begin with one of those,
//! so an empty result means "this is a document" with no lookahead of its
//! own, and the spine walk takes over.
//!
//! Header family: `use package M`, `use M`, `use M of \`path\``, and the
//! legacy `@require:`/`@import:` — all four accepted by this one grammar
//! (which family is *legal* is the loader's question, not the outline's).

use rustyfi_syntax::cst_v1::{self as v1, ast};
use rustyfi_syntax::leaf::*;
use rustyfi_syntax::Span;
use syan::span::Spanned;

use super::{many, node_span, opt, qualified_command, Ranges, Sym, Symbol, SymbolKind};
use crate::high_water::HighWaterStream;

/// Walk a whole 0.1 buffer.
pub(super) fn walk(stream: &mut HighWaterStream, r: &Ranges<'_>) -> Vec<Symbol> {
    let mut out = Vec::new();
    for h in many::<v1::HeaderV1>(stream) {
        if let Some(s) = header(&h, r) {
            out.push(s);
        }
    }
    let binds = many::<v1::Bind>(stream);
    if binds.is_empty() {
        spine(stream, r, &mut out);
        return out;
    }
    for b in &binds {
        bind(b, r, &mut out);
    }
    out
}

fn header(h: &v1::HeaderV1, r: &Ranges<'_>) -> Option<Symbol> {
    let whole = node_span(h);
    let (name, kind, detail) = match h {
        v1::HeaderV1::UsePackage { path, .. } => {
            (path.render(), SymbolKind::Package, "use package")
        }
        v1::HeaderV1::UseOf { path, .. } => (path.render(), SymbolKind::File, "use … of"),
        v1::HeaderV1::Use { path, .. } => (path.render(), SymbolKind::Module, "use"),
        v1::HeaderV1::Legacy(l) => {
            return match l {
                v1::Header::Require(t) => Some(
                    Sym::new(t.content.trim(), SymbolKind::Package, whole, whole)
                        .detail("@require:")
                        .build(r),
                ),
                v1::Header::Import(t) => Some(
                    Sym::new(t.content.trim(), SymbolKind::File, whole, whole)
                        .detail("@import:")
                        .build(r),
                ),
                // 0.1 has no `@stage:` header — the lexer rejects one — so
                // this arm is unreachable through a 0.1 parse. Skipped for
                // the same reason 0.0.6 skips it: it names nothing.
                v1::Header::Stage(_) => None,
            };
        }
    };
    Some(Sym::new(name, kind, whole, whole).detail(detail).build(r))
}

/// One `bind` — the 0.1 declaration form, at the top level of a library or
/// inside any `struct … end`.
fn bind(b: &v1::Bind, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    // Lazy: the two `and`-chained forms below build a span per clause instead,
    // and `node_span` on a `module` walks every token it contains.
    let whole = || node_span(b);
    match b {
        v1::Bind::Value {
            stage,
            name,
            params,
            ..
        } => {
            let kind = match params.is_empty() {
                true => SymbolKind::Variable,
                false => SymbolKind::Function,
            };
            out.push(
                Sym::new(&name.name, kind, whole(), name.span)
                    .detail(staged("val", stage.as_ref()))
                    .build(r),
            );
        }
        v1::Bind::ValueInline { stage, cmd, .. } => out.push(
            command(cmd_horz(cmd), whole(), cmd_horz_span(cmd))
                .detail(staged("val inline", stage.as_ref()))
                .build(r),
        ),
        v1::Bind::ValueBlock { stage, cmd, .. } => out.push(
            command(cmd_vert(cmd), whole(), cmd_vert_span(cmd))
                .detail(staged("val block", stage.as_ref()))
                .build(r),
        ),
        v1::Bind::ValueMath { stage, cmd, .. } => out.push(
            command(cmd_horz(cmd), whole(), cmd_horz_span(cmd))
                .detail(staged("val math", stage.as_ref()))
                .build(r),
        ),
        v1::Bind::ValueRec {
            kw,
            stage,
            first,
            ands,
            ..
        } => {
            let detail = staged("val rec", stage.as_ref());
            out.push(rec_clause(first, kw.span().unite(node_span(first)), &detail).build(r));
            for a in ands {
                out.push(rec_clause(&a.clause, node_span(a), &detail).build(r));
            }
        }
        v1::Bind::ValueMutable { stage, name, .. } => out.push(
            Sym::new(&name.name, SymbolKind::Variable, whole(), name.span)
                .detail(staged("val mutable", stage.as_ref()))
                .build(r),
        ),
        v1::Bind::Type { kw, first, ands } => {
            out.push(type_clause(first, kw.span().unite(node_span(first)), r).build(r));
            for a in ands {
                out.push(type_clause(&a.bind, node_span(a), r).build(r));
            }
        }
        v1::Bind::Module {
            name,
            sig_annot,
            body,
            ..
        } => {
            let mut children = Vec::new();
            if let Some(s) = sig_annot {
                if let Some(sym) = sig_block(&s.sig_, r) {
                    children.push(sym);
                }
            }
            struct_binds(body, r, &mut children);
            out.push(
                Sym::new(&name.name, SymbolKind::Module, whole(), name.span)
                    .detail("module")
                    .children(children)
                    .build(r),
            );
        }
        v1::Bind::Signature { name, sig_, .. } => {
            out.push(
                Sym::new(&name.name, SymbolKind::Interface, whole(), name.span)
                    .detail("signature")
                    .children(sig_decls(sig_, r))
                    .build(r),
            );
        }
        v1::Bind::Include { kw, body } => out.push(
            Sym::new(mod_expr_label(body), SymbolKind::Module, whole(), kw.span())
                .detail("include")
                .build(r),
        ),
    }
}

fn rec_clause(c: &ast::RecClauseV1, whole: Span, detail: &str) -> Sym {
    Sym::new(&c.name.name, SymbolKind::Function, whole, c.name.span).detail(detail)
}

fn command(name: String, whole: Span, sel: Span) -> Sym {
    Sym::new(name, SymbolKind::Function, whole, sel)
}

fn cmd_horz(c: &AnyHorzCmdTok) -> String {
    match c {
        AnyHorzCmdTok::Plain(t) => t.name.clone(),
        AnyHorzCmdTok::Mod(t) => qualified_command(&t.mods, &t.name),
    }
}

fn cmd_horz_span(c: &AnyHorzCmdTok) -> Span {
    match c {
        AnyHorzCmdTok::Plain(t) => t.span,
        AnyHorzCmdTok::Mod(t) => t.span,
    }
}

fn cmd_vert(c: &AnyVertCmdTok) -> String {
    match c {
        AnyVertCmdTok::Plain(t) => t.name.clone(),
        AnyVertCmdTok::Mod(t) => qualified_command(&t.mods, &t.name),
    }
}

fn cmd_vert_span(c: &AnyVertCmdTok) -> Span {
    match c {
        AnyVertCmdTok::Plain(t) => t.span,
        AnyVertCmdTok::Mod(t) => t.span,
    }
}

fn type_clause(t: &v1::TypeBindSingleV1, whole: Span, r: &Ranges<'_>) -> Sym {
    match &t.body {
        v1::TypeBodyV1::Variant { first, rest, .. } => {
            let mut children = vec![ctor(first, r)];
            children.extend(rest.iter().map(|b| ctor(&b.def, r)));
            Sym::new(&t.name.name, SymbolKind::Enum, whole, t.name.span)
                .detail("type")
                .children(children)
        }
        v1::TypeBodyV1::Synonym(_) => {
            Sym::new(&t.name.name, SymbolKind::TypeParameter, whole, t.name.span).detail("type")
        }
    }
}

fn ctor(v: &v1::VariantDefV1, r: &Ranges<'_>) -> Symbol {
    Sym::new(
        &v.ctor.name,
        SymbolKind::EnumMember,
        node_span(v),
        v.ctor.span,
    )
    .build(r)
}

/// The binds of a module expression, as that module's children.
///
/// Only a `struct … end` has members to list. A functor's body is descended
/// into (`fun (X : S) -> struct … end` declares the same things a plain
/// `struct` does, once applied); a bare path, an application and a coercion
/// name a module declared elsewhere and contribute nothing this file can see.
fn struct_binds(m: &ast::ModExpr, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    match m {
        ast::ModExpr::Struct { binds, .. } => {
            for b in binds {
                bind(&b.0, r, out);
            }
        }
        ast::ModExpr::Functor { body, .. } => struct_binds(body, r, out),
        ast::ModExpr::Coerce { .. } | ast::ModExpr::App { .. } | ast::ModExpr::Var(_) => {}
    }
}

/// A `:>` annotation's `sig … end` block, as one collapsible node — the same
/// treatment 0.0.6's `: sig … end` gets, for the same reason (its items
/// re-declare what the `struct` body binds).
///
/// A `:> S` naming a signature declared elsewhere has no items *here*, so it
/// contributes no node at all rather than an empty one.
fn sig_block(s: &ast::SigExpr, r: &Ranges<'_>) -> Option<Symbol> {
    let ast::SigBotV1::Sig { sig_kw, decls, .. } = sig_bot(s)? else {
        return None;
    };
    Some(
        Sym::new("sig", SymbolKind::Interface, node_span(s), sig_kw.span())
            .children(decl_list(decls, r))
            .build(r),
    )
}

/// The items of a signature expression, flattened into the caller's own
/// children (used by `signature S = sig … end`, where the `sig` block *is*
/// the declaration rather than an annotation on one).
fn sig_decls(s: &ast::SigExpr, r: &Ranges<'_>) -> Vec<Symbol> {
    match sig_bot(s) {
        Some(ast::SigBotV1::Sig { decls, .. }) => decl_list(decls, r),
        _ => Vec::new(),
    }
}

fn decl_list(decls: &[v1::StructDeclV1], r: &Ranges<'_>) -> Vec<Symbol> {
    let mut out = Vec::with_capacity(decls.len());
    for d in decls {
        decl(&d.0, r, &mut out);
    }
    out
}

/// The `sigexpr_bot` whose members a signature expression ultimately has.
///
/// - `S with type t = …` refines `S`; the members are still `S`'s.
/// - `(X : S1) -> S2` is a functor signature, and a functor's members are the
///   **codomain's** — `module Make : (Ord : Ord) -> sig … end` is how the
///   bundled `set.satyg`/`map.satyg` declare theirs, so declining here would
///   leave those two declaring nothing at all.
///
/// A bare `S`/`M.N.S` names a signature declared elsewhere; this file has no
/// decl list for it, which is exactly what the caller has to distinguish from
/// "an empty `sig … end`".
fn sig_bot(s: &ast::SigExpr) -> Option<&ast::SigBotV1> {
    match s {
        ast::SigExpr::Bot(b) => Some(b),
        ast::SigExpr::WithType { base, .. } => Some(base),
        ast::SigExpr::Functor { cod, .. } => sig_bot(cod),
    }
}

/// One item of a `sig … end`, appended to `out`.
///
/// Appended rather than returned for the same reason [`bind`] is: `type t = …
/// and u = …` is one `decl` naming two types, and they belong beside each
/// other rather than one inside the other.
fn decl(d: &ast::Decl, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    let whole = || node_span(d);
    match d {
        ast::Decl::Val {
            stage, name, ty, ..
        } => out.push(
            Sym::new(&name.name, declared_kind(ty), whole(), name.span)
                .detail(staged("val", stage.as_ref()))
                .build(r),
        ),
        // A command is always callable, whatever its declared type looks
        // like: `inline`/`block`/`math` command types are type ATOMS, not
        // arrows, so reading the type would call every `val \emph : …` a
        // variable.
        ast::Decl::ValHorzCmd { cmd, .. } => out.push(
            Sym::new(&cmd.name, SymbolKind::Function, whole(), cmd.span)
                .detail("val")
                .build(r),
        ),
        ast::Decl::ValVertCmd { cmd, .. } => out.push(
            Sym::new(&cmd.name, SymbolKind::Function, whole(), cmd.span)
                .detail("val")
                .build(r),
        ),
        ast::Decl::TypeOpaque { name, .. } => out.push(
            Sym::new(&name.name, SymbolKind::TypeParameter, whole(), name.span)
                .detail("type")
                .build(r),
        ),
        // A `type` decl carries a whole `bind_type` chain: the head clause
        // takes the declaration's own range and each `and` clause its own,
        // the same shaping `Bind::Type` uses.
        ast::Decl::Type { kw, binds } => {
            out.push(
                type_clause(&binds.first, kw.span().unite(node_span(&binds.first)), r).build(r),
            );
            for a in &binds.ands {
                out.push(type_clause(&a.bind, node_span(a), r).build(r));
            }
        }
        ast::Decl::Module { name, sig_, .. } => out.push(
            Sym::new(&name.name, SymbolKind::Module, whole(), name.span)
                .detail("module")
                .children(sig_decls(sig_, r))
                .build(r),
        ),
        ast::Decl::Signature { name, sig_, .. } => out.push(
            Sym::new(&name.name, SymbolKind::Interface, whole(), name.span)
                .detail("signature")
                .children(sig_decls(sig_, r))
                .build(r),
        ),
        ast::Decl::Include { kw, sig_ } => out.push(
            Sym::new(
                sig_expr_label(sig_),
                SymbolKind::Interface,
                whole(),
                kw.span(),
            )
            .detail("include")
            .children(sig_decls(sig_, r))
            .build(r),
        ),
    }
}

fn declared_kind(ty: &ast::TypeExpr) -> SymbolKind {
    match ty {
        ast::TypeExpr::Fun { .. } | ast::TypeExpr::OptRowFun { .. } => SymbolKind::Function,
        ast::TypeExpr::Atom(_) => SymbolKind::Variable,
    }
}

/// What to call an `include`d module expression in the outline.
fn mod_expr_label(m: &ast::ModExpr) -> String {
    match m {
        ast::ModExpr::Var(c) => c.render(),
        ast::ModExpr::App { func, arg } => format!("{} {}", func.render(), arg.render()),
        ast::ModExpr::Coerce { name, .. } => name.name.clone(),
        ast::ModExpr::Struct { .. } => "struct".to_string(),
        ast::ModExpr::Functor { param, .. } => format!("fun ({})", param.name),
    }
}

/// What to call an `include`d signature expression.
fn sig_expr_label(s: &ast::SigExpr) -> String {
    let Some(bot) = sig_bot(s) else {
        return "sig".to_string();
    };
    match bot {
        ast::SigBotV1::Var(c) => c.name.clone(),
        ast::SigBotV1::Path(p) => {
            let mut parts = p.mods.clone();
            parts.push(p.name.clone());
            parts.join(".")
        }
        ast::SigBotV1::Sig { .. } => "sig".to_string(),
    }
}

/// The document body's `let … in` spine, one clause at a time.
///
/// This is where a 0.1 *document* keeps everything it declares, so partial
/// recovery matters far more here than it does under 0.0.6: parsing the body
/// as one expression would mean an unfinished `let` at the bottom of the file
/// costs every symbol above it. Each arm mirrors one `let`-headed variant of
/// `cst_v1::ast::Expr`; 0.1 spells `let rec`/`let mutable`/`let open` as two
/// tokens where 0.0.6 fuses them into one, so the second token is what
/// discriminates.
fn spine(stream: &mut HighWaterStream, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    loop {
        let Some(kw) = opt::<KwLet>(stream) else {
            return;
        };
        // `let rec f … and g … in`
        if opt::<KwRec>(stream).is_some() {
            let Some(first) = opt::<ast::RecClauseV1>(stream) else {
                return;
            };
            out.push(rec_clause(&first, kw.span().unite(node_span(&first)), "let rec").build(r));
            for a in many::<ast::AndClauseV1>(stream) {
                out.push(rec_clause(&a.clause, node_span(&a), "let rec").build(r));
            }
            if opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `let mutable x <- init in`
        if opt::<KwMutable>(stream).is_some() {
            let Some(name) = opt::<VarTok>(stream) else {
                return;
            };
            if opt::<OverwriteEqTok>(stream).is_none() {
                return;
            }
            let Some(init) = opt::<v1::ExprErasedV1>(stream) else {
                return;
            };
            out.push(
                Sym::new(
                    &name.name,
                    SymbolKind::Variable,
                    kw.span().unite(node_span(&init)),
                    name.span,
                )
                .detail("let mutable")
                .build(r),
            );
            if opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `let open Foo in` — binds nothing, but the spine continues past it.
        if opt::<KwOpen>(stream).is_some() {
            if opt::<CtorTok>(stream).is_none() || opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `let name p* = value in`, or the destructuring `let pat = value in`.
        if let Some(name) = opt::<v1::BindName>(stream) {
            let params = many::<v1::Param>(stream);
            if opt::<DefEqTok>(stream).is_none() {
                return;
            }
            let Some(value) = opt::<v1::ExprErasedV1>(stream) else {
                return;
            };
            let kind = match params.is_empty() {
                true => SymbolKind::Variable,
                false => SymbolKind::Function,
            };
            out.push(
                Sym::new(
                    &name.name,
                    kind,
                    kw.span().unite(node_span(&value)),
                    name.span,
                )
                .detail("let")
                .build(r),
            );
        } else {
            // Destructuring: the names live inside a pattern, which is a
            // separate walk; the spine steps over it.
            if opt::<v1::PatErasedV1>(stream).is_none()
                || opt::<DefEqTok>(stream).is_none()
                || opt::<v1::ExprErasedV1>(stream).is_none()
            {
                return;
            }
        }
        if opt::<KwIn>(stream).is_none() {
            return;
        }
    }
}

/// 0.1 declares a stage per binding rather than per file, so this is on every
/// `val` arm. `val ~x` is stage 0, `val persistent ~x` the persistent stage,
/// and a bare `val` the document stage — which needs no annotation.
fn staged(kw: &str, stage: Option<&v1::BindStageV1>) -> String {
    match stage {
        None => kw.to_string(),
        Some(s) if s.persistent.is_some() => format!("{kw} (persistent)"),
        Some(_) => format!("{kw} (stage 0)"),
    }
}
