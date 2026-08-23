//! Filling a [`Builder`] from the SATySFi **0.0.6** concrete syntax tree.
//!
//! # Scopes without spans
//!
//! The CST nodes derive `Parse`/`Unparse` but not syan's `Spanned`, so there
//! is no `expr.span()` to ask for — and adding one would mean putting a third
//! derive through the `#[recurse]` engine that `cst.rs` documents as
//! compile-time-explosive. It is not needed: every construct in this grammar
//! is delimited by tokens that *do* carry spans (`in`, `->`, `then`, `else`,
//! `=`, and the group delimiters), so each walk function takes the byte offset
//! where its node ends and derives every inner boundary from a keyword.
//!
//! The one boundary a keyword cannot give is between two consecutive top-level
//! bindings, and that one comes from the *next* binding's leading keyword
//! ([`binding_start`]).
//!
//! # Two ends, not one
//!
//! A top-level binding has a **text** end (where its own value expression
//! stops) and a **scope** end (how far the name it binds is visible). They
//! differ for the last binding of a document: its value stops at `in`, but the
//! name it bound is visible throughout the body that follows. Conflating them
//! puts every top-level `let`'s *parameters* in scope in the document body,
//! which is exactly the kind of quietly-wrong answer this crate is trying not
//! to give.

use rustyfi_syntax::cst::{self, ast};
use rustyfi_syntax::leaf::*;
use rustyfi_syntax::span::Span;

use crate::model::{node_span, Builder, ByteRange, Def, Ns};

/// Byte offset of a span's first byte — the boundary form every scope
/// computation below wants.
fn at(span: Span) -> usize {
    span.start.byte
}

/// Byte offset one past a span's last byte.
fn past(span: Span) -> usize {
    span.end.byte
}

/// The range an ascription's type occupies, for hover to quote.
fn ascription_span(a: Option<&ast::RecAscription>) -> Option<ByteRange> {
    a.map(|a| node_span(&a.ty))
}

/// Where a top-level (or `struct`-level) binding's text begins.
///
/// Every alternative starts with a keyword, so this is total and needs no
/// span on the binding itself.
fn binding_start(b: &cst::TopBinding) -> usize {
    use cst::TopBinding as T;
    match b {
        T::LetRec { kw, .. } => at(kw.0),
        T::Let(l) => at(l.let_kw.0),
        T::LetPattern { let_kw, .. } => at(let_kw.0),
        T::LetInline { kw, .. } => at(kw.0),
        T::LetBlock { kw, .. } => at(kw.0),
        T::LetMath { kw, .. } => at(kw.0),
        T::Type(t) => at(t.kw.0),
        T::LetMutable { kw, .. } => at(kw.0),
        T::Module { kw, .. } => at(kw.0),
        T::Open { kw, .. } => at(kw.0),
    }
}

impl Builder<'_> {
    // -- files ---------------------------------------------------------------

    pub(crate) fn v006_file(&mut self, f: &cst::File) {
        // A library file's bindings are visible to the end of the file; a
        // document's are visible through its body too, and the body starts
        // after `in`. `eoi`'s span is the honest end of the text either way.
        let scope_end = self.source.len();
        let text_end = f.in_kw.as_ref().map(|k| at(k.0)).unwrap_or(scope_end);
        self.v006_headers(&f.headers);
        self.v006_bindings(&f.prelude, None, scope_end, text_end);
        if let Some(body) = &f.body {
            self.v006_expr(body, scope_end);
        }
    }

    /// The recovery path's entry point: the same three pieces, but reached
    /// without the `eoi` the file rule insists on.
    ///
    /// `text_end` is where the recovered bindings stop and the half-typed
    /// remainder begins. The *scope* end stays the end of the file, which is
    /// the point of the whole exercise: the names recovered here have to be
    /// visible in the text the user is still writing.
    pub(crate) fn v006_parts(
        &mut self,
        headers: &[cst::Header],
        prelude: &[cst::TopBinding],
        body: Option<&ast::Expr>,
        text_end: usize,
    ) {
        let end = self.source.len();
        self.v006_headers(headers);
        self.v006_bindings(prelude, None, end, text_end);
        if let Some(body) = body {
            self.v006_expr(body, end);
        }
    }

    fn v006_headers(&mut self, headers: &[cst::Header]) {
        for h in headers {
            match h {
                cst::Header::Require(t) => {
                    self.header(crate::model::HeaderKind::Require, &t.content, t.span)
                }
                cst::Header::Import(t) => {
                    self.header(crate::model::HeaderKind::Import, &t.content, t.span)
                }
                // `@stage:` names no file and no name.
                cst::Header::Stage(_) => {}
            }
        }
    }

    // -- binding lists -------------------------------------------------------

    fn v006_bindings(
        &mut self,
        list: &[cst::TopBinding],
        container: Option<usize>,
        scope_end: usize,
        text_end: usize,
    ) {
        for (i, b) in list.iter().enumerate() {
            let stop = list.get(i + 1).map(binding_start).unwrap_or(text_end);
            self.v006_binding(b, container, scope_end, stop);
        }
    }

    fn v006_struct_decls(&mut self, decls: &[cst::StructDecl], container: usize, end: usize) {
        for (i, d) in decls.iter().enumerate() {
            let stop = decls.get(i + 1).map(|n| binding_start(&n.0)).unwrap_or(end);
            self.v006_binding(&d.0, Some(container), end, stop);
        }
    }

    /// One binding. `scope_end` is how far a name it binds stays visible;
    /// `stop` is where its own text ends.
    fn v006_binding(
        &mut self,
        b: &cst::TopBinding,
        container: Option<usize>,
        scope_end: usize,
        stop: usize,
    ) {
        use cst::TopBinding as T;
        // The scope of a name bound here, for the non-recursive forms: from
        // just after this binding to the end of the enclosing region.
        let after = ByteRange::new(stop, scope_end);
        match b {
            T::LetRec {
                kw, first, ands, ..
            } => {
                // Every clause of a `let-rec` chain sees every other one, and
                // itself — hence a scope opening at the keyword.
                let scope = ByteRange::new(at(kw.0), scope_end);
                self.v006_rec_name(first, scope, container);
                for a in ands {
                    self.v006_rec_name(&a.binding, scope, container);
                }
                let group_end = |i: usize| ands.get(i).map(|a| at(a.and_kw.0)).unwrap_or(stop);
                self.v006_rec_body(first, group_end(0));
                for (i, a) in ands.iter().enumerate() {
                    let e = group_end(i + 1);
                    self.v006_rec_body(&a.binding, e);
                }
            }
            T::Let(l) => {
                let value = ByteRange::new(past(l.eq.0), stop);
                self.def(Def {
                    ns: Ns::Value,
                    name: l.name.name.clone(),
                    name_span: ByteRange::of(l.name.span),
                    scope: after,
                    form: "let",
                    ty: ascription_span(l.ascription.as_ref()),
                    container,
                    declaration: false,
                });
                if let Some(a) = &l.ascription {
                    self.v006_type(&a.ty);
                }
                for p in &l.params {
                    self.v006_param(p, value);
                }
                self.v006_expr(&l.value, stop);
            }
            T::LetPattern { pat, eq, value, .. } => {
                self.v006_pattern(pat, after, "let");
                let _ = eq;
                self.v006_expr(value, stop);
            }
            T::LetInline {
                ctx,
                cmd,
                params,
                eq,
                value,
                ..
            } => {
                self.v006_command_binding(
                    Ns::InlineCmd,
                    &cmd.name,
                    cmd.span,
                    after,
                    "let-inline",
                    container,
                );
                self.v006_cmd_body(ctx.as_ref(), params, past(eq.0), stop, value);
            }
            T::LetBlock {
                ctx,
                cmd,
                params,
                eq,
                value,
                ..
            } => {
                self.v006_command_binding(
                    Ns::BlockCmd,
                    &cmd.name,
                    cmd.span,
                    after,
                    "let-block",
                    container,
                );
                self.v006_cmd_body(ctx.as_ref(), params, past(eq.0), stop, value);
            }
            T::LetMath {
                cmd,
                params,
                eq,
                value,
                ..
            } => {
                self.v006_command_binding(
                    Ns::MathCmd,
                    &cmd.name,
                    cmd.span,
                    after,
                    "let-math",
                    container,
                );
                self.v006_cmd_body(None, params, past(eq.0), stop, value);
            }
            T::Type(t) => {
                // A `type … and …` chain is mutually recursive, so every name
                // in it is visible from the head keyword.
                let scope = ByteRange::new(at(t.kw.0), scope_end);
                let clause_end = |i: usize| t.ands.get(i).map(|a| at(a.and_kw.0)).unwrap_or(stop);
                self.v006_type_clause(
                    &t.tyvars,
                    &t.name,
                    &t.body,
                    scope,
                    ByteRange::new(at(t.kw.0), clause_end(0)),
                    container,
                );
                for (i, a) in t.ands.iter().enumerate() {
                    self.v006_type_clause(
                        &a.tyvars,
                        &a.name,
                        &a.body,
                        scope,
                        ByteRange::new(at(a.and_kw.0), clause_end(i + 1)),
                        container,
                    );
                }
            }
            T::LetMutable { name, value, .. } => {
                self.def(Def {
                    ns: Ns::Value,
                    name: name.name.clone(),
                    name_span: ByteRange::of(name.span),
                    scope: after,
                    form: "let-mutable",
                    ty: None,
                    container,
                    declaration: false,
                });
                self.v006_expr(value, stop);
            }
            T::Module {
                name,
                sig,
                struct_kw,
                decls,
                end_kw,
                ..
            } => {
                let inner = ByteRange::new(past(struct_kw.0), at(end_kw.0));
                let idx = self.def(Def {
                    ns: Ns::Module,
                    name: name.name.clone(),
                    name_span: ByteRange::of(name.span),
                    scope: after,
                    form: "module",
                    ty: None,
                    container,
                    declaration: false,
                });
                if let Some(sig) = sig {
                    self.v006_sig(sig, idx);
                }
                self.v006_struct_decls(decls, idx, inner.end);
            }
            T::Open { name, .. } => {
                self.reference(Ns::Module, &name.name, name.span);
                self.open_module(&[], &name.name, after);
            }
        }
    }

    /// The `[ctx] params = value` tail every command binding shares.
    fn v006_cmd_body(
        &mut self,
        ctx: Option<&VarTok>,
        params: &[ast::Param],
        value_start: usize,
        stop: usize,
        value: &ast::Expr,
    ) {
        let scope = ByteRange::new(value_start, stop);
        if let Some(c) = ctx {
            self.bind(Ns::Value, &c.name, c.span, scope, "context parameter");
        }
        for p in params {
            self.v006_param(p, scope);
        }
        self.v006_expr(value, stop);
    }

    fn v006_command_binding(
        &mut self,
        ns: Ns,
        name: &str,
        span: Span,
        scope: ByteRange,
        form: &'static str,
        container: Option<usize>,
    ) {
        self.def(Def {
            ns,
            name: name.to_string(),
            name_span: ByteRange::of(span),
            scope,
            form,
            ty: None,
            container,
            declaration: false,
        });
    }

    fn v006_rec_name(&mut self, rb: &ast::RecBinding, scope: ByteRange, container: Option<usize>) {
        self.def(Def {
            ns: Ns::Value,
            name: rb.name.name.clone(),
            name_span: ByteRange::of(rb.name.span),
            scope,
            form: "let-rec",
            ty: ascription_span(rb.ascription.as_ref()),
            container,
            declaration: false,
        });
    }

    fn v006_rec_body(&mut self, rb: &ast::RecBinding, end: usize) {
        if let Some(a) = &rb.ascription {
            self.v006_type(&a.ty);
        }
        let first_end = rb.extra.first().map(|c| at(c.bar.0)).unwrap_or(end);
        let scope = ByteRange::new(past(rb.eq.0), first_end);
        for p in &rb.params {
            self.v006_patbot(p, scope, "parameter");
        }
        self.v006_expr(&rb.value, first_end);
        for (i, c) in rb.extra.iter().enumerate() {
            let e = rb.extra.get(i + 1).map(|n| at(n.bar.0)).unwrap_or(end);
            let scope = ByteRange::new(past(c.eq.0), e);
            for p in &c.params {
                self.v006_patbot(p, scope, "parameter");
            }
            self.v006_expr(&c.value, e);
        }
    }

    fn v006_type_clause(
        &mut self,
        tyvars: &[TypeVarTok],
        name: &VarTok,
        body: &cst::TypeDeclBody,
        scope: ByteRange,
        clause: ByteRange,
        container: Option<usize>,
    ) {
        let form = match body {
            cst::TypeDeclBody::Variant { .. } => "type",
            cst::TypeDeclBody::Synonym(_) => "type synonym",
        };
        self.def(Def {
            ns: Ns::Type,
            name: name.name.clone(),
            name_span: ByteRange::of(name.span),
            scope,
            form,
            ty: match body {
                cst::TypeDeclBody::Synonym(t) => Some(node_span(t)),
                _ => None,
            },
            container,
            declaration: false,
        });
        for tv in tyvars {
            self.bind(Ns::TypeVar, &tv.name, tv.span, clause, "type parameter");
        }
        match body {
            cst::TypeDeclBody::Variant { first, rest, .. } => {
                self.v006_variant(first, scope, container);
                for r in rest {
                    self.v006_variant(&r.def, scope, container);
                }
            }
            cst::TypeDeclBody::Synonym(t) => self.v006_type(t),
        }
    }

    fn v006_variant(&mut self, v: &cst::VariantDef, scope: ByteRange, container: Option<usize>) {
        self.def(Def {
            ns: Ns::Ctor,
            name: v.ctor.name.clone(),
            name_span: ByteRange::of(v.ctor.span),
            scope,
            form: "variant constructor",
            ty: v.of_ty.as_ref().map(|o| node_span(&o.ty)),
            container,
            declaration: false,
        });
        if let Some(o) = &v.of_ty {
            self.v006_type(&o.ty);
        }
    }

    // -- signatures ----------------------------------------------------------

    fn v006_sig(&mut self, sig: &cst::SigAnnot, module: usize) {
        for item in &sig.items {
            self.v006_sig_item(item, module);
        }
    }

    fn v006_sig_item(&mut self, item: &cst::SigItem, module: usize) {
        use cst::SigItem as S;
        let declare = |b: &mut Self, ns: Ns, name: &str, span: Span, ty: &ast::TypeExpr, form| {
            b.def(Def {
                ns,
                name: name.to_string(),
                name_span: ByteRange::of(span),
                // A declaration binds nothing, so it is visible nowhere;
                // `Model::member` reaches it by container, not by scope.
                scope: ByteRange::new(0, 0),
                form,
                ty: Some(node_span(ty)),
                container: Some(module),
                declaration: true,
            });
            b.v006_type(ty);
        };
        match item {
            S::ValHorzCmd {
                name,
                ty,
                constraints,
                ..
            } => {
                declare(self, Ns::InlineCmd, &name.name, name.span, ty, "val");
                self.v006_constraints(constraints);
            }
            S::ValVertCmd {
                name,
                ty,
                constraints,
                ..
            } => {
                declare(self, Ns::BlockCmd, &name.name, name.span, ty, "val");
                self.v006_constraints(constraints);
            }
            S::Val {
                name,
                ty,
                constraints,
                ..
            } => {
                declare(self, Ns::Value, &name.name, name.span, ty, "val");
                self.v006_constraints(constraints);
            }
            S::DirectHorzCmd {
                name,
                ty,
                constraints,
                ..
            } => {
                declare(self, Ns::InlineCmd, &name.name, name.span, ty, "direct");
                self.v006_constraints(constraints);
            }
            S::DirectVertCmd {
                name,
                ty,
                constraints,
                ..
            } => {
                declare(self, Ns::BlockCmd, &name.name, name.span, ty, "direct");
                self.v006_constraints(constraints);
            }
            S::Type {
                tyvars,
                name,
                constraints,
                ..
            } => {
                self.def(Def {
                    ns: Ns::Type,
                    name: name.name.clone(),
                    name_span: ByteRange::of(name.span),
                    scope: ByteRange::new(0, 0),
                    form: "type",
                    ty: None,
                    container: Some(module),
                    declaration: true,
                });
                for tv in tyvars {
                    self.reference(Ns::TypeVar, &tv.name, tv.span);
                }
                self.v006_constraints(constraints);
            }
        }
    }

    fn v006_constraints(&mut self, cs: &[cst::SigConstraint]) {
        for c in cs {
            self.reference(Ns::TypeVar, &c.tyvar.name, c.tyvar.span);
            for f in &c.kind.fields {
                self.reference(Ns::Field, &f.name.name, f.name.span);
                self.v006_type(&f.ty);
            }
        }
    }

    // -- expressions ---------------------------------------------------------

    fn v006_expr(&mut self, e: &ast::Expr, end: usize) {
        use ast::Expr as E;
        match e {
            E::LetRecIn {
                kw,
                first,
                ands,
                in_kw,
                body,
            } => {
                let scope = ByteRange::new(at(kw.0), end);
                self.v006_rec_name(first, scope, None);
                for a in ands {
                    self.v006_rec_name(&a.binding, scope, None);
                }
                let group_end =
                    |i: usize| ands.get(i).map(|a| at(a.and_kw.0)).unwrap_or(at(in_kw.0));
                self.v006_rec_body(first, group_end(0));
                for (i, a) in ands.iter().enumerate() {
                    self.v006_rec_body(&a.binding, group_end(i + 1));
                }
                self.v006_expr(body, end);
            }
            E::LetIn {
                name,
                ascription,
                params,
                eq,
                value,
                in_kw,
                body,
                ..
            } => {
                let value_end = at(in_kw.0);
                self.def(Def {
                    ns: Ns::Value,
                    name: name.name.clone(),
                    name_span: ByteRange::of(name.span),
                    scope: ByteRange::new(past(in_kw.0), end),
                    form: "let",
                    ty: ascription_span(ascription.as_ref()),
                    container: None,
                    declaration: false,
                });
                if let Some(a) = ascription {
                    self.v006_type(&a.ty);
                }
                let param_scope = ByteRange::new(past(eq.0), value_end);
                for p in params {
                    self.v006_param(p, param_scope);
                }
                self.v006_expr(value, value_end);
                self.v006_expr(body, end);
            }
            E::LetPatternIn {
                pat,
                value,
                in_kw,
                body,
                ..
            } => {
                self.v006_pattern(pat, ByteRange::new(past(in_kw.0), end), "let");
                self.v006_expr(value, at(in_kw.0));
                self.v006_expr(body, end);
            }
            E::If {
                cond,
                then_kw,
                then_branch,
                else_kw,
                else_branch,
                ..
            } => {
                self.v006_expr(cond, at(then_kw.0));
                self.v006_expr(then_branch, at(else_kw.0));
                self.v006_expr(else_branch, end);
            }
            E::Fun {
                params,
                arrow,
                body,
                ..
            } => {
                let scope = ByteRange::new(past(arrow.0), end);
                for p in params {
                    self.v006_patbot(p, scope, "parameter");
                }
                self.v006_expr(body, end);
            }
            E::FunRows {
                opts,
                param,
                arrow,
                body,
                ..
            } => {
                let scope = ByteRange::new(past(arrow.0), end);
                self.v006_opt_binders(opts, scope);
                self.v006_patbot(param, scope, "parameter");
                self.v006_expr(body, end);
            }
            E::Match {
                scrutinee,
                with_kw,
                first,
                rest,
                ..
            } => {
                self.v006_expr(scrutinee, at(with_kw.0));
                let arm_end = |i: usize| rest.get(i).map(|b| at(b.bar.0)).unwrap_or(end);
                self.v006_arm(first, arm_end(0));
                for (i, b) in rest.iter().enumerate() {
                    self.v006_arm(&b.arm, arm_end(i + 1));
                }
            }
            E::LetMutableIn {
                name,
                init,
                in_kw,
                body,
                ..
            } => {
                self.bind(
                    Ns::Value,
                    &name.name,
                    name.span,
                    ByteRange::new(past(in_kw.0), end),
                    "let-mutable",
                );
                self.v006_expr(init, at(in_kw.0));
                self.v006_expr(body, end);
            }
            E::LetMathIn {
                cmd,
                params,
                eq,
                value,
                in_kw,
                body,
                ..
            } => {
                self.bind(
                    Ns::MathCmd,
                    &cmd.name,
                    cmd.span,
                    ByteRange::new(past(in_kw.0), end),
                    "let-math",
                );
                let scope = ByteRange::new(past(eq.0), at(in_kw.0));
                for p in params {
                    self.v006_param(p, scope);
                }
                self.v006_expr(value, at(in_kw.0));
                self.v006_expr(body, end);
            }
            E::OpenIn {
                name, in_kw, body, ..
            } => {
                self.reference(Ns::Module, &name.name, name.span);
                self.open_module(&[], &name.name, ByteRange::new(past(in_kw.0), end));
                self.v006_expr(body, end);
            }
            E::WhileDo {
                cond, do_kw, body, ..
            } => {
                self.v006_expr(cond, at(do_kw.0));
                self.v006_expr(body, end);
            }
            E::Overwrite { name, value, .. } => {
                self.reference(Ns::Value, &name.name, name.span);
                self.v006_expr(value, end);
            }
            E::Ops(chain) => self.v006_ops(chain, end),
        }
    }

    fn v006_arm(&mut self, arm: &ast::MatchArm, end: usize) {
        // A guard sees the pattern's bindings, so the scope opens at whichever
        // of `when`/`->` comes first.
        let from = arm
            .guard
            .as_ref()
            .map(|g| at(g.when_kw.0))
            .unwrap_or(at(arm.arrow.0));
        self.v006_pattern(&arm.pat, ByteRange::new(from, end), "match binding");
        if let Some(g) = &arm.guard {
            self.v006_expr(&g.cond, at(arm.arrow.0));
        }
        self.v006_expr(&arm.body, end);
    }

    fn v006_ops(&mut self, chain: &ast::OpChain, end: usize) {
        self.v006_app(&chain.head);
        for r in &chain.tail {
            // A binary operator is an ordinary value name: `let ( +++> ) = …`
            // binds one, so a mention of it must resolve like any other.
            self.reference(Ns::Value, &r.op.op_text(), r.op.span);
            self.v006_app(&r.rhs);
        }
        if let Some(b) = &chain.before {
            self.v006_expr(&b.body, end);
        }
    }

    /// An application chain. Needs no end: every expression reachable from
    /// here is inside a delimiter group that carries its own.
    fn v006_app(&mut self, app: &ast::AppExpr) {
        self.v006_atomic(&app.head);
        for a in &app.head_accesses {
            self.reference(Ns::Field, &a.label.name, a.label.span);
        }
        for arg in &app.args {
            self.v006_apparg(arg);
        }
    }

    fn v006_apparg(&mut self, arg: &ast::AppArg) {
        use ast::AppArg as A;
        match arg {
            A::Optional { value, .. } => self.v006_atomic(value),
            A::Omission(_) => {}
            A::Atom { atom, accesses, .. } => {
                self.v006_atomic(atom);
                for a in accesses {
                    self.reference(Ns::Field, &a.label.name, a.label.span);
                }
            }
            A::Ctor(c) => self.reference(Ns::Ctor, &c.name, c.span),
            A::Bundled {
                opts,
                atom,
                accesses,
                ..
            } => {
                self.v006_opt_args(opts);
                self.v006_atomic(atom);
                for a in accesses {
                    self.reference(Ns::Field, &a.label.name, a.label.span);
                }
            }
            A::BundledCtor { opts, ctor } => {
                self.v006_opt_args(opts);
                self.reference(Ns::Ctor, &ctor.name, ctor.span);
            }
        }
    }

    fn v006_opt_args(&mut self, opts: &ast::CstOptArgs) {
        let close = at(opts.paren.close.0);
        for e in &opts.entries {
            self.reference(Ns::Field, &e.label.name, e.label.span);
            let end = e.comma.as_ref().map(|c| at(c.0)).unwrap_or(close);
            self.v006_expr(&e.value, end);
        }
    }

    fn v006_opt_binders(&mut self, opts: &ast::CstOptBinders, scope: ByteRange) {
        for e in &opts.entries {
            self.reference(Ns::Field, &e.label.name, e.label.span);
            self.bind(
                Ns::Value,
                &e.var.name,
                e.var.span,
                scope,
                "optional parameter",
            );
        }
    }

    fn v006_atomic(&mut self, a: &ast::Atomic) {
        use ast::Atomic as A;
        match a {
            A::Length(_) | A::Float(_) | A::Int(_) | A::Literal(_) | A::True(_) | A::False(_) => {}
            A::Ctor(c) => self.reference(Ns::Ctor, &c.name, c.span),
            A::Var(v) => self.reference(Ns::Value, &v.name, v.span),
            A::VarWithMod(v) => self.qualified(Ns::Value, &v.mods, &v.name, v.span),
            A::OpRef(o) => self.reference(Ns::Value, &o.name, o.span),
            A::Command { name, .. } => self.v006_horz_ref(name),
            A::Unit { .. } => {}
            A::Paren { paren, inner } => self.v006_paren_body(inner, at(paren.close.0)),
            A::OpenModule { grp, body } => {
                let inner = ByteRange::new(past(grp.open.span), at(grp.close.0));
                self.reference(Ns::Module, &grp.open.name, grp.open.span);
                self.open_module(&[], &grp.open.name, inner);
                self.v006_paren_body(body, inner.end);
            }
            A::Record { rec, body } => self.v006_record_body(body, at(rec.close.0)),
            A::List { list, items } => {
                let close = at(list.close.0);
                for it in items {
                    let end = it.semi.as_ref().map(|s| at(s.0)).unwrap_or(close);
                    self.v006_expr(&it.value, end);
                }
            }
            A::InlineText { igrp, elems } => {
                let close = at(igrp.close.0);
                for e in elems {
                    self.v006_inline(e, close);
                }
            }
            A::BlockText { bgrp, elems } => {
                let close = at(bgrp.close.0);
                for e in elems {
                    self.v006_block(e, close);
                }
            }
            A::MathText { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for e in elems {
                    self.v006_math(e, close);
                }
            }
        }
    }

    fn v006_paren_body(&mut self, body: &ast::ParenBody, end: usize) {
        let sep = |i: usize| body.rest.get(i).map(|c| at(c.comma.0)).unwrap_or(end);
        self.v006_expr(&body.first, sep(0));
        for (i, c) in body.rest.iter().enumerate() {
            self.v006_expr(&c.value, sep(i + 1));
        }
    }

    fn v006_record_body(&mut self, body: &ast::RecordBody, end: usize) {
        match body {
            ast::RecordBody::Update {
                base,
                with_kw,
                fields,
            } => {
                self.v006_expr(base, at(with_kw.0));
                self.v006_record_fields(fields, end);
            }
            ast::RecordBody::Fields(fields) => self.v006_record_fields(fields, end),
        }
    }

    fn v006_record_fields(&mut self, fields: &[ast::RecordField], end: usize) {
        for f in fields {
            self.reference(Ns::Field, &f.name.name, f.name.span);
            let e = f.semi.as_ref().map(|s| at(s.0)).unwrap_or(end);
            self.v006_expr(&f.value, e);
        }
    }

    // -- text modes ----------------------------------------------------------

    fn v006_inline(&mut self, e: &ast::InlineElem, end: usize) {
        use ast::InlineElem as I;
        match e {
            I::Char(_)
            | I::CodeText(_)
            | I::Space(_)
            | I::Break(_)
            | I::ItemBullet(_)
            | I::Sep(_) => {}
            I::Embed { var, .. } => self.qualified(Ns::Value, &var.mods, &var.name, var.span),
            I::EmbedMath { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for m in elems {
                    self.v006_math(m, close);
                }
            }
            I::Cmd { name, tail } => {
                self.v006_horz_ref(name);
                self.v006_cmd_tail(tail, end);
            }
        }
    }

    fn v006_block(&mut self, e: &ast::BlockElem, end: usize) {
        match e {
            ast::BlockElem::Embed { var, .. } => {
                self.qualified(Ns::Value, &var.mods, &var.name, var.span)
            }
            ast::BlockElem::Cmd { name, tail } => {
                self.v006_vert_ref(name);
                self.v006_cmd_tail(tail, end);
            }
        }
    }

    fn v006_cmd_tail(&mut self, tail: &ast::CmdTail, _end: usize) {
        match tail {
            ast::CmdTail::Semi(_) => {}
            ast::CmdTail::Args { first, rest, .. } => {
                self.v006_apparg(first);
                for a in rest {
                    self.v006_apparg(a);
                }
            }
        }
    }

    fn v006_math(&mut self, m: &ast::MathElemCst, end: usize) {
        self.v006_mathbot(&m.base, end);
        for s in &m.scripts {
            match s {
                ast::MathScript::Super { group, .. } | ast::MathScript::Sub { group, .. } => {
                    match group {
                        ast::MathGroupArg::Group { mgrp, elems } => {
                            let close = at(mgrp.close.0);
                            for e in elems {
                                self.v006_math(e, close);
                            }
                        }
                        ast::MathGroupArg::Bot(b) => self.v006_mathbot(b, end),
                    }
                }
                ast::MathScript::Primes(_) => {}
            }
        }
    }

    fn v006_mathbot(&mut self, m: &ast::MathBot, end: usize) {
        use ast::MathBot as M;
        match m {
            M::Cmd { name, args } => {
                self.v006_math_ref(name);
                for a in args {
                    match a {
                        ast::MathArg::Optional { body, .. } | ast::MathArg::Plain(body) => {
                            self.v006_math_arg_body(body)
                        }
                        ast::MathArg::Omission(_) => {}
                    }
                }
            }
            M::Chars(_) | M::Sep(_) => {}
            M::Embed(v) => self.qualified(Ns::Value, &v.mods, &v.name, v.span),
            M::Group { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for e in elems {
                    self.v006_math(e, close);
                }
            }
        }
        let _ = end;
    }

    fn v006_math_arg_body(&mut self, b: &ast::MathArgBody) {
        use ast::MathArgBody as B;
        match b {
            B::Math { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for e in elems {
                    self.v006_math(e, close);
                }
            }
            B::Inline { igrp, elems } => {
                let close = at(igrp.close.0);
                for e in elems {
                    self.v006_inline(e, close);
                }
            }
            B::Block { bgrp, elems } => {
                let close = at(bgrp.close.0);
                for e in elems {
                    self.v006_block(e, close);
                }
            }
            B::ParenEscape { paren, inner } => self.v006_paren_body(inner, at(paren.close.0)),
            B::ListEscape { list, items } => {
                let close = at(list.close.0);
                for it in items {
                    let end = it.semi.as_ref().map(|s| at(s.0)).unwrap_or(close);
                    self.v006_expr(&it.value, end);
                }
            }
            B::RecordEscape { rec, body } => self.v006_record_body(body, at(rec.close.0)),
        }
    }

    fn v006_horz_ref(&mut self, c: &AnyHorzCmdTok) {
        match c {
            AnyHorzCmdTok::Plain(t) => self.reference(Ns::InlineCmd, &t.name, t.span),
            AnyHorzCmdTok::Mod(t) => self.qualified(Ns::InlineCmd, &t.mods, &t.name, t.span),
        }
    }

    fn v006_vert_ref(&mut self, c: &AnyVertCmdTok) {
        match c {
            AnyVertCmdTok::Plain(t) => self.reference(Ns::BlockCmd, &t.name, t.span),
            AnyVertCmdTok::Mod(t) => self.qualified(Ns::BlockCmd, &t.mods, &t.name, t.span),
        }
    }

    fn v006_math_ref(&mut self, c: &AnyMathCmdTok) {
        match c {
            AnyMathCmdTok::Plain(t) => self.reference(Ns::MathCmd, &t.name, t.span),
            AnyMathCmdTok::Mod(t) => self.qualified(Ns::MathCmd, &t.mods, &t.name, t.span),
        }
    }

    // -- patterns ------------------------------------------------------------

    fn v006_param(&mut self, p: &ast::Param, scope: ByteRange) {
        match p {
            ast::Param::Optional { name, .. } => {
                self.bind(
                    Ns::Value,
                    &name.name,
                    name.span,
                    scope,
                    "optional parameter",
                );
            }
            ast::Param::Pat(p) => self.v006_patbot(p, scope, "parameter"),
            ast::Param::Bundled { opts, body } => {
                self.v006_opt_binders(opts, scope);
                self.v006_patbot(body, scope, "parameter");
            }
        }
    }

    fn v006_pattern(&mut self, p: &ast::Pattern, scope: ByteRange, form: &'static str) {
        self.v006_patcons(&p.head, scope, form);
        if let Some(a) = &p.as_clause {
            self.bind(Ns::Value, &a.name.name, a.name.span, scope, form);
        }
    }

    fn v006_patcons(&mut self, p: &ast::PatCons, scope: ByteRange, form: &'static str) {
        self.v006_patbot(&p.head, scope, form);
        for c in &p.tail {
            self.v006_patbot(&c.tail, scope, form);
        }
    }

    fn v006_patbot(&mut self, p: &ast::PatBot, scope: ByteRange, form: &'static str) {
        use ast::PatBot as P;
        match p {
            P::CtorApplied { ctor, arg } => {
                self.reference(Ns::Ctor, &ctor.name, ctor.span);
                self.v006_patbot(arg, scope, form);
            }
            P::Ctor(c) => self.reference(Ns::Ctor, &c.name, c.span),
            P::Int(_) | P::True(_) | P::False(_) | P::Str(_) | P::Wild(_) | P::Unit { .. } => {}
            P::Var(v) => {
                self.bind(Ns::Value, &v.name, v.span, scope, form);
            }
            P::Paren { inner, .. } => {
                self.v006_pattern(&inner.first, scope, form);
                for c in &inner.rest {
                    self.v006_pattern(&c.value, scope, form);
                }
            }
            P::List { items, .. } => {
                for it in items {
                    self.v006_pattern(&it.value, scope, form);
                }
            }
        }
    }

    // -- types ---------------------------------------------------------------

    fn v006_type(&mut self, t: &ast::TypeExpr) {
        use ast::TypeExpr as T;
        match t {
            T::Fun { opts, dom, cod, .. } => {
                for o in opts {
                    self.v006_type_prod(&o.ty);
                }
                self.v006_type_prod(dom);
                self.v006_type(cod);
            }
            T::Atom(p) => self.v006_type_prod(p),
            T::OptRowFun {
                opt_dom, dom, cod, ..
            } => {
                for e in &opt_dom.entries {
                    self.reference(Ns::Field, &e.label.name, e.label.span);
                    self.v006_type(&e.ty);
                }
                self.v006_type_prod(dom);
                self.v006_type(cod);
            }
        }
    }

    fn v006_type_prod(&mut self, p: &ast::TypeProd) {
        self.v006_type_app(&p.first);
        for s in &p.rest {
            self.v006_type_app(&s.ty);
        }
    }

    fn v006_type_app(&mut self, a: &ast::TypeApp) {
        self.v006_type_atom(&a.head);
        for r in &a.rest {
            self.v006_type_atom(r);
        }
    }

    fn v006_type_atom(&mut self, a: &ast::TypeAtom) {
        use ast::TypeAtom as A;
        match a {
            A::Cmd { args, .. } => {
                for it in args {
                    for l in &it.opt_labels {
                        self.reference(Ns::Field, &l.label.name, l.label.span);
                        self.v006_type(&l.ty);
                    }
                    self.v006_type(&it.ty);
                }
            }
            A::Paren { inner, .. } => self.v006_type(inner),
            A::Record { fields, .. } => {
                for f in fields {
                    self.reference(Ns::Field, &f.name.name, f.name.span);
                    self.v006_type(&f.ty);
                }
            }
            A::Var(v) => self.reference(Ns::TypeVar, &v.name, v.span),
            A::Name(v) => self.reference(Ns::Type, &v.name, v.span),
            A::NameMod(v) => self.qualified(Ns::Type, &v.mods, &v.name, v.span),
            A::RecordOpen { inner, .. } => {
                for f in &inner.fields {
                    self.reference(Ns::Field, &f.name.name, f.name.span);
                    self.v006_type(&f.ty);
                }
                self.reference(Ns::TypeVar, &inner.var.name, inner.var.span);
            }
        }
    }
}
