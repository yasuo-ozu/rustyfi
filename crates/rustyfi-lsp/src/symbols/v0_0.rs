//! The SATySFi **0.0.6** outline.
//!
//! What a 0.0.6 file declares, and where:
//!
//! | construct | where it lives |
//! |---|---|
//! | `@require:` / `@import:` / `@stage:` | the header block |
//! | `let` / `let-rec` / `let-mutable` / `type` | the top-level prelude, and a `struct` body |
//! | `let-inline` / `let-block` / `let-math` | the same — no `in`-bodied form |
//! | `module M [: sig … end] = struct … end` | the top-level prelude only |
//! | `val` / `direct` / `type` items | a `sig … end` block |
//! | `let … in` / `let-math … in` / `let-mutable … in` | the document body, after `in` |
//!
//! The prelude is where a 0.0.6 document puts everything, so the body spine
//! matters much less here than it does under 0.1 — but it costs one shared
//! walk, and `open Foo in let x = …` after the `in` is legal and does happen.

use rustyfi_syntax::cst::{self, ast};
use rustyfi_syntax::leaf::*;
use rustyfi_syntax::Span;
use syan::span::Spanned;

use super::{many, node_span, opt, Ranges, Sym, Symbol, SymbolKind};
use crate::high_water::HighWaterStream;

/// Walk a whole 0.0.6 buffer: `header* binding* [in body]`.
pub(super) fn walk(stream: &mut HighWaterStream, r: &Ranges<'_>) -> Vec<Symbol> {
    let mut out = Vec::new();
    for h in many::<cst::Header>(stream) {
        if let Some(s) = header(&h, r) {
            out.push(s);
        }
    }
    for b in many::<cst::TopBinding>(stream) {
        binding(&b, r, &mut out);
    }
    // `in` is absent for a library, and also for a document with an empty
    // prelude (`@require: x` then straight into `document (| … |) '<…>`), so
    // it is optional and the spine walk runs either way — on a body that is
    // not `let`-headed it stops immediately.
    let _ = opt::<KwIn>(stream);
    spine(stream, r, &mut out);
    out
}

/// `@require: foo` / `@import: bar`. `@stage:` declares nothing and is
/// skipped — it qualifies the file, it does not name anything in it.
fn header(h: &cst::Header, r: &Ranges<'_>) -> Option<Symbol> {
    let (name, kind, detail) = match h {
        cst::Header::Require(t) => (&t.content, SymbolKind::Package, "@require:"),
        cst::Header::Import(t) => (&t.content, SymbolKind::File, "@import:"),
        cst::Header::Stage(_) => return None,
    };
    let span = node_span(h);
    Some(
        Sym::new(name.trim(), kind, span, span)
            .detail(detail)
            .build(r),
    )
}

/// One top-level (or `struct`-body) declaration, appended to `out`.
///
/// Appended rather than returned because one declaration can name several
/// things: `let-rec f = … and g = …` and `type t = … and u = …` each produce
/// one symbol per clause, so that "go to symbol" can reach `g` and `u`.
fn binding(b: &cst::TopBinding, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    // Lazy: the two `and`-chained forms below build a span per clause instead,
    // and `node_span` on a `module` walks every token it contains.
    let whole = || node_span(b);
    // Matched exhaustively on purpose: `TopBinding` is not
    // `#[non_exhaustive]`, so a new declaration form added to the grammar is a
    // compile error here rather than a silently missing symbol.
    match b {
        cst::TopBinding::LetRec {
            kw,
            stage,
            first,
            ands,
            ..
        } => {
            let detail = staged("let-rec", stage.as_ref());
            out.push(rec_clause(first, kw.span().unite(node_span(first)), &detail).build(r));
            for a in ands {
                out.push(rec_clause(&a.binding, node_span(a), &detail).build(r));
            }
        }
        cst::TopBinding::Let(l) => {
            let kind = match l.params.is_empty() {
                true => SymbolKind::Variable,
                false => SymbolKind::Function,
            };
            out.push(
                Sym::new(&l.name.name, kind, whole(), l.name.span)
                    .detail(staged("let", l.stage.as_ref()))
                    .build(r),
            );
        }
        // A destructuring `let (a, b) = …` binds names inside a pattern.
        // Walking patterns is a separate (and much larger) job than walking
        // declarations, and this shape is rare at the top level, so it is
        // deliberately left out rather than half-done.
        cst::TopBinding::LetPattern { .. } => {}
        cst::TopBinding::LetInline { stage, cmd, .. } => out.push(
            Sym::new(&cmd.name, SymbolKind::Function, whole(), cmd.span)
                .detail(staged("let-inline", stage.as_ref()))
                .build(r),
        ),
        cst::TopBinding::LetBlock { stage, cmd, .. } => out.push(
            Sym::new(&cmd.name, SymbolKind::Function, whole(), cmd.span)
                .detail(staged("let-block", stage.as_ref()))
                .build(r),
        ),
        cst::TopBinding::LetMath { stage, cmd, .. } => out.push(
            Sym::new(&cmd.name, SymbolKind::Function, whole(), cmd.span)
                .detail(staged("let-math", stage.as_ref()))
                .build(r),
        ),
        cst::TopBinding::Type(t) => type_decl(t, r, out),
        cst::TopBinding::LetMutable { stage, name, .. } => out.push(
            Sym::new(&name.name, SymbolKind::Variable, whole(), name.span)
                .detail(staged("let-mutable", stage.as_ref()))
                .build(r),
        ),
        cst::TopBinding::Module {
            name, sig, decls, ..
        } => {
            let mut children = Vec::new();
            if let Some(sig) = sig {
                children.push(sig_annot(sig, r));
            }
            for d in decls {
                binding(&d.0, r, &mut children);
            }
            out.push(
                Sym::new(&name.name, SymbolKind::Module, whole(), name.span)
                    .detail("module")
                    .children(children)
                    .build(r),
            );
        }
        // `open Foo` introduces no name of its own.
        cst::TopBinding::Open { .. } => {}
    }
}

/// One `let-rec` clause (the head one or an `and` continuation).
fn rec_clause(c: &ast::RecBinding, whole: Span, detail: &str) -> Sym {
    Sym::new(&c.name.name, SymbolKind::Function, whole, c.name.span).detail(detail)
}

/// `type t = …  and u = …`, one symbol per clause, each variant's
/// constructors as that clause's children.
fn type_decl(t: &cst::TypeDecl, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    out.push(type_clause(&t.name, &t.body, t.kw.span().unite(node_span(&t.body)), r).build(r));
    for a in &t.ands {
        out.push(type_clause(&a.name, &a.body, node_span(a), r).build(r));
    }
}

fn type_clause(name: &VarTok, body: &cst::TypeDeclBody, whole: Span, r: &Ranges<'_>) -> Sym {
    match body {
        cst::TypeDeclBody::Variant { first, rest, .. } => {
            let mut children = vec![ctor(first, r)];
            children.extend(rest.iter().map(|b| ctor(&b.def, r)));
            Sym::new(&name.name, SymbolKind::Enum, whole, name.span)
                .detail("type")
                .children(children)
        }
        cst::TypeDeclBody::Synonym(_) => {
            Sym::new(&name.name, SymbolKind::TypeParameter, whole, name.span).detail("type")
        }
    }
}

fn ctor(v: &cst::VariantDef, r: &Ranges<'_>) -> Symbol {
    Sym::new(
        &v.ctor.name,
        SymbolKind::EnumMember,
        node_span(v),
        v.ctor.span,
    )
    .build(r)
}

/// The `: sig … end` annotation on a module, as one collapsible node.
///
/// Its items are *declarations of the same names* the `struct` body binds, so
/// hoisting them up beside the implementations would double every entry.
/// Under their own node they stay one keystroke away and one fold out of the
/// way — and the `direct` items, which are the only place a 0.0.6 package
/// states which commands it exports, are visible.
fn sig_annot(s: &cst::SigAnnot, r: &Ranges<'_>) -> Symbol {
    let children = s.items.iter().map(|i| sig_item(i, r)).collect();
    Sym::new("sig", SymbolKind::Interface, node_span(s), s.sig_kw.span())
        .children(children)
        .build(r)
}

fn sig_item(i: &cst::SigItem, r: &Ranges<'_>) -> Symbol {
    let whole = node_span(i);
    // A command is always callable, whatever its declared type looks like:
    // `inline-cmd`/`block-cmd`/`math-cmd` are type ATOMS, not arrows, so
    // reading the type would call every `direct \href : … inline-cmd` a
    // variable.
    let (name, sel, kind, detail) = match i {
        cst::SigItem::ValHorzCmd { name, .. } => {
            (name.name.clone(), name.span, SymbolKind::Function, "val")
        }
        cst::SigItem::ValVertCmd { name, .. } => {
            (name.name.clone(), name.span, SymbolKind::Function, "val")
        }
        cst::SigItem::Val { name, ty, .. } => {
            (name.name.clone(), name.span, declared_kind(ty), "val")
        }
        cst::SigItem::DirectHorzCmd { name, .. } => {
            (name.name.clone(), name.span, SymbolKind::Function, "direct")
        }
        cst::SigItem::DirectVertCmd { name, .. } => {
            (name.name.clone(), name.span, SymbolKind::Function, "direct")
        }
        cst::SigItem::Type { name, .. } => (
            name.name.clone(),
            name.span,
            SymbolKind::TypeParameter,
            "type",
        ),
    };
    Sym::new(name, kind, whole, sel).detail(detail).build(r)
}

/// A declared type tells the outline whether the name is a function: an arrow
/// (with or without an optional-argument prefix) is, anything else is not.
fn declared_kind(ty: &ast::TypeExpr) -> SymbolKind {
    match ty {
        ast::TypeExpr::Fun { .. } | ast::TypeExpr::OptRowFun { .. } => SymbolKind::Function,
        ast::TypeExpr::Atom(_) => SymbolKind::Variable,
    }
}

/// The document body's `let … in` spine, one clause at a time.
///
/// Clause by clause rather than "parse the body, then walk the tree": a
/// failure in clause seven then still leaves the first six on screen. Each
/// arm mirrors one variant of `cst::ast::Expr`'s `let`-headed family field
/// for field; anything else ends the spine, which is the common case (the
/// body is normally `document (| … |) '<…>`).
fn spine(stream: &mut HighWaterStream, r: &Ranges<'_>, out: &mut Vec<Symbol>) {
    loop {
        // `let-rec … and … in`
        if let Some(kw) = opt::<KwLetRec>(stream) {
            let Some(first) = opt::<ast::RecBinding>(stream) else {
                return;
            };
            out.push(rec_clause(&first, kw.span().unite(node_span(&first)), "let-rec").build(r));
            for a in many::<ast::AndBinding>(stream) {
                out.push(rec_clause(&a.binding, node_span(&a), "let-rec").build(r));
            }
            if opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `let-mutable x <- init in`
        if let Some(kw) = opt::<KwLetMutable>(stream) {
            let Some(name) = opt::<VarTok>(stream) else {
                return;
            };
            if opt::<OverwriteEqTok>(stream).is_none() {
                return;
            }
            let Some(init) = opt::<cst::ExprErased>(stream) else {
                return;
            };
            out.push(
                Sym::new(
                    &name.name,
                    SymbolKind::Variable,
                    kw.span().unite(node_span(&init)),
                    name.span,
                )
                .detail("let-mutable")
                .build(r),
            );
            if opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `let-math \cmd p* = value in`
        if let Some(kw) = opt::<KwLetMath>(stream) {
            let Some(cmd) = opt::<HorzCmdTok>(stream) else {
                return;
            };
            let _ = many::<ast::Param>(stream);
            if opt::<DefEqTok>(stream).is_none() {
                return;
            }
            let Some(value) = opt::<cst::ExprErased>(stream) else {
                return;
            };
            out.push(
                Sym::new(
                    &cmd.name,
                    SymbolKind::Function,
                    kw.span().unite(node_span(&value)),
                    cmd.span,
                )
                .detail("let-math")
                .build(r),
            );
            if opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `open Foo in` — binds nothing, but the spine continues past it.
        if opt::<KwOpen>(stream).is_some() {
            if opt::<CtorTok>(stream).is_none() || opt::<KwIn>(stream).is_none() {
                return;
            }
            continue;
        }
        // `let name p* = value in`, or the destructuring `let pat = value in`.
        let Some(kw) = opt::<KwLet>(stream) else {
            return;
        };
        if let Some(name) = opt::<cst::BindName>(stream) {
            let _ = opt::<ast::RecAscription>(stream);
            let _ = opt::<BarTok>(stream);
            let params = many::<ast::Param>(stream);
            if opt::<DefEqTok>(stream).is_none() {
                return;
            }
            let Some(value) = opt::<cst::ExprErased>(stream) else {
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
            // Destructuring: skipped for the same reason as
            // `TopBinding::LetPattern`, but the spine walks past it.
            if opt::<cst::PatErased>(stream).is_none()
                || opt::<DefEqTok>(stream).is_none()
                || opt::<cst::ExprErased>(stream).is_none()
            {
                return;
            }
        }
        if opt::<KwIn>(stream).is_none() {
            return;
        }
    }
}

/// `"let"` → `"let"`, but `"let"` under `~` → `"let (stage 0)"`.
///
/// The stage qualifier is the one thing about a 0.0.6 binding that is not
/// visible from its name, and 0.1 puts it on every binding, so both walks
/// render it the same way.
fn staged(kw: &str, stage: Option<&cst::TopStage>) -> String {
    match stage {
        None => kw.to_string(),
        Some(s) if s.persistent.is_some() => format!("{kw} (persistent)"),
        Some(_) => format!("{kw} (stage 0)"),
    }
}
