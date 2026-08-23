//! Filling a [`Builder`] from the SATySFi **0.1** concrete syntax tree.
//!
//! Same contract as [`crate::walk006`] — boundaries come from keyword spans,
//! never from a node span that does not exist — over a grammar that differs in
//! the places that matter most to a language server:
//!
//! - a library file *is* one `module M :> S = struct … end`, so every name in
//!   it is a member of that module rather than a file-level binding;
//! - `val` replaces the five `let-*` forms, with the command-ness moved into a
//!   `val inline`/`val block`/`val math` keyword;
//! - signatures are first-class (`signature S = sig … end`, `module M : S`,
//!   `include S`, `S with type t = …`), so a module's exports may be described
//!   somewhere other than at the module itself;
//! - `let open M in`, `use … open` and `include` all splice names in from
//!   elsewhere, which is what [`Builder::open_module`] is for.
//!
//! A signature reached by *name* (`module M : S`) is deliberately **not**
//! followed to its declaration. Following it would mean re-deriving
//! `module_check::resolve_sig`'s lookup here, and a signature this walk
//! resolves wrongly would put names in scope that are not — so an unresolved
//! signature simply leaves the module with no declared exports, and
//! [`Builder::open_module`] then exports everything the `struct` binds, which
//! is a superset and never a fabrication.

use rustyfi_syntax::cst_v1::{self as v1, ast};
use rustyfi_syntax::leaf::*;
use rustyfi_syntax::span::Span;

use crate::model::{node_span, Builder, ByteRange, Def, HeaderKind, Ns};

fn at(span: Span) -> usize {
    span.start.byte
}

fn past(span: Span) -> usize {
    span.end.byte
}

/// Where a `struct`-level binding's text begins.
fn bind_start(b: &v1::Bind) -> usize {
    use v1::Bind as B;
    match b {
        B::Value { kw, .. }
        | B::ValueInline { kw, .. }
        | B::ValueBlock { kw, .. }
        | B::ValueMath { kw, .. }
        | B::ValueRec { kw, .. }
        | B::ValueMutable { kw, .. } => at(kw.0),
        B::Type { kw, .. } => at(kw.0),
        B::Module { module_kw, .. } => at(module_kw.0),
        B::Signature { kw, .. } => at(kw.0),
        B::Include { kw, .. } => at(kw.0),
    }
}

/// The module path a `ModChainV1` names, split into qualifiers and last
/// segment — the same shape [`Builder::qualified`] takes.
fn chain(c: &ast::ModChainV1) -> (Vec<String>, String, Span) {
    match c {
        ast::ModChainV1::Long(t) => (t.mods.clone(), t.name.clone(), t.span),
        ast::ModChainV1::Single(t) => (Vec::new(), t.name.clone(), t.span),
    }
}

impl Builder<'_> {
    // -- files ---------------------------------------------------------------

    pub(crate) fn v01_file(&mut self, f: &v1::FileV1) {
        match f {
            v1::FileV1::Document { headers, body, .. } => {
                self.v01_document_parts(headers, Some(body))
            }
            v1::FileV1::Library {
                headers,
                module_kw,
                name,
                sig_annot,
                eq,
                struct_kw,
                binds,
                end_kw,
                ..
            } => {
                let head = crate::model::V01LibraryHead {
                    module_kw: module_kw.clone(),
                    name: name.clone(),
                    sig_annot: sig_annot.clone(),
                    eq: eq.clone(),
                    struct_kw: struct_kw.clone(),
                };
                self.v01_library_parts(headers, &head, binds, at(end_kw.0), at(end_kw.0));
            }
        }
    }

    pub(crate) fn v01_document_parts(
        &mut self,
        headers: &[v1::HeaderV1],
        body: Option<&ast::Expr>,
    ) {
        let end = self.source.len();
        self.v01_headers(headers, ByteRange::new(0, end));
        if let Some(b) = body {
            self.v01_expr(b, end);
        }
    }

    /// `scope_end` is how far a member of this module stays visible (the
    /// `end` keyword, or the end of the file when it has not been typed yet);
    /// `text_end` is where the last recovered binding's own text stops. They
    /// are the same for a file that parsed and differ for one that did not —
    /// see [`crate::walk006`]'s "Two ends, not one".
    pub(crate) fn v01_library_parts(
        &mut self,
        headers: &[v1::HeaderV1],
        head: &crate::model::V01LibraryHead,
        binds: &[v1::Bind],
        scope_end: usize,
        text_end: usize,
    ) {
        let file_end = self.source.len();
        self.v01_headers(headers, ByteRange::new(0, file_end));
        let inner = ByteRange::new(past(head.struct_kw.0), scope_end);
        let module = self.def(Def {
            ns: Ns::Module,
            name: head.name.name.clone(),
            name_span: ByteRange::of(head.name.span),
            // The module a library file declares is visible over the whole
            // file, which is what makes hovering its name at the `module`
            // keyword and at a recursive mention behave the same.
            scope: ByteRange::new(0, file_end),
            form: "module",
            ty: None,
            container: None,
            declaration: false,
        });
        if let Some(sig) = &head.sig_annot {
            self.v01_sig(&sig.sig_, Some(module));
        }
        self.v01_binds(binds, module, inner.end, text_end);
    }

    fn v01_headers(&mut self, headers: &[v1::HeaderV1], file: ByteRange) {
        for h in headers {
            match h {
                v1::HeaderV1::UsePackage { open_kw, path, .. }
                | v1::HeaderV1::UseOf { open_kw, path, .. }
                | v1::HeaderV1::Use { open_kw, path, .. } => {
                    let (quals, name, span) = chain(path);
                    self.header(HeaderKind::Use, &render(&quals, &name), span);
                    // The header binds the module name for the rest of the
                    // file. Its *contents* live in another file, so a member
                    // lookup through it finds nothing and says so.
                    self.def(Def {
                        ns: Ns::Module,
                        name: name.clone(),
                        name_span: ByteRange::of(span),
                        scope: file,
                        form: match h {
                            v1::HeaderV1::UsePackage { .. } => "use package",
                            v1::HeaderV1::UseOf { .. } => "use … of",
                            _ => "use",
                        },
                        ty: None,
                        container: None,
                        declaration: false,
                    });
                    if open_kw.is_some() {
                        // `use … open` splices the package's names in at file
                        // scope. They are not in this buffer, so nothing below
                        // may be resolved against a name they could shadow.
                        self.opaque(file);
                    }
                }
                v1::HeaderV1::Legacy(l) => match l {
                    rustyfi_syntax::cst::Header::Require(t) => {
                        self.header(HeaderKind::Require, &t.content, t.span)
                    }
                    rustyfi_syntax::cst::Header::Import(t) => {
                        self.header(HeaderKind::Import, &t.content, t.span)
                    }
                    rustyfi_syntax::cst::Header::Stage(_) => {}
                },
            }
        }
    }

    // -- binding lists -------------------------------------------------------

    fn v01_binds(
        &mut self,
        binds: &[v1::Bind],
        container: usize,
        scope_end: usize,
        text_end: usize,
    ) {
        for (i, b) in binds.iter().enumerate() {
            let stop = binds.get(i + 1).map(bind_start).unwrap_or(text_end);
            self.v01_bind(b, container, scope_end, stop);
        }
    }

    /// The `struct … end` body of a nested module. Its element type is the
    /// recursion-breaking wrapper rather than `Bind` itself (`cst_v1.rs`
    /// routes the edge through `StructBindV1` for the same compile-time reason
    /// `cst.rs` routes `StructDecl`), so this is the same loop over a
    /// different container.
    fn v01_struct_binds(&mut self, binds: &[v1::StructBindV1], container: usize, end: usize) {
        for (i, b) in binds.iter().enumerate() {
            let stop = binds.get(i + 1).map(|n| bind_start(&n.0)).unwrap_or(end);
            self.v01_bind(&b.0, container, end, stop);
        }
    }

    fn v01_bind(&mut self, b: &v1::Bind, container: usize, scope_end: usize, stop: usize) {
        use v1::Bind as B;
        let after = ByteRange::new(stop, scope_end);
        let value = |eq: &DefEqTok| ByteRange::new(past(eq.0), stop);
        match b {
            B::Value {
                name,
                params,
                eq,
                body,
                ..
            } => {
                self.member_def(Ns::Value, &name.name, name.span, after, "val", container);
                for p in params {
                    self.v01_param(p, value(eq));
                }
                self.v01_expr(body, stop);
            }
            B::ValueInline {
                ctx,
                cmd,
                params,
                eq,
                body,
                ..
            } => {
                let (name, span) = horz_name(cmd);
                self.member_def(Ns::InlineCmd, &name, span, after, "val inline", container);
                self.v01_cmd_body(ctx.as_ref(), None, params, value(eq), body, stop);
            }
            B::ValueBlock {
                ctx,
                cmd,
                params,
                eq,
                body,
                ..
            } => {
                let (name, span) = vert_name(cmd);
                self.member_def(Ns::BlockCmd, &name, span, after, "val block", container);
                self.v01_cmd_body(ctx.as_ref(), None, params, value(eq), body, stop);
            }
            B::ValueMath {
                ctx,
                cmd,
                params,
                scripts,
                eq,
                body,
                ..
            } => {
                let (name, span) = horz_name(cmd);
                self.member_def(Ns::MathCmd, &name, span, after, "val math", container);
                self.v01_cmd_body(Some(ctx), scripts.as_ref(), params, value(eq), body, stop);
            }
            B::ValueRec {
                rec_kw,
                first,
                ands,
                ..
            } => {
                let scope = ByteRange::new(at(rec_kw.0), scope_end);
                self.v01_rec_name(first, scope, Some(container));
                for a in ands {
                    self.v01_rec_name(&a.clause, scope, Some(container));
                }
                let group_end = |i: usize| ands.get(i).map(|a| at(a.and_kw.0)).unwrap_or(stop);
                self.v01_rec_body(first, group_end(0));
                for (i, a) in ands.iter().enumerate() {
                    self.v01_rec_body(&a.clause, group_end(i + 1));
                }
            }
            B::ValueMutable { name, value, .. } => {
                self.member_def(
                    Ns::Value,
                    &name.name,
                    name.span,
                    after,
                    "val mutable",
                    container,
                );
                self.v01_expr(value, stop);
            }
            B::Type {
                kw, first, ands, ..
            } => {
                let scope = ByteRange::new(at(kw.0), scope_end);
                let clause_end = |i: usize| ands.get(i).map(|a| at(a.and_kw.0)).unwrap_or(stop);
                self.v01_type_bind(
                    first,
                    scope,
                    ByteRange::new(at(kw.0), clause_end(0)),
                    container,
                );
                for (i, a) in ands.iter().enumerate() {
                    self.v01_type_bind(
                        &a.bind,
                        scope,
                        ByteRange::new(at(a.and_kw.0), clause_end(i + 1)),
                        container,
                    );
                }
            }
            B::Module {
                name,
                sig_annot,
                body,
                ..
            } => {
                let idx = self.def(Def {
                    ns: Ns::Module,
                    name: name.name.clone(),
                    name_span: ByteRange::of(name.span),
                    scope: after,
                    form: "module",
                    ty: None,
                    container: Some(container),
                    declaration: false,
                });
                if let Some(sig) = sig_annot {
                    self.v01_sig(&sig.sig_, Some(idx));
                }
                self.v01_mod_expr(body, idx, stop);
            }
            B::Signature { name, sig_, .. } => {
                self.member_def(
                    Ns::Signature,
                    &name.name,
                    name.span,
                    after,
                    "signature",
                    container,
                );
                self.v01_sig(sig_, None);
            }
            B::Include { body, .. } => {
                // `include M` splices another module's bindings in here.
                // Which ones depends on `M`'s signature, so nothing bound
                // before this point can be resolved confidently after it.
                self.opaque(after);
                self.v01_mod_expr(body, container, stop);
            }
        }
    }

    fn v01_cmd_body(
        &mut self,
        ctx: Option<&VarTok>,
        scripts: Option<&v1::ScriptsParamV1>,
        params: &[ast::Param],
        value: ByteRange,
        body: &ast::Expr,
        stop: usize,
    ) {
        if let Some(c) = ctx {
            self.bind(Ns::Value, &c.name, c.span, value, "context parameter");
        }
        if let Some(s) = scripts {
            self.bind(
                Ns::Value,
                &s.sub.name,
                s.sub.span,
                value,
                "subscript parameter",
            );
            self.bind(
                Ns::Value,
                &s.sup.name,
                s.sup.span,
                value,
                "superscript parameter",
            );
        }
        for p in params {
            self.v01_param(p, value);
        }
        self.v01_expr(body, stop);
    }

    fn member_def(
        &mut self,
        ns: Ns,
        name: &str,
        span: Span,
        scope: ByteRange,
        form: &'static str,
        container: usize,
    ) {
        self.def(Def {
            ns,
            name: name.to_string(),
            name_span: ByteRange::of(span),
            scope,
            form,
            ty: None,
            container: Some(container),
            declaration: false,
        });
    }

    fn v01_rec_name(&mut self, c: &ast::RecClauseV1, scope: ByteRange, container: Option<usize>) {
        self.def(Def {
            ns: Ns::Value,
            name: c.name.name.clone(),
            name_span: ByteRange::of(c.name.span),
            scope,
            form: "val rec",
            ty: None,
            container,
            declaration: false,
        });
    }

    fn v01_rec_body(&mut self, c: &ast::RecClauseV1, end: usize) {
        let scope = ByteRange::new(past(c.eq.0), end);
        for p in &c.params {
            self.v01_param(p, scope);
        }
        self.v01_expr(&c.value, end);
    }

    fn v01_type_bind(
        &mut self,
        t: &v1::TypeBindSingleV1,
        scope: ByteRange,
        clause: ByteRange,
        container: usize,
    ) {
        let form = match &t.body {
            v1::TypeBodyV1::Variant { .. } => "type",
            v1::TypeBodyV1::Synonym(_) => "type synonym",
        };
        self.def(Def {
            ns: Ns::Type,
            name: t.name.name.clone(),
            name_span: ByteRange::of(t.name.span),
            scope,
            form,
            ty: match &t.body {
                v1::TypeBodyV1::Synonym(ty) => Some(node_span(ty)),
                _ => None,
            },
            container: Some(container),
            declaration: false,
        });
        for tv in &t.tyvars {
            self.bind(Ns::TypeVar, &tv.name, tv.span, clause, "type parameter");
        }
        match &t.body {
            v1::TypeBodyV1::Variant { first, rest, .. } => {
                self.v01_variant(first, scope, container);
                for r in rest {
                    self.v01_variant(&r.def, scope, container);
                }
            }
            v1::TypeBodyV1::Synonym(ty) => self.v01_type(ty),
        }
    }

    fn v01_variant(&mut self, v: &v1::VariantDefV1, scope: ByteRange, container: usize) {
        self.def(Def {
            ns: Ns::Ctor,
            name: v.ctor.name.clone(),
            name_span: ByteRange::of(v.ctor.span),
            scope,
            form: "variant constructor",
            ty: v.of_ty.as_ref().map(|o| node_span(&o.ty)),
            container: Some(container),
            declaration: false,
        });
        if let Some(o) = &v.of_ty {
            self.v01_type(&o.ty);
        }
    }

    // -- module and signature expressions ------------------------------------

    fn v01_mod_expr(&mut self, m: &ast::ModExpr, module: usize, stop: usize) {
        match m {
            ast::ModExpr::Functor {
                param, dom, body, ..
            } => {
                // A functor's parameter is a module name visible in the body,
                // but the body's members exist only at an application's path —
                // which is not in this file — so the parameter is bound and
                // the body walked for its mentions, and nothing is exported.
                self.bind(
                    Ns::Module,
                    &param.name,
                    param.span,
                    ByteRange::new(past(param.span), stop),
                    "functor parameter",
                );
                self.v01_sig(dom, None);
                self.v01_mod_expr(body, module, stop);
            }
            ast::ModExpr::Coerce { name, sig_, .. } => {
                self.reference(Ns::Module, &name.name, name.span);
                self.v01_sig(sig_, None);
            }
            ast::ModExpr::App { func, arg } => {
                for c in [func, arg] {
                    let (quals, name, span) = chain(c);
                    self.qualified(Ns::Module, &quals, &name, span);
                }
                // The result's members come from a functor body this file
                // cannot evaluate.
                self.opaque(ByteRange::new(stop, stop));
            }
            ast::ModExpr::Var(c) => {
                let (quals, name, span) = chain(c);
                self.qualified(Ns::Module, &quals, &name, span);
            }
            ast::ModExpr::Struct {
                struct_kw,
                binds,
                end_kw,
            } => {
                let inner = ByteRange::new(past(struct_kw.0), at(end_kw.0));
                self.v01_struct_binds(binds, module, inner.end);
            }
        }
    }

    /// A signature, whose `val`/`type` items *declare* names rather than bind
    /// them. `owner` is the module those declarations describe, when the
    /// signature is written directly on one.
    fn v01_sig(&mut self, s: &ast::SigExpr, owner: Option<usize>) {
        match s {
            ast::SigExpr::Functor {
                param, dom, cod, ..
            } => {
                self.bind(
                    Ns::Module,
                    &param.name,
                    param.span,
                    ByteRange::new(past(param.span), past(param.span)),
                    "functor parameter",
                );
                self.v01_sig(dom, None);
                self.v01_sig(cod, None);
            }
            ast::SigExpr::WithType {
                base, path, binds, ..
            } => {
                self.v01_sig_bot(base, owner);
                if let Some(p) = path {
                    let (quals, name, span) = chain(p);
                    self.qualified(Ns::Module, &quals, &name, span);
                }
                // `with type t = …` refines a name the signature already
                // declares; the refinement's own right-hand side is ordinary
                // type text.
                self.v01_type_binds(binds, owner);
            }
            ast::SigExpr::Bot(b) => self.v01_sig_bot(b, owner),
        }
    }

    fn v01_type_binds(&mut self, binds: &v1::TypeBindsV1, owner: Option<usize>) {
        let mut all = vec![&binds.first];
        all.extend(binds.ands.iter().map(|a| &a.bind));
        for t in all {
            self.declare(
                Ns::Type,
                &t.name.name,
                t.name.span,
                owner,
                "type",
                match &t.body {
                    v1::TypeBodyV1::Synonym(ty) => Some(node_span(ty)),
                    _ => None,
                },
            );
            for tv in &t.tyvars {
                self.reference(Ns::TypeVar, &tv.name, tv.span);
            }
            match &t.body {
                v1::TypeBodyV1::Variant { first, rest, .. } => {
                    self.declare(
                        Ns::Ctor,
                        &first.ctor.name,
                        first.ctor.span,
                        owner,
                        "variant constructor",
                        None,
                    );
                    if let Some(o) = &first.of_ty {
                        self.v01_type(&o.ty);
                    }
                    for r in rest {
                        self.declare(
                            Ns::Ctor,
                            &r.def.ctor.name,
                            r.def.ctor.span,
                            owner,
                            "variant constructor",
                            None,
                        );
                        if let Some(o) = &r.def.of_ty {
                            self.v01_type(&o.ty);
                        }
                    }
                }
                v1::TypeBodyV1::Synonym(ty) => self.v01_type(ty),
            }
        }
    }

    fn v01_sig_bot(&mut self, b: &ast::SigBotV1, owner: Option<usize>) {
        match b {
            ast::SigBotV1::Path(t) => self.qualified(Ns::Signature, &t.mods, &t.name, t.span),
            ast::SigBotV1::Var(t) => self.reference(Ns::Signature, &t.name, t.span),
            ast::SigBotV1::Sig { decls, .. } => {
                for d in decls {
                    self.v01_decl(&d.0, owner);
                }
            }
        }
    }

    fn v01_decl(&mut self, d: &ast::Decl, owner: Option<usize>) {
        use ast::Decl as D;
        match d {
            D::Val {
                name, quant, ty, ..
            } => {
                self.declare(
                    Ns::Value,
                    &name.name,
                    name.span,
                    owner,
                    "val",
                    Some(node_span(ty)),
                );
                self.v01_quant(quant);
                self.v01_type(ty);
            }
            D::ValHorzCmd { cmd, quant, ty, .. } => {
                self.declare(
                    Ns::InlineCmd,
                    &cmd.name,
                    cmd.span,
                    owner,
                    "val",
                    Some(node_span(ty)),
                );
                self.v01_quant(quant);
                self.v01_type(ty);
            }
            D::ValVertCmd { cmd, quant, ty, .. } => {
                self.declare(
                    Ns::BlockCmd,
                    &cmd.name,
                    cmd.span,
                    owner,
                    "val",
                    Some(node_span(ty)),
                );
                self.v01_quant(quant);
                self.v01_type(ty);
            }
            D::TypeOpaque { name, kind, .. } => {
                self.declare(Ns::Type, &name.name, name.span, owner, "type", None);
                self.reference(Ns::Type, &kind.first.name, kind.first.span);
                for k in &kind.rest {
                    self.reference(Ns::Type, &k.base.name, k.base.span);
                }
            }
            D::Type { binds, .. } => self.v01_type_binds(binds, owner),
            D::Module { name, sig_, .. } => {
                self.declare(Ns::Module, &name.name, name.span, owner, "module", None);
                self.v01_sig(sig_, None);
            }
            D::Signature { name, sig_, .. } => {
                self.declare(
                    Ns::Signature,
                    &name.name,
                    name.span,
                    owner,
                    "signature",
                    None,
                );
                self.v01_sig(sig_, None);
            }
            D::Include { sig_, .. } => self.v01_sig(sig_, owner),
        }
    }

    fn v01_quant(&mut self, quant: &[TypeVarTok]) {
        for tv in quant {
            self.reference(Ns::TypeVar, &tv.name, tv.span);
        }
    }

    fn declare(
        &mut self,
        ns: Ns,
        name: &str,
        span: Span,
        owner: Option<usize>,
        form: &'static str,
        ty: Option<ByteRange>,
    ) {
        self.def(Def {
            ns,
            name: name.to_string(),
            name_span: ByteRange::of(span),
            // A declaration is visible nowhere as a binding; it is reached
            // through its container.
            scope: ByteRange::new(0, 0),
            form,
            ty,
            container: owner,
            declaration: true,
        });
    }

    // -- expressions ---------------------------------------------------------

    fn v01_expr(&mut self, e: &ast::Expr, end: usize) {
        use ast::Expr as E;
        match e {
            E::LetRecIn {
                rec_kw,
                first,
                ands,
                in_kw,
                body,
                ..
            } => {
                let scope = ByteRange::new(at(rec_kw.0), end);
                self.v01_rec_name(first, scope, None);
                for a in ands {
                    self.v01_rec_name(&a.clause, scope, None);
                }
                let group_end =
                    |i: usize| ands.get(i).map(|a| at(a.and_kw.0)).unwrap_or(at(in_kw.0));
                self.v01_rec_body(first, group_end(0));
                for (i, a) in ands.iter().enumerate() {
                    self.v01_rec_body(&a.clause, group_end(i + 1));
                }
                self.v01_expr(body, end);
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
                    "let mutable",
                );
                self.v01_expr(init, at(in_kw.0));
                self.v01_expr(body, end);
            }
            E::LetIn {
                name,
                params,
                eq,
                value,
                in_kw,
                body,
                ..
            } => {
                self.bind(
                    Ns::Value,
                    &name.name,
                    name.span,
                    ByteRange::new(past(in_kw.0), end),
                    "let",
                );
                let scope = ByteRange::new(past(eq.0), at(in_kw.0));
                for p in params {
                    self.v01_param(p, scope);
                }
                self.v01_expr(value, at(in_kw.0));
                self.v01_expr(body, end);
            }
            E::LetPatternIn {
                pat,
                value,
                in_kw,
                body,
                ..
            } => {
                self.v01_pattern(pat, ByteRange::new(past(in_kw.0), end), "let");
                self.v01_expr(value, at(in_kw.0));
                self.v01_expr(body, end);
            }
            E::OpenIn {
                name, in_kw, body, ..
            } => {
                self.reference(Ns::Module, &name.name, name.span);
                self.open_module(&[], &name.name, ByteRange::new(past(in_kw.0), end));
                self.v01_expr(body, end);
            }
            E::If {
                cond,
                then_kw,
                then_branch,
                else_kw,
                else_branch,
                ..
            } => {
                self.v01_expr(cond, at(then_kw.0));
                self.v01_expr(then_branch, at(else_kw.0));
                self.v01_expr(else_branch, end);
            }
            E::Fun {
                params,
                arrow,
                body,
                ..
            } => {
                let scope = ByteRange::new(past(arrow.0), end);
                for p in params {
                    self.v01_param(p, scope);
                }
                self.v01_expr(body, end);
            }
            E::Match {
                scrutinee,
                with_kw,
                first,
                rest,
                end_kw,
                ..
            } => {
                // 0.1's `match` is closed by `end`, so the last arm's extent
                // is known exactly rather than running to the enclosing end.
                let last = at(end_kw.0);
                self.v01_expr(scrutinee, at(with_kw.0));
                let arm_end = |i: usize| rest.get(i).map(|b| at(b.bar.0)).unwrap_or(last);
                self.v01_arm(first, arm_end(0));
                for (i, b) in rest.iter().enumerate() {
                    self.v01_arm(&b.arm, arm_end(i + 1));
                }
            }
            E::Overwrite { name, value, .. } => {
                self.reference(Ns::Value, &name.name, name.span);
                self.v01_expr(value, end);
            }
            E::Ops(chain) => {
                self.v01_app(&chain.head);
                for r in &chain.tail {
                    self.reference(Ns::Value, &r.op.op_text(), r.op.span);
                    self.v01_app(&r.rhs);
                }
            }
        }
    }

    fn v01_arm(&mut self, arm: &ast::MatchArm, end: usize) {
        self.v01_pattern(
            &arm.pat,
            ByteRange::new(at(arm.arrow.0), end),
            "match binding",
        );
        self.v01_expr(&arm.body, end);
    }

    fn v01_app(&mut self, app: &ast::AppExpr) {
        self.v01_atomic(&app.head);
        for a in &app.head_accesses {
            self.reference(Ns::Field, &a.label.name, a.label.span);
        }
        for arg in &app.args {
            self.v01_apparg(arg);
        }
    }

    fn v01_apparg(&mut self, arg: &ast::AppArg) {
        use ast::AppArg as A;
        match arg {
            A::Bundled {
                opts,
                atom,
                accesses,
                ..
            } => {
                self.v01_opt_args(opts);
                self.v01_atomic(atom);
                for a in accesses {
                    self.reference(Ns::Field, &a.label.name, a.label.span);
                }
            }
            A::BundledCtor { opts, ctor } => {
                self.v01_opt_args(opts);
                self.reference(Ns::Ctor, &ctor.name, ctor.span);
            }
            A::Atom { atom, accesses, .. } => {
                self.v01_atomic(atom);
                for a in accesses {
                    self.reference(Ns::Field, &a.label.name, a.label.span);
                }
            }
            A::Ctor(c) => self.reference(Ns::Ctor, &c.name, c.span),
        }
    }

    fn v01_opt_args(&mut self, opts: &ast::OptArgsV1) {
        let close = at(opts.paren.close.0);
        for e in &opts.entries {
            self.reference(Ns::Field, &e.label.name, e.label.span);
            let end = e.comma.as_ref().map(|c| at(c.0)).unwrap_or(close);
            self.v01_expr(&e.value, end);
        }
    }

    fn v01_atomic(&mut self, a: &ast::Atomic) {
        use ast::Atomic as A;
        match a {
            A::Length(_) | A::Float(_) | A::Int(_) | A::Literal(_) | A::True(_) | A::False(_) => {}
            A::Ctor(c) => self.reference(Ns::Ctor, &c.name, c.span),
            A::Var(v) => self.reference(Ns::Value, &v.name, v.span),
            A::VarWithMod(v) => self.qualified(Ns::Value, &v.mods, &v.name, v.span),
            A::Command { name, .. } => self.v01_horz_ref(name),
            A::Unit { .. } => {}
            A::Paren { paren, inner } => self.v01_paren_body(inner, at(paren.close.0)),
            A::Record { rec, body } => self.v01_record_body(body, at(rec.close.0)),
            A::List { list, items } => {
                let close = at(list.close.0);
                for it in items {
                    let end = it.comma.as_ref().map(|c| at(c.0)).unwrap_or(close);
                    self.v01_expr(&it.value, end);
                }
            }
            A::InlineText { igrp, elems } => {
                let close = at(igrp.close.0);
                for e in elems {
                    self.v01_inline(e, close);
                }
            }
            A::BlockText { bgrp, elems } => {
                let close = at(bgrp.close.0);
                for e in elems {
                    self.v01_block(e, close);
                }
            }
            A::MathText { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for e in elems {
                    self.v01_math(e, close);
                }
            }
        }
    }

    fn v01_paren_body(&mut self, body: &ast::ParenBody, end: usize) {
        let sep = |i: usize| body.rest.get(i).map(|c| at(c.comma.0)).unwrap_or(end);
        self.v01_expr(&body.first, sep(0));
        for (i, c) in body.rest.iter().enumerate() {
            self.v01_expr(&c.value, sep(i + 1));
        }
    }

    fn v01_record_body(&mut self, body: &ast::RecordBody, end: usize) {
        match body {
            ast::RecordBody::Update {
                base,
                with_kw,
                fields,
            } => {
                self.v01_expr(base, at(with_kw.0));
                self.v01_record_fields(fields, end);
            }
            ast::RecordBody::Fields(fields) => self.v01_record_fields(fields, end),
        }
    }

    fn v01_record_fields(&mut self, fields: &[ast::RecordField], end: usize) {
        for f in fields {
            self.reference(Ns::Field, &f.name.name, f.name.span);
            let e = f.comma.as_ref().map(|c| at(c.0)).unwrap_or(end);
            self.v01_expr(&f.value, e);
        }
    }

    // -- text modes ----------------------------------------------------------

    fn v01_inline(&mut self, e: &ast::InlineElem, end: usize) {
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
                    self.v01_math(m, close);
                }
            }
            I::Cmd { name, tail } => {
                self.v01_horz_ref(name);
                self.v01_cmd_tail(tail, end);
            }
        }
    }

    fn v01_block(&mut self, e: &ast::BlockElem, end: usize) {
        match e {
            ast::BlockElem::Embed { var, .. } => {
                self.qualified(Ns::Value, &var.mods, &var.name, var.span)
            }
            ast::BlockElem::Cmd { name, tail } => {
                self.v01_vert_ref(name);
                self.v01_cmd_tail(tail, end);
            }
        }
    }

    fn v01_cmd_tail(&mut self, tail: &ast::CmdTail, end: usize) {
        match tail {
            ast::CmdTail::Semi(_) => {}
            ast::CmdTail::Args {
                lead_opts,
                args,
                semi,
            } => {
                if let Some(o) = lead_opts {
                    self.v01_opt_args(o);
                }
                // The argument chain is one expression, closed by the `;` when
                // the command has one and by the enclosing text otherwise.
                let stop = semi.as_ref().map(|s| at(s.0)).unwrap_or(end);
                self.v01_expr(args, stop);
            }
        }
    }

    fn v01_math(&mut self, m: &ast::MathElemCst, end: usize) {
        self.v01_mathbot(&m.base, end);
        for s in &m.scripts {
            match s {
                ast::MathScript::Super { group, .. } | ast::MathScript::Sub { group, .. } => {
                    match group {
                        ast::MathGroupArg::Group { mgrp, elems } => {
                            let close = at(mgrp.close.0);
                            for e in elems {
                                self.v01_math(e, close);
                            }
                        }
                        ast::MathGroupArg::Bot(b) => self.v01_mathbot(b, end),
                    }
                }
                ast::MathScript::Primes(_) => {}
            }
        }
    }

    fn v01_mathbot(&mut self, m: &ast::MathBot, end: usize) {
        use ast::MathBot as M;
        match m {
            M::Cmd { name, args } => {
                self.v01_math_ref(name);
                for a in args {
                    self.v01_math_arg(a);
                }
            }
            M::Chars(_) | M::Sep(_) => {}
            M::Embed(v) => self.qualified(Ns::Value, &v.mods, &v.name, v.span),
            M::Group { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for e in elems {
                    self.v01_math(e, close);
                }
            }
        }
        let _ = end;
    }

    fn v01_math_arg(&mut self, a: &ast::MathArg) {
        use ast::MathArg as A;
        match a {
            A::Math { mgrp, elems } => {
                let close = at(mgrp.close.0);
                for e in elems {
                    self.v01_math(e, close);
                }
            }
            A::Inline { igrp, elems } => {
                let close = at(igrp.close.0);
                for e in elems {
                    self.v01_inline(e, close);
                }
            }
            A::Block { bgrp, elems } => {
                let close = at(bgrp.close.0);
                for e in elems {
                    self.v01_block(e, close);
                }
            }
            A::ParenEscape { paren, inner } => self.v01_paren_body(inner, at(paren.close.0)),
            A::ListEscape { list, items } => {
                let close = at(list.close.0);
                for it in items {
                    let end = it.comma.as_ref().map(|c| at(c.0)).unwrap_or(close);
                    self.v01_expr(&it.value, end);
                }
            }
            A::RecordEscape { rec, body } => self.v01_record_body(body, at(rec.close.0)),
        }
    }

    fn v01_horz_ref(&mut self, c: &AnyHorzCmdTok) {
        match c {
            AnyHorzCmdTok::Plain(t) => self.reference(Ns::InlineCmd, &t.name, t.span),
            AnyHorzCmdTok::Mod(t) => self.qualified(Ns::InlineCmd, &t.mods, &t.name, t.span),
        }
    }

    fn v01_vert_ref(&mut self, c: &AnyVertCmdTok) {
        match c {
            AnyVertCmdTok::Plain(t) => self.reference(Ns::BlockCmd, &t.name, t.span),
            AnyVertCmdTok::Mod(t) => self.qualified(Ns::BlockCmd, &t.mods, &t.name, t.span),
        }
    }

    fn v01_math_ref(&mut self, c: &AnyMathCmdTok) {
        match c {
            AnyMathCmdTok::Plain(t) => self.reference(Ns::MathCmd, &t.name, t.span),
            AnyMathCmdTok::Mod(t) => self.qualified(Ns::MathCmd, &t.mods, &t.name, t.span),
        }
    }

    // -- patterns and parameters ---------------------------------------------

    fn v01_param(&mut self, p: &ast::Param, scope: ByteRange) {
        if let Some(opts) = &p.opts {
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
        match &p.body {
            ast::ParamBody::Pat(b) => self.v01_patbot(b, scope, "parameter"),
            ast::ParamBody::Ascribed { inner, .. } => {
                self.v01_pattern(&inner.pat, scope, "parameter");
                self.v01_type(&inner.ty);
            }
        }
    }

    fn v01_pattern(&mut self, p: &ast::Pattern, scope: ByteRange, form: &'static str) {
        self.v01_patbot(&p.head.head, scope, form);
        for c in &p.head.tail {
            self.v01_patbot(&c.tail, scope, form);
        }
        if let Some(a) = &p.as_clause {
            self.bind(Ns::Value, &a.name.name, a.name.span, scope, form);
        }
    }

    fn v01_patbot(&mut self, p: &ast::PatBot, scope: ByteRange, form: &'static str) {
        use ast::PatBot as P;
        match p {
            P::CtorApplied { ctor, arg } => {
                self.reference(Ns::Ctor, &ctor.name, ctor.span);
                self.v01_patbot(arg, scope, form);
            }
            P::Ctor(c) => self.reference(Ns::Ctor, &c.name, c.span),
            P::Int(_) | P::True(_) | P::False(_) | P::Str(_) | P::Wild(_) | P::Unit { .. } => {}
            P::Var(v) => {
                self.bind(Ns::Value, &v.name, v.span, scope, form);
            }
            P::Paren { inner, .. } => {
                self.v01_pattern(&inner.first, scope, form);
                for c in &inner.rest {
                    self.v01_pattern(&c.value, scope, form);
                }
            }
            P::List { items, .. } => {
                for it in items {
                    self.v01_pattern(&it.value, scope, form);
                }
            }
        }
    }

    // -- types ---------------------------------------------------------------

    fn v01_type(&mut self, t: &ast::TypeExpr) {
        use ast::TypeExpr as T;
        match t {
            T::OptRowFun {
                opt_dom, dom, cod, ..
            } => {
                for e in &opt_dom.inner.entries {
                    self.reference(Ns::Field, &e.label.name, e.label.span);
                    self.v01_type(&e.ty);
                }
                if let Some(r) = &opt_dom.inner.row_tail {
                    self.reference(Ns::TypeVar, &r.var.name, r.var.span);
                }
                self.v01_type_prod(dom);
                self.v01_type(cod);
            }
            T::Fun { dom, cod, .. } => {
                self.v01_type_prod(dom);
                self.v01_type(cod);
            }
            T::Atom(p) => self.v01_type_prod(p),
        }
    }

    fn v01_type_prod(&mut self, p: &ast::TypeProd) {
        self.v01_type_app(&p.first);
        for s in &p.rest {
            self.v01_type_app(&s.ty);
        }
    }

    fn v01_type_app(&mut self, a: &ast::TypeApp) {
        use ast::TypeApp as A;
        match a {
            A::InlineCmdTy { args, .. }
            | A::BlockCmdTy { args, .. }
            | A::MathCmdTy { args, .. } => {
                for it in args {
                    if let Some(o) = &it.opts {
                        for e in &o.entries {
                            self.reference(Ns::Field, &e.label.name, e.label.span);
                            self.v01_type(&e.ty);
                        }
                    }
                    self.v01_type(&it.ty);
                }
            }
            A::AppliedLong { ctor, first, rest } => {
                self.qualified(Ns::Type, &ctor.mods, &ctor.name, ctor.span);
                self.v01_type_atom(first);
                for r in rest {
                    self.v01_type_atom(r);
                }
            }
            A::Applied { ctor, first, rest } => {
                self.reference(Ns::Type, &ctor.name, ctor.span);
                self.v01_type_atom(first);
                for r in rest {
                    self.v01_type_atom(r);
                }
            }
            A::Atom(t) => self.v01_type_atom(t),
        }
    }

    fn v01_type_atom(&mut self, a: &ast::TypeAtom) {
        use ast::TypeAtom as A;
        match a {
            A::Paren { inner, .. } => self.v01_type(inner),
            A::Record { inner, .. } => {
                for f in &inner.fields {
                    self.reference(Ns::Field, &f.name.name, f.name.span);
                    self.v01_type(&f.ty);
                }
                if let Some(r) = &inner.row_tail {
                    self.reference(Ns::TypeVar, &r.var.name, r.var.span);
                }
            }
            A::Var(v) => self.reference(Ns::TypeVar, &v.name, v.span),
            A::LongName(v) => self.qualified(Ns::Type, &v.mods, &v.name, v.span),
            A::Name(v) => self.reference(Ns::Type, &v.name, v.span),
        }
    }
}

fn horz_name(c: &AnyHorzCmdTok) -> (String, Span) {
    match c {
        AnyHorzCmdTok::Plain(t) => (t.name.clone(), t.span),
        AnyHorzCmdTok::Mod(t) => (t.name.clone(), t.span),
    }
}

fn vert_name(c: &AnyVertCmdTok) -> (String, Span) {
    match c {
        AnyVertCmdTok::Plain(t) => (t.name.clone(), t.span),
        AnyVertCmdTok::Mod(t) => (t.name.clone(), t.span),
    }
}

fn render(quals: &[String], name: &str) -> String {
    match quals.is_empty() {
        true => name.to_string(),
        false => format!("{}.{name}", quals.join(".")),
    }
}
