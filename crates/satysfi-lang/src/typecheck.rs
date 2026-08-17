//! The Hindley–Milner type inferencer (phase 3, part 2): walks an
//! [`crate::elaborate::Program`] and reports the first type error it finds,
//! mirroring v0.0.6's `typecheck`/`typecheck_sub`
//! (`src/frontend/typechecker.ml`) — unification itself lives in
//! `crate::unify`, generalization/instantiation in `crate::types`, this
//! module only walks the AST applying those primitives at each rule, exactly
//! as `typechecker.ml` does over its own `unify`/`Typeenv`.
//!
//! This is validation only: `typecheck` returns `Result<(), TypeError>` and
//! never touches the (unchanged, untyped) evaluator — a program that passes
//! `typecheck` is then evaluated exactly as it always was.
//!
//! **Deviations from v0.0.6 and permissive corners** are called out inline
//! at each rule with a `PERMISSIVE:` comment; see this module's doc comment
//! in the crate report for the full list. The short version: math-mode
//! command/embed typing (real typesetting is phase 7) and unbound type-name/
//! type-variable references inside a `type` declaration's payload are
//! accepted with a fresh/nominal stand-in type rather than rejected, because
//! rejecting them would regress fixtures and tests this milestone still
//! needs to pass untyped.

use crate::ast::{Ast, BText, IText, MathElem, Pattern};
use crate::elaborate::{Program, UserTypeDecl};
use crate::prim_types::{
    self, arrow, builtin_variants, list, mandatory, product, reff, t_block_boxes,
    t_block_text, t_bool, t_context, t_document, t_float, t_inline_boxes, t_inline_text, t_int,
    t_length, t_string, t_unit, VariantDecl,
};
use crate::types::{
    self, generalize, instantiate, resolve, BaseType, CmdArgType, MonoType, PolyType, Row,
    TypeContext,
};
use crate::unify::{unify, UnifyError};
use satysfi_syntax::cst::ast::{TypeAtom, TypeExpr};
use satysfi_syntax::span::Span;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

// ============================================================================
// Errors
// ============================================================================

/// A type error: a best-effort span (see `ast.rs`'s module doc comment — only
/// `Var`/command/embed nodes carry spans, so most rules fall back to `None`),
/// a "while typing …" context message, and — for anything that actually came
/// from a failed [`unify`] call — the [`UnifyError`] itself, whose `Display`
/// already renders both types involved.
#[derive(Debug)]
pub struct TypeError {
    pub span: Option<Span>,
    pub message: String,
    pub source: Option<UnifyError>,
}

impl TypeError {
    fn from_unify(span: Option<Span>, what: impl Into<String>, source: UnifyError) -> TypeError {
        TypeError {
            span,
            message: format!("while typing {}", what.into()),
            source: Some(source),
        }
    }

    fn simple(span: Option<Span>, message: impl Into<String>) -> TypeError {
        TypeError {
            span,
            message: message.into(),
            source: None,
        }
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(f, "{span}: {}", self.message)?,
            None => write!(f, "{}", self.message)?,
        }
        if let Some(src) = &self.source {
            write!(f, ": {src}")?;
        }
        Ok(())
    }
}

impl std::error::Error for TypeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

// ============================================================================
// The primitive name table.
//
// `prim_types::primitive_type` is a pure name -> scheme lookup with no way to
// enumerate its own domain, and `primitives.rs`'s `PRIM_DEFS` table (the
// actual source of truth) is private to that module — so, per this
// milestone's contract (`primitives.rs`/`prim_types.rs` are read-only), this
// list is hand-kept in sync and cross-checked against `primitives.rs`'s
// source text by a test (`tests/typecheck.rs`) rather than derived
// mechanically. It matches `types_unify.rs`'s `every_registered_primitive_
// has_a_type` test's own `NAMES` list (phase 4 dropped `document`/`+p`/
// `\emph` from both — they're no longer primitives at all, see
// `primitives.rs`'s module doc comment — and added `set-font-key`, the one
// genuinely new primitive phase 4 introduces).
// ============================================================================

pub const PRIMITIVE_NAMES: &[&str] = &[
    "read-inline",
    "read-block",
    "line-break",
    "page-break",
    "+",
    "-",
    "*",
    "/",
    "mod",
    "==",
    "<>",
    "<",
    ">",
    "<=",
    ">=",
    "&&",
    "||",
    "not",
    "+.",
    "-.",
    "*.",
    "/.",
    "float",
    "round",
    "+'",
    "-'",
    "*'",
    "/'",
    "<'",
    ">'",
    "^",
    "arabic",
    "string-same",
    "::",
    "!",
    "string-length",
    "string-sub",
    "string-explode",
    "embed-string",
    "inline-fil",
    // ---- phase 4, part 1 additions (context ops / box combinators) ----
    "set-font-size",
    "get-font-size",
    "set-leading",
    "set-paragraph-margin",
    "get-text-width",
    "get-initial-context",
    "++",
    "+++",
    "inline-nil",
    "block-nil",
    "inline-skip",
    "inline-glue",
    "block-skip",
    // ---- phase 4, part 2 addition (see primitives.rs's `prims!` table
    // comment on `"set-font-key"`) ----
    "set-font-key",
];

fn base_type_env() -> TypeEnv {
    let mut env = TypeEnv::default();
    for name in PRIMITIVE_NAMES {
        if let Some(poly) = prim_types::primitive_type(name) {
            env = env.with(*name, poly);
        }
    }
    env
}

// ============================================================================
// The type environment — a flat, persistent-clone name -> scheme map, the
// same shape as `elaborate::Scope` (see its doc comment); cloning is cheap
// enough at this milestone's program sizes, and keeps this module's style
// consistent with the elaborator it sits directly behind.
// ============================================================================

#[derive(Clone, Default)]
struct TypeEnv {
    vars: HashMap<String, PolyType>,
}

impl TypeEnv {
    fn with(&self, name: impl Into<String>, poly: PolyType) -> TypeEnv {
        let mut e = self.clone();
        e.vars.insert(name.into(), poly);
        e
    }

    fn get(&self, name: &str) -> Option<&PolyType> {
        self.vars.get(name)
    }
}

// ============================================================================
// Lowering CST `TypeExpr` (a `type` declaration's ctor payload syntax) to
// `MonoType`. The grammar (`satysfi_syntax::cst::ast::TypeExpr`/`TypeAtom`)
// is deliberately minimal — only function arrows, parens, type variables,
// and bare names; no products, no `list`/`ref` postfix, no applied type
// constructors (see that module's doc comment) — so this lowering is total
// (never fails) and needs no arity checking of its own.
// ============================================================================

/// Map a `type` declaration's bare type name to a `MonoType`. Every base
/// type this milestone's primitives use is recognized by its surface name;
/// anything else becomes a nominal, zero-argument `Variant` reference — the
/// only shape a bare name in this minimal grammar could sensibly mean (no
/// applied-constructor syntax exists to give it arguments), which is exactly
/// what makes mutually-recursive user variant types (`type t = .. of t`) and
/// forward references (a later declaration's name used by an earlier one)
/// "just work": the name is resolved nominally, not by looking anything up
/// at lowering time.
fn name_to_mono(name: &str) -> MonoType {
    match name {
        "unit" => t_unit(),
        "bool" => t_bool(),
        "int" => t_int(),
        "float" => t_float(),
        "length" => t_length(),
        "string" => t_string(),
        "inline-text" => t_inline_text(),
        "block-text" => t_block_text(),
        "math" => MonoType::Base(BaseType::MathText),
        "inline-boxes" => t_inline_boxes(),
        "block-boxes" => t_block_boxes(),
        "context" => t_context(),
        "document" => t_document(),
        other => MonoType::Variant(other.to_string(), Vec::new()),
    }
}

fn lower_type_atom(atom: &TypeAtom, tyvars: &HashMap<String, MonoType>) -> MonoType {
    match atom {
        TypeAtom::Paren { inner, .. } => lower_type_expr(inner, tyvars),
        TypeAtom::Var(tv) => match tyvars.get(&tv.name) {
            Some(v) => v.clone(),
            // PERMISSIVE: a type variable not among the declaration's own
            // `tyvars` (should not happen for anything the parser accepts,
            // since `TypeAtom::Var` can only ever spell one of them here —
            // there is no scoping construct in this grammar that could
            // introduce any other free type variable) — treat it as its own
            // fresh, ungeneralized variable rather than rejecting the whole
            // declaration.
            None => MonoType::Var(types::new_ty_var(0)),
        },
        TypeAtom::Name(name) => name_to_mono(&name.name),
    }
}

fn lower_type_expr(ty: &TypeExpr, tyvars: &HashMap<String, MonoType>) -> MonoType {
    match ty {
        TypeExpr::Fun { dom, cod, .. } => arrow(
            lower_type_atom(dom, tyvars),
            lower_type_expr(cod, tyvars),
        ),
        TypeExpr::Atom(atom) => lower_type_atom(atom, tyvars),
    }
}

/// Lower one [`UserTypeDecl`] (surfaced by `elaborate::elaborate_program`)
/// into a [`VariantDecl`], the same shape `prim_types::builtin_variants`
/// produces for `option`/`itemize` — see that struct's doc comment for how
/// `param_vars` and `instantiate_ctor` fit together.
fn build_variant_decl(decl: &UserTypeDecl) -> VariantDecl {
    let param_vars: Vec<types::TyVarRef> =
        decl.params.iter().map(|_| types::new_ty_var(0)).collect();
    let tyvar_map: HashMap<String, MonoType> = decl
        .params
        .iter()
        .cloned()
        .zip(param_vars.iter().cloned().map(MonoType::Var))
        .collect();
    let ctors = decl
        .ctors
        .iter()
        .map(|(name, ty)| (name.clone(), ty.as_ref().map(|t| lower_type_expr(t, &tyvar_map))))
        .collect();
    VariantDecl {
        name: decl.name.clone(),
        params: decl.params.len(),
        ctors,
        param_vars,
    }
}

// ============================================================================
// The checker.
// ============================================================================

struct Checker {
    ctx: TypeContext,
    /// Constructor name -> the (`Rc`-shared) declaration it belongs to.
    /// Later declarations shadow earlier ones of the same ctor name, mirroring
    /// ordinary name shadowing elsewhere in this port.
    ctors: HashMap<String, Rc<VariantDecl>>,
}

impl Checker {
    fn new(program: &Program) -> Checker {
        let mut ctors = HashMap::new();
        for decl in builtin_variants() {
            let decl = Rc::new(decl);
            for (cname, _) in &decl.ctors {
                ctors.insert(cname.clone(), decl.clone());
            }
        }
        for utd in &program.type_decls {
            let decl = Rc::new(build_variant_decl(utd));
            for (cname, _) in &decl.ctors {
                ctors.insert(cname.clone(), decl.clone());
            }
        }
        Checker {
            ctx: TypeContext::new(),
            ctors,
        }
    }

    fn fresh(&mut self) -> MonoType {
        MonoType::Var(self.ctx.fresh_var())
    }

    fn unify_ctx(
        &mut self,
        expected: &MonoType,
        found: &MonoType,
        span: Option<Span>,
        what: &str,
    ) -> Result<(), TypeError> {
        unify(expected, found).map_err(|e| TypeError::from_unify(span, what, e))
    }

    /// Turn a `\`/`+`-named `LetIn` binding's ordinarily-inferred value type
    /// `tv` into the genuine command type (`MonoType::InlineCmd`/`BlockCmd`)
    /// it gets bound under, per this phase's mandate (see this module's
    /// crate-report entry): a user-defined command is no longer typed as a
    /// plain "context-curried" function, but as `[τ1; ..; τn] inline-cmd`
    /// (resp. `block-cmd`), matching v0.0.6's real `HorzCommandType`/
    /// `VertCommandType` (`typechecker.ml`'s `UTLetHorzIn`/`UTLetVertIn`
    /// rules).
    ///
    /// Two shapes reach this function, per [`command_sigil`]'s call site:
    ///
    /// * a genuine `let-inline`/`let-block` definition, whose value is
    ///   exactly the `Lambda(ctxvar, Lambda(p1, .., Lambda(pn, body)))` chain
    ///   `elaborate::elaborate_let_inline` builds — so `tv` is a plain `Func`
    ///   chain `ctx_ty -> t1 -> .. -> tn -> result_ty`. [`peel_func_chain`]
    ///   recovers that shape; the leading domain must unify with `context`,
    ///   the final codomain with `inline-boxes`/`block-boxes`, and the
    ///   domains in between become the command's `CmdArgType` list.
    /// * a qualified-name *alias* of an already-command-typed binding (a
    ///   module's own `M.\cmd` re-export, or `open`'s re-binding of it under
    ///   its bare suffix — both build a `LetIn(name, Ast::Var(qualified),
    ///   body)`, see `elaborate.rs`'s `export_alias`/`Expr::OpenIn` case): by
    ///   the time such an alias is processed, the aliased name was *already*
    ///   run through this same function at its own original `let-inline`/
    ///   `let-block` site, so its scheme's body is already
    ///   `MonoType::InlineCmd`/`BlockCmd` — `self.infer` on the `Ast::Var`
    ///   simply instantiates that scheme, so `tv` here already *is* the
    ///   command type. This branch is transparent: it passes such a `tv`
    ///   through unchanged (re-generalized) rather than trying to peel a
    ///   `Func` chain out of something that isn't one.
    fn command_scheme(
        &mut self,
        name: &str,
        sigil: char,
        tv: MonoType,
        span: Option<Span>,
    ) -> Result<PolyType, TypeError> {
        debug_assert!(sigil == '\\' || sigil == '+');
        let is_inline = sigil == '\\';
        let (want_result, kind, other_kind) = if is_inline {
            (t_inline_boxes(), "inline", "block")
        } else {
            (t_block_boxes(), "block", "inline")
        };

        match resolve(&tv) {
            MonoType::InlineCmd(_) if is_inline => {
                return Ok(generalize(self.ctx.level(), &tv));
            }
            MonoType::BlockCmd(_) if !is_inline => {
                return Ok(generalize(self.ctx.level(), &tv));
            }
            MonoType::InlineCmd(_) | MonoType::BlockCmd(_) => {
                return Err(TypeError::simple(
                    span,
                    format!(
                        "'{name}' is bound to a {other_kind} command, but its \
                         name marks it as {article} {kind} command",
                        article = if kind == "inline" { "an" } else { "a" },
                    ),
                ));
            }
            _ => {}
        }

        let (mut doms, result) = peel_func_chain(tv);
        if doms.is_empty() {
            return Err(TypeError::simple(
                span,
                format!(
                    "the binding for '{name}' must be a function taking a \
                     context as its first argument (e.g. via `let-inline ctx \
                     {name} .. = ..`)"
                ),
            ));
        }
        let ctx_ty = doms.remove(0);
        self.unify_ctx(
            &t_context(),
            &ctx_ty,
            span,
            &format!("the context argument of '{name}'"),
        )?;
        self.unify_ctx(
            &want_result,
            &result,
            span,
            &format!("the result of '{name}'"),
        )?;
        let params: Vec<CmdArgType> = doms.into_iter().map(mandatory).collect();
        let cmd_ty = if is_inline {
            MonoType::InlineCmd(params)
        } else {
            MonoType::BlockCmd(params)
        };
        Ok(generalize(self.ctx.level(), &cmd_ty))
    }

    /// Shared by `check_itext`'s `IText::Cmd` and `check_btext`'s
    /// `BText::Cmd`: check a command application's argument count (exact —
    /// no optional arguments exist among today's commands, see this
    /// function's callers) and each argument's type against `params`
    /// (already resolved to a concrete `MonoType::InlineCmd`/`BlockCmd`'s
    /// payload by the caller).
    fn check_cmd_args(
        &mut self,
        env: &TypeEnv,
        name: &str,
        span: Span,
        params: &[CmdArgType],
        args: &[Ast],
    ) -> Result<(), TypeError> {
        if params.len() != args.len() {
            return Err(TypeError::simple(
                Some(span),
                format!(
                    "command '{name}' expects {} argument{}, got {}",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            ));
        }
        for (i, (param, arg)) in params.iter().zip(args.iter()).enumerate() {
            let targ = self.infer(env, arg)?;
            self.unify_ctx(
                &param.ty,
                &targ,
                ast_span(arg).or(Some(span)),
                &format!("argument {} of '{name}'", i + 1),
            )?;
        }
        Ok(())
    }

    // ---- expressions -------------------------------------------------------

    fn infer(&mut self, env: &TypeEnv, ast: &Ast) -> Result<MonoType, TypeError> {
        match ast {
            Ast::Unit => Ok(t_unit()),
            Ast::Bool(_) => Ok(t_bool()),
            Ast::Int(_) => Ok(t_int()),
            Ast::Float(_) => Ok(t_float()),
            Ast::Length(_) => Ok(t_length()),
            Ast::Str(_) => Ok(t_string()),

            Ast::Var(name, span) => match env.get(name) {
                Some(poly) => Ok(instantiate(poly, self.ctx.level())),
                // Should not happen post-elaboration: `elaborate.rs`'s
                // `scoped_var` already rejects any unbound name before this
                // ever runs. Surfaced as a (spanned) error rather than a
                // panic anyway, since "should not happen" isn't "cannot".
                None => Err(TypeError::simple(
                    Some(*span),
                    format!("internal error: unbound variable '{name}' reached the typechecker"),
                )),
            },

            Ast::Apply(f, a) => {
                let tf = self.infer(env, f)?;
                let ta = self.infer(env, a)?;
                let tr = self.fresh();
                self.unify_ctx(
                    &tf,
                    &arrow(ta, tr.clone()),
                    ast_span(f),
                    "function application",
                )?;
                Ok(tr)
            }

            Ast::Lambda(param, body) => {
                let tp = self.fresh();
                let inner = env.with(param.clone(), PolyType::mono(tp.clone()));
                let tb = self.infer(&inner, body)?;
                Ok(arrow(tp, tb))
            }

            Ast::LetIn(name, value, body) => {
                self.ctx.enter_level();
                let tv = self.infer(env, value)?;
                self.ctx.leave_level();
                let scheme = match command_sigil(name) {
                    // A `\`/`+`-named binding: either a genuine `let-inline`/
                    // `let-block` definition (`value` is the
                    // `Lambda(ctxvar, Lambda(p1, .., body))` chain
                    // `elaborate_let_inline` builds) or a qualified-name
                    // alias of one (`value` is a bare `Ast::Var`, from a
                    // module's own `M.\cmd` re-export or an `open`) — see
                    // `command_scheme`.
                    Some(sigil) => self.command_scheme(name, sigil, tv, ast_span(value))?,
                    None => generalize(self.ctx.level(), &tv),
                };
                let inner = env.with(name.clone(), scheme);
                self.infer(&inner, body)
            }

            Ast::LetRecIn(bindings, body) => {
                self.ctx.enter_level();
                let mut rec_env = env.clone();
                let mut vars = Vec::with_capacity(bindings.len());
                for (name, _) in bindings {
                    let v = self.fresh();
                    vars.push(v.clone());
                    rec_env = rec_env.with(name.clone(), PolyType::mono(v));
                }
                for ((name, val), v) in bindings.iter().zip(vars.iter()) {
                    let tv = self.infer(&rec_env, val)?;
                    self.unify_ctx(
                        v,
                        &tv,
                        ast_span(val),
                        &format!("let-rec binding '{name}'"),
                    )?;
                }
                self.ctx.leave_level();
                let mut body_env = env.clone();
                for ((name, _), v) in bindings.iter().zip(vars.iter()) {
                    let scheme = generalize(self.ctx.level(), v);
                    body_env = body_env.with(name.clone(), scheme);
                }
                self.infer(&body_env, body)
            }

            Ast::IfThenElse(cond, then_b, else_b) => {
                let tc = self.infer(env, cond)?;
                self.unify_ctx(&t_bool(), &tc, ast_span(cond), "the condition of 'if'")?;
                let tt = self.infer(env, then_b)?;
                let te = self.infer(env, else_b)?;
                self.unify_ctx(&tt, &te, ast_span(else_b), "the branches of 'if'")?;
                Ok(tt)
            }

            Ast::Match(scrutinee, arms) => {
                let tscrut = self.infer(env, scrutinee)?;
                let mut result: Option<MonoType> = None;
                for arm in arms {
                    let arm_env = self.bind_pattern(env.clone(), &arm.pat, &tscrut)?;
                    if let Some(guard) = &arm.guard {
                        let tg = self.infer(&arm_env, guard)?;
                        self.unify_ctx(&t_bool(), &tg, ast_span(guard), "a match guard")?;
                    }
                    let tbody = self.infer(&arm_env, &arm.body)?;
                    match &result {
                        None => result = Some(tbody),
                        Some(r) => {
                            self.unify_ctx(r, &tbody, ast_span(&arm.body), "the arms of 'match'")?
                        }
                    }
                }
                // `Match`'s `arms` is always non-empty (`c::Expr::Match`
                // requires a `first` arm plus zero or more `rest`), so
                // `result` is always `Some` in practice; the fallback fresh
                // variable is defensive only.
                Ok(result.unwrap_or_else(|| self.fresh()))
            }

            Ast::Tuple(items) => {
                let tys = items
                    .iter()
                    .map(|it| self.infer(env, it))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(product(tys))
            }

            Ast::Ctor(name, payload) => self.infer_ctor(env, name, payload.as_deref(), None),

            Ast::Record(fields) => {
                let mut typed = Vec::with_capacity(fields.len());
                for (label, e) in fields {
                    typed.push((label.clone(), self.infer(env, e)?));
                }
                let mut row = Row::Empty;
                for (label, ty) in typed.into_iter().rev() {
                    row = Row::Cons(label, Box::new(ty), Box::new(row));
                }
                Ok(MonoType::Record(row))
            }

            Ast::List(items) => {
                let elem = self.fresh();
                for it in items {
                    let t = self.infer(env, it)?;
                    self.unify_ctx(&elem, &t, ast_span(it), "a list element")?;
                }
                Ok(list(elem))
            }

            Ast::InlineText(elems) => {
                for e in elems.iter() {
                    self.check_itext(env, e)?;
                }
                Ok(t_inline_text())
            }

            Ast::BlockText(elems) => {
                for e in elems.iter() {
                    self.check_btext(env, e)?;
                }
                Ok(t_block_text())
            }

            Ast::MathText(elems) => {
                for e in elems.iter() {
                    self.check_math_elem(env, e)?;
                }
                Ok(MonoType::Base(BaseType::MathText))
            }

            Ast::LetMutableIn(name, init, body) => {
                // NO generalization: `let-mutable`'s binding is the
                // classic ML "value restriction" case — a mutable reference
                // must stay monomorphic, or `let-mutable r <- [] in ((r <-
                // 1 :: !r); (r <- true :: !r); !r)`-style code could smuggle
                // an `int` and a `bool` through the very same cell. Binding
                // it via `PolyType::mono` (not `generalize`) enforces this
                // directly: every use of `name` in `body` shares the exact
                // same `Ref` type, not a fresh instantiation.
                let tinit = self.infer(env, init)?;
                let inner = env.with(name.clone(), PolyType::mono(reff(tinit)));
                self.infer(&inner, body)
            }

            Ast::Overwrite(name, span, value) => {
                let t_ref = match env.get(name) {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                                "internal error: unbound mutable variable '{name}' reached the typechecker"
                            ),
                        ))
                    }
                };
                let inner = self.fresh();
                self.unify_ctx(
                    &t_ref,
                    &reff(inner.clone()),
                    Some(*span),
                    &format!("the overwrite target '{name}'"),
                )?;
                let tvalue = self.infer(env, value)?;
                // Prefer the overwrite's own (always-present) span over
                // `ast_span(value)`, which is `None` for most value shapes
                // (literals carry no span at all — see `ast.rs`'s module
                // doc comment) and would otherwise leave this common error
                // unlocated.
                self.unify_ctx(
                    &inner,
                    &tvalue,
                    ast_span(value).or(Some(*span)),
                    &format!("the overwrite value for '{name}'"),
                )?;
                Ok(t_unit())
            }

            Ast::WhileDo(cond, body) => {
                let tc = self.infer(env, cond)?;
                self.unify_ctx(&t_bool(), &tc, ast_span(cond), "the condition of 'while'")?;
                let tb = self.infer(env, body)?;
                self.unify_ctx(&t_unit(), &tb, ast_span(body), "the body of 'while'")?;
                Ok(t_unit())
            }

            Ast::Sequential(a, b) => {
                let ta = self.infer(env, a)?;
                // v0.0.6 requires the left-hand side of `before`/`;` to be
                // `unit` (`typechecker.ml`'s `UTSequential` case): not just
                // "evaluated and discarded" but type-checked as `unit`
                // specifically, so e.g. a stray non-unit expression used
                // only for effect (but returning, say, an `int`) is rejected
                // rather than silently ignored.
                self.unify_ctx(
                    &t_unit(),
                    &ta,
                    ast_span(a),
                    "the left-hand side of 'before'",
                )?;
                self.infer(env, b)
            }

            Ast::AccessField(e, label, span) => {
                let te = self.infer(env, e)?;
                let field = self.fresh();
                let rv = self.ctx.fresh_row_var();
                let open_row = MonoType::Record(Row::Cons(
                    label.clone(),
                    Box::new(field.clone()),
                    Box::new(Row::Var(rv)),
                ));
                self.unify_ctx(
                    &open_row,
                    &te,
                    Some(*span),
                    &format!("the field access '#{label}'"),
                )?;
                Ok(field)
            }

            Ast::UpdateField(base, label, value) => {
                let tbase = self.infer(env, base)?;
                let tvalue = self.infer(env, value)?;
                let rv = self.ctx.fresh_row_var();
                let open_row = MonoType::Record(Row::Cons(
                    label.clone(),
                    Box::new(tvalue),
                    Box::new(Row::Var(rv)),
                ));
                self.unify_ctx(
                    &open_row,
                    &tbase,
                    ast_span(base),
                    &format!("the record update of '{label}'"),
                )?;
                Ok(tbase)
            }
        }
    }

    /// Shared by `Ast::Ctor` and pattern-matching's `Pattern::Ctor`: look up
    /// `name`'s declaration, mint fresh type arguments for its (possibly
    /// zero) parameters, and check the payload — either an already-inferred
    /// expression type to unify against (`Ast::Ctor`'s case, via `infer`
    /// directly) or nothing (patterns bind their own payload separately, in
    /// `bind_pattern`). `expected_result`, if given, is unified against the
    /// application's result type — used by nothing yet in this milestone's
    /// rules but kept general for symmetry; always `None` from `infer`,
    /// which just returns the result type instead.
    fn infer_ctor(
        &mut self,
        env: &TypeEnv,
        name: &str,
        payload: Option<&Ast>,
        expected_result: Option<&MonoType>,
    ) -> Result<MonoType, TypeError> {
        let decl = self.ctors.get(name).cloned().ok_or_else(|| {
            TypeError::simple(None, format!("unknown constructor '{name}'"))
        })?;
        let args: Vec<MonoType> = (0..decl.params).map(|_| self.fresh()).collect();
        let (payload_ty, result_ty) = decl.instantiate_ctor(name, &args).ok_or_else(|| {
            TypeError::simple(
                None,
                format!("constructor '{name}' applied with the wrong number of type arguments"),
            )
        })?;
        if let Some(expected) = expected_result {
            self.unify_ctx(expected, &result_ty, None, &format!("constructor '{name}'"))?;
        }
        match (payload_ty, payload) {
            (Some(expected), Some(actual)) => {
                let actual_ty = self.infer(env, actual)?;
                self.unify_ctx(
                    &expected,
                    &actual_ty,
                    ast_span(actual),
                    &format!("the payload of constructor '{name}'"),
                )?;
            }
            (None, None) => {}
            (Some(_), None) => {
                return Err(TypeError::simple(
                    None,
                    format!("constructor '{name}' expects a payload but none was given"),
                ))
            }
            (None, Some(_)) => {
                return Err(TypeError::simple(
                    None,
                    format!("constructor '{name}' takes no payload but one was given"),
                ))
            }
        }
        Ok(result_ty)
    }

    // ---- patterns ------------------------------------------------------

    /// Type-check `pat` against `ty`, extending (a clone of) `env` with
    /// every name it binds. Mirrors `typechecker.ml`'s `typecheck_pattern`.
    fn bind_pattern(
        &mut self,
        env: TypeEnv,
        pat: &Pattern,
        ty: &MonoType,
    ) -> Result<TypeEnv, TypeError> {
        match pat {
            Pattern::Wild => Ok(env),
            Pattern::Var(name) => Ok(env.with(name.clone(), PolyType::mono(ty.clone()))),
            Pattern::Unit => {
                self.unify_ctx(&t_unit(), ty, None, "a unit pattern")?;
                Ok(env)
            }
            Pattern::Bool(_) => {
                self.unify_ctx(&t_bool(), ty, None, "a boolean pattern")?;
                Ok(env)
            }
            Pattern::Int(_) => {
                self.unify_ctx(&t_int(), ty, None, "an integer pattern")?;
                Ok(env)
            }
            Pattern::Str(_) => {
                self.unify_ctx(&t_string(), ty, None, "a string pattern")?;
                Ok(env)
            }
            Pattern::Tuple(pats) => {
                let elem_tys: Vec<MonoType> = pats.iter().map(|_| self.fresh()).collect();
                self.unify_ctx(&product(elem_tys.clone()), ty, None, "a tuple pattern")?;
                let mut env = env;
                for (p, t) in pats.iter().zip(elem_tys.iter()) {
                    env = self.bind_pattern(env, p, t)?;
                }
                Ok(env)
            }
            Pattern::EmptyList => {
                let elem = self.fresh();
                self.unify_ctx(&list(elem), ty, None, "an empty-list pattern")?;
                Ok(env)
            }
            Pattern::Cons(head, tail) => {
                let elem = self.fresh();
                self.unify_ctx(&list(elem.clone()), ty, None, "a cons pattern")?;
                let env = self.bind_pattern(env, head, &elem)?;
                self.bind_pattern(env, tail, &list(elem))
            }
            Pattern::Ctor(name, payload) => {
                let decl = self.ctors.get(name).cloned().ok_or_else(|| {
                    TypeError::simple(None, format!("unknown constructor '{name}' in a pattern"))
                })?;
                let args: Vec<MonoType> = (0..decl.params).map(|_| self.fresh()).collect();
                let (payload_ty, result_ty) = decl.instantiate_ctor(name, &args).ok_or_else(|| {
                    TypeError::simple(
                        None,
                        format!(
                            "constructor '{name}' applied with the wrong number of type arguments in a pattern"
                        ),
                    )
                })?;
                self.unify_ctx(
                    &result_ty,
                    ty,
                    None,
                    &format!("the constructor pattern '{name}'"),
                )?;
                match (payload_ty, payload) {
                    (Some(expected), Some(p)) => self.bind_pattern(env, p, &expected),
                    (None, None) => Ok(env),
                    (Some(_), None) => Err(TypeError::simple(
                        None,
                        format!("constructor pattern '{name}' expects a payload but none was given"),
                    )),
                    (None, Some(_)) => Err(TypeError::simple(
                        None,
                        format!("constructor pattern '{name}' takes no payload but one was given"),
                    )),
                }
            }
            Pattern::As(inner, name) => {
                let env = self.bind_pattern(env, inner, ty)?;
                Ok(env.with(name.clone(), PolyType::mono(ty.clone())))
            }
        }
    }

    // ---- inline / block / math text -------------------------------------

    /// Check one inline-text element. A command's own type is a genuine
    /// `MonoType::InlineCmd(params)` (`[...] inline-cmd`, mirroring v0.0.6's
    /// `HorzCommandType`) — bound either by `Ast::LetIn`'s command-binding
    /// rule (`Checker::command_scheme`) or, for the milestone's built-in
    /// commands, directly by `prim_types::primitive_type`'s `\emph` entry.
    /// Checking an application here is exact-arity plus one unification per
    /// argument against `params`, via `check_cmd_args` — there is no longer
    /// any `context -> arg1 -> .. -> inline-boxes` function shape to unify
    /// the whole command type against.
    fn check_itext(&mut self, env: &TypeEnv, it: &IText) -> Result<(), TypeError> {
        match it {
            IText::Text(_) => Ok(()),
            IText::Cmd { name, span, args } => {
                let tcmd = match env.get(name) {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                                "internal error: unbound inline command '{name}' reached the typechecker"
                            ),
                        ))
                    }
                };
                match resolve(&tcmd) {
                    MonoType::InlineCmd(params) => {
                        self.check_cmd_args(env, name, *span, &params, args)
                    }
                    other => Err(TypeError::simple(
                        Some(*span),
                        format!(
                            "internal error: inline command '{name}' does not have an \
                             inline-cmd type (found `{other}`)"
                        ),
                    )),
                }
            }
            IText::Embed { expr, span } => {
                let te = self.infer(env, expr)?;
                self.unify_ctx(
                    &t_inline_text(),
                    &te,
                    Some(*span),
                    "an inline-text '#…;' embed",
                )?;
                Ok(())
            }
            IText::EmbedMath { elems, span: _ } => {
                // PERMISSIVE: quoted-math embedded in inline text is only
                // ever read at run time by `read-inline`, which currently
                // (milestone 1, ahead of phase 7's real math typesetting)
                // always errors on it — so there is no real type to check
                // its embedded expressions against yet. Type each embedded
                // expression against its own fresh variable, purely so
                // unbound-name mistakes elsewhere inside it still get
                // (indirectly) exercised, without asserting anything about
                // what the result should be.
                for me in elems.iter() {
                    self.check_math_elem(env, me)?;
                }
                Ok(())
            }
        }
    }

    /// Block-text analogue of `check_itext`'s `IText::Cmd` case — see its
    /// doc comment; a `BText::Cmd`'s type is `MonoType::BlockCmd(params)`.
    fn check_btext(&mut self, env: &TypeEnv, bt: &BText) -> Result<(), TypeError> {
        match bt {
            BText::Cmd { name, span, args } => {
                let tcmd = match env.get(name) {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                                "internal error: unbound block command '{name}' reached the typechecker"
                            ),
                        ))
                    }
                };
                match resolve(&tcmd) {
                    MonoType::BlockCmd(params) => {
                        self.check_cmd_args(env, name, *span, &params, args)
                    }
                    other => Err(TypeError::simple(
                        Some(*span),
                        format!(
                            "internal error: block command '{name}' does not have a \
                             block-cmd type (found `{other}`)"
                        ),
                    )),
                }
            }
            BText::Embed { expr, span } => {
                let te = self.infer(env, expr)?;
                self.unify_ctx(
                    &t_block_text(),
                    &te,
                    Some(*span),
                    "a block-text '#…;' embed",
                )?;
                Ok(())
            }
        }
    }

    /// PERMISSIVE (phase 7 owns real math typesetting): every quoted math
    /// element is walked purely to type-check whatever program-mode
    /// expressions it embeds (`Embed`, and each `Cmd` argument), each
    /// against its own fresh, unconstrained type — nothing here asserts
    /// what a math command's own signature should be (there is no
    /// `MathCmd`-typed primitive registered anywhere yet in
    /// `prim_types.rs` to check against), matching `read-inline`'s runtime
    /// behavior of simply erroring out on any embedded math today.
    fn check_math_elem(&mut self, env: &TypeEnv, m: &MathElem) -> Result<(), TypeError> {
        match m {
            MathElem::Chars(_) => Ok(()),
            MathElem::Group(elems) => {
                for e in elems {
                    self.check_math_elem(env, e)?;
                }
                Ok(())
            }
            MathElem::Sub(base, script) | MathElem::Sup(base, script) => {
                self.check_math_elem(env, base)?;
                for e in script {
                    self.check_math_elem(env, e)?;
                }
                Ok(())
            }
            MathElem::Primes(base, _) => self.check_math_elem(env, base),
            MathElem::Cmd { args, .. } => {
                for a in args {
                    self.infer(env, a)?;
                }
                Ok(())
            }
            MathElem::Embed { expr, .. } => {
                self.infer(env, expr)?;
                Ok(())
            }
        }
    }
}

/// If `name` (an `Ast::LetIn` binding's name) is command-shaped, the sigil
/// that says which kind — `'\\'` for an inline command, `'+'` for a block
/// command — else `None` for an ordinary variable binding.
///
/// Looks only at the *local* segment (after the last `.`): `elaborate.rs`'s
/// module name-mangling (`qualify_key`) spells a module-qualified command as
/// e.g. `"M.\cmd"`, sigil included on the local part but never on the
/// `mods.join(".")` prefix (module names are ordinary identifiers, so they
/// can never themselves start with `\`/`+`) — see `qualify_key`'s doc
/// comment. A bare (unqualified) name has no `.` at all, so
/// `rsplit('.').next()` degrades to the whole string, which is exactly what
/// we want.
fn command_sigil(name: &str) -> Option<char> {
    let local = name.rsplit('.').next().unwrap_or(name);
    match local.chars().next() {
        Some(c @ ('\\' | '+')) => Some(c),
        _ => None,
    }
}

/// Greedily unwrap a (resolved) `Func` chain into its list of domains and
/// final codomain: `dom1 -> dom2 -> .. -> domN -> result` becomes
/// `(vec![dom1, .., domN], result)`. Only ever follows the *codomain* at
/// each step (never recurses into a domain, even one that is itself a
/// `Func`) — used by `Checker::command_scheme` to recover a `let-inline`/
/// `let-block` binding's `context -> arg1 -> .. -> argN -> result` shape
/// from its ordinarily-inferred function type.
fn peel_func_chain(ty: MonoType) -> (Vec<MonoType>, MonoType) {
    let mut doms = Vec::new();
    let mut cur = ty;
    loop {
        match resolve(&cur) {
            MonoType::Func(dom, cod) => {
                doms.push(*dom);
                cur = *cod;
            }
            other => return (doms, other),
        }
    }
}

/// A best-effort span for an `Ast` node: only `Var`/`Overwrite`/
/// `AccessField` carry one directly (see `ast.rs`'s module doc comment);
/// everything else falls back to `None`; the resulting `TypeError` then just
/// prints without a location prefix.
fn ast_span(ast: &Ast) -> Option<Span> {
    match ast {
        Ast::Var(_, span) => Some(*span),
        Ast::Overwrite(_, span, _) => Some(*span),
        Ast::AccessField(_, _, span) => Some(*span),
        _ => None,
    }
}

/// Type-check a whole elaborated [`Program`]. Validation only: on success
/// the caller proceeds to evaluate `program.body` exactly as before (the
/// evaluator is untouched by this phase).
pub fn typecheck(program: &Program) -> Result<(), TypeError> {
    let mut checker = Checker::new(program);
    let env = base_type_env();
    checker.infer(&env, &program.body)?;
    Ok(())
}
