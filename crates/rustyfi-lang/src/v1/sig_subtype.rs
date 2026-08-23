//! The subsumption algorithm — "does the module's inferred
//! scheme subsume the declared one" (upstream `subtype_poly_type`,
//! `signatureSubtyping.ml:530-553`). Pure over the existing public type
//! machinery — no new unifier, no `types.rs`/`unify.rs` edits.
//!
//! **Encoding (skolemize-by-lowering).** The declared side's
//! quantified variables are pre-lowered to rigid nominal stamps (`"'a#7"`,
//! by [`crate::v1::module_check`]'s seal-table walk, NOT here); this module
//! only instantiates the inferred scheme with fresh FLEXIBLE variables and
//! `unify`s once. Success means subsumption; the rigid stamps make
//! "declared more general than inferred" fail exactly like upstream's
//! intern-refusal does (a rigid stamp can never unify with a concrete type
//! or with another distinctly-stamped rigid — only with a flexible
//! variable, which is exactly what an instantiated bound variable is).
//!
//! **The escaped-skolem check** (upstream's `PolyFree` rejection,
//! `signatureSubtyping.ml:411-413`). After a successful unify,
//! resolve-walk the ORIGINAL (un-instantiated) inferred scheme's body
//! looking for any `MonoType::Variant` name ending in this check's own
//! stamp marker. `instantiate` never touches the inferred scheme's own
//! *quantified* variable cells (it substitutes them for FRESH copies in the
//! returned tree, leaving the originals free) — so if a rigid stamp shows
//! up when re-resolving the original body, it can only have gotten there by
//! binding one of the scheme's SHARED (free, unquantified) variables during
//! this very unify call, i.e. the module's value is *less polymorphic* than
//! its signature claims (e.g. a `val mutable` cell declared with a
//! quantifier).

use crate::types::{instantiate, resolve, resolve_row, MonoType, PolyType, Row, TypeContext};
use crate::unify::{unify, UnifyError};
use rustyfi_syntax::cst_v1::ast as ast_v1;
use rustyfi_syntax::span::Span;

/// Why `inferred` failed to subsume `declared_rigid`.
#[derive(Debug)]
pub(crate) enum SubsumeError {
    /// Structural mismatch / declared-more-general — the `unify` failure
    /// itself; its `Display` already renders both offending types.
    Mismatch(UnifyError),
    /// A rigid stamp leaked into the inferred scheme's SHARED (free,
    /// unquantified) variables during the check — see this module's doc
    /// comment.
    EscapedSkolem,
}

/// Does `inferred` (a module member's ACTUAL scheme, from
/// [`crate::typecheck::Checker::infer_binding`]) subsume `declared_rigid`
/// (a signature `val`'s declared body, with its own quantified variables
/// pre-lowered to rigid stamps sharing the suffix `stamp_marker`, e.g.
/// `"#7"`)? `ctx` is only consulted for its current level (the level
/// `instantiate` mints fresh copies at) — see [`TypeContext::level`].
pub(crate) fn val_subsumes(
    ctx: &mut TypeContext,
    inferred: &PolyType,
    declared_rigid: &MonoType,
    stamp_marker: &str,
) -> Result<(), SubsumeError> {
    let level = ctx.level();
    let instantiated = instantiate(inferred, level);
    unify(declared_rigid, &instantiated).map_err(SubsumeError::Mismatch)?;
    if mono_mentions_stamp(inferred.body(), stamp_marker) {
        return Err(SubsumeError::EscapedSkolem);
    }
    Ok(())
}

/// Resolve-walk `ty` looking for any `Variant` name ending in `marker`
/// (`"#7"`) — the escaped-skolem probe, and (structurally) the v1 twin of
/// `typecheck.rs`'s `collect_generalizable`.
fn mono_mentions_stamp(ty: &MonoType, marker: &str) -> bool {
    match &*resolve(ty) {
        MonoType::Var(_) | MonoType::Base(_) => false,
        MonoType::Func(row, a, b) => {
            row_mentions_stamp(&row, marker)
                || mono_mentions_stamp(&a, marker)
                || mono_mentions_stamp(&b, marker)
        }
        MonoType::Product(ts) => ts.iter().any(|t| mono_mentions_stamp(t, marker)),
        MonoType::List(t) | MonoType::Ref(t) | MonoType::Code(t) => mono_mentions_stamp(&t, marker),
        MonoType::Record(row) => row_mentions_stamp(&row, marker),
        MonoType::Variant(name, args) => {
            name.ends_with(marker) || args.iter().any(|t| mono_mentions_stamp(t, marker))
        }
        // Both `MonoType`-bearing fields of a slot — a skolem escaping
        // through a `?(l : τ)` optional-label bundle used to slip past this
        // probe silently, which is a sealing-soundness hole rather than a
        // crash. Structurally the derived traversal in `crate::visit` would
        // cover it, but this walk `resolve`s at every level and so cannot
        // use it.
        MonoType::InlineCmd(cs) | MonoType::BlockCmd(cs) | MonoType::MathCmd(cs) => {
            cs.iter().any(|c| {
                c.opt_labels
                    .iter()
                    .any(|(_, t)| mono_mentions_stamp(t, marker))
                    || mono_mentions_stamp(&c.ty, marker)
            })
        }
    }
}

fn row_mentions_stamp(row: &Row, marker: &str) -> bool {
    match &*resolve_row(row) {
        Row::Empty | Row::Var(_) => false,
        Row::Cons(_, t, rest) => {
            mono_mentions_stamp(&t, marker) || row_mentions_stamp(&rest, marker)
        }
    }
}

// ============================================================================
// Functor SIGNATURE subtyping errors — a structurally
// distinct concern from ordinary VALUE subsumption ([`SubsumeError`],
// above), reached through functor sig-members (`Decl::Module { Make :
// (K:S)->… }` in a `:> sig`) and the codomain-abstraction substitution. The
// one shape upstream leaves undefined gets a TYPED rejection here.
// ============================================================================

/// Errors from functor signature subtyping. A separate enum from
/// [`SubsumeError`]: functor-sig subtyping is a recursive structural match
/// over domain/codomain pairs, not a single `unify` call, so its error shape
/// grows independently (domain contravariance, codomain covariance, arity
/// mismatches, …).
#[derive(Debug)]
pub(crate) enum SigSubtypeError {
    /// Upstream `signatureSubtyping.ml`'s `substitute_concrete` `ConcFunctor`
    /// arm (`dev-0-1-0:116`'s `failwith "TODO: substitute_concrete,
    /// closure"`; the heavier `substitute_closure` on `saphe-split`,
    /// flagged "TODO: remove this … Bochao–Ohori style static
    /// interpretation"). Reached when a functor sig-member's declared
    /// CODOMAIN is itself a functor signature (`(Y:S2)->S3`, a curried/
    /// higher-order functor member) — this port's first-order functor support
    /// never constructs such a signature (no demand package is
    /// higher-order), so this is a DEFINED rejection for the one shape
    /// upstream leaves undefined, never a panic/`failwith`.
    NestedFunctorSubstitution { span: Span },
}

/// The codomain-shape guard, driven both at functor
/// sig-member REGISTRATION time (`module_check.rs`'s `Decl::Module`-functor
/// arm) and at APPLICATION-substitution time (the per-application
/// abstract-result sealing). `cod` is the functor
/// member's declared codomain (already substituted `[param := arg]` at the
/// application site, or un-substituted at registration time — the shape
/// guard is substitution-invariant). A `SigExpr::Functor` codomain (curried/
/// higher-order) refuses with [`SigSubtypeError::NestedFunctorSubstitution`]
/// rather than recurse into upstream's undefined `failwith`; every other
/// codomain shape (`Bot`/`WithType`) is fine at THIS guard (their own
/// resolution/width checks happen elsewhere).
pub(crate) fn substitute_result_sig(
    cod: &ast_v1::SigExpr,
    span: Span,
) -> Result<(), SigSubtypeError> {
    match cod {
        ast_v1::SigExpr::Functor { .. } => Err(SigSubtypeError::NestedFunctorSubstitution { span }),
        ast_v1::SigExpr::Bot(_) | ast_v1::SigExpr::WithType { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{self, TyVarRef};

    fn ctx() -> TypeContext {
        TypeContext::new()
    }

    fn rigid(tvname: &str, marker: &str) -> MonoType {
        MonoType::Variant(format!("'{tvname}{marker}"), Vec::new())
    }

    /// Identity: a monomorphic `int` inferred scheme subsumes a
    /// monomorphic `int` declared type — no tyvars, no stamps involved.
    #[test]
    fn identity_mono_accepts() {
        let mut c = ctx();
        let inferred = PolyType::mono(MonoType::Base(types::BaseType::Int));
        let declared = MonoType::Base(types::BaseType::Int);
        assert!(val_subsumes(&mut c, &inferred, &declared, "#0").is_ok());
    }

    /// Specialize: `∀a. a -> a` subsumes the declared `int -> int` (the
    /// module's polymorphic value is used at a specific instance) — the
    /// unit-level twin of the equivalent end-to-end test.
    #[test]
    fn specialize_accepts() {
        let mut c = ctx();
        let v: TyVarRef = types::new_ty_var(1);
        let body = crate::prim_types::arrow(MonoType::Var(v.clone()), MonoType::Var(v.clone()));
        let inferred = PolyType::from_vars(vec![v], Vec::new(), body);
        let declared = crate::prim_types::arrow(
            MonoType::Base(types::BaseType::Int),
            MonoType::Base(types::BaseType::Int),
        );
        assert!(val_subsumes(&mut c, &inferred, &declared, "#0").is_ok());
    }

    /// Generalize-rejection: a monomorphic `int -> int` inferred scheme
    /// does NOT subsume a declared `'a -> 'a` (the signature claims MORE
    /// polymorphism than the implementation has) — the unit-level twin of
    /// the equivalent end-to-end test.
    #[test]
    fn generalize_rejection_fails() {
        let mut c = ctx();
        let inferred = PolyType::mono(crate::prim_types::arrow(
            MonoType::Base(types::BaseType::Int),
            MonoType::Base(types::BaseType::Int),
        ));
        let declared = crate::prim_types::arrow(rigid("a", "#3"), rigid("a", "#3"));
        let err = val_subsumes(&mut c, &inferred, &declared, "#3").unwrap_err();
        assert!(matches!(err, SubsumeError::Mismatch(_)), "{err:?}");
    }

    /// Skolem-escape: a `val mutable`-shaped inferred scheme (mono, with a
    /// SHARED — not quantified — free variable inside a `ref`) checked
    /// against a declared `('a list) ref` binds that free variable to the
    /// rigid stamp during unify, which the escape check must catch — the
    /// unit-level twin of the equivalent end-to-end test.
    #[test]
    fn skolem_escape_detected() {
        let mut c = ctx();
        let shared = types::new_ty_var(0);
        let inferred = PolyType::mono(crate::prim_types::reff(crate::prim_types::list(
            MonoType::Var(shared),
        )));
        let declared = crate::prim_types::reff(crate::prim_types::list(rigid("a", "#5")));
        let err = val_subsumes(&mut c, &inferred, &declared, "#5").unwrap_err();
        assert!(matches!(err, SubsumeError::EscapedSkolem), "{err:?}");
    }

    /// A DIFFERENT check's stamp marker must not false-positive the escape
    /// check — per-`DeclaredVal` stamp uniqueness (module_check.rs's
    /// seal-table walk mints one marker per decl) is what makes the suffix
    /// test exact.
    #[test]
    fn unrelated_stamp_marker_does_not_escape() {
        let c = ctx();
        let shared = types::new_ty_var(0);
        let inferred = PolyType::mono(crate::prim_types::reff(crate::prim_types::list(
            MonoType::Var(shared),
        )));
        let declared = crate::prim_types::reff(crate::prim_types::list(rigid("a", "#5")));
        // Bind against "#5" but probe for "#6" — the shared var DID capture
        // "#5", but that isn't THIS (hypothetical other) check's marker.
        let level = c.level();
        let instantiated = instantiate(&inferred, level);
        unify(&declared, &instantiated).expect("unify should succeed structurally");
        assert!(!mono_mentions_stamp(inferred.body(), "#6"));
    }

    /// Interleaved mint draws for the
    /// ABSTRACT-TYPE stamp shape (`"M.t#1"`, `module_check.rs`'s opaque
    /// stamping — distinct from this test file's own skolemized-tyvar
    /// shape `"'a#7"`, but the exact same suffix-probe mechanism). A
    /// module member whose inferred scheme mentions an abstract-type stamp
    /// minted by an EARLIER, unrelated `Decl::TypeOpaque` (`"M.t#1"`) must
    /// not false-positive THIS val decl's own escape check (marker `"#2"`,
    /// a later, distinct draw) — the same per-draw-uniqueness argument as
    /// [`unrelated_stamp_marker_does_not_escape`], made empirical for the
    /// shape this port actually mints.
    #[test]
    fn interleaved_abstract_type_stamp_does_not_escape() {
        let mut c = ctx();
        let inferred = PolyType::mono(crate::prim_types::arrow(
            MonoType::Base(types::BaseType::Unit),
            MonoType::Variant("M.t#1".to_string(), Vec::new()),
        ));
        let declared = crate::prim_types::arrow(
            MonoType::Base(types::BaseType::Unit),
            MonoType::Variant("M.t#1".to_string(), Vec::new()),
        );
        // THIS check's own marker ("#2") is a DIFFERENT draw than the one
        // baked into the inferred scheme's nominal ("#1") — subsumption
        // must still succeed (structural/nominal match) with no escape.
        assert!(val_subsumes(&mut c, &inferred, &declared, "#2").is_ok());
    }

    /// Parse a library's `:> sig … end` umbrella and return its first
    /// `Decl::Module`'s declared type — used by both tests below to reach a
    /// real, parsed `SigExpr::Functor` codomain without needing this module
    /// to hand-construct the (span-heavy) AST directly.
    fn first_decl_module_sig(src: &str) -> ast_v1::SigExpr {
        use rustyfi_syntax::{cst_v1, parse_file_v1};
        let file = parse_file_v1(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
        let cst_v1::FileV1::Library { sig_annot, .. } = &file else {
            panic!("expected a library")
        };
        let ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Sig { decls, .. }) =
            &*sig_annot.as_ref().unwrap().sig_.0
        else {
            panic!("expected an inline `sig … end` umbrella")
        };
        let ast_v1::Decl::Module { sig_, .. } = &*decls[0].0 else {
            panic!("expected the umbrella's first decl to be a `Decl::Module`")
        };
        (**sig_).clone()
    }

    /// A curried
    /// functor-sig MEMBER (`module F : (X : S) -> (Y : S2) -> S3`) parsed
    /// off a real umbrella, driven through `substitute_result_sig`, yields
    /// the typed rejection.
    #[test]
    fn nested_functor_substitution_is_defined_and_live() {
        let sig = first_decl_module_sig(
            "module M :> sig\n\
             module F : (X : S) -> (Y : S2) -> S3\n\
             end = struct\n\
             module F = fun (X : S) -> struct end\n\
             end",
        );
        let ast_v1::SigExpr::Functor { cod, .. } = &sig else {
            panic!("expected a functor sig")
        };
        let err = substitute_result_sig(cod, Span::default()).unwrap_err();
        assert!(matches!(
            err,
            SigSubtypeError::NestedFunctorSubstitution { .. }
        ));
    }

    /// The non-nested (ordinary) codomain shape is unaffected — the guard
    /// only fires on the higher-order shape.
    #[test]
    fn ordinary_codomain_is_not_rejected() {
        let sig = first_decl_module_sig(
            "module M :> sig\n\
             module F : (X : S) -> S2\n\
             end = struct\n\
             module F = fun (X : S) -> struct end\n\
             end",
        );
        let ast_v1::SigExpr::Functor { cod, .. } = &sig else {
            panic!("expected a functor sig")
        };
        assert!(substitute_result_sig(cod, Span::default()).is_ok());
    }
}
