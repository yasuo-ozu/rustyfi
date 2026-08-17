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
use crate::elaborate::{Program, UserSynonymDecl, UserTypeDecl};
pub use crate::exhaustive::MatchWarning;
use crate::prim_types::{
    self, arrow, builtin_variants, list, mandatory, optional, product, reff, t_block_boxes,
    t_block_text, t_bool, t_context, t_document, t_float, t_inline_boxes, t_inline_text, t_int,
    t_length, t_math_text, t_option, t_string, t_unit, VariantDecl,
};
use crate::types::{
    self, generalize, instantiate, resolve, BaseType, CmdArgType, Kind, MonoType, PolyType, Row,
    TypeContext,
};
use crate::unify::{unify, UnifyError};
use satysfi_syntax::cst::ast::{CmdTypeKind, TypeApp, TypeAtom, TypeExpr, TypeProd};
use satysfi_syntax::cst::{RecordKind, SigConstraint, SigItem};
use satysfi_syntax::span::Span;
use std::collections::{BTreeSet, HashMap};
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
    "page-break-multicolumn",
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
    // ---- frontend-completion.md §Slice 1.A: the ~18 pure primitives ----
    // (`|>` excluded — see primitives.rs's `prims!` table comment; it has
    // no primitive of its own, so it never belongs in this list).
    "sin",
    "asin",
    "cos",
    "acos",
    "tan",
    "atan",
    "atan2",
    "log",
    "exp",
    "ceil",
    "floor",
    "show-float",
    "string-byte-length",
    "string-sub-bytes",
    "string-unexplode",
    "display-message",
    "abort-with-message",
    // ---- Slice 1 additions (raster images; docs/plans/math-images.md) ----
    "load-image",
    "use-image-by-width",
    // ---- Slice 1 graphics primitives (docs/plans/graphics-subsystem.md) ----
    "start-path",
    "line-to",
    "terminate-path",
    "close-with-line",
    "fill",
    "stroke",
    "inline-graphics",
    // ---- docs/plans/table-subsystem.md §Slice 1 ----
    "tabular",
    // ---- gr.satyh roadmap prims (docs/plans/graphics-subsystem.md §Full
    // roadmap A/B/C/D) ----
    "bezier-to",
    "close-with-bezier",
    "shift-path",
    "linear-transform-path",
    "shift-graphics",
    "linear-transform-graphics",
    "get-graphics-bbox",
    "dashed-stroke",
    "draw-text",
    // ---- pervasives.satyh unblockers (docs/plans/stdlib-port.md) ----
    "get-natural-metrics",
    "inline-frame-outer",
    "set-manual-rising",
    "script-guard",
    "discretionary",
    // ---- Tier-2 decoration/graphics packages (docs/plans/stdlib-port.md) ----
    "get-axis-height",
    // ---- docs/plans/hooks-annotations-crossref.md §Slice 1 ----
    "hook-page-break",
    "hook-page-break-block",
    "register-cross-reference",
    "get-cross-reference",
    // ---- docs/plans/hooks-annotations-crossref.md §B/§D (annot.satyh) ----
    "get-leftmost-script",
    "get-rightmost-script",
    "inline-frame-breakable",
    "register-destination",
    "register-link-to-uri",
    "register-link-to-location",
    // ---- docs/plans/math-engine.md §A + §G ----
    "math-char",
    "math-big-char",
    "math-char-with-kern",
    "math-big-char-with-kern",
    "math-concat",
    "math-group",
    "math-sup",
    "math-sub",
    "math-frac",
    "math-radical",
    "math-lower",
    "math-upper",
    "math-pull-in-scripts",
    "math-color",
    "math-char-class",
    "math-variant-char",
    "math-paren",
    "math-paren-with-middle",
    "text-in-math",
    "convert-string-for-math",
    "embed-math",
    "set-math-command",
    "set-math-font",
    "space-between-maths",
    "raise-inline",
    "embed-block-breakable",
    "unite-path",
    "set-min-gap-of-lines",
    "omit-skip-after",
    // ---- docs/plans/context-box-prims.md §Slice 1 (rows 1-10) ----
    "set-text-color",
    "get-text-color",
    "set-hyphen-penalty",
    "set-space-ratio",
    "split-into-lines",
    "block-frame-breakable",
    "embed-block-top",
    "set-font",
    "set-code-text-command",
    "get-natural-length",
    // ---- `docs/plans/build-order-to-stdja.md` step 8/9 orphans ----
    "set-dominant-wide-script",
    "set-dominant-narrow-script",
    "set-language",
    "set-every-word-break",
    "register-outline",
    "extract-string",
    // ---- proof.satyh/footnote-scheme.satyh unblockers (tail-prims sweep) ----
    "embed-block-bottom",
    "line-stack-bottom",
    "add-footnote",
    // ---- page-level prims blocking mitou-report/stdjareport ----
    "clear-page",
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
// Lowering CST `TypeExpr` (a `type` declaration's ctor payload syntax, a
// synonym's own body, and a `sig .. end`'s `val` annotations — the last
// parsed but not yet consulted, see `elaborate.rs`'s module doc comment) to
// `MonoType`. The grammar
// (`satysfi_syntax::cst::ast::TypeExpr`/`TypeProd`/`TypeApp`/`TypeAtom`)
// supports function arrows, parens, type variables, bare names, 2+-way
// product types (`*`), and a SINGLE-argument postfix type-constructor
// application (`'a option`, `'a list`) — no record/list-literal/command
// types or N-ary applied constructors (see that module's doc comment) — so
// this lowering is total (never fails) and needs no arity checking of its
// own. `list`/`ref` are recognized specially (they map to this port's
// dedicated `MonoType::List`/`MonoType::Ref` formers, not a nominal
// `Variant`, mirroring `prim_types::list`/`reff`); every other applied name
// (e.g. `option`) becomes a one-argument `MonoType::Variant`. A *synonym*
// reference is left exactly as `name_to_mono` produces it (indistinguishable
// from an unresolved variant name); transparently replacing it with the
// synonym's body — where the cyclic-synonym rejection lives — is
// `expand_synonyms`'s job, below.
// ============================================================================

/// Map a `type` declaration's bare type name to a `MonoType`. Every base
/// type this milestone's primitives use is recognized by its surface name;
/// anything else becomes a nominal, zero-argument `Variant` reference — the
/// only shape a bare name in this minimal grammar could sensibly mean (no
/// applied-constructor syntax exists to give it arguments), which is exactly
/// what makes mutually-recursive user variant types (`type t = .. of t`),
/// forward references (a later declaration's name used by an earlier one),
/// and type *synonyms* (a synonym reference is resolved the same nominal
/// way — see `expand_synonyms`) "just work": the name is resolved nominally,
/// not by looking anything up at lowering time.
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
        // `[ty; ty?; ..] inline-cmd`/`block-cmd`/`math-cmd` — the direct
        // wire-up to the existing `CmdArgType.optional` field
        // (`docs/plans/class-signature-lang-gaps.md` gap 2, step 1): each
        // bracketed element lowers to one `CmdArgType`, `optional` set
        // exactly when the element carried a trailing `?`.
        TypeAtom::Cmd { args, kind, .. } => {
            let cmd_args: Vec<CmdArgType> = args
                .iter()
                .map(|a| {
                    let ty = lower_type_expr(&a.ty, tyvars);
                    if a.opt.is_some() {
                        optional(ty)
                    } else {
                        mandatory(ty)
                    }
                })
                .collect();
            match kind {
                CmdTypeKind::Inline(_) => MonoType::InlineCmd(cmd_args),
                CmdTypeKind::Block(_) => MonoType::BlockCmd(cmd_args),
                CmdTypeKind::Math(_) => MonoType::MathCmd(cmd_args),
            }
        }
        TypeAtom::Paren { inner, .. } => lower_type_expr(inner, tyvars),
        // `(| l1 : ty1; l2 : ty2; … |)` — a CLOSED record type: fold the
        // fields into a `Row::Cons` chain (in source order) ending in
        // `Row::Empty`, matching `MonoType::Record`'s row representation
        // (`types.rs`'s module doc comment) — distinct from `RecordKind`'s
        // label-only `Kind::Record` bound (`lower_record_kind`, below),
        // which drops field types entirely; a type-position record keeps
        // them, since it's a concrete type, not a lower-bound obligation.
        TypeAtom::Record { fields, .. } => {
            let row = fields.iter().rev().fold(Row::Empty, |rest, f| {
                Row::Cons(f.name.name.clone(), Box::new(lower_type_expr(&f.ty, tyvars)), Box::new(rest))
            });
            MonoType::Record(row)
        }
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

/// `txprod`: a [`TypeProd`] is either a single [`TypeApp`] (returned as-is)
/// or a genuine `*`-separated product (`MonoType::Product`, always 2+ items
/// by construction — see [`prim_types::product`]).
fn lower_type_prod(prod: &TypeProd, tyvars: &HashMap<String, MonoType>) -> MonoType {
    if prod.rest.is_empty() {
        lower_type_app(&prod.first, tyvars)
    } else {
        let mut items = Vec::with_capacity(1 + prod.rest.len());
        items.push(lower_type_app(&prod.first, tyvars));
        for st in &prod.rest {
            items.push(lower_type_app(&st.ty, tyvars));
        }
        product(items)
    }
}

/// `txapppre`/`txapp` (restricted to a single argument — see [`TypeApp`]'s
/// doc comment): either a bare atom, or one atom applied to a single postfix
/// type-constructor name (`'a option`, `('a list) list`).
fn lower_type_app(app: &TypeApp, tyvars: &HashMap<String, MonoType>) -> MonoType {
    match app {
        TypeApp::Atom(atom) => lower_type_atom(atom, tyvars),
        TypeApp::Applied { arg, ctor } => {
            let arg_ty = lower_type_atom(arg, tyvars);
            match ctor.name.as_str() {
                "list" => list(arg_ty),
                "ref" => reff(arg_ty),
                other => MonoType::Variant(other.to_string(), vec![arg_ty]),
            }
        }
    }
}

/// `dom -> cod`, with `?->`'s optional-argument prefix (`opts`) folded in as
/// leading `option`-wrapped mandatory domains — the Slice-1 stand-in
/// `docs/plans/class-signature-lang-gaps.md`'s R2 calls out: `config ?->
/// block-text -> document` lowers to `Func(option(config), Func(block-text,
/// document))`, exactly the shape `frontend-completion.md` Sub-area 2's
/// call-site model produces (`Some`/`None` applied to a plain, `option`-typed
/// domain — see `elaborate.rs`'s `app_arg_to_ast`) — the "one consistent
/// optional-arg model" the two plans share. Not upstream's real
/// `option_row`/arity-changing encoding (that's the full-roadmap R2 item);
/// this only needs the two encodings to *unify*, which a plain `option`
/// domain already does.
fn lower_type_expr(ty: &TypeExpr, tyvars: &HashMap<String, MonoType>) -> MonoType {
    match ty {
        TypeExpr::Fun {
            opts, dom, cod, ..
        } => {
            let result = arrow(lower_type_prod(dom, tyvars), lower_type_expr(cod, tyvars));
            opts.iter().rev().fold(result, |acc, opt| {
                arrow(t_option(lower_type_prod(&opt.ty, tyvars)), acc)
            })
        }
        TypeExpr::Atom(prod) => lower_type_prod(prod, tyvars),
    }
}

// ============================================================================
// Signature items (`docs/plans/class-signature-lang-gaps.md` gap 3): the
// `constraint 'a :: (| l1; l2; … |)` per-item suffix.
// ============================================================================

/// Every distinct type-variable name occurring in `ty`, in first-occurrence
/// order. Unlike a `type` declaration, a [`SigItem`] has no upfront `tyvars`
/// list (`val document : 'a -> …` names `'a` inline, mid-type), so building
/// its lowering's `tyvars` map requires walking the type first — every
/// occurrence of the same name must resolve to the *same* fresh variable,
/// which is also the one a matching `constraint` suffix (naming that same
/// `'a`) attaches its `Kind::Record` bound to.
///
/// `#[allow(dead_code)]`: only exercised by this module's own tests today —
/// no sig-enforcement pass calls [`lower_sig_item`] yet
/// (`typechecker-completion.md` §3, still roadmap).
#[allow(dead_code)]
fn collect_type_vars(ty: &TypeExpr, out: &mut Vec<String>) {
    fn push(name: &str, out: &mut Vec<String>) {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    fn walk_atom(atom: &TypeAtom, out: &mut Vec<String>) {
        match atom {
            TypeAtom::Cmd { args, .. } => {
                for a in args {
                    walk_expr(&a.ty, out);
                }
            }
            TypeAtom::Paren { inner, .. } => walk_expr(inner, out),
            TypeAtom::Record { fields, .. } => {
                for f in fields {
                    walk_expr(&f.ty, out);
                }
            }
            TypeAtom::Var(tv) => push(&tv.name, out),
            TypeAtom::Name(_) => {}
        }
    }
    fn walk_app(app: &TypeApp, out: &mut Vec<String>) {
        match app {
            TypeApp::Atom(atom) => walk_atom(atom, out),
            TypeApp::Applied { arg, .. } => walk_atom(arg, out),
        }
    }
    fn walk_prod(prod: &TypeProd, out: &mut Vec<String>) {
        walk_app(&prod.first, out);
        for st in &prod.rest {
            walk_app(&st.ty, out);
        }
    }
    fn walk_expr(ty: &TypeExpr, out: &mut Vec<String>) {
        match ty {
            TypeExpr::Fun { opts, dom, cod, .. } => {
                for opt in opts {
                    walk_prod(&opt.ty, out);
                }
                walk_prod(dom, out);
                walk_expr(cod, out);
            }
            TypeExpr::Atom(prod) => walk_prod(prod, out),
        }
    }
    walk_expr(ty, out);
}

/// Lower a [`RecordKind`]'s field list to its label set, dropping field
/// *types* — `Kind::Record` (`types.rs`) stores labels only, so
/// `constraint 'a :: (| title : inline-text; … |)` checks label
/// *presence*, not the field's declared type. A documented Slice-1
/// limitation (`class-signature-lang-gaps.md` R3), not a grammar gap: the
/// impl's row still gets its own field types from ordinary usage, so this
/// is unlikely to admit a wrong program in practice.
#[allow(dead_code)]
fn lower_record_kind(rk: &RecordKind) -> BTreeSet<String> {
    rk.fields.iter().map(|f| f.name.name.clone()).collect()
}

/// Lower one value/direct [`SigItem`] to its name and [`MonoType`],
/// attaching each `constraint 'a :: (| … |)` suffix as a `Kind::Record`
/// bound on `'a`'s freshly-minted variable — "this variable must be a
/// record containing at least these labels", built on the existing
/// Rémy-style row machinery. Returns `None` for a bare `SigItem::Type`
/// item (a type *name* declaration, not a value with a `MonoType` of its
/// own).
///
/// **The obligation check itself rides on existing code, for free.** Once
/// this variable is ever unified against a concrete `MonoType::Record`
/// (which is exactly what enforcing the signature against a real `struct`
/// implementation — `typechecker-completion.md` §3, not yet built — would
/// do), `unify::bind_var`'s `Kind::Record` branch already rejects a row
/// missing any declared label via `row_require_label`. Slice 1 only wires
/// the constraint *into* that existing machinery; no sig-enforcement pass
/// exists yet to *drive* the unification (see this module's
/// `sig_constraint_tests` for a direct demonstration against `unify`
/// itself).
#[allow(dead_code)]
pub(crate) fn lower_sig_item(item: &SigItem, ctx: &mut TypeContext) -> Option<(String, MonoType)> {
    let (name, ty, constraints): (&str, &TypeExpr, &[SigConstraint]) = match item {
        SigItem::ValHorzCmd { name, ty, constraints, .. } => (&name.name, ty, constraints),
        SigItem::ValVertCmd { name, ty, constraints, .. } => (&name.name, ty, constraints),
        SigItem::Val { name, ty, constraints, .. } => (&name.name, ty, constraints),
        SigItem::DirectHorzCmd { name, ty, constraints, .. } => (&name.name, ty, constraints),
        SigItem::DirectVertCmd { name, ty, constraints, .. } => (&name.name, ty, constraints),
        SigItem::Type { .. } => return None,
    };
    let mut names = Vec::new();
    collect_type_vars(ty, &mut names);
    let mut tyvars = HashMap::new();
    for n in names {
        let found = constraints.iter().find(|c| c.tyvar.name == n);
        let v = match found {
            Some(c) => ctx.fresh_var_with_kind(Kind::Record(lower_record_kind(&c.kind))),
            None => ctx.fresh_var_with_kind(Kind::Universal),
        };
        tyvars.insert(n, MonoType::Var(v));
    }
    Some((name.to_string(), lower_type_expr(ty, &tyvars)))
}

/// Lower one [`UserTypeDecl`] (surfaced by `elaborate::elaborate_program`)
/// into a [`VariantDecl`], the same shape `prim_types::builtin_variants`
/// produces for `option`/`itemize` — see that struct's doc comment for how
/// `param_vars` and `instantiate_ctor` fit together. Each ctor's payload is
/// passed through [`expand_synonyms`] so a payload that names a synonym
/// (`type wrap = | W of point`) is stored already-transparent — `unify`
/// never has to know synonyms exist.
fn build_variant_decl(
    decl: &UserTypeDecl,
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<VariantDecl, TypeError> {
    let param_vars: Vec<types::TyVarRef> =
        decl.params.iter().map(|_| types::new_ty_var(0)).collect();
    let tyvar_map: HashMap<String, MonoType> = decl
        .params
        .iter()
        .cloned()
        .zip(param_vars.iter().cloned().map(MonoType::Var))
        .collect();
    let mut ctors = Vec::with_capacity(decl.ctors.len());
    for (name, ty) in &decl.ctors {
        let payload = match ty {
            None => None,
            Some(t) => Some(expand_synonyms(&lower_type_expr(t, &tyvar_map), synonyms)?),
        };
        ctors.push((name.clone(), payload));
    }
    Ok(VariantDecl {
        name: decl.name.clone(),
        params: decl.params.len(),
        ctors,
        param_vars,
    })
}

// ============================================================================
// Type synonyms (`type point = length * length`): registration, plus the
// transparent expansion that keeps a synonym's name from ever reaching
// `unify` — mirrors upstream's `SynonymType`/`add_synonym`
// (`typechecker.ml`), just against this port's `MonoType`/`substitute`
// machinery instead of a substitution-on-the-fly unifier case.
// ============================================================================

/// A user type-synonym declaration, lowered from [`UserSynonymDecl`] — the
/// transparent-expansion counterpart of [`VariantDecl`] (`prim_types.rs`).
/// Unlike a variant, a synonym never gets a runtime tag or a
/// `Checker::ctors` entry; the only thing that ever looks at one is
/// [`expand_synonyms`].
struct SynonymDecl {
    /// The declaration's own type-parameter placeholders — the same
    /// technique as `VariantDecl::param_vars` (matched by pointer identity
    /// via `types::substitute`). Realistically always empty: this grammar
    /// has no applied-type-constructor syntax to *reference* a synonym with
    /// arguments (`TypeAtom`'s doc comment), so a real reference site always
    /// supplies zero args — kept general anyway, so a nonzero-param synonym
    /// fails with a clear arity error rather than silently misbehaving if
    /// that ever changes.
    param_vars: Vec<types::TyVarRef>,
    /// The synonym's body, lowered exactly once via `lower_type_expr`. Any
    /// *other* synonym name it mentions is still an opaque
    /// `MonoType::Variant(name, [])` at this point (`lower_type_expr` has no
    /// notion of the synonym table) — [`expand_synonyms`] resolves those
    /// lazily, on demand, at each reference site.
    body: MonoType,
}

fn build_synonym_decl(decl: &UserSynonymDecl) -> SynonymDecl {
    let param_vars: Vec<types::TyVarRef> =
        decl.params.iter().map(|_| types::new_ty_var(0)).collect();
    let tyvar_map: HashMap<String, MonoType> = decl
        .params
        .iter()
        .cloned()
        .zip(param_vars.iter().cloned().map(MonoType::Var))
        .collect();
    SynonymDecl {
        param_vars,
        body: lower_type_expr(&decl.body, &tyvar_map),
    }
}

/// Collect the name of every *synonym* (i.e. present in `synonyms`) directly
/// mentioned inside `ty`, ignoring argument count — used only to build the
/// "synonym references synonym" graph for [`check_synonym_cycles`], where
/// arity is irrelevant (a cycle is a cycle no matter how many arguments each
/// step is nominally applied to).
fn synonym_refs(ty: &MonoType, synonyms: &HashMap<String, SynonymDecl>, out: &mut Vec<String>) {
    match ty {
        MonoType::Var(_) | MonoType::Base(_) => {}
        MonoType::Func(dom, cod) => {
            synonym_refs(dom, synonyms, out);
            synonym_refs(cod, synonyms, out);
        }
        MonoType::Product(ts) => ts.iter().for_each(|t| synonym_refs(t, synonyms, out)),
        MonoType::List(t) | MonoType::Ref(t) => synonym_refs(t, synonyms, out),
        MonoType::Record(row) => synonym_refs_row(row, synonyms, out),
        MonoType::Variant(name, args) => {
            if synonyms.contains_key(name) {
                out.push(name.clone());
            }
            args.iter().for_each(|t| synonym_refs(t, synonyms, out));
        }
        MonoType::InlineCmd(cs) | MonoType::BlockCmd(cs) | MonoType::MathCmd(cs) => {
            cs.iter().for_each(|c| synonym_refs(&c.ty, synonyms, out));
        }
    }
}

fn synonym_refs_row(row: &Row, synonyms: &HashMap<String, SynonymDecl>, out: &mut Vec<String>) {
    match row {
        Row::Empty | Row::Var(_) => {}
        Row::Cons(_, t, rest) => {
            synonym_refs(t, synonyms, out);
            synonym_refs_row(rest, synonyms, out);
        }
    }
}

/// Reject a cyclic synonym (`type a = b` / `type b = a`, or a directly
/// self-referential `type a = a * a`) with a clear error instead of letting
/// [`expand_synonyms`] recurse forever. Run unconditionally over every
/// registered synonym at [`Checker::new`] time, so a cyclic pair is caught
/// even if nothing in the program actually references it.
fn check_synonym_cycles(synonyms: &HashMap<String, SynonymDecl>) -> Result<(), TypeError> {
    for start in synonyms.keys() {
        let mut stack = vec![start.clone()];
        check_synonym_cycles_from(start, synonyms, &mut stack)?;
    }
    Ok(())
}

fn check_synonym_cycles_from(
    name: &str,
    synonyms: &HashMap<String, SynonymDecl>,
    stack: &mut Vec<String>,
) -> Result<(), TypeError> {
    let mut refs = Vec::new();
    synonym_refs(&synonyms[name].body, synonyms, &mut refs);
    for r in refs {
        if stack.contains(&r) {
            let mut cycle = stack.clone();
            cycle.push(r);
            return Err(TypeError::simple(
                None,
                format!("cyclic type synonym: {}", cycle.join(" -> ")),
            ));
        }
        stack.push(r.clone());
        check_synonym_cycles_from(&r, synonyms, stack)?;
        stack.pop();
    }
    Ok(())
}

/// Recursively and transparently expand every synonym reference inside
/// `ty`, so `unify` never sees a synonym's name — only real base/product/
/// function/variant types. `synonyms` was already validated acyclic by
/// [`check_synonym_cycles`] (at `Checker::new` time), so the recursion here
/// is guaranteed to terminate; arity (a reference's argument count against
/// the synonym's own parameter count) is checked here, the one place that
/// actually has concrete arguments to check it against.
fn expand_synonyms(
    ty: &MonoType,
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<MonoType, TypeError> {
    match ty {
        MonoType::Var(_) | MonoType::Base(_) => Ok(ty.clone()),
        MonoType::Func(dom, cod) => Ok(MonoType::Func(
            Box::new(expand_synonyms(dom, synonyms)?),
            Box::new(expand_synonyms(cod, synonyms)?),
        )),
        MonoType::Product(ts) => Ok(MonoType::Product(
            ts.iter()
                .map(|t| expand_synonyms(t, synonyms))
                .collect::<Result<_, _>>()?,
        )),
        MonoType::List(t) => Ok(MonoType::List(Box::new(expand_synonyms(t, synonyms)?))),
        MonoType::Ref(t) => Ok(MonoType::Ref(Box::new(expand_synonyms(t, synonyms)?))),
        MonoType::Record(row) => Ok(MonoType::Record(expand_synonyms_row(row, synonyms)?)),
        MonoType::Variant(name, args) => {
            let args: Vec<MonoType> = args
                .iter()
                .map(|t| expand_synonyms(t, synonyms))
                .collect::<Result<_, _>>()?;
            let Some(syn) = synonyms.get(name) else {
                return Ok(MonoType::Variant(name.clone(), args));
            };
            if args.len() != syn.param_vars.len() {
                return Err(TypeError::simple(
                    None,
                    format!(
                        "type synonym '{name}' expects {} argument{}, got {}",
                        syn.param_vars.len(),
                        if syn.param_vars.len() == 1 { "" } else { "s" },
                        args.len()
                    ),
                ));
            }
            let mut var_map: HashMap<usize, MonoType> = HashMap::new();
            for (pv, arg) in syn.param_vars.iter().zip(args.iter()) {
                var_map.insert(types::ptr_key(pv), arg.clone());
            }
            let substituted = types::substitute(&syn.body, &var_map, &HashMap::new());
            expand_synonyms(&substituted, synonyms)
        }
        MonoType::InlineCmd(cs) => Ok(MonoType::InlineCmd(expand_synonyms_cmd_args(cs, synonyms)?)),
        MonoType::BlockCmd(cs) => Ok(MonoType::BlockCmd(expand_synonyms_cmd_args(cs, synonyms)?)),
        MonoType::MathCmd(cs) => Ok(MonoType::MathCmd(expand_synonyms_cmd_args(cs, synonyms)?)),
    }
}

fn expand_synonyms_row(
    row: &Row,
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<Row, TypeError> {
    match row {
        Row::Empty => Ok(Row::Empty),
        Row::Var(v) => Ok(Row::Var(v.clone())),
        Row::Cons(label, t, rest) => Ok(Row::Cons(
            label.clone(),
            Box::new(expand_synonyms(t, synonyms)?),
            Box::new(expand_synonyms_row(rest, synonyms)?),
        )),
    }
}

fn expand_synonyms_cmd_args(
    cs: &[CmdArgType],
    synonyms: &HashMap<String, SynonymDecl>,
) -> Result<Vec<CmdArgType>, TypeError> {
    cs.iter()
        .map(|c| {
            Ok(CmdArgType {
                optional: c.optional,
                ty: expand_synonyms(&c.ty, synonyms)?,
            })
        })
        .collect()
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
    /// The same declarations, keyed by *type* name instead — needed by the
    /// exhaustiveness pass (`exhaustive::check_match`) to enumerate a
    /// variant's full constructor set given the scrutinee's resolved
    /// `MonoType::Variant(name, _)`.
    variants: HashMap<String, Rc<VariantDecl>>,
    /// Non-fatal diagnostics accumulated by the exhaustiveness/redundancy
    /// pass (see `typecheck_verbose`); v0.0.6's `exhchecker.ml` warns and
    /// continues rather than rejecting the program.
    warnings: Vec<MatchWarning>,
}

impl Checker {
    fn new(program: &Program) -> Result<Checker, TypeError> {
        // Synonyms are registered (and checked for cycles) before any
        // variant decl is lowered, since a variant's ctor payload may name a
        // synonym (`build_variant_decl` expands through `synonyms`).
        let mut synonyms: HashMap<String, SynonymDecl> = HashMap::new();
        for usd in &program.synonym_decls {
            synonyms.insert(usd.name.clone(), build_synonym_decl(usd));
        }
        check_synonym_cycles(&synonyms)?;

        let mut ctors = HashMap::new();
        let mut variants = HashMap::new();
        for decl in builtin_variants() {
            let decl = Rc::new(decl);
            variants.insert(decl.name.clone(), decl.clone());
            for (cname, _) in &decl.ctors {
                ctors.insert(cname.clone(), decl.clone());
            }
        }
        for utd in &program.type_decls {
            let decl = Rc::new(build_variant_decl(utd, &synonyms)?);
            variants.insert(decl.name.clone(), decl.clone());
            for (cname, _) in &decl.ctors {
                ctors.insert(cname.clone(), decl.clone());
            }
        }
        Ok(Checker {
            ctx: TypeContext::new(),
            ctors,
            variants,
            warnings: Vec::new(),
        })
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
            // A qualified-name alias (`M.\cmd` re-export, or an `open`) of a
            // GENUINE `let-math` binding: math commands share the `\` sigil
            // with inline commands (`docs/plans/math-engine.md` §G — there
            // is no separate math-command token), so an alias site only
            // ever reaches this generic `Ast::LetIn` path (never
            // `Ast::LetMathIn`, which is produced only at a math command's
            // OWN definition site — see that variant's doc comment). Pass a
            // already-`MathCmd`-typed alias through unchanged, exactly like
            // the `InlineCmd`/`BlockCmd` arms above do for their own kind.
            MonoType::MathCmd(_) if is_inline => {
                return Ok(generalize(self.ctx.level(), &tv));
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
        // Optional command params, this milestone's simplification
        // (`docs/plans/frontend-completion.md` Sub-area 2 / `command_scheme`'s
        // doc comment): there is no def-site `?:param` marker (this grammar
        // has none), so a param is treated as optional exactly when its
        // *inferred* domain resolves to `_ option` — i.e. the body actually
        // uses it as an `option` (`match p with Some .. | None -> ..`, etc.).
        // `CmdArgType.ty` then stores the option's INNER type (peeled), so it
        // matches the `[ty?; ..]` signature-lowering shape 1:1
        // (`lower_type_atom`'s `TypeAtom::Cmd` arm) — `check_cmd_args` re-wraps
        // it in `option(..)` per call, since call-site args always arrive
        // pre-wrapped as `Some`/`None` (`elaborate.rs`'s `app_arg_to_ast`).
        let params: Vec<CmdArgType> = doms
            .into_iter()
            .map(|d| match resolve(&d) {
                MonoType::Variant(vname, mut vargs) if vname == "option" && vargs.len() == 1 => {
                    optional(vargs.pop().unwrap())
                }
                _ => mandatory(d),
            })
            .collect();
        let cmd_ty = if is_inline {
            MonoType::InlineCmd(params)
        } else {
            MonoType::BlockCmd(params)
        };
        Ok(generalize(self.ctx.level(), &cmd_ty))
    }

    /// `Ast::LetMathIn`'s scheme-building rule (`docs/plans/math-engine.md`
    /// §G) — the math-command analog of `command_scheme` above, but
    /// simpler: a math command has **no** implicit context argument (see
    /// `elaborate.rs`'s `elaborate_let_math`), so every domain of `tv`'s
    /// function-chain becomes a `CmdArgType` (the same optional-param
    /// heuristic as `command_scheme`), and the bare result — not a peeled
    /// first argument — must be `math`. A zero-arity binding (`tv` not a
    /// `Func` at all, e.g. `let-math \to = rel \`→\``) falls out naturally:
    /// `peel_func_chain` returns no domains and `tv` itself as the result.
    fn math_command_scheme(
        &mut self,
        name: &str,
        tv: MonoType,
        span: Option<Span>,
    ) -> Result<PolyType, TypeError> {
        let (doms, result) = peel_func_chain(tv);
        self.unify_ctx(
            &t_math_text(),
            &result,
            span,
            &format!("the result of math command '{name}'"),
        )?;
        let params: Vec<CmdArgType> = doms
            .into_iter()
            .map(|d| match resolve(&d) {
                MonoType::Variant(vname, mut vargs) if vname == "option" && vargs.len() == 1 => {
                    optional(vargs.pop().unwrap())
                }
                _ => mandatory(d),
            })
            .collect();
        Ok(generalize(self.ctx.level(), &MonoType::MathCmd(params)))
    }

    /// Shared by `check_itext`'s `IText::Cmd` and `check_btext`'s
    /// `BText::Cmd`: check a command application's argument count (exact —
    /// every optional param must carry an explicit `?:`/`?*` marker at the
    /// call site, so its slot is never actually *absent* from `args`; see
    /// `elaborate.rs`'s `cmd_args`) and each argument's type against `params`
    /// (already resolved to a concrete `MonoType::InlineCmd`/`BlockCmd`'s
    /// payload by the caller). An `optional` param's `args[i]` is always a
    /// `Some(..)`/`None` value (`app_arg_to_ast`'s desugaring), so it's
    /// checked against `option(param.ty)`, not `param.ty` directly.
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
            let expected = if param.optional {
                t_option(param.ty.clone())
            } else {
                param.ty.clone()
            };
            self.unify_ctx(
                &expected,
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

            // `let-math \cmd param* = expr in body` (`docs/plans/math-
            // engine.md` §G) — structurally identical to the `Ast::LetIn`
            // command-binding rule above, but for a binding that is ALREADY
            // known (by construction, via the dedicated Ast variant — see
            // its doc comment) to be a math command, so there is no sigil
            // to dispatch on and no "which kind of `\`-binding is this"
            // ambiguity to resolve.
            Ast::LetMathIn(name, value, body) => {
                self.ctx.enter_level();
                let tv = self.infer(env, value)?;
                self.ctx.leave_level();
                let scheme = self.math_command_scheme(name, tv, ast_span(value))?;
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
                // Exhaustiveness/redundancy (typechecker-completion plan,
                // §Slice 1): non-fatal, so it runs only after every arm has
                // typechecked, against `tscrut` as resolved as inference will
                // ever make it. See `exhaustive::check_match`'s doc comment.
                let resolved_scrut = resolve(&tscrut);
                let new_warnings = crate::exhaustive::check_match(
                    &resolved_scrut,
                    ast_span(scrutinee),
                    arms,
                    &self.variants,
                );
                self.warnings.extend(new_warnings);
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

    /// Walk one quoted math element. `Chars`/`Group`/`Sub`/`Sup`/`Primes`
    /// carry no program-mode content of their own (nothing to check beyond
    /// recursing). `Cmd`/`Embed` are where math meets the ordinary
    /// expression language (`docs/plans/math-engine.md` §G): a `Cmd`'s
    /// `name` must resolve to a genuine `MathCmd` type (checked exactly
    /// like `check_itext`'s `IText::Cmd`, via `check_cmd_args` — math
    /// commands never carry an optional `?:` argument, but `check_cmd_args`
    /// handles that generically anyway), and an `Embed`'s (`#expr`) type
    /// must unify with `math` (a math command parameter, or another
    /// program-mode value that itself produces math — `Value::Math`/
    /// `Value::MathText` are the two runtime shapes this unifies against,
    /// see `value.rs`).
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
            MathElem::Cmd { name, span, args } => {
                let tcmd = match env.get(name) {
                    Some(poly) => instantiate(poly, self.ctx.level()),
                    None => {
                        return Err(TypeError::simple(
                            Some(*span),
                            format!(
                                "internal error: unbound math command '{name}' reached the typechecker"
                            ),
                        ))
                    }
                };
                match resolve(&tcmd) {
                    MonoType::MathCmd(params) => {
                        self.check_cmd_args(env, name, *span, &params, args)
                    }
                    other => Err(TypeError::simple(
                        Some(*span),
                        format!(
                            "internal error: math command '{name}' does not have a \
                             math-cmd type (found `{other}`)"
                        ),
                    )),
                }
            }
            MathElem::Embed { expr, span } => {
                let te = self.infer(env, expr)?;
                self.unify_ctx(&t_math_text(), &te, Some(*span), "a math '#…' embed")?;
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
///
/// **Must also check the second character.** A genuine command name's
/// sigil is always immediately followed by an identifier (`+p`, `\emph`,
/// `lexer.rs`'s `name_len_at`-gated `+`/`\` branches never emit `VertCmd`/
/// `HorzCmd` otherwise) — but a parenthesized-operator NAME (`cst.rs`'s
/// `BindName`) can bind a string that merely happens to *start* with the
/// same sigil character, e.g. `let (+++>) = ..` (`itemize.satyh`) or `let
/// (+.) = ..`. Requiring an alphabetic second character mirrors the
/// lexer's own split and keeps such an operator name an ordinary variable
/// binding rather than a false-positive command.
fn command_sigil(name: &str) -> Option<char> {
    let local = name.rsplit('.').next().unwrap_or(name);
    let mut chars = local.chars();
    match chars.next() {
        Some(c @ ('\\' | '+')) if chars.next().is_some_and(|c2| c2.is_ascii_alphabetic()) => {
            Some(c)
        }
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
pub(crate) fn ast_span(ast: &Ast) -> Option<Span> {
    match ast {
        Ast::Var(_, span) => Some(*span),
        Ast::Overwrite(_, span, _) => Some(*span),
        Ast::AccessField(_, _, span) => Some(*span),
        _ => None,
    }
}

/// Type-check a whole elaborated [`Program`], additionally returning every
/// non-fatal [`MatchWarning`] the exhaustiveness/redundancy pass collected
/// (typechecker-completion plan, §Slice 1) — v0.0.6's `exhchecker.ml` warns
/// on a non-exhaustive or redundant `match` rather than rejecting the
/// program, so these never turn a would-have-passed program into a
/// `TypeError`.
pub fn typecheck_verbose(program: &Program) -> Result<Vec<MatchWarning>, TypeError> {
    let mut checker = Checker::new(program)?;
    let env = base_type_env();
    checker.infer(&env, &program.body)?;
    Ok(checker.warnings)
}

/// Type-check a whole elaborated [`Program`]. Validation only: on success
/// the caller proceeds to evaluate `program.body` exactly as before (the
/// evaluator is untouched by this phase). A thin wrapper over
/// [`typecheck_verbose`] that discards its warnings — every existing caller
/// (`lib.rs`'s `compile_document_cst`, `compile.rs`, and every test that
/// predates §Slice 1) is therefore unaffected by the new pass.
pub fn typecheck(program: &Program) -> Result<(), TypeError> {
    typecheck_verbose(program).map(|_warnings| ())
}

// ============================================================================
// `docs/plans/class-signature-lang-gaps.md` Slice 1 acceptance: the real
// `stdja.satyh` `sig … end` block (gaps 1/3 — command values are covered by
// `crates/satysfi-lang/tests/typecheck.rs`'s end-to-end fixtures; this module
// covers the `SigItem`/`constraint` lowering directly, since `lower_sig_item`
// is a crate-private entry point no sig-enforcement pass calls yet).
// ============================================================================
#[cfg(test)]
mod sig_constraint_tests {
    use super::*;
    use satysfi_syntax::cst::{SigAnnot, TopBinding};

    fn parse_module_sig(src: &str) -> SigAnnot {
        let file = satysfi_syntax::parse_file(src).expect("parse failed");
        for b in &file.prelude {
            if let TopBinding::Module { sig: Some(sig), .. } = b {
                return sig.clone();
            }
        }
        panic!("no `module .. : sig .. end` found in {src:?}");
    }

    #[test]
    fn constraint_suffix_lowers_to_a_kind_record_bound_on_its_tyvar() {
        let sig = parse_module_sig(
            "module M : sig\n\
             val document : 'a -> config ?-> block-text -> document\n\
             constraint 'a :: (| title : inline-text; author : inline-text |)\n\
             end = struct\n\
             let document x c bt = bt\n\
             end",
        );
        let mut ctx = TypeContext::new();
        let mut saw_record_kind = false;
        for item in &sig.items {
            let (name, ty) = lower_sig_item(item, &mut ctx).expect("a value item");
            assert_eq!(name, "document");
            // Walk the lowered `Func` chain: `'a`'s fresh variable is the
            // very first domain (`Func(Var('a), Func(option(config),
            // Func(block-text, document)))` — see `lower_type_expr`'s doc
            // comment for the `?->` shape).
            if let MonoType::Func(dom, _) = &ty {
                if let MonoType::Var(v) = &**dom {
                    if let Kind::Record(labels) = v.kind() {
                        saw_record_kind = true;
                        let expected: BTreeSet<String> =
                            ["title", "author"].iter().map(|s| s.to_string()).collect();
                        assert_eq!(labels, expected);
                    }
                }
            }
        }
        assert!(
            saw_record_kind,
            "expected 'a's fresh variable to carry a Kind::Record bound"
        );
    }

    #[test]
    fn kind_record_bound_accepts_a_row_with_every_required_label() {
        // Direct demonstration that the constraint's lowered `Kind::Record`
        // bound rides on *existing* `unify`/`bind_var` machinery for free —
        // no sig-enforcement pass exists yet to drive this against a real
        // `struct` implementation (`class-signature-lang-gaps.md` R3), but
        // the positive-presence check itself already works once something
        // does.
        let mut ctx = TypeContext::new();
        let labels: BTreeSet<String> =
            ["title", "author"].iter().map(|s| s.to_string()).collect();
        let v = ctx.fresh_var_with_kind(Kind::Record(labels));
        let constrained = MonoType::Var(v);
        let full = MonoType::Record(Row::Cons(
            "title".to_string(),
            Box::new(t_inline_text()),
            Box::new(Row::Cons(
                "author".to_string(),
                Box::new(t_inline_text()),
                Box::new(Row::Empty),
            )),
        ));
        unify(&constrained, &full).expect("row has both required labels");
    }

    #[test]
    fn kind_record_bound_rejects_a_row_missing_a_required_label() {
        let mut ctx = TypeContext::new();
        let labels: BTreeSet<String> =
            ["title", "author"].iter().map(|s| s.to_string()).collect();
        let v = ctx.fresh_var_with_kind(Kind::Record(labels));
        let constrained = MonoType::Var(v);
        let missing_author = MonoType::Record(Row::Cons(
            "title".to_string(),
            Box::new(t_inline_text()),
            Box::new(Row::Empty),
        ));
        let err = unify(&constrained, &missing_author)
            .expect_err("row is missing the required 'author' label");
        assert!(format!("{err:?}").contains("author"), "error should name the missing label: {err:?}");
    }

    #[test]
    fn real_stdja_sig_block_lowers_every_item_to_a_monotype() {
        // Mirrors the whole `sig … end` block of the real upstream
        // `stdja.satyh:24-51` (v0.0.6 checkout) — command values, command
        // types, `?->`, and the `constraint` suffix all together. Proves
        // Slice 1's acceptance gate: every item parses and lowers without
        // error (an empty `struct end` body is enough — sig enforcement
        // against a real implementation is `typechecker-completion.md`
        // §3's job, not Slice 1's).
        let sig = parse_module_sig(
            "module StdJa : sig\n\
             val default-config : config\n\
             val document : 'a -> config ?-> block-text -> document\n\
             constraint 'a :: (|\n\
             title : inline-text;\n\
             author : inline-text;\n\
             show-toc : bool;\n\
             show-title : bool;\n\
             |)\n\
             val font-latin-roman : string * float * float\n\
             direct \\ref : [string] inline-cmd\n\
             direct \\ref-page : [string] inline-cmd\n\
             direct \\figure : [inline-text; block-text] inline-cmd\n\
             direct +p : [inline-text] block-cmd\n\
             direct +pn : [inline-text] block-cmd\n\
             direct +section : [string?; string?; inline-text; block-text] block-cmd\n\
             direct +subsection : [string?; string?; inline-text; block-text] block-cmd\n\
             direct \\emph : [inline-text] inline-cmd\n\
             end = struct\n\
             end",
        );
        let mut ctx = TypeContext::new();
        let mut names = Vec::new();
        for item in &sig.items {
            let (name, _ty) = lower_sig_item(item, &mut ctx).expect("a value item");
            names.push(name);
        }
        assert_eq!(
            names,
            vec![
                "default-config",
                "document",
                "font-latin-roman",
                "\\ref",
                "\\ref-page",
                "\\figure",
                "+p",
                "+pn",
                "+section",
                "+subsection",
                "\\emph",
            ]
        );
    }
}
