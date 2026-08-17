//! Slice X3a (`docs/plans/design-cross-version-import.md`, "Slice X3 —
//! forked-type export adapter (detailed design)", specifically the X3a
//! sub-slice, X3.1-X3.3): the boundary TYPE adapter for a `V0_0_6`
//! dependency spliced into a `V0_1` program.
//!
//! X2a already narrows the *value* half of X1's forked-name guard away (a
//! spliced binding's RHS runs inside `Ast::VersionScope(V0_0_6, _)`, so an
//! internally-used version-forked *primitive* resolves against its own
//! version). The *type* half stays conservative: `lib.rs`'s
//! `compile_document_v1_with_trials` still hard-rejects a dependency that
//! textually names ANY version-forked type (`typecheck::forked_type_names`)
//! anywhere in its `prelude`. X3a turns exactly ONE of those rejections into
//! an acceptance: `math` (0.0.6's math-text type) is representationally
//! IDENTICAL to 0.1's `math-text` — both lower to the same
//! `BaseType::MathText` (`typecheck.rs`'s `name_to_mono`), and both share the
//! same runtime `Value::MathText`/`Value::Math` representation
//! (`value.rs:39-56`, NOT forked between versions at all). So a `math`
//! reference can be safely RELABELED to `math-text` (the name a `V0_1`-typed
//! whole-program inference actually recognizes) with **zero runtime value
//! coercion** — the evaluated value crosses the boundary untouched. Every
//! OTHER forked type name (`math-text`/`math-boxes`/`pre-path`/`path`/
//! `graphics`/`image`/`deco`/`deco-set`/`font`/`paren`), plus `page` (X3.1's
//! note: `page`'s bare NAME lowers to the same nominal `Variant("page",[])`
//! under both versions, so it never appears in `forked_type_names()`'s
//! automatic name_to_mono diff — but its VALUE representation still forks
//! (0.0.6: a 9-ctor ADT, `Value::Ctor`; 0.1: a `length*length` tuple,
//! `Value::Product`), so it is added to the reject set here explicitly),
//! stays rejected — X3.8/S1's soundness backbone: no code path may ever hand
//! a `Value` of one shape to a site expecting another.
//!
//! **Where the type text that actually matters lives.** This port's 0.0.6
//! CST (`satysfi_syntax::cst`) has NO type-ascription syntax on an ordinary
//! `let`/`let-inline`/`let-block`/`let-math` binding at all (`cst::TopLet`
//! etc. carry no `: ty` field) — a binding's type is 100% HM-inferred. Two
//! surface forms DO carry a `cst::ast::TypeExpr`:
//! `cst::ast::RecBinding::ascription` (a `let-rec`'s optional `: ty`) and
//! `cst::SigAnnot`'s `SigItem::Val*`/`DirectHorzCmd`/`DirectVertCmd` items (a
//! `module .. : sig .. end`'s declared types) — but BOTH are parsed and then
//! **entirely ignored** by `elaborate.rs` ("Parsed but not enforced" /
//! "accepted and ignored" — see that module's `TopBinding::Module`/
//! `RecBinding` handling): neither ever reaches `typecheck.rs`, so relabeling
//! them has zero effect on how the merged program actually typechecks.
//!
//! The ONE surface form that IS load-bearing is `cst::TopBinding::Type` (a
//! `type` declaration): `elaborate.rs`'s `lower_type_decl` clones its ctor
//! payload / synonym body `ast::TypeExpr`s VERBATIM into
//! `Program::type_decls`/`synonym_decls`, and `typecheck.rs`'s
//! `Checker::declare_variant`/`declare_synonym` — called ONCE per merged
//! program, under the single whole-program `Checker.version` (`V0_1` for
//! `compile_document_v1`'s pipeline, fixed by `v1::module_check::
//! check_program`'s session setup, NOT swapped by `Ast::VersionScope`, which
//! only wraps a binding's RHS *body*, never a `type` declaration) — lower
//! them via `name_to_mono(_, V0_1)`. A `type foo = A of math` spliced
//! unchanged would register `A`'s payload as the nominal
//! `Variant("math",[])` (0.1's "unbound type name" fallback,
//! `name_to_mono`'s doc comment) rather than `Base(MathText)`, so any
//! REAL math value later handed to `A` would fail to unify — a spurious
//! `TypeError`, not the transparent pass-through X3a promises. So the
//! splice-arm's realization (`lib.rs`) relabels exactly the
//! `TopBinding::Type` bodies (recursing through `TopBinding::Module`'s
//! nested `decls`, the only other place a `type` declaration can appear) —
//! see `relabel_type_decls` below. This is the textual-vs-boundary decision
//! X3a had latitude on (the design's own §X3.3 describes "for each exported
//! TopBinding with a type annotation" generically; this codebase's grammar
//! makes `TopBinding::Type` the ONLY site whose text is enforced, so that is
//! what gets rewritten — simplest AND sound, since leaving the decorative
//! `RecAscription`/`SigAnnot` sites un-rewritten has no behavioral effect).
//!
//! The ACCEPT/REJECT decision itself stays exactly where X1/X2a already
//! compute it (`lib.rs`'s `collect_free_globals` free-type scan, unchanged,
//! still conservatively over-approximating every textual site including the
//! decorative ones) — X3a only changes what happens once that scan says the
//! dependency's ENTIRE forked-type touch is exactly `{"math"}`.

use satysfi_syntax::cst;
use satysfi_syntax::cst::ast::{TypeApp, TypeAtom, TypeExpr, TypeProd};
use satysfi_syntax::cst_v1;
use satysfi_syntax::SatysfiVersion;

use crate::types::{CmdArgType, MonoType, PolyType, Row};

/// The verdict for one crossing binding's boundary type (design doc X3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryError {
    /// A forked type name appears in the export signature and is not
    /// representationally shared across the boundary.
    ForkedTypeExport {
        /// The crossing binding/type this offending leaf was found under,
        /// e.g. `"Mod.bar"` or a bare type-declaration name — best-effort,
        /// may be empty when no such context is available (the standalone
        /// [`adapt_export_type`]/[`adapt_export_annotation`] helpers below
        /// don't take one; their callers can wrap/annotate further).
        binding: String,
        /// The offending leaf, e.g. `"page"` / `"deco"` / `"math-boxes"`.
        ty_name: String,
        /// The producer version (the dependency's own).
        from: SatysfiVersion,
        /// The consumer version (the splicing program's).
        to: SatysfiVersion,
        /// A human hint (why this particular name can't be relabeled).
        note: &'static str,
    },
}

impl std::fmt::Display for BoundaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundaryError::ForkedTypeExport {
                binding,
                ty_name,
                from,
                to,
                note,
            } => {
                write!(
                    f,
                    "cross-version import (X3): {}exports a value whose type \
                     names `{ty_name}`, version-forked between {from:?} and \
                     {to:?} with no proven-identical runtime representation — \
                     {note}",
                    if binding.is_empty() {
                        String::new()
                    } else {
                        format!("`{binding}` ")
                    }
                )
            }
        }
    }
}

impl std::error::Error for BoundaryError {}

/// The set of type names X3a refuses to let cross the `V0_0_6` -> `V0_1`
/// boundary unadapted: every name [`crate::typecheck::forked_type_names`]
/// flags, plus `page` (X3.1's note — see this module's doc comment for why
/// `page` needs adding explicitly rather than falling out of the automatic
/// diff). `math` is deliberately NOT a member: it is the sole X3a
/// allow-and-relabel case, checked separately by every caller below.
pub fn reject_type_names() -> std::collections::BTreeSet<String> {
    let mut set = crate::typecheck::forked_type_names();
    set.insert("page".to_string());
    set
}

/// A human hint for why `name` (a member of [`reject_type_names`]) can't be
/// relabeled across the boundary — X3.1's classification table.
fn forked_note(name: &str) -> &'static str {
    match name {
        "page" => {
            "0.0.6's page is a 9-ctor ADT (Value::Ctor); 0.1's is a length*length \
             tuple (Value::Product) — no shared runtime representation"
        }
        "math-boxes" => {
            "math-boxes is 0.1-only (the evaluated math tree); math must relabel to \
             math-text, never math-boxes (X3.8/S2) — no 0.0.6 value is ever a \
             math-boxes to begin with"
        }
        "math-text" => {
            "0.0.6 has no math-text primitive; a 0.0.6 package's OWN type named \
             math-text is an unrelated opaque user nominal, not a math value"
        }
        "deco" | "deco-set" => {
            "0.0.6 deco returns `graphics list`; 0.1 deco returns a single `graphics` \
             — the return shape differs, so crossing needs a value-level adapter. X3b \
             (classify_deco_exports/deco_coercion_prelude) handles exactly a BARE \
             top-level `let-rec name : deco | .. = ..`/`: deco-set` export (no leading \
             Fun-arrow arguments, not nested in a `module .. sig .. end`); this \
             particular occurrence is outside that scoped support (e.g. a module `val` \
             item, or `deco`/`deco-set` wrapped in extra arguments)"
        }
        "paren" => {
            "0.0.6's paren closure takes explicit fontsize/axis/color arguments; \
             0.1's pulls them from `context` — different arity, needs a wrapper \
             (deferred to X3b)"
        }
        "pre-path" | "path" | "graphics" | "image" | "font" => {
            "0.0.6 has no such primitive; this name is an opaque user-nominal \
             stand-in there, with no shared representation against 0.1's real \
             primitive type"
        }
        _ => "no proven-identical Value representation across the version boundary \
              (X3a's whitelist is `math` only)",
    }
}

fn reject_if_forked(name: &str, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    if name == "math" {
        return Ok(());
    }
    if reject_type_names().contains(name) {
        return Err(BoundaryError::ForkedTypeExport {
            binding: String::new(),
            ty_name: name.to_string(),
            from,
            to,
            note: forked_note(name),
        });
    }
    Ok(())
}

// ============================================================================
// `adapt_export_type` — the X3.2 spec function, operating on an already-
// inferred `PolyType` (a structural MonoType walk). This is the pure,
// directly-unit-testable statement of X3a's classification rule. It is NOT
// what the splice arm calls (X3.0(2): there is no standalone per-export
// `PolyType` object at the splice site — a 0.0.6 export's type only exists
// after whole-program inference); the splice arm instead calls
// `adapt_export_annotation`/`relabel_type_decls` below, on the SURFACE
// syntax, justified by the same rule this function states formally.
// ============================================================================

/// Adapt a producer-version export type into an equivalent consumer-version
/// type whose runtime `Value` representation is provably identical, or
/// reject.
///
/// `ty` is the export's type as it would be read under `from` (its `V0_0_6`
/// meaning). On `Ok`, the returned `PolyType` is what a `to`-version
/// consumer scope should bind for this export. On `Err`, the caller should
/// raise a compile error and abort.
///
/// X3a invariant: `adapt_export_type` only ever returns a type reachable
/// from `ty` by a **pure relabel with no value coercion** — any leaf that
/// would require a runtime value adapter is an `Err`. This is the soundness
/// backbone (X3.6/S1). Because the sole accepted case (`math`) is already
/// `MathText` <-> `MathText` at the `MonoType` level (`types.rs` draws no
/// `math`/`math-text` distinction at all — see `BaseType::MathText`'s doc
/// comment), the `Ok` branch is simply `ty.clone()`: the "relabel" only ever
/// shows up at the surface-annotation level (`adapt_export_annotation`).
pub fn adapt_export_type(
    ty: &PolyType,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<PolyType, BoundaryError> {
    check_mono_type(ty.body(), from, to)?;
    Ok(ty.clone())
}

fn check_mono_type(ty: &MonoType, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    match ty {
        MonoType::Var(_) | MonoType::Base(_) => Ok(()),
        MonoType::Func(row, dom, cod) => {
            check_row(row, from, to)?;
            check_mono_type(dom, from, to)?;
            check_mono_type(cod, from, to)
        }
        MonoType::Product(items) => {
            for t in items {
                check_mono_type(t, from, to)?;
            }
            Ok(())
        }
        MonoType::List(t) | MonoType::Ref(t) => check_mono_type(t, from, to),
        MonoType::Record(row) => check_row(row, from, to),
        MonoType::Variant(name, args) => {
            reject_if_forked(name, from, to)?;
            for t in args {
                check_mono_type(t, from, to)?;
            }
            Ok(())
        }
        MonoType::InlineCmd(items) | MonoType::BlockCmd(items) | MonoType::MathCmd(items) => {
            for c in items {
                check_cmd_arg(c, from, to)?;
            }
            Ok(())
        }
    }
}

fn check_row(row: &Row, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    match row {
        Row::Empty | Row::Var(_) => Ok(()),
        Row::Cons(_, ty, rest) => {
            check_mono_type(ty, from, to)?;
            check_row(rest, from, to)
        }
    }
}

fn check_cmd_arg(c: &CmdArgType, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    for (_, ty) in &c.opt_labels {
        check_mono_type(ty, from, to)?;
    }
    check_mono_type(&c.ty, from, to)
}

// ============================================================================
// `adapt_export_annotation` — the CST-level helper the splice arm actually
// exercises (X3.3): walks a SURFACE `cst::ast::TypeExpr`, rewriting every
// free `math` leaf to `math-text` and rejecting any other forked leaf
// (structural walk of `Fun`/`Atom`/`OptRowFun`/`TypeProd`/`TypeApp`/
// `TypeAtom`, mirroring `lib.rs`'s read-only `walk_type_expr` family used by
// `collect_free_globals`, but mutating instead of collecting).
// ============================================================================

/// Adapt one surface type annotation: clone `ann`, rewrite every `math` leaf
/// to `math-text`, and reject if any OTHER forked leaf (`reject_type_names`)
/// appears anywhere in the structure (S3: a forked leaf nested in a
/// compound rejects the WHOLE annotation, no partial acceptance).
pub fn adapt_export_annotation(
    ann: &TypeExpr,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<TypeExpr, BoundaryError> {
    let mut out = ann.clone();
    relabel_type_expr(&mut out, from, to)?;
    Ok(out)
}

fn relabel_type_expr(te: &mut TypeExpr, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    match te {
        TypeExpr::Fun { opts, dom, cod, .. } => {
            for o in opts.iter_mut() {
                relabel_type_prod(&mut o.ty, from, to)?;
            }
            relabel_type_prod(dom, from, to)?;
            relabel_type_expr(cod, from, to)
        }
        TypeExpr::Atom(prod) => relabel_type_prod(prod, from, to),
        TypeExpr::OptRowFun { opt_dom, dom, cod, .. } => {
            for e in opt_dom.entries.iter_mut() {
                relabel_type_expr(&mut e.ty.0, from, to)?;
            }
            relabel_type_prod(dom, from, to)?;
            relabel_type_expr(cod, from, to)
        }
    }
}

fn relabel_type_prod(tp: &mut TypeProd, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    relabel_type_app(&mut tp.first, from, to)?;
    for st in tp.rest.iter_mut() {
        relabel_type_app(&mut st.ty, from, to)?;
    }
    Ok(())
}

fn relabel_type_app(ta: &mut TypeApp, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    match ta {
        TypeApp::Applied { arg, ctor } => {
            relabel_type_atom(arg, from, to)?;
            relabel_or_reject_name(&mut ctor.name, from, to)
        }
        TypeApp::Atom(atom) => relabel_type_atom(atom, from, to),
    }
}

fn relabel_type_atom(atom: &mut TypeAtom, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    match atom {
        TypeAtom::Cmd { args, .. } => {
            for a in args.iter_mut() {
                for l in a.opt_labels.iter_mut() {
                    relabel_type_expr(&mut l.ty.0, from, to)?;
                }
                relabel_type_expr(&mut a.ty.0, from, to)?;
            }
            Ok(())
        }
        TypeAtom::Paren { inner, .. } => relabel_type_expr(&mut inner.0, from, to),
        TypeAtom::Record { fields, .. } => {
            for f in fields.iter_mut() {
                relabel_type_expr(&mut f.ty.0, from, to)?;
            }
            Ok(())
        }
        // A bound type variable — never a forked-name candidate.
        TypeAtom::Var(_) => Ok(()),
        TypeAtom::Name(n) => relabel_or_reject_name(&mut n.name, from, to),
        TypeAtom::RecordOpen { inner, .. } => {
            for f in inner.fields.iter_mut() {
                relabel_type_expr(&mut f.ty.0, from, to)?;
            }
            Ok(())
        }
    }
}

/// The one leaf-level policy decision (X3.1), generalized by DIRECTION for
/// Slice X4a (`docs/plans/design-cross-version-import.md` §X4.3 item 5 —
/// this function's `from`/`to` parameters were already threaded through
/// every caller; only the branching itself was hardcoded to the one forward
/// case before X4a).
///
/// **Where each direction is actually WIRED (load-bearing asymmetry).** The
/// FORWARD `(V0_0_6, V0_1)` arm below IS reached from `lib.rs`'s splice arm,
/// via `relabel_type_decls` — necessary there because
/// `v1::module_check::check_program_inner` hard-codes `Checker.version =
/// V0_1` for EVERY type declaration in the merged program (`module_check.rs`
/// :238-239,271), so a spliced 0.0.6 dependency's own "math" spelling MUST
/// be rewritten into ambient vocabulary before it reaches
/// `program.type_decls`. The REVERSE `(V0_1, V0_0_6)` arm below is NOT wired
/// into `lib.rs`'s reverse splice arm (`compile_document_v006_xver_with_
/// trials`): a foreign 0.1 dependency's own "math-text"/"math-boxes"
/// spelling is ALREADY that same hard-coded ambient (`V0_1`) vocabulary, so
/// relabeling it to 0.0.6's "math" would corrupt, not fix, it — that splice
/// arm instead uses this module's `reject_type_names()` as a pure WHITELIST
/// GUARD (accept-if-`{"math-text","math-boxes"}`-only, else reject; splice
/// VERBATIM either way) — see that function's own doc comment. The reverse
/// arm below stays correct and unit-tested (this module's `tests` — the
/// `adapt_export_annotation_reverse_*` group) as a direction-complete pure
/// utility, ready for a future caller (e.g. an X4b elaborated-IR-level
/// adapter) not bound by `check_program`'s hard-coded-`V0_1` constraint.
///
/// - **`V0_0_6` -> `V0_1`** (X3a, unchanged): `"math"` relabels in place to
///   `"math-text"` — 0.0.6's undifferentiated math type maps to 0.1's
///   UNEVALUATED half, the correct direction since a crossing 0.0.6 `math`
///   value is exactly a `Value::MathText`/`Value::Math` (never a
///   `Value::MathBoxes`, which 0.0.6 has no primitive that ever produces —
///   X3.8/S2's soundness backbone).
/// - **`V0_1` -> `V0_0_6`** (X4a, NEW): `"math-text"` OR `"math-boxes"`
///   relabels in place to `"math"` — 0.1's split math types both COARSEN
///   safely into 0.0.6's one undifferentiated type. Unlike the forward
///   direction, there is no symmetric "must never alias" risk here (X3.8/S2's
///   mirror does not apply): 0.0.6-authored code has no syntax that could
///   ever observe the lost `math-text`/`math-boxes` distinction — it never
///   had that distinction to begin with, so a `math-boxes` (the EVALUATED
///   tree, `Value::Math`) or a `math-text` (the UNPARSED source,
///   `Value::MathText`) crossing into 0.0.6-authored code both land as
///   exactly what a 0.0.6 `math` value already always was: EITHER shape,
///   `value.rs:39-56`'s shared representation. Re-derived, not assumed (task
///   brief's soundness bar): both `Value::MathText` and `Value::Math` are
///   ALREADY valid `math`-typed runtime values under 0.0.6 (0.0.6 code
///   pattern-matches/embeds a `math` value structurally, never on which of
///   the two variants produced it), so this is a pure, zero-coercion relabel
///   exactly like the forward case.
/// - Every other member of [`reject_type_names`], in EITHER direction, is a
///   hard `Err` (X4a's guard is deliberately conservative — see this
///   module's doc comment and X4b's future work for `page`/`graphics`/`deco`/
///   `deco-set`/`pre-path`/`path`/`image`/`font`/`paren`).
/// - Everything else (an ordinary user type name, or a shared builtin like
///   `int`/`string`) passes through untouched, in either direction.
fn relabel_or_reject_name(name: &mut String, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    match (from, to) {
        (SatysfiVersion::V0_0_6, SatysfiVersion::V0_1) if name == "math" => {
            *name = "math-text".to_string();
            Ok(())
        }
        (SatysfiVersion::V0_1, SatysfiVersion::V0_0_6)
            if name == "math-text" || name == "math-boxes" =>
        {
            *name = "math".to_string();
            Ok(())
        }
        _ => reject_if_forked(name, from, to),
    }
}

// ============================================================================
// `relabel_type_decls` — the actual splice-arm entry point (`lib.rs`'s
// `compile_document_v1_with_trials`): rewrite every LOAD-BEARING type-text
// site in a spliced `V0_0_6` dependency's `prelude` (see this module's doc
// comment for why `TopBinding::Type`, recursed through `TopBinding::
// Module`'s nested `decls`, is the complete set of such sites in this
// port's 0.0.6 grammar).
// ============================================================================

/// Clone `prelude` and relabel every `math` leaf inside a `TopBinding::Type`
/// declaration's body (a variant's ctor payloads, or a synonym's body),
/// recursing into `TopBinding::Module`'s nested `decls` — the only other
/// place a `type` declaration can appear. Every other `TopBinding` variant
/// carries no type text this port's typechecker ever consults (see the
/// module doc comment), so it is returned unchanged.
///
/// Precondition (enforced by the caller, `lib.rs`): the dependency's ENTIRE
/// free-type touch (`collect_free_globals`, checked against
/// [`reject_type_names`]) is exactly `{"math"}` — so this walk can never
/// actually observe another forked name and return `Err` in practice; the
/// `Result` is kept for defense in depth / symmetry with
/// `adapt_export_annotation`, not because failure is expected here.
pub(crate) fn relabel_type_decls(
    prelude: &[cst::TopBinding],
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<Vec<cst::TopBinding>, BoundaryError> {
    prelude
        .iter()
        .cloned()
        .map(|tb| relabel_top_binding_types(tb, from, to))
        .collect()
}

fn relabel_top_binding_types(
    mut tb: cst::TopBinding,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<cst::TopBinding, BoundaryError> {
    match &mut tb {
        cst::TopBinding::Type(td) => {
            relabel_type_decl_body(&mut td.body, from, to)?;
        }
        cst::TopBinding::Module { decls, .. } => {
            for d in decls.iter_mut() {
                let inner = (*d.0).clone();
                *d.0 = relabel_top_binding_types(inner, from, to)?;
            }
        }
        // `LetRec`/`Let`/`LetInline`/`LetBlock`/`LetMath`/`LetMutable`/
        // `Open` carry no `type`-declaration text (this module's doc
        // comment) — nothing to relabel.
        _ => {}
    }
    Ok(tb)
}

fn relabel_type_decl_body(
    body: &mut cst::TypeDeclBody,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<(), BoundaryError> {
    match body {
        cst::TypeDeclBody::Variant { first, rest, .. } => {
            relabel_variant_def(first, from, to)?;
            for bv in rest.iter_mut() {
                relabel_variant_def(&mut bv.def, from, to)?;
            }
            Ok(())
        }
        cst::TypeDeclBody::Synonym(ty) => relabel_type_expr(ty, from, to),
    }
}

fn relabel_variant_def(
    vd: &mut cst::VariantDef,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<(), BoundaryError> {
    if let Some(of_ty) = &mut vd.of_ty {
        relabel_type_expr(&mut of_ty.ty, from, to)?;
    }
    Ok(())
}

// ============================================================================
// X3b (design-cross-version-import.md's "Slice X3" §X3.5, deferred by X3a):
// a real value-level coercion for the `deco`/`deco-set` (b)-class case —
// "0.0.6 deco returns `graphics list`; 0.1 deco returns a single `graphics`".
//
// Unlike `math`, `deco`'s bare NAME needs no textual relabel at all: once
// accepted, `typecheck::name_to_mono("deco", V0_1)` already resolves it to
// `t_deco(V0_1)` unconditionally (`typecheck.rs`'s `name_to_mono` — gated on
// `version == V0_1`, with no analogous `V0_0_6` arm, so the SAME bare word
// means the SAME 0.1 arrow-type once the splice stops rejecting it). So a
// `deco`/`deco-set` mention with no VALUE attached (a bare `type foo = deco`
// synonym, exactly the fixture `xver_boundary_deco_export_coerces_and_renders`
// exercises) is *already* safe with zero further work — `relabel_type_decls`
// above is not even called for it.
//
// What genuinely needs adapting is the VALUE: a `V0_0_6`-authored `deco`
// closure's body, once fully applied, evaluates to `Value::List` of
// `Value::Graphics` (`prim_types::t_graphics_output`'s `!graphics_is_collection`
// arm; `primitives::coerce_graphics_result`'s `else` branch, which the
// compile-time `Ast::VersionScope` mechanism (X2) does NOT retroactively
// change — a closure's *body* just runs whatever it literally constructs).
// Every real `V0_1` call site that *applies* a `deco`
// (`primitives::apply_deco`, invoked by `lib.rs`'s `fire_inline_frame`/
// `fire_hooks` at render time — a genuine consumer, not a stand-in: see
// `prim_types::t_deco`'s doc comment for which primitives are stand-ins and
// which aren't) runs `coerce_graphics_result` under the AMBIENT
// `interp.version` at the time of that call — `V0_1` for any call reached
// from ordinary consumer code outside a `VersionScope` — which then expects
// a SINGLE `graphics`, not a list, and `as_graphics` on a `Value::List`
// fails. So a crossing `deco` value must be COERCED, not merely relabeled.
//
// **Scope (deliberately narrow — S1/S4-style conservatism).** X3b supports
// exactly ONE shape: a bare-leaf `: deco`/`: deco-set` ascription on a
// TOP-LEVEL (prelude-root, not nested inside a `module .. = struct .. end`)
// `TopBinding::LetRec` (its `first` clause or an `and` sibling) — i.e. the
// export's OWN declared type is *exactly* `deco`/`deco-set`, with no leading
// `Fun` arrows (`length -> color -> deco` is OUT of scope) and no module
// qualification. This is exactly the arity SATySFi's own `deco`/`deco-set`
// grammar already commits to (`t_deco`'s 4-arg point/length/length/length
// curry; `t_decoset`'s 4-tuple-of-deco), so the wrap below needs no
// currying-prefix arithmetic at all — it just re-applies the (still
// visible, not-yet-shadowed) original name positionally. Every OTHER shape
// (a `module .. sig .. end`'s `val` item, `deco`/`deco-set` nested inside a
// module's own `decls`, or wrapped in extra leading arguments) has NO sound
// wrap here — building one needs either a currying-prefix eta-expansion (the
// arity IS statically knowable from the ascription text, but the general
// case also needs a qualified-name rebinding trick this port's plain `let`
// surface syntax cannot express directly — see this module's earlier
// exploration) or a per-member module-decls rewrite; both are deferred.
// `classify_deco_exports` REJECTS (rather than silently drops) every such
// occurrence, so the splice arm (`lib.rs`) fails loudly, matching every
// other unsupported-shape case in this file.
//
// One more narrowing specific to `deco-set`: its RecBinding's own
// `rb.params` must be EXACTLY one `PatBot::Unit` (`()`). `deco-set` is a
// plain 4-TUPLE, not a function (unlike `deco`) — but `elaborate.rs`
// REQUIRES a `let-rec` binding's RHS to be a function (a recursive VALUE
// binding, as opposed to a recursive FUNCTION binding, is rejected: "must
// be a function, got tuple"), so a bare `deco-set` export can ONLY ever be
// legally WRITTEN as `let-rec name : deco-set | () = (d0, d1, d2, d3)` —
// the `| ()` idiom this test suite's OTHER "plain value" fixtures also use
// (e.g. `xver_import.rs`'s `xver-get-page : page | () = A4Paper`) — never
// as a bare bodyless `| = ..` (which does not elaborate at all for a
// non-function RHS). `deco_coercion_prelude`'s wrap therefore applies the
// mandatory unit thunk (`{name} ()`) before destructuring the tuple. (`deco`
// itself needs no such params check: the wrap always APPLIES exactly 4
// fresh arguments through ordinary function application — matching `deco`'s
// own natural function shape directly, no unit thunk involved — and HM
// unification is structural, so a `rb.params` mismatch of any shape can
// only ever produce an ordinary `TypeError`, since a partially-applied
// curried-function type can never unify with `unite-graphics`'s expected
// `list graphics` argument type. `deco-set`'s `{name} ()`-then-`match` has
// no such automatic backstop against a WRONG params shape: applying `()`
// to, or tuple-matching, a value of some other shape is *also* an ordinary
// `TypeError`, but a needlessly confusing one — checking `rb.params`
// up front turns it into this module's clear `BoundaryError` instead.)
// ============================================================================

/// Which `deco`-family shape a [`DecoExport`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecoKind {
    /// `deco = point -> length -> length -> length -> {list graphics /
    /// graphics}` — a 4-ary curried closure once fully applied.
    Deco,
    /// `deco-set = deco * deco * deco * deco` — a 4-tuple of `deco`s.
    DecoSet,
}

/// One top-level `V0_0_6` binding X3b can soundly value-coerce: its own
/// `: ty` ascription (`RecBinding.ascription`) is *exactly* the bare leaf
/// `deco`/`deco-set` — see this section's doc comment for the scope.
///
/// **Why this stays forward-only (X4b does NOT extend it — see the X4b
/// section near the bottom of this file for the full derivation).** 0.1's
/// grammar has no bare top-level type-ascription syntax at all
/// (`cst_v1::Bind::Value`/`ValueRec`'s own doc comments), so the only
/// textual site a 0.1 export's `deco`/`deco-set` type could even be NAMED
/// is a `module M :> sig val name : deco .. end = struct .. end` SIG item —
/// and EVERY such `:>` annotation is unconditionally conformance-enforced
/// by `v1::module_check`'s phase-D spine walk (name-keyed, HARD-CONSTRAINT-
/// untouched). A splice-time coercion that makes the crossing value's real
/// shape (`graphics list`) differ from the module's own declared `deco`
/// scheme (`graphics`) — which is the entire point of a reverse coercion —
/// trips that SAME enforcement again on the coercion's own binding, no
/// matter where it is spliced (verified empirically). So the reverse
/// direction cannot soundly wrap a module-sig `deco`/`deco-set` VALUE
/// export at all; it only detects-and-REJECTS one, via
/// `reject_deco_exports_v01_sig` (X4b section, below) — a wholly separate,
/// additive function, not a modification of this one.
#[derive(Debug, Clone)]
pub(crate) struct DecoExport {
    pub name: String,
    pub kind: DecoKind,
}

/// Scan a spliced `V0_0_6` dependency's `prelude` for every `deco`/
/// `deco-set` occurrence reachable from a `V0_1` consumer (the SAME
/// boundary sites `lib.rs`'s `collect_free_globals` already treats as
/// export text: a top-level `TopBinding::LetRec`'s own ascription, a
/// `TopBinding::Module`'s `sig` items, and — recursively — a module's own
/// `decls`), and classify each:
///
/// - a bare-leaf ascription on a TOP-LEVEL `TopBinding::LetRec` → sound to
///   wrap, pushed onto the returned `Vec<DecoExport>`;
/// - a `TopBinding::Type` body (a synonym/ctor payload merely NAMING
///   `deco`/`deco-set`, no value attached) → SAFE, no coercion needed at
///   all (see this section's doc comment) — silently skipped (not even
///   visited: `classify_top_binding_deco`'s `_` arm);
/// - anything else that could carry a REAL `deco`/`deco-set`-typed VALUE
///   across the boundary (a module `sig`'s `val` item, a `TopBinding::LetRec`
///   nested inside a module's own `decls`, or a bare leaf wrapped in extra
///   `Fun` arrows) → `Err` — X3b has no sound wrap for these; the caller
///   (`lib.rs`) rejects the WHOLE dependency, exactly as X3a did before any
///   `DecoExport` existed.
pub(crate) fn classify_deco_exports(
    prelude: &[cst::TopBinding],
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<Vec<DecoExport>, BoundaryError> {
    let mut out = Vec::new();
    for tb in prelude {
        classify_top_binding_deco(tb, &mut out, from, to)?;
    }
    Ok(out)
}

fn classify_top_binding_deco(
    tb: &cst::TopBinding,
    out: &mut Vec<DecoExport>,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<(), BoundaryError> {
    match tb {
        cst::TopBinding::LetRec { first, ands, .. } => {
            classify_rec_binding_deco(first, out, from, to)?;
            for a in ands {
                classify_rec_binding_deco(&a.binding, out, from, to)?;
            }
            Ok(())
        }
        cst::TopBinding::Module { sig, decls, .. } => {
            if let Some(sig) = sig {
                for item in &sig.items {
                    if let Some(ty) = sig_item_value_ty(item) {
                        reject_if_mentions_deco(ty, from, to)?;
                    }
                }
            }
            for d in decls {
                reject_if_nested_value_mentions_deco(&d.0, from, to)?;
            }
            Ok(())
        }
        // `Let`/`LetInline`/`LetBlock`/`LetMath`/`LetMutable`/`Open` carry no
        // `: ty` ascription this port's grammar could name `deco`/
        // `deco-set` in at all; `Type`'s body is the SAFE, no-value case
        // (this section's doc comment) — nothing to classify or reject.
        _ => Ok(()),
    }
}

fn classify_rec_binding_deco(
    rb: &cst::ast::RecBinding,
    out: &mut Vec<DecoExport>,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<(), BoundaryError> {
    let Some(asc) = &rb.ascription else {
        return Ok(());
    };
    match type_expr_bare_name(&asc.ty) {
        Some("deco") => {
            out.push(DecoExport {
                name: rb.name.name.clone(),
                kind: DecoKind::Deco,
            });
            Ok(())
        }
        // `deco-set` (unlike `deco`) is NOT itself a function — its VALUE is
        // a bare 4-tuple. But `elaborate.rs` REQUIRES a `let-rec` binding's
        // RHS to be a function (recursive VALUE bindings, as opposed to
        // recursive FUNCTION bindings, are rejected — "must be a function,
        // got tuple"), so a `deco-set` export can ONLY legally be written
        // with the `| ()` idiom — `let-rec name : deco-set | () = (d0, d1,
        // d2, d3)` — the SAME idiom every other "plain value" `let-rec`
        // fixture in this test suite uses (e.g. `xver_import.rs`'s
        // `xver-get-page : page | () = A4Paper`). So `rb.params` must be
        // EXACTLY one `PatBot::Unit` — `deco_coercion_prelude`'s wrapper
        // applies `{name} ()` (the mandatory unit thunk) before matching the
        // tuple. Any OTHER params shape (empty, or a real destructuring
        // pattern) is outside X3b's scoped support.
        Some("deco-set") if matches!(rb.params.as_slice(), [cst::ast::PatBot::Unit { .. }]) => {
            out.push(DecoExport {
                name: rb.name.name.clone(),
                kind: DecoKind::DecoSet,
            });
            Ok(())
        }
        _ => reject_if_mentions_deco(&asc.ty, from, to),
    }
}

/// A `module .. decls ..` member that could itself carry a real `deco`/
/// `deco-set`-typed VALUE: recurse into nested `LetRec` ascriptions and
/// nested `Module`s (their own `sig` + `decls`); a nested `Type` body is
/// the SAFE no-value case, skipped.
fn reject_if_nested_value_mentions_deco(
    tb: &cst::TopBinding,
    from: SatysfiVersion,
    to: SatysfiVersion,
) -> Result<(), BoundaryError> {
    match tb {
        cst::TopBinding::LetRec { first, ands, .. } => {
            if let Some(asc) = &first.ascription {
                reject_if_mentions_deco(&asc.ty, from, to)?;
            }
            for a in ands {
                if let Some(asc) = &a.binding.ascription {
                    reject_if_mentions_deco(&asc.ty, from, to)?;
                }
            }
            Ok(())
        }
        cst::TopBinding::Module { sig, decls, .. } => {
            if let Some(sig) = sig {
                for item in &sig.items {
                    if let Some(ty) = sig_item_value_ty(item) {
                        reject_if_mentions_deco(ty, from, to)?;
                    }
                }
            }
            for d in decls {
                reject_if_nested_value_mentions_deco(&d.0, from, to)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn sig_item_value_ty(item: &cst::SigItem) -> Option<&TypeExpr> {
    use cst::SigItem;
    match item {
        SigItem::ValHorzCmd { ty, .. }
        | SigItem::ValVertCmd { ty, .. }
        | SigItem::Val { ty, .. }
        | SigItem::DirectHorzCmd { ty, .. }
        | SigItem::DirectVertCmd { ty, .. } => Some(ty),
        SigItem::Type { .. } => None,
    }
}

/// `Some(name)` iff `te` is *exactly* one bare `TypeAtom::Name(name)` with
/// no `Fun` wrapper and no `TypeProd` continuation (`rest` empty) — the one
/// shape X3b's wrap knows how to handle with no currying-prefix arithmetic.
fn type_expr_bare_name(te: &TypeExpr) -> Option<&str> {
    match te {
        TypeExpr::Atom(TypeProd {
            first: TypeApp::Atom(TypeAtom::Name(n)),
            rest,
        }) if rest.is_empty() => Some(n.name.as_str()),
        _ => None,
    }
}

fn reject_if_mentions_deco(te: &TypeExpr, from: SatysfiVersion, to: SatysfiVersion) -> Result<(), BoundaryError> {
    if let Some(name) = type_expr_mentions_deco(te) {
        return Err(BoundaryError::ForkedTypeExport {
            binding: String::new(),
            ty_name: name.clone(),
            from,
            to,
            note: forked_note(&name),
        });
    }
    Ok(())
}

/// Structural (read-only) walk mirroring `relabel_type_expr`'s traversal —
/// `Some(name)` for the first `"deco"`/`"deco-set"` leaf found anywhere in
/// `te`, `None` if there is none.
fn type_expr_mentions_deco(te: &TypeExpr) -> Option<String> {
    match te {
        TypeExpr::Fun { opts, dom, cod, .. } => opts
            .iter()
            .find_map(|o| type_prod_mentions_deco(&o.ty))
            .or_else(|| type_prod_mentions_deco(dom))
            .or_else(|| type_expr_mentions_deco(cod)),
        TypeExpr::Atom(prod) => type_prod_mentions_deco(prod),
        TypeExpr::OptRowFun { opt_dom, dom, cod, .. } => opt_dom
            .entries
            .iter()
            .find_map(|e| type_expr_mentions_deco(&e.ty.0))
            .or_else(|| type_prod_mentions_deco(dom))
            .or_else(|| type_expr_mentions_deco(cod)),
    }
}

fn type_prod_mentions_deco(tp: &TypeProd) -> Option<String> {
    type_app_mentions_deco(&tp.first).or_else(|| tp.rest.iter().find_map(|st| type_app_mentions_deco(&st.ty)))
}

fn type_app_mentions_deco(ta: &TypeApp) -> Option<String> {
    match ta {
        TypeApp::Applied { arg, ctor } => type_atom_mentions_deco(arg).or_else(|| deco_leaf_name(&ctor.name)),
        TypeApp::Atom(atom) => type_atom_mentions_deco(atom),
    }
}

fn type_atom_mentions_deco(atom: &TypeAtom) -> Option<String> {
    match atom {
        TypeAtom::Cmd { args, .. } => args.iter().find_map(|a| {
            a.opt_labels
                .iter()
                .find_map(|l| type_expr_mentions_deco(&l.ty.0))
                .or_else(|| type_expr_mentions_deco(&a.ty.0))
        }),
        TypeAtom::Paren { inner, .. } => type_expr_mentions_deco(&inner.0),
        TypeAtom::Record { fields, .. } => fields.iter().find_map(|f| type_expr_mentions_deco(&f.ty.0)),
        TypeAtom::Var(_) => None,
        TypeAtom::Name(n) => deco_leaf_name(&n.name),
        TypeAtom::RecordOpen { inner, .. } => inner.fields.iter().find_map(|f| type_expr_mentions_deco(&f.ty.0)),
    }
}

fn deco_leaf_name(name: &str) -> Option<String> {
    if name == "deco" || name == "deco-set" {
        Some(name.to_string())
    } else {
        None
    }
}

/// Build synthetic, `V0_1`-authored `TopBinding`s that COERCE each
/// [`DecoExport`]'s already-spliced (`V0_0_6`-semantics, list-returning)
/// value into the single-`graphics` shape a `V0_1` consumer's call sites
/// expect (this section's doc comment). For `DecoKind::Deco`, splices a
/// SECOND top-level `let` of the SAME name that shadows the original
/// (ordinary non-recursive-`let` shadowing: its own body's reference to
/// that name resolves to the PREVIOUS, not-yet-shadowed binding — the
/// original `let-rec`), forwarding the deco's own 4 curried arguments
/// (point, length, length, length — `prim_types::t_deco`'s arity)
/// positionally and uniting the `graphics list` result via the real `V0_1`
/// `unite-graphics : list graphics -> graphics` primitive
/// (`primitives.rs`'s `prim_unite_graphics`). For `DecoKind::DecoSet`,
/// destructures the original 4-tuple and rewraps each component the same
/// way. Every identifier this generates is `xver-`-prefixed to avoid
/// colliding with the export's own (unknown-to-us) parameter names.
///
/// The generated source is parsed via [`satysfi_syntax::parse_file`]
/// (mirroring this module's own test helpers, `prelude_of`/`parse_ty`) —
/// `panic!`s on a parse failure, since that would mean this function itself
/// generated malformed syntax (an internal bug), never a symptom of the
/// user's own dependency source.
pub(crate) fn deco_coercion_prelude(exports: &[DecoExport]) -> Vec<cst::TopBinding> {
    if exports.is_empty() {
        return Vec::new();
    }
    let mut src = String::new();
    for exp in exports {
        match exp.kind {
            DecoKind::Deco => {
                src.push_str(&format!(
                    "let {name} xver-p xver-w xver-h xver-d =\n\
                     \x20 unite-graphics ({name} xver-p xver-w xver-h xver-d)\n",
                    name = exp.name
                ));
            }
            DecoKind::DecoSet => {
                // `{name}` is `unit -> deco-set` (the `| ()` idiom
                // `classify_rec_binding_deco` requires) — apply the mandatory
                // unit thunk first, then destructure.
                src.push_str(&format!(
                    "let {name} =\n\
                     \x20 match {name} () with\n\
                     \x20 | (xver-d0, xver-d1, xver-d2, xver-d3) ->\n\
                     \x20   ((fun xver-p xver-w xver-h xver-d -> unite-graphics (xver-d0 xver-p xver-w xver-h xver-d)),\n\
                     \x20    (fun xver-p xver-w xver-h xver-d -> unite-graphics (xver-d1 xver-p xver-w xver-h xver-d)),\n\
                     \x20    (fun xver-p xver-w xver-h xver-d -> unite-graphics (xver-d2 xver-p xver-w xver-h xver-d)),\n\
                     \x20    (fun xver-p xver-w xver-h xver-d -> unite-graphics (xver-d3 xver-p xver-w xver-h xver-d)))\n",
                    name = exp.name
                ));
            }
        }
    }
    // Deliberately NO trailing dummy body: `File.body` is legitimately
    // `Option`-al (`cst.rs`'s doc comment — "Absent for a library file
    // (`nxtopsubseq`'s bare `EOI` case)"), and a bare literal like `0` is a
    // valid ATOM that a preceding `let`'s value expression's application
    // chain would happily keep consuming as one more argument (nothing
    // about a top-level decl boundary is whitespace-sensitive here — only a
    // following reserved keyword like `let`/`type`/`module` stops an
    // application chain). Parsing as a bare `prelude* EOI` library file
    // sidesteps that trap entirely; only `.prelude` is ever read below.
    let file = satysfi_syntax::parse_file(&src).unwrap_or_else(|e| {
        panic!(
            "xver_adapt::deco_coercion_prelude: internally-generated X3b wrapper \
             source failed to parse (a bug in xver_adapt.rs, not user input): {e}\n\
             --- generated source ---\n{src}"
        )
    });
    file.prelude
}

// ============================================================================
// X4b (the reverse mirror of X3b, above) — FINDING, not a feature: a
// foreign `V0_1` dependency's `deco`/`deco-set` export returns a single
// `graphics` (0.1 semantics); every REAL `V0_0_6`-authored consumer call
// site (`primitives::apply_deco`/`coerce_graphics_result`, fired at render
// time under `interp.version == V0_0_6` — `lib.rs`'s
// `compile_document_v006_xver_with_trials` always calls
// `eval_document_trials(.., SatysfiVersion::V0_0_6)`) expects a `graphics
// list` back (`coerce_graphics_result`'s `!graphics_is_collection()`
// branch, `as_list`). A sound coercion would need to wrap the single
// `graphics` in a SINGLETON LIST — `let name p w h d = [name p w h d]`,
// the literal inverse of X3b's `unite-graphics` wrap.
//
// **Why this is NOT implemented as a coercion (the task brief's own
// "STOP and report" escape hatch).** 0.1's grammar has NO bare top-level
// type-ascription syntax at all (`cst_v1::Bind::Value`/`ValueRec`'s own
// doc comments — no `: ty` on a plain `val`/`val rec`), so the ONLY
// textual site a 0.1 export's `deco`/`deco-set` type could ever be NAMED
// is a `module M :> sig val name : deco .. end = struct .. end` SIG item.
// But EVERY 0.1 module signature annotation is the `:>` (COERCE) form —
// 0.1 has no OTHER annotation keyword (`SigAnnotV1.coerce: CoerceTok`,
// unconditionally) — and `v1::module_check`'s (HARD-CONSTRAINT-untouched)
// phase-D spine walk enforces EVERY such annotation's conformance,
// UNCONDITIONALLY, for every `Ast::LetIn` node whose name matches a
// `static_env.seals` entry — this check is PURELY NAME-KEYED, not
// "first occurrence only": it fires on ANY binding sharing that exact
// qualified name, wherever it appears in the merged program.
//
// This was verified empirically, not merely reasoned about: an earlier
// version of this slice appended a coercing member INSIDE the exporting
// module's own `decls` (shadowing the original for both intra-module and
// qualified-external lookups, mirroring X3b's top-level shadow trick one
// level deeper) — compiling it produced `v1::module_check`'s own
// "module `M` does not match its signature: value `name` has type
// .. graphics list .. but its signature declares .. graphics .." error,
// because the phase-D walk's `static_env.seals.get(name)` check fires on
// the SHADOWING binding too (it is keyed by the qualified STRING, not by
// "the module's own first/original member"). A later attempt to splice
// the wrap as a SEPARATE top-level binding under the same fully-qualified
// dotted key (bypassing the module's own `decls` entirely, exploiting the
// fact that `elaborate.rs`'s top-level `push_named_binding` treats a
// binder name as an opaque `String` with no identifier-syntax validation)
// does not escape this either: `module_check`'s spine walk still matches
// on the exact name at THAT binding's own `Ast::LetIn` node, wherever in
// the merged program's single sequential chain it occurs. There is no
// splice-time position for a differently-shaped binding under the sealed
// name that `v1::module_check` (unmodified) will not re-check against the
// module's own declared scheme — and re-shaping the VALUE (list vs.
// single) is the entire point of this coercion, so it can never pass that
// re-check. Fixing this would require teaching `module_check.rs` to skip
// (or special-case) a cross-version coercion shadow, which the task's
// HARD CONSTRAINTS forbid ("compile.rs/eval.rs/module_check.rs/types.rs/
// value.rs must stay UNTOUCHED — if you need to change them, STOP and
// report").
//
// **What this leaves in place.** `v1::xver_adapt`'s existing forward-only
// `DecoExport`/`classify_deco_exports`/`deco_coercion_prelude` (above) are
// UNCHANGED — X4b does not touch X3b's own file/behavior at all. The
// reverse direction keeps X4a's original conservative posture (reject
// every forked type name except the proven-identical `math-text`/
// `math-boxes`), with exactly one additive improvement: a `deco`/
// `deco-set` VALUE export via a module's `sig` is now rejected with a
// SPECIFIC, EARLY diagnostic (`reject_deco_exports_v01_sig`, below) —
// naming the exact offending type — instead of silently splicing verbatim
// and letting the caller hit a confusing downstream `module_check`/
// ordinary-`TypeError` failure with no indication of WHY. A bare `type
// foo = deco` synonym (no value attached — safe with zero coercion, the
// same reasoning as the forward direction's `type xver-deco-alias = deco`)
// is unaffected either way: it is not a sig `val` item at all, so this
// function never sees it, and the ordinary POST-lowering
// `collect_free_globals` scan (`lib.rs`) already lets it splice verbatim.
// ============================================================================

/// Detect (to REJECT, never to accept — see this section's doc comment)
/// a `deco`/`deco-set` mention anywhere in a foreign `V0_1` dependency's OWN
/// top-level module signature. Operates on the dependency's file's
/// ORIGINAL, PRE-lowering `cst_v1::FileV1` — `v1::lower::
/// lower_file_v1_with_surfaces` DROPS a 0.1 module's `sig_annot` entirely
/// (`v1/lower.rs`'s own "sig_annot is then simply DROPPED" doc comment),
/// so this is the only stage at which the sig's text still exists; the
/// ordinary POST-lowering `collect_free_globals` scan (`lib.rs`) can never
/// see it.
///
/// `Ok(())` for a `FileV1::Document` (never a dependency), a `Library`
/// with no `sig_annot` at all, or one whose signature this port's
/// unresolved-reference-avoidance (`v1_sigbot_mentions_deco`'s own doc
/// comment) does not chase (a named signature reference, or a functor) —
/// a forked type hiding behind one of those is NOT a soundness gap
/// (X4.8/S2): HM still infers the crossing value's REAL shape at every use
/// site regardless of what this textual scan saw, so the worst case is an
/// ordinary `TypeError` far from its cause, never silent corruption.
pub(crate) fn reject_deco_exports_v01_sig(file: &cst_v1::FileV1) -> Result<(), BoundaryError> {
    let cst_v1::FileV1::Library {
        sig_annot: Some(sig_annot),
        ..
    } = file
    else {
        return Ok(());
    };
    if let Some(name) = v1_sigexpr_mentions_deco(&sig_annot.sig_.0) {
        return Err(BoundaryError::ForkedTypeExport {
            binding: String::new(),
            ty_name: name.clone(),
            from: SatysfiVersion::V0_1,
            to: SatysfiVersion::V0_0_6,
            note: forked_note(&name),
        });
    }
    Ok(())
}

/// Structural (read-only) walk of a 0.1 signature expression, looking for
/// any `deco`/`deco-set` mention anywhere reachable from it: a direct
/// inline `sig .. end` body's `val`/`val \cmd`/`val +cmd` items (recursing
/// into any nested `module`/`signature`/`include` declaration too), or a
/// `with type` refinement's base. A named-signature reference
/// (`SigBotV1::Path`/`Var`) or a functor signature is NOT chased further —
/// see [`reject_deco_exports_v01_sig`]'s own doc comment for why that is
/// still sound (a false negative here is an ordinary `TypeError`, never
/// unsoundness).
fn v1_sigexpr_mentions_deco(se: &cst_v1::ast::SigExpr) -> Option<String> {
    use cst_v1::ast::SigExpr;
    match se {
        SigExpr::Functor { dom, cod, .. } => v1_sigexpr_mentions_deco(dom).or_else(|| v1_sigexpr_mentions_deco(cod)),
        SigExpr::WithType { base, .. } => v1_sigbot_mentions_deco(base),
        SigExpr::Bot(bot) => v1_sigbot_mentions_deco(bot),
    }
}

fn v1_sigbot_mentions_deco(bot: &cst_v1::ast::SigBotV1) -> Option<String> {
    use cst_v1::ast::SigBotV1;
    match bot {
        // An unresolved named-signature reference — not chased (this
        // section's doc comment).
        SigBotV1::Path(_) | SigBotV1::Var(_) => None,
        SigBotV1::Sig { decls, .. } => decls.iter().find_map(|d| v1_decl_mentions_deco(&d.0)),
    }
}

fn v1_decl_mentions_deco(decl: &cst_v1::ast::Decl) -> Option<String> {
    use cst_v1::ast::Decl;
    match decl {
        Decl::Val { ty, .. } | Decl::ValHorzCmd { ty, .. } | Decl::ValVertCmd { ty, .. } => {
            v1_type_expr_mentions_deco(ty)
        }
        // A `type`/opaque-`type` sig item merely NAMES `deco`/`deco-set`
        // with no attached VALUE — safe, no coercion needed at all (this
        // section's doc comment's "bare `type foo = deco` synonym" case).
        Decl::TypeOpaque { .. } | Decl::Type { .. } => None,
        Decl::Module { sig_, .. } | Decl::Signature { sig_, .. } | Decl::Include { sig_, .. } => {
            v1_sigexpr_mentions_deco(sig_)
        }
    }
}

/// Structural (read-only) walk of the WIDENED 0.1 type-expression grammar
/// (`cst_v1::ast::TypeExpr`) — the 0.1-grammar twin of this module's own
/// `type_expr_mentions_deco`, additionally covering 0.1-only shapes
/// (`OptRowFun`, prefix `TypeApp::Applied`/`AppliedLong`, the `inline
/// [..]`/`block [..]`/`math [..]` command-type forms). `Some(name)` for
/// the first `"deco"`/`"deco-set"` leaf found anywhere in `te`, `None` if
/// there is none.
fn v1_type_expr_mentions_deco(te: &cst_v1::ast::TypeExpr) -> Option<String> {
    use cst_v1::ast::TypeExpr;
    match te {
        TypeExpr::OptRowFun { opt_dom, dom, cod, .. } => opt_dom
            .inner
            .entries
            .iter()
            .find_map(|e| v1_type_expr_mentions_deco(&e.ty.0))
            .or_else(|| v1_type_prod_mentions_deco(dom))
            .or_else(|| v1_type_expr_mentions_deco(cod)),
        TypeExpr::Fun { dom, cod, .. } => {
            v1_type_prod_mentions_deco(dom).or_else(|| v1_type_expr_mentions_deco(cod))
        }
        TypeExpr::Atom(prod) => v1_type_prod_mentions_deco(prod),
    }
}

fn v1_type_prod_mentions_deco(tp: &cst_v1::ast::TypeProd) -> Option<String> {
    v1_type_app_mentions_deco(&tp.first).or_else(|| tp.rest.iter().find_map(|st| v1_type_app_mentions_deco(&st.ty)))
}

fn v1_type_app_mentions_deco(ta: &cst_v1::ast::TypeApp) -> Option<String> {
    use cst_v1::ast::TypeApp;
    match ta {
        // Prefix application (`list int`, 0.1-only shape): the CTOR itself
        // is the bare-name position here (unlike the universal postfix
        // grammar) — check it, plus every argument atom.
        TypeApp::Applied { ctor, first, rest } => deco_leaf_name(&ctor.name)
            .or_else(|| v1_type_atom_mentions_deco(first))
            .or_else(|| rest.iter().find_map(v1_type_atom_mentions_deco)),
        // `M.t τ…` — a QUALIFIED ctor name; never itself a bare
        // `"deco"`/`"deco-set"` (those are unqualified builtins), only its
        // arguments can mention one.
        TypeApp::AppliedLong { first, rest, .. } => {
            v1_type_atom_mentions_deco(first).or_else(|| rest.iter().find_map(v1_type_atom_mentions_deco))
        }
        TypeApp::InlineCmdTy { args, .. } | TypeApp::BlockCmdTy { args, .. } | TypeApp::MathCmdTy { args, .. } => {
            args.iter().find_map(v1_type_cmd_arg_mentions_deco)
        }
        TypeApp::Atom(atom) => v1_type_atom_mentions_deco(atom),
    }
}

fn v1_type_cmd_arg_mentions_deco(item: &cst_v1::ast::TypeCmdArgItemV1) -> Option<String> {
    item.opts
        .as_ref()
        .and_then(|o| o.entries.iter().find_map(|e| v1_type_expr_mentions_deco(&e.ty.0)))
        .or_else(|| v1_type_expr_mentions_deco(&item.ty.0))
}

fn v1_type_atom_mentions_deco(atom: &cst_v1::ast::TypeAtom) -> Option<String> {
    use cst_v1::ast::TypeAtom;
    match atom {
        TypeAtom::Paren { inner, .. } => v1_type_expr_mentions_deco(&inner.0),
        TypeAtom::Record { inner, .. } => inner.fields.iter().find_map(|f| v1_type_expr_mentions_deco(&f.ty.0)),
        // A bound type variable — never a forked-name candidate.
        TypeAtom::Var(_) => None,
        // `M.t` — a qualified name; never itself a bare builtin fork name.
        TypeAtom::LongName(_) => None,
        TypeAtom::Name(n) => deco_leaf_name(&n.name),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BaseType;

    fn v006() -> SatysfiVersion {
        SatysfiVersion::V0_0_6
    }
    fn v01() -> SatysfiVersion {
        SatysfiVersion::V0_1
    }

    // ------------------------------------------------------------------
    // adapt_export_type (PolyType-level spec function)
    // ------------------------------------------------------------------

    #[test]
    fn adapt_export_type_accepts_bare_math_text_base() {
        // `math`'s 0.0.6 meaning IS `Base(MathText)` (name_to_mono already
        // performed the math->MathText fold before this function ever sees
        // it) — the identity/no-op accept path.
        let ty = PolyType::mono(MonoType::Base(BaseType::MathText));
        let out = adapt_export_type(&ty, v006(), v01()).expect("math (MathText) must be accepted");
        assert!(matches!(out.body(), MonoType::Base(BaseType::MathText)));
    }

    #[test]
    fn adapt_export_type_accepts_math_nested_in_function_and_list() {
        let ty = PolyType::mono(MonoType::Func(
            Box::new(Row::Empty),
            Box::new(MonoType::List(Box::new(MonoType::Base(BaseType::MathText)))),
            Box::new(MonoType::Base(BaseType::MathText)),
        ));
        assert!(adapt_export_type(&ty, v006(), v01()).is_ok());
    }

    #[test]
    fn adapt_export_type_rejects_page_nominal() {
        let ty = PolyType::mono(MonoType::Variant("page".to_string(), vec![]));
        let err = adapt_export_type(&ty, v006(), v01()).expect_err("page must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "page"),
        }
    }

    #[test]
    fn adapt_export_type_rejects_opaque_nominals() {
        for name in ["math-text", "math-boxes", "pre-path", "path", "graphics", "image", "font"] {
            let ty = PolyType::mono(MonoType::Variant(name.to_string(), vec![]));
            let err = adapt_export_type(&ty, v006(), v01()).unwrap_err();
            match err {
                BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, name),
            }
        }
    }

    #[test]
    fn adapt_export_type_rejects_forked_leaf_nested_in_a_compound() {
        // S3: a forked leaf anywhere in the structure rejects the whole
        // export, even alongside an otherwise-fine math leaf.
        let ty = PolyType::mono(MonoType::Product(vec![
            MonoType::Base(BaseType::MathText),
            MonoType::Variant("page".to_string(), vec![]),
        ]));
        assert!(adapt_export_type(&ty, v006(), v01()).is_err());
    }

    #[test]
    fn adapt_export_type_accepts_ordinary_user_nominal() {
        // A ordinary, non-forked user type name must pass through untouched
        // (S4's flip side: only the specific reject set is conservative,
        // not every nominal).
        let ty = PolyType::mono(MonoType::Variant("option".to_string(), vec![MonoType::Base(BaseType::Int)]));
        assert!(adapt_export_type(&ty, v006(), v01()).is_ok());
    }

    // ------------------------------------------------------------------
    // adapt_export_annotation (CST-level helper)
    // ------------------------------------------------------------------

    fn parse_ty(src: &str) -> TypeExpr {
        // Reuse a `type` declaration's RHS to get a real parsed TypeExpr —
        // simplest way to build one without hand-rolling the CST.
        let file = satysfi_syntax::parse_file(&format!("type xver-probe = {src}\n0\n")).expect("parse");
        for tb in &file.prelude {
            if let cst::TopBinding::Type(td) = tb {
                if let cst::TypeDeclBody::Synonym(ty) = &td.body {
                    return ty.clone();
                }
            }
        }
        panic!("expected a type synonym declaration");
    }

    #[test]
    fn adapt_export_annotation_relabels_bare_math() {
        let ann = parse_ty("math");
        let out = adapt_export_annotation(&ann, v006(), v01()).expect("math must be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first: TypeApp::Atom(TypeAtom::Name(n)),
                ..
            }) => assert_eq!(n.name, "math-text"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    #[test]
    fn adapt_export_annotation_relabels_math_nested_in_function_type() {
        let ann = parse_ty("math -> math");
        let out = adapt_export_annotation(&ann, v006(), v01()).expect("math -> math must be accepted");
        // Round-trip through Display/Debug is overkill; just confirm no
        // `Err` and that unparsing still contains `math-text` twice, not
        // bare `math`.
        let unparsed = format!("{out:?}");
        assert!(!unparsed.contains("\"math\""), "no bare `math` should survive: {unparsed}");
    }

    #[test]
    fn adapt_export_annotation_rejects_page() {
        let ann = parse_ty("page -> document");
        let err = adapt_export_annotation(&ann, v006(), v01()).expect_err("page must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "page"),
        }
    }

    #[test]
    fn adapt_export_annotation_rejects_deco() {
        let ann = parse_ty("deco");
        let err = adapt_export_annotation(&ann, v006(), v01()).expect_err("deco must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    // ------------------------------------------------------------------
    // Slice X4a: the REVERSE (V0_1 -> V0_0_6) direction of the leaf policy
    // (`relabel_or_reject_name`'s new match arm). NOTE: these pin the PURE
    // function's contract only — `lib.rs`'s `compile_document_v006_xver_
    // with_trials` deliberately does NOT call `adapt_export_annotation`/
    // `relabel_type_decls` in this direction (see that function's own doc
    // comment for why: `v1::module_check::check_program`'s `Checker.version`
    // is hard-coded `V0_1` for EVERY type declaration in the merged program,
    // so a foreign 0.1 dependency's OWN "math-text"/"math-boxes" spelling is
    // ALREADY the vocabulary that matters — relabeling it to 0.0.6's "math"
    // would corrupt it). This function stays available (and correctly
    // direction-aware) for a future caller — e.g. an X4b elaborated-IR-level
    // adapter — that isn't bound by that same hard-coded-V0_1 constraint.
    // ------------------------------------------------------------------

    #[test]
    fn adapt_export_annotation_reverse_relabels_math_text_to_math() {
        let ann = parse_ty("math-text");
        let out = adapt_export_annotation(&ann, v01(), v006()).expect("math-text must be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first: TypeApp::Atom(TypeAtom::Name(n)),
                ..
            }) => assert_eq!(n.name, "math"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    #[test]
    fn adapt_export_annotation_reverse_relabels_math_boxes_to_math() {
        let ann = parse_ty("math-boxes");
        let out = adapt_export_annotation(&ann, v01(), v006()).expect("math-boxes must be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first: TypeApp::Atom(TypeAtom::Name(n)),
                ..
            }) => assert_eq!(n.name, "math"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    #[test]
    fn adapt_export_annotation_reverse_rejects_page() {
        let ann = parse_ty("page -> document");
        let err = adapt_export_annotation(&ann, v01(), v006()).expect_err("page must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "page"),
        }
    }

    #[test]
    fn adapt_export_annotation_reverse_rejects_graphics() {
        let ann = parse_ty("graphics");
        let err = adapt_export_annotation(&ann, v01(), v006()).expect_err("graphics must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "graphics"),
        }
    }

    #[test]
    fn adapt_export_annotation_forward_math_relabel_is_unaffected_by_the_reverse_arm() {
        // Non-regression: adding the (V0_1, V0_0_6) arm must not perturb the
        // EXISTING (V0_0_6, V0_1) "math"->"math-text" relabel.
        let ann = parse_ty("math");
        let out = adapt_export_annotation(&ann, v006(), v01()).expect("math must still be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first: TypeApp::Atom(TypeAtom::Name(n)),
                ..
            }) => assert_eq!(n.name, "math-text"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // relabel_type_decls (prelude-level splice-arm entry point)
    // ------------------------------------------------------------------

    fn prelude_of(src: &str) -> Vec<cst::TopBinding> {
        let file = satysfi_syntax::parse_file(src).expect("parse");
        file.prelude
    }

    #[test]
    fn relabel_type_decls_rewrites_variant_ctor_payload() {
        let prelude = prelude_of("type xver-wrap = XverWrap of math\n0\n");
        let out = relabel_type_decls(&prelude, v006(), v01()).expect("math-only prelude must relabel");
        match &out[0] {
            cst::TopBinding::Type(td) => match &td.body {
                cst::TypeDeclBody::Variant { first, .. } => {
                    let ty = &first.of_ty.as_ref().unwrap().ty;
                    match ty {
                        TypeExpr::Atom(TypeProd {
                            first: TypeApp::Atom(TypeAtom::Name(n)),
                            ..
                        }) => assert_eq!(n.name, "math-text"),
                        other => panic!("expected relabeled Name, got {other:?}"),
                    }
                }
                other => panic!("expected a Variant body, got {other:?}"),
            },
            other => panic!("expected a Type binding, got {other:?}"),
        }
    }

    #[test]
    fn relabel_type_decls_recurses_into_nested_module() {
        let prelude = prelude_of(
            "module M = struct\n  type inner-wrap = InnerWrap of math\nend\n0\n",
        );
        let out = relabel_type_decls(&prelude, v006(), v01()).expect("nested math-only prelude must relabel");
        match &out[0] {
            cst::TopBinding::Module { decls, .. } => match decls[0].0.as_ref() {
                cst::TopBinding::Type(td) => match &td.body {
                    cst::TypeDeclBody::Variant { first, .. } => {
                        let ty = &first.of_ty.as_ref().unwrap().ty;
                        match ty {
                            TypeExpr::Atom(TypeProd {
                                first: TypeApp::Atom(TypeAtom::Name(n)),
                                ..
                            }) => assert_eq!(n.name, "math-text"),
                            other => panic!("expected relabeled Name, got {other:?}"),
                        }
                    }
                    other => panic!("expected a Variant body, got {other:?}"),
                },
                other => panic!("expected a nested Type binding, got {other:?}"),
            },
            other => panic!("expected a Module binding, got {other:?}"),
        }
    }

    #[test]
    fn relabel_type_decls_leaves_non_math_untouched() {
        // Sanity: a prelude with no forked type text at all is unaffected
        // (this exercises the same walk the touched=={} fast path in
        // lib.rs never even calls, but confirms the walk itself is a no-op
        // absent any `math`).
        let prelude = prelude_of("type ordinary = Foo of int\n0\n");
        let out = relabel_type_decls(&prelude, v006(), v01()).expect("no forked names present");
        assert_eq!(format!("{prelude:?}"), format!("{out:?}"));
    }

    // ------------------------------------------------------------------
    // X3b: classify_deco_exports / deco_coercion_prelude
    // ------------------------------------------------------------------

    #[test]
    fn classify_deco_exports_accepts_bare_top_level_letrec() {
        let prelude = prelude_of("let-rec xver-my-deco : deco | (x, y) w h d = []\n0\n");
        let exports = classify_deco_exports(&prelude, v006(), v01()).expect("bare `: deco` must be accepted");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "xver-my-deco");
        assert_eq!(exports[0].kind, DecoKind::Deco);
    }

    #[test]
    fn classify_deco_exports_accepts_bare_decoset() {
        // `| ()` is the ONLY legal idiom for a plain `deco-set` VALUE export
        // — `elaborate.rs` requires a `let-rec`'s RHS to be a function, so
        // `let-rec name : deco-set | = (tuple)` (zero params, no `()`) does
        // not even elaborate; see `classify_rec_binding_deco`'s doc comment.
        let prelude = prelude_of("let-rec xver-my-decoset : deco-set | () = (0, 0, 0, 0)\n0\n");
        let exports = classify_deco_exports(&prelude, v006(), v01()).expect("`| ()` deco-set must be accepted");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "xver-my-decoset");
        assert_eq!(exports[0].kind, DecoKind::DecoSet);
    }

    #[test]
    fn classify_deco_exports_rejects_decoset_with_wrong_params() {
        // A `deco-set` export with a REAL (non-unit) param — outside X3b's
        // scoped support (only the bare `| ()` idiom is handled).
        let prelude = prelude_of("let-rec xver-my-decoset : deco-set | t = (0, 0, 0, 0)\n0\n");
        let err = classify_deco_exports(&prelude, v006(), v01())
            .expect_err("a deco-set export with a non-unit param must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco-set"),
        }
    }

    #[test]
    fn classify_deco_exports_ignores_type_synonym() {
        // A bare `type .. = deco` synonym has no attached value — safe with
        // zero coercion, so it must not be classified as a `DecoExport` at
        // all (nothing to wrap).
        let prelude = prelude_of("type xver-deco-alias = deco\n0\n");
        let exports = classify_deco_exports(&prelude, v006(), v01()).expect("a type synonym must be accepted");
        assert!(exports.is_empty());
    }

    #[test]
    fn classify_deco_exports_rejects_curried_prefix() {
        // `length -> color -> color -> deco` — out of X3b's scoped support
        // (a leading Fun-arrow prefix before the `deco` leaf).
        let prelude = prelude_of("let-rec xver-my-deco : length -> deco | t (x, y) w h d = []\n0\n");
        let err = classify_deco_exports(&prelude, v006(), v01())
            .expect_err("a curried-prefix deco export must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn classify_deco_exports_rejects_module_sig_item() {
        let prelude = prelude_of(
            "module M : sig\n  val simple : deco\nend = struct\n  let simple (x, y) w h d = []\nend\n0\n",
        );
        let err = classify_deco_exports(&prelude, v006(), v01())
            .expect_err("a module-qualified deco export must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn deco_coercion_prelude_generates_parseable_wrap() {
        let exports = vec![
            DecoExport {
                name: "xver-my-deco".to_string(),
                kind: DecoKind::Deco,
            },
            DecoExport {
                name: "xver-my-decoset".to_string(),
                kind: DecoKind::DecoSet,
            },
        ];
        let out = deco_coercion_prelude(&exports);
        // One synthetic `TopBinding::Let` per export, in order.
        assert_eq!(out.len(), 2);
        match &out[0] {
            cst::TopBinding::Let(tl) => assert_eq!(tl.name.name, "xver-my-deco"),
            other => panic!("expected a Let binding for the deco wrapper, got {other:?}"),
        }
        match &out[1] {
            cst::TopBinding::Let(tl) => assert_eq!(tl.name.name, "xver-my-decoset"),
            other => panic!("expected a Let binding for the deco-set wrapper, got {other:?}"),
        }
    }

    #[test]
    fn deco_coercion_prelude_empty_is_empty() {
        assert!(deco_coercion_prelude(&[]).is_empty());
    }

    // ------------------------------------------------------------------
    // X4b: reject_deco_exports_v01_sig (the reverse direction's finding —
    // detect-and-reject, not classify-and-wrap; see that function's own
    // doc comment for why no sound wrap exists here)
    // ------------------------------------------------------------------

    fn v1_file(src: &str) -> cst_v1::FileV1 {
        cst_v1::parse_file_v1(src).expect("parse v1 fixture")
    }

    #[test]
    fn reject_deco_exports_v01_sig_rejects_bare_sig_val() {
        // The ONE shape 0.1's grammar can express a bare `deco` ascription
        // at all: a top-level module's own `sig val name : deco` item
        // (`cst_v1::Bind::Value` has no ascription syntax of its own) —
        // still rejected (no sound wrap exists, this function's doc
        // comment), now with a specific X4b diagnostic.
        let file = v1_file("module M :> sig\n  val my-deco : deco\nend = struct\n  val my-deco = 0\nend\n");
        let err =
            reject_deco_exports_v01_sig(&file).expect_err("a module sig `val : deco` item must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn reject_deco_exports_v01_sig_rejects_bare_sig_decoset() {
        let file = v1_file("module M :> sig\n  val my-decoset : deco-set\nend = struct\n  val my-decoset = 0\nend\n");
        let err = reject_deco_exports_v01_sig(&file)
            .expect_err("a module sig `val : deco-set` item must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco-set"),
        }
    }

    #[test]
    fn reject_deco_exports_v01_sig_rejects_curried_sig_val() {
        let file =
            v1_file("module M :> sig\n  val my-deco : length -> deco\nend = struct\n  val my-deco t p w h d = 0\nend\n");
        let err = reject_deco_exports_v01_sig(&file).expect_err("a curried sig `val` type must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn reject_deco_exports_v01_sig_rejects_nested_module() {
        let file = v1_file(
            "module Outer :> sig\n  module Inner : sig val my-deco : deco end\nend = struct\n  \
             module Inner :> sig val my-deco : deco end = struct val my-deco = 0 end\nend\n",
        );
        let err =
            reject_deco_exports_v01_sig(&file).expect_err("a NESTED module's sig `val : deco` item must still reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn reject_deco_exports_v01_sig_ignores_type_only_mention() {
        // A transparent `type .. = deco` sig item merely NAMING
        // `deco`/`deco-set` (no value attached) is safe with zero coercion
        // — this function must not reject it.
        let file = v1_file(
            "module M :> sig\n  type xver-deco-alias = deco\nend = struct\n  type xver-deco-alias = deco\nend\n",
        );
        assert!(reject_deco_exports_v01_sig(&file).is_ok());
    }

    #[test]
    fn reject_deco_exports_v01_sig_ok_for_no_sig() {
        let file = v1_file("module M = struct\n  val my-deco p w h d = 0\nend\n");
        assert!(
            reject_deco_exports_v01_sig(&file).is_ok(),
            "an UNSEALED module (no sig_annot at all) has no textual site for this check to see"
        );
    }

    #[test]
    fn reject_deco_exports_v01_sig_ok_for_document() {
        let file = v1_file("0\n");
        assert!(reject_deco_exports_v01_sig(&file).is_ok(), "a document is never a dependency");
    }

    #[test]
    fn reject_deco_exports_v01_sig_does_not_perturb_forward_toplevel_letrec() {
        // Non-regression: X4b's addition must not change the FORWARD
        // direction's existing top-level `let-rec` acceptance path at all
        // (a wholly separate function operating on a wholly separate CST
        // type — `cst_v1::FileV1`, not `cst::TopBinding`).
        let prelude = prelude_of("let-rec xver-my-deco : deco | (x, y) w h d = []\n0\n");
        let exports =
            classify_deco_exports(&prelude, v006(), v01()).expect("bare `: deco` must still be accepted forward");
        assert_eq!(exports.len(), 1);
    }
}
