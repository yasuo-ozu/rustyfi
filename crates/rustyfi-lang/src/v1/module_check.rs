//! The per-binding module-checking driver — the V0_1 replacement for
//! `typecheck::typecheck_with_version` at `lib.rs`'s V0_1 pipeline (upstream
//! analogue: `moduleTypechecker.ml:596 typecheck_module` +
//! `coerce_signature:375`, collapsed onto the flat-spine model `elaborate.rs`
//! produces).
//!
//! **A spine walk keyed by a cst_v1-derived seal table.** Signature info
//! comes from the `cst_v1` trees (the only place `:>` survives — `v1/lower.rs`
//! erases it); the expressions checked are the elaborated spine's
//! (`program.body`'s `Let`-chain). [`check_program`]'s phases (each phase
//! function documents its own detail):
//!
//! - **Phase A** ([`phase_a_prescan`]/[`prescan_seal_types`]): syntactic seal
//!   pre-scan, no [`Checker`] yet — mints stamps for opaque types, queues
//!   transparent-type equality checks for phase C, and builds the
//!   hidden-type/ctor-hide tables. Vals are deferred to phase C.
//! - **Phase B** ([`maybe_rewrite_program_types`]): session setup, rewriting
//!   every top-level type decl through [`rename_type_expr`] so it can't
//!   resolve straight through a seal — skipped (an empty-map fast path) when
//!   phase A sealed nothing.
//! - **Phase C** ([`phase_c_finish`]): resolves the queued transparent-type
//!   checks, then checks each seal's val/command members
//!   ([`process_seal_member`]) and inserts them into `seals`.
//! - **Phase D** (the main loop in [`check_program`]): the spine walk — each
//!   `Ast::LetIn` alias is looked up in `seals`/`hidden` and either sealed,
//!   hidden, or committed ordinarily, with any deferred ctor-hide fired right
//!   after (sound because elaboration emits every member's alias
//!   contiguously in source order).
//!
//! **Sealing has zero runtime residue**: this module reads `cst_v1` purely
//! for type-checking; the elaborated/compiled/evaluated program never
//! differs from its unsealed twin.
//!
//! **Every outgoing message is stamp-stripped** ([`strip_stamps`]), once, at
//! [`check_program`]'s own boundary, so no `#N` stamp reaches user-facing
//! text regardless of which phase produced the error.
//!
//! Named signatures resolve at every ascription site ([`resolve_sig`]);
//! opaque types stamp FRESH per site (generativity — pinned by a
//! dedicated `v01_sealing.rs` test). Module aliases are expanded entirely in
//! `v1/lower.rs`, so an unsealed alias's copies just type-check as ordinary
//! bindings here.
//!
//! Further pieces with their own doc comments at point of use: nested
//! module/signature sig-members ([`ImplView`], [`PendingLink`],
//! [`handle_nested_module_decl`]); functor sig-members and their
//! per-application instantiation ([`StaticEnv::sealed_functors`],
//! [`InstantiatedApp`]); struct `include` bookkeeping
//! ([`struct_member_names_spliced`], [`build_impl_type_table`]);
//! sig-side `include`/`with type` ([`resolve_sig`], [`check_sig_conflicts`]).
//!
//! **Still explicitly out of scope** — permanent, sound placeholders:
//! semantic (alpha-variant) signature equality; structural functor-domain
//! subtyping; higher-order functors
//! ([`sig_subtype::SigSubtypeError::NestedFunctorSubstitution`]); a
//! functor-sig ascription directly on a struct bind; `Decl::Module`/
//! `Signature` inside a functor parameter signature; relative-sibling
//! references inside signature bodies; `include` of a sealed functor's
//! application result.

use crate::ast::branded::Ast;
use crate::elaborate::{Program, UserSynonymDecl, UserTypeDecl};
use crate::typecheck::{self, BindingView, Checker, MatchWarning, TypeError};
use crate::types::{self, MonoType, PolyType};
use crate::unify::unify;
use crate::v1::functor;
use crate::v1::lower::{self, TypeNameEnv};
use crate::v1::sig_subtype::{self, SigSubtypeError, SubsumeError};
use crate::v1::static_env::{
    DeclaredType, DeclaredVal, HiddenCtor, SealedFunctorSig, StampMint, StaticEnv, TypeOpacity,
};
use crate::v1::surface::{self, ModSurface, SurfaceEnv};
use rustyfi_syntax::cst;
use rustyfi_syntax::cst_v1::{self, ast as ast_v1};
use rustyfi_syntax::leaf::{AnyHorzCmdTok, AnyVertCmdTok, TypeVarTok, VarTok};
use rustyfi_syntax::span::Span;
use rustyfi_syntax::RustyfiVersion;
use std::collections::HashMap;

/// Check one whole elaborated V0_1 program, per binding, enforcing every
/// `:>` seal found in `deps` (the original `cst_v1` trees). Returns the same
/// warnings the whole-program path would; errors are ordinary
/// `typecheck::TypeError`s (pub fields) so `lib.rs`'s
/// `CompileError::Type(#[from])` covers them unchanged. Stamp-strips every
/// outgoing error message (see the module doc comment) — the one thing this
/// wrapper adds over [`check_program_inner`].
pub(crate) fn check_program<'s>(
    deps: &[&cst_v1::FileV1],
    program: &Program<'s>,
) -> Result<Vec<MatchWarning>, TypeError> {
    check_program_with_xver_shadows(deps, program, &std::collections::HashSet::new())
}

/// [`check_program`] plus the reverse deco/paren cross-version coercion exemption.
///
/// `xver_shadows` names qualified members (`"M.frame"`) `lib.rs`'s reverse
/// splice arm has REBOUND, after the exporting module closed, to a
/// version-adapted view of its own export
/// (`v1::xver_adapt::deco_downgrade_prelude`). That rebinding is a SECOND
/// `Ast::LetIn` under a name phase D already has a `seals` entry for, and
/// the whole point is to have a DIFFERENT shape from the sealed one — so it
/// must not be re-checked against the exporter's signature.
///
/// The exemption is minimal and cannot silence a genuine violation: it
/// applies only to the SECOND-and-later `Ast::LetIn` of a listed name (the
/// FIRST — the module's own `export_alias` — is still conformance-checked);
/// the exempted binding still commits its own INFERRED scheme (fully
/// HM-checked, just unsealed); and the set is empty for every caller but the
/// reverse cross-version arm, so no other compile path changes behaviour.
pub(crate) fn check_program_with_xver_shadows<'s>(
    deps: &[&cst_v1::FileV1],
    program: &Program<'s>,
    xver_shadows: &std::collections::HashSet<String>,
) -> Result<Vec<MatchWarning>, TypeError> {
    check_program_inner(deps, program, xver_shadows).map_err(strip_stamps_error)
}

fn check_program_inner<'s, 'a>(
    deps: &'a [&'a cst_v1::FileV1],
    program: &Program<'s>,
    xver_shadows: &std::collections::HashSet<String>,
) -> Result<Vec<MatchWarning>, TypeError> {
    // The syntactic surface + named-signature table, rebuilt
    // from the same `deps` `v1/lower.rs` builds its own copy from — feeds
    // named-signature resolution and alias-body width surfaces (phase C).
    let mut surfaces = SurfaceEnv::default();
    for file in deps.iter().copied() {
        surface::build_file_surface(file, &mut surfaces);
    }

    // ---- phase A: syntactic seal pre-scan (no `Checker`) ----
    let mut static_env = StaticEnv::default();
    let mut mint = StampMint::default();
    let mut immediate_hides: Vec<(String, String)> = Vec::new();

    // An early pass discovering every functor sig-member
    // (`sealed_functors`/`hidden_functors`) before `check_functor_
    // applications`/phase A0 need them; `phase_a_prescan` re-registers
    // `sealed_functors` idempotently later.
    discover_sealed_functors(deps, &surfaces, &mut static_env)?;

    // The functor-application parameter-signature check,
    // purely off `surfaces` — deliberately not reusing the seal machinery's
    // `Checker`/`StampMint` state (see this function's own doc comment).
    check_functor_applications(&surfaces, &static_env)?;

    // Materialize every sealed-functor application's
    // substituted codomain + body before `pending`/`links`, which borrow
    // from this store (never the other way).
    let inst_store = build_instantiations(deps, &surfaces, &static_env)?;

    let (pending, links) = phase_a_prescan(
        deps,
        &surfaces,
        &inst_store,
        &mut mint,
        &mut static_env,
        &mut immediate_hides,
    )?;

    // ---- phase B: session setup with the external-reference rewrite ----
    let mut ck = Checker::empty(program.store);
    ck.set_version(RustyfiVersion::V0_1);
    let rewritten = maybe_rewrite_program_types(program, &static_env)?;
    match &rewritten {
        Some((synonym_decls, type_decls)) => {
            for usd in synonym_decls {
                ck.declare_synonym(usd)?;
            }
            ck.check_cycles()?;
            ck.install_builtin_variants(RustyfiVersion::V0_1);
            for utd in type_decls {
                ck.declare_variant(utd)?;
            }
        }
        None => {
            for usd in &program.synonym_decls {
                ck.declare_synonym(usd)?;
            }
            ck.check_cycles()?;
            ck.install_builtin_variants(RustyfiVersion::V0_1);
            for utd in &program.type_decls {
                ck.declare_variant(utd)?;
            }
        }
    }
    if !immediate_hides.is_empty() {
        ck.hide_ctors(&immediate_hides);
    }

    // ---- phase C: the seal-table val/type half ----
    phase_c_finish(&pending, &links, &mut ck, &mut mint, &mut static_env)?;

    // ---- phase D: the spine walk with interception ----
    // `seals`/`hidden`/`ctor_hide_triggers` are keyed by member-name TEXT
    // (built from signature decls, not the elaborated tree), so the spine
    // walk resolves each binder symbol back to text to probe them.
    let store = program.store;
    let mut env = typecheck::base_type_env_with_version(store, RustyfiVersion::V0_1);
    let mut ast: &Ast<'s> = &program.body;
    // Names whose ORIGINAL alias is already sealed, so a later
    // rebinding is the cross-version coercion shadow (see
    // `check_program_with_xver_shadows`).
    let mut xver_sealed_once: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        ast = match ast {
            Ast::LetIn(name, value, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::Let { name: *name, value }),
                    &static_env,
                )?;
                let name_text = store.resolve(*name);
                let is_xver_shadow = xver_sealed_once.contains(name_text);
                // 0.1's per-binding `val ~x` / `val persistent ~x` reaches
                // here as an `Ast::StageScope` on the RHS; the sealed arm
                // below commits the SIGNATURE's scheme but the binding is
                // still the same binding, so it keeps its own stage.
                let bind_stage = ck.binding_stage(value);
                env = match static_env.seals.get(name_text) {
                    // The cross-version coercion REBINDING of an
                    // already-sealed member — commit its own inferred scheme
                    // (the version-adapted view), do NOT re-check it against
                    // the exporter's signature.
                    Some(_) if is_xver_shadow => env.with_all(schemes, bind_stage),
                    // the alias binding of a SEALED member: subsumption-
                    // check, then commit the DECLARED scheme (sealing).
                    Some(decl) => {
                        // The STAGE half of conformance, checked before the
                        // type half exactly as upstream orders it
                        // (`signatureSubtyping.ml:286-298`): `sig val ~c :
                        // int end = struct val c = 1 end` has matching
                        // types but the wrong stage — type-first would
                        // report nothing here, the gap this closes.
                        if !stage_conforms(bind_stage, decl.stage) {
                            return Err(stage_mismatch_error(name_text, decl, bind_stage));
                        }
                        let (_, inferred) = &schemes[0];
                        sig_subtype::val_subsumes(
                            ck.ctx_mut(),
                            inferred,
                            &decl.rigid,
                            &decl.stamp_marker,
                        )
                        .map_err(|e| seal_mismatch_error(name_text, decl, inferred, e))?;
                        if xver_shadows.contains(name_text) {
                            xver_sealed_once.insert(name_text.to_string());
                        }
                        env.with(*name, decl.scheme.clone(), bind_stage)
                    }
                    // the alias binding of a HIDDEN member: commit NOTHING.
                    None if static_env.hidden.contains_key(name_text) => env,
                    // every ordinary binding (locals, unsealed aliases,
                    // opens).
                    None => env.with_all(schemes, bind_stage),
                };
                // This alias may be a deferred
                // ctor-hide trigger (the sealing module's LAST value
                // member) — fire it AFTER the commit above, so the
                // module's own members (which just finished checking)
                // still saw the concrete ctors.
                if let Some(hides) = static_env.ctor_hide_triggers.get(name_text) {
                    ck.hide_ctors(hides);
                }
                // This alias may also be a deferred
                // parent-imposed member-revocation trigger — fired AFTER the
                // commit above (so the child's own members still saw the
                // un-narrowed bindings while checking), and only inserted
                // into `hidden` now, or it would trip the skip-commit arm
                // above and break sibling visibility.
                if let Some((owner, revoked)) =
                    static_env.member_revoke_triggers.get(name_text).cloned()
                {
                    let revoked_syms: Vec<_> = revoked.iter().map(|r| store.intern(r)).collect();
                    env = env.without_all(&revoked_syms);
                    for r in &revoked {
                        static_env.hidden.insert(r.clone(), owner.clone());
                    }
                }
                body
            }
            Ast::LetMathIn(name, value, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::LetMath { name: *name, value }),
                    &static_env,
                )?;
                env = env.with_all(schemes, ck.binding_stage(value));
                body
            }
            Ast::LetRecIn(bindings, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::LetRec(bindings)),
                    &static_env,
                )?;
                env = env.with_all(schemes, ck.binding_stage_rec(bindings));
                body
            }
            Ast::LetMutableIn(name, init, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::LetMutable { name: *name, init }),
                    &static_env,
                )?;
                env = env.with_all(schemes, ck.binding_stage(init));
                body
            }
            other => {
                catch_hidden(ck.infer_expr(&env, other), &static_env)?;
                break;
            }
        };
    }
    Ok(ck.take_warnings())
}

/// `Result` combinator threading every `infer_binding`/`infer_expr` call
/// through [`rewrite_hidden_error`].
fn catch_hidden<T>(r: Result<T, TypeError>, static_env: &StaticEnv) -> Result<T, TypeError> {
    r.map_err(|e| rewrite_hidden_error(e, static_env))
}

/// Any `TypeError` propagating out of the spine walk passes through this
/// filter. Five exact message formats are pinned, from `typecheck.rs`'s
/// call sites at the time of writing: the plain unbound-variable format
/// (`Ast::Var`, `:1564`) and the unbound-inline/-block-command formats
/// (`:1981`/`:2038`) consult [`StaticEnv::hidden`] (hidden COMMAND
/// members too); the two "unknown constructor" formats (`:1833`/`:1923`)
/// consult [`StaticEnv::hidden_ctors`]. Every other error passes
/// through unchanged.
fn rewrite_hidden_error(err: TypeError, static_env: &StaticEnv) -> TypeError {
    for (prefix, suffix) in [
        (
            "internal error: unbound variable '",
            "' reached the typechecker",
        ),
        (
            "internal error: unbound inline command '",
            "' reached the typechecker",
        ),
        (
            "internal error: unbound block command '",
            "' reached the typechecker",
        ),
    ] {
        if let Some(name) = err
            .message
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
        {
            if let Some(owner) = static_env.hidden.get(name) {
                return TypeError {
                    span: err.span,
                    message: format!(
                        "value `{name}` exists in module `{owner}` but is not exported by its signature"
                    ),
                    source: None,
                };
            }
        }
    }
    for suffix in ["'", "' in a pattern"] {
        if let Some(name) = err
            .message
            .strip_prefix("unknown constructor '")
            .and_then(|rest| rest.strip_suffix(suffix))
        {
            if let Some(hidden) = static_env.hidden_ctors.get(name) {
                return TypeError {
                    span: err.span,
                    message: format!(
                        "constructor `{name}` belongs to type `{}`, which module `{}`'s \
                         signature seals abstract",
                        local_name(&hidden.type_name),
                        hidden.module,
                    ),
                    source: None,
                };
            }
        }
    }
    err
}

/// A depth-mismatch (or declared-more-general, or escaped-skolem)
/// diagnostic for the sealed member `qualified` (e.g. `"M.x"`).
fn seal_mismatch_error(
    qualified: &str,
    decl: &DeclaredVal,
    inferred: &PolyType,
    err: SubsumeError,
) -> TypeError {
    let module_name = qualified.rsplit_once('.').map(|(m, _)| m).unwrap_or("");
    match err {
        SubsumeError::Mismatch(unify_err) => TypeError {
            span: Some(decl.span),
            message: format!(
                "module `{module_name}` does not match its signature: value `{}` \
                 has type {inferred} but its signature declares {}",
                decl.name, decl.scheme
            ),
            source: Some(unify_err),
        },
        SubsumeError::EscapedSkolem => TypeError {
            span: Some(decl.span),
            message: format!(
                "module `{module_name}` does not match its signature: value `{}` \
                 is less polymorphic than its signature declares (has type \
                 {inferred}, but the signature declares {})",
                decl.name, decl.scheme
            ),
            source: None,
        },
    }
}

/// The stage twin of [`seal_mismatch_error`]: the sealed member `qualified`
/// (e.g. `"M.c"`) is written at one stage and its signature declares another.
/// Both are named, and both are named the way the rest of the staging
/// diagnostics name a stage ([`types::Stage::as_str`]), so the reader can see
/// which `~` to add or drop.
fn stage_mismatch_error(
    qualified: &str,
    decl: &DeclaredVal,
    implemented: types::Stage,
) -> TypeError {
    let module_name = qualified.rsplit_once('.').map(|(m, _)| m).unwrap_or("");
    simple_error(
        Some(decl.span),
        format!(
            "module `{module_name}` does not match its signature: value `{}` is bound at \
             {} but its signature declares it at {}",
            decl.name,
            implemented.as_str(),
            decl.stage.as_str(),
        ),
    )
}

fn simple_error(span: Option<Span>, message: String) -> TypeError {
    TypeError {
        span,
        message,
        source: None,
    }
}

/// The bare local name off a qualified nominal (`"M.t"` → `"t"`) — used only
/// for diagnostic text.
fn local_name(qualified: &str) -> &str {
    qualified
        .rsplit_once('.')
        .map(|(_, t)| t)
        .unwrap_or(qualified)
}

/// Remove every maximal `#[0-9]+` run — `#` is unlexable in either
/// version's grammar, so this can only strip a stamp this module minted
/// (`StampMint`'s doc comment), never mangle a legitimate rendering.
fn strip_stamps(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '#' {
            let mut consumed = false;
            while matches!(chars.peek(), Some(d) if d.is_ascii_digit()) {
                chars.next();
                consumed = true;
            }
            if consumed {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// [`check_program`]'s outermost error transform: fold `source` (may itself
/// render a stamped nominal) into `message` before stripping, so the ENTIRE
/// diagnostic is stamp-free. Drops the `source` chain, but nothing in the
/// V0_1 pipeline inspects it beyond `Display`.
fn strip_stamps_error(e: TypeError) -> TypeError {
    let mut msg = e.message;
    if let Some(src) = &e.source {
        msg.push_str(&format!(": {src}"));
    }
    TypeError {
        span: e.span,
        message: strip_stamps(&msg),
        source: None,
    }
}

// ============================================================================
// Phase A: the syntactic seal pre-scan (no `Checker`).
// ============================================================================

/// An impl `type` bind's shape, syntactic-only (no lowering): a variant's
/// ctor names (for the ctor-hide list), or "it's a synonym" (nothing more
/// needed by phase A — phase C's `check_transparent_type` re-derives the
/// concrete body through `expand_synonyms_in`).
enum ImplTypeBody {
    Variant(Vec<String>),
    Synonym,
}

struct ImplTypeInfo {
    arity: usize,
    body: ImplTypeBody,
}

/// A sig `type t = τ` decl (synonym-bodied) queued in phase A for phase C's
/// checker-needed equality check — borrows straight from the `cst_v1` tree
/// (`deps` outlives the whole call), so no cloning is needed.
struct PendingTransparent<'a> {
    /// The type's qualified name, e.g. `"M.sz"`.
    qualified: String,
    /// The sig decl's own tyvars, e.g. `['a]` for `type t 'a = …`.
    quant: &'a [TypeVarTok],
    /// The declared body τ, not yet lowered.
    ty: &'a ast_v1::TypeExpr,
    span: Span,
}

/// One sealed module, fully pre-scanned by phase A, awaiting phase C's
/// checker-needed val/type-equality work.
struct PendingSeal<'a> {
    /// FLATTENED (every `Decl::Include` spliced away) and
    /// owned — a `resolve_sig` call may collect decls from more than one
    /// borrowed source, so there's no single contiguous slice to point at.
    sig_decls: Vec<&'a cst_v1::StructDeclV1>,
    mod_path: Vec<String>,
    tyenv: TypeNameEnv,
    module_name: String,
    pending_transparent: Vec<PendingTransparent<'a>>,
    /// Whether this seal's sig declares at least one `type`/`type ::` decl
    /// — phase C's self-containment enforcement is opt-in on this flag. A
    /// sig with ZERO type decls never restricted type visibility; once
    /// it declares one, any OTHER undeclared own-type reference is a real
    /// gap.
    declares_any_type: bool,
    /// Member-name lists, INCLUDE-SPLICED
    /// ([`struct_member_names_spliced`]) and computed ONCE in phase A —
    /// [`phase_c_finish`] reads these back rather than recomputing (which
    /// would miss included members), avoiding re-threading `surfaces` into
    /// phase C.
    value_names: Vec<String>,
    other_names: Vec<String>,
    /// `Some((trigger, owner))` for a SYNTHETIC child seal
    /// (an unsealed/alias/app-bodied child narrowed by an ENCLOSING seal,
    /// never its own `:>`) — undeclared members defer to `owner`'s
    /// `member_revoke_triggers`/`ctor_hide_triggers` under `trigger` instead
    /// of committing to `hidden`/this seal's own trigger immediately
    /// ("hiding is parent-deferred"). `None` for every real seal —
    /// immediate hiding.
    parent_trigger: Option<(String, String)>,
}

/// What a signature is checked AGAINST — a struct
/// literal's own binds, or an alias/coerce/application-result body's
/// already seal-filtered syntactic surface (`narrow_alias_body`). A functor
/// application's instantiated result reuses `Struct` over its OWNED
/// substituted binds — no third variant needed.
enum ImplView<'a> {
    Struct(Vec<&'a cst_v1::Bind>),
    Surface(&'a ModSurface),
}

fn impl_view_type_table(
    view: &ImplView,
    mod_path: &[String],
    surfaces: &SurfaceEnv,
) -> HashMap<String, ImplTypeInfo> {
    match view {
        ImplView::Struct(binds) => build_impl_type_table(binds, mod_path, surfaces),
        ImplView::Surface(surf) => surf
            .types
            .iter()
            .map(|(n, arity)| {
                (
                    n.clone(),
                    ImplTypeInfo {
                        arity: *arity,
                        body: ImplTypeBody::Synonym,
                    },
                )
            })
            .collect(),
    }
}

fn impl_view_member_names(
    view: &ImplView,
    mod_path: &[String],
    surfaces: &SurfaceEnv,
) -> (Vec<String>, Vec<String>) {
    match view {
        ImplView::Struct(binds) => struct_member_names_spliced(binds, mod_path, surfaces),
        ImplView::Surface(surf) => {
            let mut others: Vec<String> = surf.types.iter().map(|(n, _)| n.clone()).collect();
            others.extend(surf.mods.iter().map(|(n, _)| n.clone()));
            others.extend(surf.sigs.iter().cloned());
            (surf.vals.clone(), others)
        }
    }
}

/// The impl's own module and signature names, SEPARATELY
/// (distinct from `impl_view_member_names`'s conflated `other_names`) —
/// needed to width-check a `Decl::Module`/`Decl::Signature` member and to
/// compute `hidden_sigs`.
fn impl_view_mod_and_sig_names(view: &ImplView) -> (Vec<String>, Vec<String>) {
    match view {
        ImplView::Struct(binds) => {
            let mut mods = Vec::new();
            let mut sigs = Vec::new();
            for b in binds.iter().copied() {
                match b {
                    cst_v1::Bind::Module { name, .. } => mods.push(name.name.clone()),
                    cst_v1::Bind::Signature { name, .. } => sigs.push(name.name.clone()),
                    _ => {}
                }
            }
            (mods, sigs)
        }
        ImplView::Surface(surf) => (
            surf.mods.iter().map(|(n, _)| n.clone()).collect(),
            surf.sigs.clone(),
        ),
    }
}

/// What a `Decl::Module { N : S_N }` member's
/// implementation looks like, located by scanning the PARENT's own
/// [`ImplView`].
enum ChildModuleShape<'a> {
    /// An unsealed struct body (no own `:>`) — recurse with a SYNTHETIC
    /// (parent-imposed) child seal.
    UnsealedStruct(Vec<&'a cst_v1::Bind>),
    /// A struct body WITH its own `:>` (carrying its own struct binds, for
    /// building a correct child [`TypeNameEnv`]) — the child's own seal is
    /// processed by the ordinary top-down walk; only a [`PendingLink`] is
    /// needed here (no second `PendingSeal` at the same qualified path).
    SealedStruct(Vec<&'a cst_v1::Bind>),
    /// An alias/coerce/application-result body, resolved to its (possibly
    /// already seal-filtered) surface — recurse with a SYNTHETIC child seal
    /// over `ImplView::Surface`.
    ViaSurface(&'a ModSurface),
    /// A functor literal, or an unresolved/absent surface (an unresolved
    /// alias/app target already died in lowering before `check_program`
    /// ever runs — a defensive arm) — cannot recurse.
    Unavailable,
}

fn locate_child_module<'a>(
    view: &ImplView<'a>,
    child_name: &str,
    child_path: &[String],
    surfaces: &'a SurfaceEnv<'a>,
) -> ChildModuleShape<'a> {
    match view {
        ImplView::Struct(binds) => {
            for b in binds.iter().copied() {
                if let cst_v1::Bind::Module {
                    name,
                    sig_annot,
                    body,
                    ..
                } = b
                {
                    if name.name != child_name {
                        continue;
                    }
                    return match &*body.0 {
                        ast_v1::ModExpr::Struct { binds: inner, .. } => {
                            let inner_binds: Vec<&cst_v1::Bind> =
                                inner.iter().map(|sb| sb.0.as_ref()).collect();
                            if sig_annot.is_some() {
                                ChildModuleShape::SealedStruct(inner_binds)
                            } else {
                                ChildModuleShape::UnsealedStruct(inner_binds)
                            }
                        }
                        ast_v1::ModExpr::Var(_)
                        | ast_v1::ModExpr::Coerce { .. }
                        | ast_v1::ModExpr::App { .. } => {
                            match surfaces.modules.get(&child_path.join(".")) {
                                Some(surf) => ChildModuleShape::ViaSurface(surf),
                                None => ChildModuleShape::Unavailable,
                            }
                        }
                        ast_v1::ModExpr::Functor { .. } => ChildModuleShape::Unavailable,
                    };
                }
            }
            ChildModuleShape::Unavailable
        }
        ImplView::Surface(surf) => match surf.mods.iter().find(|(n, _)| n == child_name) {
            Some((_, child_surf)) => ChildModuleShape::ViaSurface(child_surf),
            None => ChildModuleShape::Unavailable,
        },
    }
}

fn phase_a_prescan<'a>(
    deps: &'a [&'a cst_v1::FileV1],
    surfaces: &'a SurfaceEnv<'a>,
    inst_store: &'a [InstantiatedApp],
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
) -> Result<(Vec<PendingSeal<'a>>, Vec<PendingLink<'a>>), TypeError> {
    let mut pending = Vec::new();
    let mut links = Vec::new();
    for file in deps.iter().copied() {
        let cst_v1::FileV1::Library {
            name,
            sig_annot,
            binds,
            ..
        } = file
        else {
            // A dependency is always a Library (the loader already rejects
            // anything else, mirroring `v1/lower.rs::lower_file_v1`'s own
            // defensive arm).
            continue;
        };
        let mod_path = vec![name.name.clone()];
        let bind_refs: Vec<&cst_v1::Bind> = binds.iter().collect();
        let tyenv = TypeNameEnv::default().child(&mod_path, bind_refs.iter().copied(), surfaces);
        if let Some(sa) = sig_annot {
            let resolved = resolve_sig(&sa.sig_.0, &mod_path.join("."), surfaces, &mod_path)?;
            prescan_seal_types(
                resolved.decls,
                &resolved.refines,
                ImplView::Struct(bind_refs.clone()),
                &mod_path,
                &tyenv,
                surfaces,
                mint,
                env,
                immediate_hides,
                None,
                &mut pending,
                &mut links,
            )?;
        }
        walk_nested_seals_a(
            &bind_refs,
            &mod_path,
            &tyenv,
            surfaces,
            mint,
            env,
            immediate_hides,
            &mut pending,
            &mut links,
        )?;
    }
    // One synthetic, immediately-hidden `PendingSeal` per
    // instantiated sealed-functor application (its codomain seal IS the
    // boundary). A `with type` there is rejected earlier, at
    // `collect_instantiations_in_binds`'s construction site (the store is
    // fully OWNED, so no `Refine<'a>` can ride along) — `cod_decls` is
    // already FLATTENED.
    for app in inst_store {
        let body_refs: Vec<&cst_v1::Bind> = app.body_binds.iter().collect();
        let cod_decls: Vec<&cst_v1::StructDeclV1> = app.cod_decls.iter().collect();
        prescan_seal_types(
            cod_decls,
            &[],
            ImplView::Struct(body_refs),
            &app.app_path,
            &app.tyenv,
            surfaces,
            mint,
            env,
            immediate_hides,
            None,
            &mut pending,
            &mut links,
        )?;
    }
    Ok((pending, links))
}

/// Recurse through every nested `Bind::Module { .. }` for further seals,
/// independent of whether THIS level is sealed.
///
/// A non-struct module body with its OWN seal (`module M
/// :> S = N`; `module M = N :> S`) is narrowed too, resolved to the
/// target's syntactic surface and fed through the SAME
/// `prescan_seal_types` pipeline. A `Coerce` body whose OUTER `sig_annot`
/// is ALSO present (`module M :> S1 = N :> S2`) is a SEAL CHAIN: the INNER
/// `S2` becomes the real `PendingSeal`, the OUTER `S1` a [`PendingLink`]
/// on top. An `App`/`Functor` body is out of scope here, left untouched.
#[allow(clippy::too_many_arguments)]
fn walk_nested_seals_a<'a>(
    binds: &[&'a cst_v1::Bind],
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    surfaces: &'a SurfaceEnv<'a>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
    pending: &mut Vec<PendingSeal<'a>>,
    links: &mut Vec<PendingLink<'a>>,
) -> Result<(), TypeError> {
    for b in binds.iter().copied() {
        let cst_v1::Bind::Module {
            name,
            sig_annot,
            body,
            ..
        } = b
        else {
            continue;
        };
        let mut child_path = mod_path.to_vec();
        child_path.push(name.name.clone());
        match &*body.0 {
            ast_v1::ModExpr::Struct { binds: inner, .. } => {
                let inner_binds: Vec<&cst_v1::Bind> =
                    inner.iter().map(|sb| sb.0.as_ref()).collect();
                let child_tyenv = tyenv.child(&child_path, inner_binds.iter().copied(), surfaces);
                if let Some(sa) = sig_annot {
                    let resolved =
                        resolve_sig(&sa.sig_.0, &child_path.join("."), surfaces, &child_path)?;
                    prescan_seal_types(
                        resolved.decls,
                        &resolved.refines,
                        ImplView::Struct(inner_binds.clone()),
                        &child_path,
                        &child_tyenv,
                        surfaces,
                        mint,
                        env,
                        immediate_hides,
                        None,
                        pending,
                        links,
                    )?;
                }
                walk_nested_seals_a(
                    &inner_binds,
                    &child_path,
                    &child_tyenv,
                    surfaces,
                    mint,
                    env,
                    immediate_hides,
                    pending,
                    links,
                )?;
            }
            ast_v1::ModExpr::Var(_) => {
                if let Some(sa) = sig_annot {
                    narrow_alias_body(
                        &sa.sig_.0,
                        &child_path,
                        tyenv,
                        surfaces,
                        mint,
                        env,
                        immediate_hides,
                        pending,
                        links,
                    )?;
                }
            }
            ast_v1::ModExpr::Coerce {
                sig_: inner_sig, ..
            } => {
                // Seal-chain rule: the `Coerce`'s OWN `sig_` is always the
                // innermost layer (the real `PendingSeal`); an OUTER
                // `sig_annot` is an additional link on top, only pushed
                // when the inner narrowing applied — else a link would
                // reference a seal that was never registered.
                let inner_applied = narrow_alias_body(
                    inner_sig,
                    &child_path,
                    tyenv,
                    surfaces,
                    mint,
                    env,
                    immediate_hides,
                    pending,
                    links,
                )?;
                if inner_applied {
                    if let Some(sa) = sig_annot {
                        if sig_is_literal_inline(&sa.sig_.0) {
                            let outer_resolved = resolve_sig(
                                &sa.sig_.0,
                                &child_path.join("."),
                                surfaces,
                                &child_path,
                            )?;
                            reject_link_refines(&outer_resolved.refines, &child_path.join("."))?;
                            links.push(PendingLink {
                                child_path: child_path.clone(),
                                // `M` itself is the "owner" (its OWN `:>`
                                // narrows over `N`'s) — unlike
                                // `handle_nested_module_decl`'s case, no
                                // grandparent imposes this.
                                parent_name: child_path.join("."),
                                decls: outer_resolved.decls,
                                tyenv: tyenv.child(&child_path, std::iter::empty(), surfaces),
                                span: name.span,
                                parent_trigger: None,
                            });
                        }
                    }
                }
            }
            ast_v1::ModExpr::App { .. } | ast_v1::ModExpr::Functor { .. } => {
                // An `App`-bodied module's own seal (if
                // any) is the per-application codomain seal (phase A0); a
                // direct functor-literal ascription is an unsupported
                // shape. Neither needs anything from this walk.
            }
        }
    }
    Ok(())
}

/// A literal `sig … end` body — the ONLY shape [`narrow_alias_body`]/
/// [`handle_nested_module_decl`]'s `ViaSurface` arm apply full narrowing to
/// (the scope guard documented below).
fn sig_is_literal_inline(s: &ast_v1::SigExpr) -> bool {
    matches!(s, ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Sig { .. }))
}

/// Resolve an alias/coerce body's TARGET to its (possibly
/// seal-filtered) syntactic surface and feed it through `prescan_seal_types`
/// as an `ImplView::Surface`. An unresolved target already died in
/// lowering — defensively skip, never invent.
///
/// Returns whether narrowing was applied (the `Coerce` caller uses this to
/// decide whether an outer seal-chain link is safe to register).
///
/// **Scope guard**: only applied to a LITERAL `sig … end`
/// (`sig_is_literal_inline`); a NAMED signature reference is left
/// un-narrowed here (too-permissive, but sound) — a named sig's own bare
/// type references need that sig's DEFINITION site's [`TypeNameEnv`], which
/// this module can't reconstruct for an arbitrary dependency module
/// (pinned by the `i9`/`i9b` regression tests). A literal inline `sig …
/// end` never has this problem: every type it references is either its own
/// declared member or an already-absolute name.
#[allow(clippy::too_many_arguments)]
fn narrow_alias_body<'a>(
    sig: &'a ast_v1::SigExpr,
    alias_path: &[String],
    tyenv: &TypeNameEnv,
    surfaces: &'a SurfaceEnv<'a>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
    pending: &mut Vec<PendingSeal<'a>>,
    links: &mut Vec<PendingLink<'a>>,
) -> Result<bool, TypeError> {
    if !sig_is_literal_inline(sig) {
        return Ok(false);
    }
    let Some(Some(target_path)) = surface::frozen_alias_target(surfaces, alias_path) else {
        return Ok(false);
    };
    // The TARGET's own surface, not `alias_path`'s own registered entry
    // (already seal-filtered by whichever annotation applies to the ALIAS
    // itself) — using the self-referential entry would apply the wrong (or
    // doubly-narrowed) filter here.
    let Some(target_surf) = surfaces.modules.get(target_path) else {
        return Ok(false);
    };
    let resolved = resolve_sig(sig, &alias_path.join("."), surfaces, alias_path)?;
    let child_tyenv =
        tyenv.child_from_names(alias_path, target_surf.types.iter().map(|(n, _)| n.clone()));
    prescan_seal_types(
        resolved.decls,
        &resolved.refines,
        ImplView::Surface(target_surf),
        alias_path,
        &child_tyenv,
        surfaces,
        mint,
        env,
        immediate_hides,
        None,
        pending,
        links,
    )?;
    Ok(true)
}

/// One parent-sig layer over a child that ALREADY has its own seal entries:
/// phase C checks inner ⊑ outer per member and REPLACES the committed
/// scheme with the outer one. Never registers a second `env.seals` entry at
/// the same key — that would silently clobber the first (the same
/// cross-contamination risk `check_functor_applications` refuses).
struct PendingLink<'a> {
    /// `"M.N"` as segments.
    child_path: Vec<String>,
    /// `"M"`, diagnostics.
    parent_name: String,
    /// `S_N` (or, for a seal chain, the OUTER `S1`), resolved and FLATTENED
    /// (same ownership note as [`PendingSeal::sig_decls`]). A non-empty
    /// `with type` refinement here is rejected at construction time — a
    /// link only re-checks `Decl::Val`-family members, never types (see
    /// [`reject_link_refines`]).
    decls: Vec<&'a cst_v1::StructDeclV1>,
    /// For lowering `S_N`'s val types at `M.N`.
    tyenv: TypeNameEnv,
    /// The `Decl::Module`/`Coerce`'s own span — diagnostics-reserved.
    #[allow(dead_code)]
    span: Span,
    /// The PARENT's deferred-revoke trigger key, `Some`
    /// when this link sits under a synthetic seal chain. `None` means
    /// omissions become this link's own module's `hidden` immediately.
    parent_trigger: Option<String>,
}

/// One materialized sealed-functor application — OWNED
/// substituted codomain decls + body binds for a per-application seal.
/// `PendingSeal`/`PendingLink` borrow FROM this store, declared before them
/// in `check_program_inner` (plain lexical outliving).
struct InstantiatedApp {
    app_path: Vec<String>,
    cod_decls: Vec<cst_v1::StructDeclV1>,
    body_binds: Vec<cst_v1::Bind>,
    tyenv: TypeNameEnv,
    #[allow(dead_code)]
    // diagnostics-reserved; every current error site names the member/functor directly.
    span: Span,
}

/// Resolve one `:> sig .. end` annotation's TYPE half against the struct
/// body it seals (phase A): width/kind/arity-check + stamp-mint every
/// `Decl::TypeOpaque`; width-check + queue every `Decl::Type`; hide every
/// un-named impl type; compute the ctor-hide list and its deferred trigger.
/// `Decl::Module`/`Decl::Signature` members recurse/compare (a
/// functor-typed `Decl::Module` dispatches elsewhere instead). `refines`
/// intercepts a matching `Decl::TypeOpaque`'s stamp mint —
/// the Abstract → Transparent rewrite BEFORE stamping.
#[allow(clippy::too_many_arguments)]
fn prescan_seal_types<'a>(
    decls: Vec<&'a cst_v1::StructDeclV1>,
    refines: &[surface::Refine<'a>],
    impl_view: ImplView<'a>,
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    surfaces: &'a SurfaceEnv<'a>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
    parent_trigger: Option<(String, String)>,
    pending: &mut Vec<PendingSeal<'a>>,
    links: &mut Vec<PendingLink<'a>>,
) -> Result<(), TypeError> {
    let module_name = mod_path.join(".");

    let impl_table = impl_view_type_table(&impl_view, mod_path, surfaces);
    let (value_names, other_names) = impl_view_member_names(&impl_view, mod_path, surfaces);
    let subtree_trigger = last_value_alias_in_subtree(&impl_view, mod_path, surfaces);

    let mut declared_types: Vec<String> = Vec::new();
    let mut declared_sigs: Vec<String> = Vec::new();
    let mut declared_functors: Vec<String> = Vec::new();
    let mut hide_list: Vec<(String, String)> = Vec::new();
    let mut pending_transparent: Vec<PendingTransparent<'a>> = Vec::new();
    // `refine` names a `TypeOpaque` decl actually consumed —
    // anything left over errors (undeclared name, or a second refine of an
    // already-transparent one). Tracked by INDEX, not name: two chained
    // refines of the SAME name are distinct entries, and only the FIRST is
    // consumed — the second falls through to the "leftover" check and
    // errors there (upstream's ordered-first-match semantics).
    let mut consumed_refine_idx: Vec<usize> = Vec::new();

    for d in decls.iter().copied() {
        match &*d.0 {
            ast_v1::Decl::Val { .. }
            | ast_v1::Decl::ValHorzCmd { .. }
            | ast_v1::Decl::ValVertCmd { .. } => {}
            ast_v1::Decl::TypeOpaque { kw, name, kind, .. } => {
                declared_types.push(name.name.clone());
                let Some(info) = impl_table.get(&name.name) else {
                    return Err(width_type_missing_error(
                        &module_name,
                        &name.name,
                        name.span,
                    ));
                };
                check_kind_all_o(kind)?;
                let declared_arity = kind.rest.len();
                if declared_arity != info.arity {
                    return Err(arity_mismatch_error(
                        &module_name,
                        &name.name,
                        name.span,
                        declared_arity,
                        info.arity,
                    ));
                }
                // A `with type` refinement on THIS decl (first match wins,
                // see above) skips the stamp mint — push what a literal
                // transparent decl here would have queued instead.
                if let Some((idx, refine)) = refines
                    .iter()
                    .enumerate()
                    .find(|(_, r)| r.path.is_empty() && r.name == name.name)
                {
                    consumed_refine_idx.push(idx);
                    let refine_arity = refine.tyvars.len();
                    if refine_arity != declared_arity {
                        return Err(refine_arity_error(
                            &module_name,
                            &name.name,
                            refine.span,
                            refine_arity,
                            declared_arity,
                        ));
                    }
                    let ty = match refine.body {
                        cst_v1::TypeBodyV1::Variant { .. } => {
                            return Err(refine_variant_body_error(
                                &module_name,
                                &name.name,
                                refine.span,
                            ));
                        }
                        cst_v1::TypeBodyV1::Synonym(ty) => ty,
                    };
                    if let ImplTypeBody::Variant(_) = &info.body {
                        return Err(simple_error(
                            Some(name.span),
                            format!(
                                "module `{module_name}` does not match its signature: type \
                                 `{}` is declared transparently (refined by `with type`) but \
                                 its implementation is a variant type — re-declaring a variant \
                                 in a signature is not supported",
                                name.name
                            ),
                        ));
                    }
                    let qualified = lower::qualify_type_key(mod_path, &name.name);
                    pending_transparent.push(PendingTransparent {
                        qualified,
                        quant: refine.tyvars,
                        ty,
                        span: refine.span,
                    });
                    continue;
                }
                let stamp = mint.next();
                let qualified = lower::qualify_type_key(mod_path, &name.name);
                let stamped = format!("{qualified}#{stamp}");
                env.types.insert(
                    qualified.clone(),
                    DeclaredType {
                        arity: info.arity,
                        opacity: TypeOpacity::Abstract { stamped },
                        span: kw.0,
                    },
                );
                if let ImplTypeBody::Variant(ctors) = &info.body {
                    for c in ctors {
                        record_hide(&mut hide_list, env, &module_name, c, &qualified);
                    }
                }
            }
            ast_v1::Decl::Type {
                binds: type_binds, ..
            } => {
                for single in flatten_type_binds(type_binds) {
                    declared_types.push(single.name.name.clone());
                    let Some(info) = impl_table.get(&single.name.name) else {
                        return Err(width_type_missing_error(
                            &module_name,
                            &single.name.name,
                            single.name.span,
                        ));
                    };
                    match &single.body {
                        cst_v1::TypeBodyV1::Variant { .. } => {
                            return Err(simple_error(
                                Some(single.name.span),
                                format!(
                                    "module `{module_name}`'s signature: re-declaring a variant \
                                     type in a signature is not supported"
                                ),
                            ));
                        }
                        cst_v1::TypeBodyV1::Synonym(ty) => {
                            if let ImplTypeBody::Variant(_) = &info.body {
                                return Err(simple_error(
                                    Some(single.name.span),
                                    format!(
                                        "module `{module_name}` does not match its signature: \
                                         type `{}` is declared transparently but its \
                                         implementation is a variant type — re-declaring a \
                                         variant in a signature is not supported",
                                        single.name.name
                                    ),
                                ));
                            }
                            let qualified = lower::qualify_type_key(mod_path, &single.name.name);
                            pending_transparent.push(PendingTransparent {
                                qualified,
                                quant: &single.tyvars,
                                ty,
                                span: single.name.span,
                            });
                        }
                    }
                }
            }
            ast_v1::Decl::Module { kw, name, sig_, .. } => match &**sig_ {
                ast_v1::SigExpr::Functor {
                    param, dom, cod, ..
                } => {
                    let _ = param;
                    // `S with Make type t = τ`: a functor has no type
                    // members of its own — a codomain's types exist only
                    // at an APPLICATION's path — so nothing to refine.
                    if let Some(r) = refines.iter().find(|r| r.path.first() == Some(&name.name)) {
                        return Err(simple_error(
                            Some(r.span),
                            format!(
                                "module `{module_name}`'s signature: `with {} type {}` refines a \
                                 type through `{}`, which the signature declares as a FUNCTOR — a \
                                 functor's types exist only at an application's own path",
                                r.path.join("."),
                                r.name,
                                name.name
                            ),
                        ));
                    }
                    handle_functor_sig_member(
                        kw.0,
                        &name.name,
                        dom,
                        cod,
                        mod_path,
                        surfaces,
                        env,
                        &module_name,
                    )?;
                    declared_functors.push(name.name.clone());
                }
                other_sig => {
                    let owned_trigger = subtree_trigger.clone().map(|t| (t, module_name.clone()));
                    // A `S with N type t = τ` refinement
                    // addressed to THIS member descends with one segment
                    // consumed, so the child sees an ordinary refinement of
                    // its own decls.
                    let child_refines: Vec<surface::Refine<'a>> = refines
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| r.path.first() == Some(&name.name))
                        .map(|(idx, r)| {
                            consumed_refine_idx.push(idx);
                            let mut r = r.clone();
                            r.path.remove(0);
                            r
                        })
                        .collect();
                    handle_nested_module_decl(
                        kw.0,
                        &name.name,
                        other_sig,
                        &impl_view,
                        mod_path,
                        tyenv,
                        surfaces,
                        mint,
                        env,
                        immediate_hides,
                        &module_name,
                        &owned_trigger,
                        &child_refines,
                        pending,
                        links,
                    )?;
                }
            },
            ast_v1::Decl::Signature { kw, name, sig_, .. } => {
                handle_signature_decl(
                    kw.0,
                    &name.name,
                    sig_,
                    &impl_view,
                    mod_path,
                    surfaces,
                    &module_name,
                )?;
                declared_sigs.push(name.name.clone());
            }
            // `decls` always arrives pre-flattened
            // ([`resolve_sig`]'s job) — a `Decl::Include` reaching THIS
            // match is unreachable in practice; a defensive
            // (non-panicking) error rather than `unreachable!()`.
            other @ ast_v1::Decl::Include { .. } => {
                return Err(non_val_decl_error(&module_name, other));
            }
        }
    }

    // Any refine no `Decl::TypeOpaque` consumed either
    // restricts an already-TRANSPARENT type or names one the signature
    // never declares.
    for (idx, refine) in refines.iter().enumerate() {
        if consumed_refine_idx.contains(&idx) {
            continue;
        }
        // A PATHED refine no `Decl::Module` claimed: no sub-module of that
        // name exists (a functor member already errored above).
        if let Some(head) = refine.path.first() {
            return Err(simple_error(
                Some(refine.span),
                format!(
                    "module `{module_name}`'s signature: `with {} type {}` refines a type \
                     through `{head}`, which the signature never declares as a module",
                    refine.path.join("."),
                    refine.name
                ),
            ));
        }
        if declared_types.contains(&refine.name) {
            return Err(simple_error(
                Some(refine.span),
                format!(
                    "module `{module_name}`'s signature: `with type {}` cannot refine `{}` \
                     — the signature already declares it transparently",
                    refine.name, refine.name
                ),
            ));
        }
        return Err(simple_error(
            Some(refine.span),
            format!(
                "module `{module_name}`'s signature: `with type {}` refines a type the \
                 signature never declares",
                refine.name
            ),
        ));
    }

    // Type hiding (phase A): every impl type not named by any sig decl.
    for (tname, info) in &impl_table {
        if !declared_types.contains(tname) {
            let qualified = lower::qualify_type_key(mod_path, tname);
            env.hidden_types
                .insert(qualified.clone(), module_name.clone());
            if let ImplTypeBody::Variant(ctors) = &info.body {
                for c in ctors {
                    record_hide(&mut hide_list, env, &module_name, c, &qualified);
                }
            }
        }
    }

    // Signature-member hiding — every struct
    // `signature S = ..` bind this seal never declared.
    let (_, sig_names) = impl_view_mod_and_sig_names(&impl_view);
    for sn in &sig_names {
        if !declared_sigs.contains(sn) {
            env.hidden_sigs
                .insert(lower::qualify_type_key(mod_path, sn), module_name.clone());
        }
    }

    // Functor hiding — every direct-child functor this
    // seal's `Decl::Module` members never declared.
    let functor_prefix = format!("{module_name}.");
    for fpath in surfaces.functors.keys() {
        if let Some(rest) = fpath.strip_prefix(functor_prefix.as_str()) {
            if !rest.contains('.') && !declared_functors.iter().any(|f| f == rest) {
                env.hidden_functors
                    .insert(fpath.clone(), module_name.clone());
            }
        }
    }

    // Seal point (phase A): parent-imposed hiding is
    // deferred to the PARENT's own trigger.
    if let Some((trigger, _owner)) = &parent_trigger {
        env.ctor_hide_triggers
            .entry(trigger.clone())
            .or_default()
            .extend(hide_list);
    } else if value_names.is_empty() {
        immediate_hides.extend(hide_list);
    } else {
        let trigger = lower::qualify_type_key(mod_path, value_names.last().unwrap());
        env.ctor_hide_triggers
            .entry(trigger)
            .or_default()
            .extend(hide_list);
    }

    pending.push(PendingSeal {
        sig_decls: decls,
        mod_path: mod_path.to_vec(),
        tyenv: tyenv.clone(),
        module_name,
        pending_transparent,
        declares_any_type: !declared_types.is_empty(),
        value_names,
        other_names,
        parent_trigger,
    });
    Ok(())
}

/// Recursive matching for a non-functor
/// `Decl::Module { N : S_N }` member.
///
/// `parent_refines` — the enclosing sig's `S with N ⟨…⟩ type t = τ`
/// refinements for THIS member, `N`-stripped — compose with (and apply
/// after) the member's own `S_N with type …`, so `sig module N : S with
/// type t = int end` and `sig module N : S end with N type t = int` reach
/// `prescan_seal_types` identically.
#[allow(clippy::too_many_arguments)]
fn handle_nested_module_decl<'a>(
    kw_span: Span,
    child_name: &str,
    s_n: &'a ast_v1::SigExpr,
    view: &ImplView<'a>,
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    surfaces: &'a SurfaceEnv<'a>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
    parent_module_name: &str,
    subtree_trigger: &Option<(String, String)>,
    parent_refines: &[surface::Refine<'a>],
    pending: &mut Vec<PendingSeal<'a>>,
    links: &mut Vec<PendingLink<'a>>,
) -> Result<(), TypeError> {
    let (mod_names, _) = impl_view_mod_and_sig_names(view);
    if !mod_names.iter().any(|m| m == child_name) {
        let (value_names, _) = impl_view_member_names(view, mod_path, surfaces);
        let is_value = value_names.iter().any(|v| v == child_name);
        return Err(nested_module_width_error(
            parent_module_name,
            child_name,
            kw_span,
            is_value,
        ));
    }
    let mut child_path = mod_path.to_vec();
    child_path.push(child_name.to_string());
    // Routing through the SAME `resolve_sig` funnel means `module N :
    // S_incl` (a nested sig member whose own sig includes/refines) just
    // works.
    let mut s_n_resolved = resolve_sig(s_n, &child_path.join("."), surfaces, &child_path)?;
    s_n_resolved.refines.extend(parent_refines.iter().cloned());
    match locate_child_module(view, child_name, &child_path, surfaces) {
        ChildModuleShape::UnsealedStruct(inner_binds) => {
            let child_tyenv = tyenv.child(&child_path, inner_binds.iter().copied(), surfaces);
            prescan_seal_types(
                s_n_resolved.decls,
                &s_n_resolved.refines,
                ImplView::Struct(inner_binds),
                &child_path,
                &child_tyenv,
                surfaces,
                mint,
                env,
                immediate_hides,
                subtree_trigger.clone(),
                pending,
                links,
            )?;
        }
        ChildModuleShape::SealedStruct(inner_binds) => {
            let child_tyenv = tyenv.child(&child_path, inner_binds.iter().copied(), surfaces);
            reject_link_refines(&s_n_resolved.refines, parent_module_name)?;
            links.push(PendingLink {
                child_path,
                parent_name: parent_module_name.to_string(),
                decls: s_n_resolved.decls,
                tyenv: child_tyenv,
                span: kw_span,
                parent_trigger: subtree_trigger.as_ref().map(|(t, _)| t.clone()),
            });
        }
        ChildModuleShape::ViaSurface(child_surf) => {
            // Same scope guard as `narrow_alias_body` —
            // only a LITERAL `sig … end` gets a synthetic child seal here
            // (width is already verified above regardless).
            if sig_is_literal_inline(s_n) {
                let child_tyenv = tyenv
                    .child_from_names(&child_path, child_surf.types.iter().map(|(n, _)| n.clone()));
                prescan_seal_types(
                    s_n_resolved.decls,
                    &s_n_resolved.refines,
                    ImplView::Surface(child_surf),
                    &child_path,
                    &child_tyenv,
                    surfaces,
                    mint,
                    env,
                    immediate_hides,
                    subtree_trigger.clone(),
                    pending,
                    links,
                )?;
            }
        }
        ChildModuleShape::Unavailable => {
            return Err(nested_module_width_error(
                parent_module_name,
                child_name,
                kw_span,
                false,
            ));
        }
    }
    Ok(())
}

fn nested_module_width_error(
    module_name: &str,
    child_name: &str,
    span: Span,
    is_value: bool,
) -> TypeError {
    let message = if is_value {
        format!(
            "module `{module_name}` signature declares `module {child_name} : ..` but its \
             `struct .. end` body defines `{child_name}` as a value, not a module"
        )
    } else {
        format!(
            "module `{module_name}` signature declares `module {child_name} : ..` but its \
             `struct .. end` body never defines `{child_name}`"
        )
    };
    simple_error(Some(span), message)
}

/// A `Decl::Signature` member — syntactic token-
/// identity equality (semantic `sig_equal` up to alpha-variance is
/// explicitly deferred).
fn handle_signature_decl<'a>(
    kw_span: Span,
    child_name: &str,
    outer_sig: &'a ast_v1::SigExpr,
    view: &ImplView<'a>,
    mod_path: &[String],
    surfaces: &'a SurfaceEnv<'a>,
    module_name: &str,
) -> Result<(), TypeError> {
    let (_, sig_names) = impl_view_mod_and_sig_names(view);
    if !sig_names.iter().any(|s| s == child_name) {
        return Err(simple_error(
            Some(kw_span),
            format!(
                "module `{module_name}` signature declares `signature {child_name} = ..` but \
                 its `struct .. end` body never defines a signature named `{child_name}`"
            ),
        ));
    }
    let outer_decls = resolve_sig_shallow(
        outer_sig,
        &format!("{module_name}.{child_name}"),
        surfaces,
        mod_path,
    )?;
    let struct_decls = surface::find_sig(surfaces, mod_path, child_name)
        .map(|d| d.decls)
        .ok_or_else(|| {
            simple_error(
                Some(kw_span),
                format!(
                    "module `{module_name}`'s signature member `signature {child_name}` could not \
                 be resolved against the struct's own definition"
                ),
            )
        })?;
    if !decls_eq_ignoring_span(outer_decls, struct_decls) {
        return Err(simple_error(
            Some(kw_span),
            format!(
                "module `{module_name}` does not match its signature: signature member \
                 `{child_name}` must be declared identically in the sig and the struct \
                 (semantic signature equality is not implemented; re-spell the two \
                 identically)"
            ),
        ));
    }
    Ok(())
}

/// A functor sig-member (`Decl::Module { Make :
/// (Key : S_dom) -> S_cod }`).
fn handle_functor_sig_member(
    kw_span: Span,
    child_name: &str,
    dom: &ast_v1::SigExpr,
    cod: &ast_v1::SigExpr,
    mod_path: &[String],
    surfaces: &SurfaceEnv,
    env: &mut StaticEnv,
    module_name: &str,
) -> Result<(), TypeError> {
    let functor_path = lower::qualify_type_key(mod_path, child_name);
    let Some(fdef) = surfaces.functors.get(&functor_path) else {
        let what = if surfaces.modules.contains_key(&functor_path) {
            "a plain module"
        } else {
            "nothing"
        };
        return Err(simple_error(
            Some(kw_span),
            format!(
                "module `{module_name}` signature declares `module {child_name} : (..) -> ..` \
                 (a functor) but its `struct .. end` body defines `{child_name}` as {what}, not \
                 a functor"
            ),
        ));
    };
    let declared_dom_decls = resolve_sig_shallow(dom, &functor_path, surfaces, mod_path)?;
    let impl_dom_decls =
        resolve_sig_shallow(fdef.param_sig, &functor_path, surfaces, &fdef.def_path)?;
    if !std::ptr::eq(declared_dom_decls, impl_dom_decls)
        && !decls_eq_ignoring_span(declared_dom_decls, impl_dom_decls)
    {
        return Err(simple_error(
            Some(kw_span),
            format!(
                "functor member `{child_name}`: its declared parameter signature must be the \
                 same signature the implementation names (structural parameter-signature \
                 subtyping is not implemented)"
            ),
        ));
    }
    sig_subtype::substitute_result_sig(cod, kw_span)
        .map_err(|e| functor_codomain_error(module_name, child_name, e))?;
    env.sealed_functors.insert(
        functor_path,
        SealedFunctorSig {
            param: fdef.param.clone(),
            dom: dom.clone(),
            cod: cod.clone(),
            def_site: mod_path.to_vec(),
            span: kw_span,
        },
    );
    Ok(())
}

fn functor_codomain_error(module_name: &str, member: &str, err: SigSubtypeError) -> TypeError {
    match err {
        SigSubtypeError::NestedFunctorSubstitution { span } => simple_error(
            Some(span),
            format!(
                "module `{module_name}`'s functor member `{member}`: higher-order functor \
                 signatures (a curried codomain) are not supported (nested-functor \
                 substitution)"
            ),
        ),
    }
}

/// The trigger key for a parent-imposed synthetic child
/// seal — the qualified alias of the PARENT's LAST value member across its
/// whole subtree, in source order (the contiguous-alias invariant).
/// `None` when the subtree has zero value members — the ordinary
/// immediate-hiding fallback already suffices.
fn last_value_alias_in_subtree(
    view: &ImplView,
    mod_path: &[String],
    surfaces: &SurfaceEnv,
) -> Option<String> {
    match view {
        ImplView::Struct(binds) => last_value_alias_in_struct_subtree(binds, mod_path, surfaces),
        ImplView::Surface(surf) => last_value_alias_in_surface_subtree(surf, mod_path),
    }
}

fn last_value_alias_in_struct_subtree(
    binds: &[&cst_v1::Bind],
    mod_path: &[String],
    surfaces: &SurfaceEnv,
) -> Option<String> {
    let mut last: Option<String> = None;
    for b in binds.iter().copied() {
        match b {
            cst_v1::Bind::Value { name, .. } => {
                last = Some(lower::qualify_type_key(mod_path, &name.name))
            }
            cst_v1::Bind::ValueInline { cmd, .. } => {
                last = Some(lower::qualify_type_key(mod_path, &any_horz_name(cmd)))
            }
            cst_v1::Bind::ValueBlock { cmd, .. } => {
                last = Some(lower::qualify_type_key(mod_path, &any_vert_name(cmd)))
            }
            cst_v1::Bind::ValueMath { cmd, .. } => {
                last = Some(lower::qualify_type_key(mod_path, &any_horz_name(cmd)))
            }
            cst_v1::Bind::ValueRec { first, ands, .. } => {
                let n = ands
                    .last()
                    .map(|a| a.clause.name.name.clone())
                    .unwrap_or_else(|| first.name.name.clone());
                last = Some(lower::qualify_type_key(mod_path, &n));
            }
            cst_v1::Bind::ValueMutable { name, .. } => {
                last = Some(lower::qualify_type_key(mod_path, &name.name))
            }
            cst_v1::Bind::Module { name, body, .. } => {
                let mut child_path = mod_path.to_vec();
                child_path.push(name.name.clone());
                match &*body.0 {
                    ast_v1::ModExpr::Struct { binds: inner, .. } => {
                        let inner_refs: Vec<&cst_v1::Bind> =
                            inner.iter().map(|sb| sb.0.as_ref()).collect();
                        if let Some(v) =
                            last_value_alias_in_struct_subtree(&inner_refs, &child_path, surfaces)
                        {
                            last = Some(v);
                        }
                    }
                    ast_v1::ModExpr::Var(_)
                    | ast_v1::ModExpr::Coerce { .. }
                    | ast_v1::ModExpr::App { .. } => {
                        if let Some(surf) = surfaces.modules.get(&child_path.join(".")) {
                            if let Some(v) = last_value_alias_in_surface_subtree(surf, &child_path)
                            {
                                last = Some(v);
                            }
                        }
                    }
                    ast_v1::ModExpr::Functor { .. } => {}
                }
            }
            cst_v1::Bind::Include { kw, body } => {
                if let ast_v1::ModExpr::Var(_) = &*body.0 {
                    if let Some(Some(target)) =
                        surface::frozen_include_target(surfaces, mod_path, kw.0)
                    {
                        if let Some(target_surf) = surfaces.modules.get(target) {
                            if let Some(v) = target_surf.vals.last() {
                                last = Some(lower::qualify_type_key(mod_path, v));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    last
}

fn last_value_alias_in_surface_subtree(surf: &ModSurface, mod_path: &[String]) -> Option<String> {
    for (name, child) in surf.mods.iter().rev() {
        let mut child_path = mod_path.to_vec();
        child_path.push(name.clone());
        if let Some(v) = last_value_alias_in_surface_subtree(child, &child_path) {
            return Some(v);
        }
    }
    surf.vals
        .last()
        .map(|v| lower::qualify_type_key(mod_path, v))
}

/// Record one ctor-hide entry: the deferred `hide_list` (fed to
/// `Checker::hide_ctors` at this seal's trigger) and `env.hidden_ctors`
/// (for [`rewrite_hidden_error`]'s diagnostic — safe to populate
/// immediately, since it only affects error TEXT).
fn record_hide(
    hide_list: &mut Vec<(String, String)>,
    env: &mut StaticEnv,
    module_name: &str,
    ctor: &str,
    qualified_type: &str,
) {
    hide_list.push((ctor.to_string(), qualified_type.to_string()));
    env.hidden_ctors.insert(
        ctor.to_string(),
        HiddenCtor {
            module: module_name.to_string(),
            type_name: qualified_type.to_string(),
        },
    );
}

/// What an ascription's `SigExpr` resolves to — the
/// FLATTENED decl list (every `Decl::Include` spliced in place) plus every
/// `with type` refinement collected along the way, in application order.
struct ResolvedSig<'a> {
    decls: Vec<&'a cst_v1::StructDeclV1>,
    refines: Vec<surface::Refine<'a>>,
}

/// Resolve one ascription's [`ast_v1::SigExpr`] to its FULLY FLATTENED decl
/// list: a literal `sig … end` is itself with every
/// `Decl::Include` spliced in; a named `Var(S)`/`Path(A.B.S)` resolves
/// outward from `site_path` (`v1/surface.rs::find_sig`) — the resolved
/// decls then feed the SAME prescan/phase-C pipeline an inline body would,
/// so opaque types stamp FRESH at THIS site (generativity — pinned by a dedicated test). An
/// unresolved name, an `include` cycle, and a duplicate post-splice decl
/// are each precise errors ([`check_sig_conflicts`]); `with type` collects
/// [`surface::Refine`]s instead of resolving further; a functor signature
/// stays its own placeholder.
fn resolve_sig<'a>(
    sig: &'a ast_v1::SigExpr,
    module_name: &str,
    surfaces: &'a SurfaceEnv<'a>,
    site_path: &[String],
) -> Result<ResolvedSig<'a>, TypeError> {
    let mut visited = Vec::new();
    let resolved = resolve_sig_visited(sig, module_name, surfaces, site_path, &mut visited)?;
    check_sig_conflicts(&resolved.decls, module_name)?;
    Ok(resolved)
}

fn resolve_sig_visited<'a>(
    sig: &'a ast_v1::SigExpr,
    module_name: &str,
    surfaces: &'a SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Result<ResolvedSig<'a>, TypeError> {
    match sig {
        ast_v1::SigExpr::Bot(bot) => {
            resolve_sig_bot(bot, module_name, surfaces, site_path, visited)
        }
        // `S with type t = τ`: resolve the base (never itself a `with` —
        // the grammar's own left-recursion note), then APPEND this node's
        // own refines. `S with M type t = τ` (a SUB-MODULE refinement)
        // resolves the same way — the chain rides along as the collected
        // [`surface::Refine::path`], which `prescan_seal_types` routes into
        // the `Decl::Module` member of that name.
        ast_v1::SigExpr::WithType {
            base, path, binds, ..
        } => {
            let mut resolved = resolve_sig_bot(base, module_name, surfaces, site_path, visited)?;
            resolved
                .refines
                .extend(surface::collect_refines(binds, surface::mod_chain_segments(path)));
            Ok(resolved)
        }
        ast_v1::SigExpr::Functor { lp, .. } => Err(simple_error(
            Some(lp.0),
            format!(
                "module `{module_name}`'s signature: a functor-signature ASCRIPTION directly \
                 on a module bind (`module M :> (X:S)->S2 = ..`) is not supported — a functor \
                 signature is only enforced as a `Decl::Module` sig MEMBER inside an \
                 enclosing `sig .. end`"
            ),
        )),
    }
}

fn resolve_sig_bot<'a>(
    bot: &'a ast_v1::SigBotV1,
    module_name: &str,
    surfaces: &'a SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Result<ResolvedSig<'a>, TypeError> {
    match bot {
        ast_v1::SigBotV1::Sig { decls, .. } => {
            let (out, refines) = splice_decls(decls, module_name, surfaces, site_path, visited)?;
            Ok(ResolvedSig {
                decls: out,
                refines,
            })
        }
        ast_v1::SigBotV1::Var(t) => {
            resolve_named_sig(&t.name, t.span, module_name, surfaces, site_path, visited)
        }
        ast_v1::SigBotV1::Path(t) => {
            let suffix = surface::sig_path_suffix(&t.mods, &t.name);
            resolve_named_sig(&suffix, t.span, module_name, surfaces, site_path, visited)
        }
    }
}

/// A NAMED signature reference (`Var`/`Path`): find it, cycle-guard on its
/// RESOLVED table key (keying the written suffix would false-positive on two
/// differently-pathed same-suffix sigs), splice its own decls, and inherit
/// its own STORED refines (named-sig composition).
fn resolve_named_sig<'a>(
    name: &str,
    span: Span,
    module_name: &str,
    surfaces: &'a SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Result<ResolvedSig<'a>, TypeError> {
    let (key, def) = surface::find_sig_keyed(surfaces, site_path, name)
        .ok_or_else(|| unknown_sig_error(name, span))?;
    if visited.contains(&key) {
        return Err(simple_error(
            Some(span),
            format!("signature `{name}` includes itself (an `include` cycle)"),
        ));
    }
    visited.push(key);
    let (out, mut refines) = splice_decls(def.decls, module_name, surfaces, site_path, visited)?;
    visited.pop();
    refines.extend(def.refines.iter().cloned());
    Ok(ResolvedSig {
        decls: out,
        refines,
    })
}

/// Sig-include flattening: a non-`Include` decl passes through; a
/// `Decl::Include { sig_ }` resolves `sig_` and splices its (recursively
/// flattened) decls AND refines in place — order preserved (deterministic
/// error order, and the spliced opaque decls sit exactly where a later
/// `TypeOpaque` interception, or the ctor-hide/revocation "last value member"
/// trigger, expects them — the same splice-position argument struct
/// `include` uses, extended to signatures).
fn splice_decls<'a>(
    decls: &'a [cst_v1::StructDeclV1],
    module_name: &str,
    surfaces: &'a SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Result<(Vec<&'a cst_v1::StructDeclV1>, Vec<surface::Refine<'a>>), TypeError> {
    let mut out = Vec::new();
    let mut refines = Vec::new();
    for d in decls {
        if let ast_v1::Decl::Include { sig_, .. } = &*d.0 {
            let inner = resolve_sig_visited(sig_, module_name, surfaces, site_path, visited)?;
            out.extend(inner.decls);
            refines.extend(inner.refines);
        } else {
            out.push(d);
        }
    }
    Ok((out, refines))
}

/// The conflict check (upstream `ConflictInSignature`, `staticEnv.ml:
/// 404-428`): one linear pass over the FLATTENED decl list, per-category
/// name sets (vals incl. command keys / types / modules / signatures) — a
/// repeat within a category is a hard error at the SECOND decl's span.
/// Applies uniformly, even to a literal sig with two DIRECT `val x` decls
/// (pinned by a dedicated test).
fn check_sig_conflicts(
    decls: &[&cst_v1::StructDeclV1],
    module_name: &str,
) -> Result<(), TypeError> {
    let mut vals: Vec<&str> = Vec::new();
    let mut types: Vec<&str> = Vec::new();
    let mut mods: Vec<&str> = Vec::new();
    let mut sigs: Vec<&str> = Vec::new();
    for d in decls {
        match &*d.0 {
            ast_v1::Decl::Val { name, .. } => {
                check_one_conflict(&mut vals, &name.name, name.span, module_name)?
            }
            ast_v1::Decl::ValHorzCmd { cmd, .. } => {
                check_one_conflict(&mut vals, &cmd.name, cmd.span, module_name)?
            }
            ast_v1::Decl::ValVertCmd { cmd, .. } => {
                check_one_conflict(&mut vals, &cmd.name, cmd.span, module_name)?
            }
            ast_v1::Decl::TypeOpaque { name, .. } => {
                check_one_conflict(&mut types, &name.name, name.span, module_name)?
            }
            ast_v1::Decl::Type { binds, .. } => {
                for single in flatten_type_binds(binds) {
                    check_one_conflict(
                        &mut types,
                        &single.name.name,
                        single.name.span,
                        module_name,
                    )?;
                }
            }
            ast_v1::Decl::Module { name, .. } => {
                check_one_conflict(&mut mods, &name.name, name.span, module_name)?
            }
            ast_v1::Decl::Signature { name, .. } => {
                check_one_conflict(&mut sigs, &name.name, name.span, module_name)?
            }
            // `splice_decls` already expanded every `Decl::Include` above —
            // unreachable in practice; defensive, non-panicking.
            ast_v1::Decl::Include { kw, .. } => {
                return Err(simple_error(
                    Some(kw.0),
                    format!(
                        "module `{module_name}`'s signature: internal — an `include` decl \
                         reached conflict-checking unspliced"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn check_one_conflict<'a>(
    seen: &mut Vec<&'a str>,
    name: &'a str,
    span: Span,
    module_name: &str,
) -> Result<(), TypeError> {
    if seen.contains(&name) {
        return Err(simple_error(
            Some(span),
            format!(
                "module `{module_name}`'s signature: conflicting declarations for `{name}` \
                 (declared both directly and via `include`, or twice)"
            ),
        ));
    }
    seen.push(name);
    Ok(())
}

/// The SHALLOW twin of [`resolve_sig`]: a literal `sig … end` is itself
/// (`Decl::Include` inside it is NOT flattened); a named `Var`/`Path`
/// resolves outward exactly once. Used only by the two structural
/// (syntactic token-identity) comparators that never need semantic
/// splicing ([`handle_signature_decl`], [`handle_functor_sig_member`]) —
/// both already handle a literal `Decl::Include` fine structurally; a
/// `with type` node, which neither comparator can compare meaningfully, is
/// an explicit reject here instead — never silently ignored.
fn resolve_sig_shallow<'a>(
    sig: &'a ast_v1::SigExpr,
    module_name: &str,
    surfaces: &'a SurfaceEnv<'a>,
    site_path: &[String],
) -> Result<&'a [cst_v1::StructDeclV1], TypeError> {
    match sig {
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Sig { decls, .. }) => Ok(decls.as_slice()),
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Var(t)) => {
            surface::find_sig(surfaces, site_path, &t.name)
                .map(|d| d.decls)
                .ok_or_else(|| unknown_sig_error(&t.name, t.span))
        }
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Path(t)) => {
            let suffix = surface::sig_path_suffix(&t.mods, &t.name);
            surface::find_sig(surfaces, site_path, &suffix)
                .map(|d| d.decls)
                .ok_or_else(|| unknown_sig_error(&suffix, t.span))
        }
        ast_v1::SigExpr::WithType { with_kw, .. } => Err(simple_error(
            Some(with_kw.0),
            format!(
                "module `{module_name}`'s signature: a `with type` refinement is not \
                 supported here (only at a module ascription site, or inside an `include`d \
                 signature)"
            ),
        )),
        ast_v1::SigExpr::Functor { lp, .. } => Err(simple_error(
            Some(lp.0),
            format!(
                "module `{module_name}`'s signature: a functor-signature ASCRIPTION directly \
                 on a module bind (`module M :> (X:S)->S2 = ..`) is not supported — a functor \
                 signature is only enforced as a `Decl::Module` sig MEMBER inside an \
                 enclosing `sig .. end`"
            ),
        )),
    }
}

fn unknown_sig_error(name: &str, span: Span) -> TypeError {
    simple_error(Some(span), format!("unknown signature name `{name}`"))
}

/// PendingLink-scope rule: a [`PendingLink`] layer only
/// re-checks `Decl::Val`-family members, never type decls — a `with type`
/// refinement there is an explicit reject instead of silent unenforcement.
fn reject_link_refines(refines: &[surface::Refine], owner: &str) -> Result<(), TypeError> {
    if let Some(refine) = refines.first() {
        return Err(simple_error(
            Some(refine.span),
            format!(
                "module `{owner}`'s signature: `with type` on an outer seal-chain layer \
                 (`module M :> S1 = N :> S2`, or a `Decl::Module` sig member over an \
                 already-sealed child) is not enforced"
            ),
        ));
    }
    Ok(())
}

// ============================================================================
// The functor-application parameter-signature check.// ============================================================================

/// Walk every frozen `ModExpr::App` resolution and width/arity-check the
/// argument's [`ModSurface`] against the functor's parameter signature `S`.
///
/// **Deliberately NAME/ARITY-only, no [`Checker`]/[`StampMint`]/
/// [`StaticEnv`] involvement.** A real ascription re-check would have to
/// register the SAME qualified member keys `prescan_seal_types`/
/// `process_seal_member` already use for the ARGUMENT's own real `:>` seal
/// — a second registration at that key would silently clobber it (whichever
/// prescan runs last wins). This check stays entirely within `surfaces`
/// instead: it catches a MISSING member/wrong-arity type precisely;
/// a wrong VALUE TYPE is caught later, less specifically, when
/// the instantiated body's use of that member fails to type-check.
fn check_functor_applications<'a>(
    surfaces: &'a SurfaceEnv<'a>,
    env: &StaticEnv,
) -> Result<(), TypeError> {
    for (_site_path, _span, resolution) in &surfaces.app_targets {
        let Some(res) = resolution else { continue };
        // An application of a functor an        // umbrella seal never exported.
        if let Some(owner) = env.hidden_functors.get(&res.functor_path) {
            return Err(simple_error(
                None,
                format!(
                    "functor `{}` exists in module `{owner}` but is not exported by its \
                     signature",
                    res.functor_path
                ),
            ));
        }
        let Some(fdef) = surfaces.functors.get(&res.functor_path) else {
            continue;
        };
        let Some(arg_surf) = surfaces.modules.get(&res.arg_path) else {
            continue;
        };
        let resolved = resolve_sig(fdef.param_sig, &res.functor_path, surfaces, &fdef.def_path)?;
        // Functor parameter signatures are checked NAME/ARITY-only — a
        // `with type` there is name-invisible, so it would be SILENTLY
        // unenforced beyond arity if allowed through. Explicit reject
        // instead.
        if let Some(refine) = resolved.refines.first() {
            return Err(simple_error(
                Some(refine.span),
                format!(
                    "functor `{}`'s parameter signature uses `with type` — refining a \
                     functor parameter signature is not enforced (parameter signatures are \
                     checked by name/arity only)",
                    res.functor_path
                ),
            ));
        }
        check_module_against_sig(&resolved.decls, arg_surf, &res.functor_path, &res.arg_path)?;
    }
    Ok(())
}

/// The width/arity half of the ascription-equivalent check: does
/// `arg_surf` provide every member `decls` declares, at matching
/// type-arity? A `Decl::Module`/`Signature`/`Include` in a functor
/// parameter signature is a precise deferred error.
fn check_module_against_sig(
    decls: &[&cst_v1::StructDeclV1],
    arg_surf: &ModSurface,
    functor_path: &str,
    arg_path: &str,
) -> Result<(), TypeError> {
    for d in decls {
        match &*d.0 {
            ast_v1::Decl::Val { name, .. } => {
                if !arg_surf.vals.iter().any(|v| v == &name.name) {
                    return Err(functor_arg_mismatch_error(
                        functor_path,
                        arg_path,
                        &format!("value `{}`", name.name),
                        name.span,
                    ));
                }
            }
            ast_v1::Decl::ValHorzCmd { cmd, .. } => {
                if !arg_surf.vals.iter().any(|v| v == &cmd.name) {
                    return Err(functor_arg_mismatch_error(
                        functor_path,
                        arg_path,
                        &format!("command `{}`", cmd.name),
                        cmd.span,
                    ));
                }
            }
            ast_v1::Decl::ValVertCmd { cmd, .. } => {
                if !arg_surf.vals.iter().any(|v| v == &cmd.name) {
                    return Err(functor_arg_mismatch_error(
                        functor_path,
                        arg_path,
                        &format!("command `{}`", cmd.name),
                        cmd.span,
                    ));
                }
            }
            ast_v1::Decl::TypeOpaque { name, kind, .. } => {
                check_functor_arg_type(
                    arg_surf,
                    &name.name,
                    kind.rest.len(),
                    name.span,
                    functor_path,
                    arg_path,
                )?;
            }
            ast_v1::Decl::Type { binds, .. } => {
                for single in flatten_type_binds(binds) {
                    check_functor_arg_type(
                        arg_surf,
                        &single.name.name,
                        single.tyvars.len(),
                        single.name.span,
                        functor_path,
                        arg_path,
                    )?;
                }
            }
            ast_v1::Decl::Module { kw, .. } => {
                return Err(functor_sig_member_error(
                    functor_path,
                    "a nested module",
                    kw.0,
                ));
            }
            ast_v1::Decl::Signature { kw, .. } => {
                return Err(functor_sig_member_error(
                    functor_path,
                    "a named signature",
                    kw.0,
                ));
            }
            ast_v1::Decl::Include { kw, .. } => {
                return Err(functor_sig_member_error(functor_path, "an `include`", kw.0));
            }
        }
    }
    Ok(())
}

fn check_functor_arg_type(
    arg_surf: &ModSurface,
    name: &str,
    declared_arity: usize,
    span: Span,
    functor_path: &str,
    arg_path: &str,
) -> Result<(), TypeError> {
    match arg_surf.types.iter().find(|(t, _)| t == name) {
        Some((_, arity)) if *arity == declared_arity => Ok(()),
        Some((_, arity)) => Err(functor_arg_mismatch_error(
            functor_path,
            arg_path,
            &format!(
                "type `{name}` (declared with arity {declared_arity} but provided with \
                 arity {arity})"
            ),
            span,
        )),
        None => Err(functor_arg_mismatch_error(
            functor_path,
            arg_path,
            &format!("type `{name}`"),
            span,
        )),
    }
}

fn functor_arg_mismatch_error(
    functor_path: &str,
    arg_path: &str,
    what: &str,
    span: Span,
) -> TypeError {
    simple_error(
        Some(span),
        format!(
            "argument module `{arg_path}` does not match functor `{functor_path}`'s \
             parameter signature: missing or mismatched {what}"
        ),
    )
}

fn functor_sig_member_error(functor_path: &str, what: &str, span: Span) -> TypeError {
    simple_error(
        Some(span),
        format!(
            "functor `{functor_path}`'s parameter signature declares {what} — \
             enforcing that is Sub-slice 2f-2"
        ),
    )
}

// ============================================================================
// The early functor sig-member discovery pass — must run
// BEFORE `check_functor_applications`/phase A0's instantiation store need
// `StaticEnv::sealed_functors`/`hidden_functors`. `phase_a_prescan` later
// re-registers `sealed_functors` identically (idempotent); only
// `hidden_functors` is computed here exclusively.
// ============================================================================

fn discover_sealed_functors<'a>(
    deps: &'a [&'a cst_v1::FileV1],
    surfaces: &'a SurfaceEnv<'a>,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    for file in deps.iter().copied() {
        let cst_v1::FileV1::Library {
            name,
            sig_annot,
            binds,
            ..
        } = file
        else {
            continue;
        };
        let mod_path = vec![name.name.clone()];
        if let Some(sa) = sig_annot {
            // Best-effort (swallowed, never `?`) — a resolution failure
            // here still surfaces for REAL later, when `phase_a_prescan`
            // re-runs the identical `resolve_sig` call. The DEEP resolver
            // is what makes a functor sig-member reachable only through a
            // spliced `include` discoverable.
            if let Ok(resolved) = resolve_sig(&sa.sig_.0, &mod_path.join("."), surfaces, &mod_path)
            {
                discover_functor_members_in_decls(&resolved.decls, &mod_path, surfaces, env)?;
            }
        }
        let bind_refs: Vec<&cst_v1::Bind> = binds.iter().collect();
        discover_sealed_functors_walk_binds(&bind_refs, &mod_path, surfaces, env)?;
    }
    Ok(())
}

fn discover_sealed_functors_walk_binds<'a>(
    binds: &[&'a cst_v1::Bind],
    mod_path: &[String],
    surfaces: &'a SurfaceEnv<'a>,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    for b in binds.iter().copied() {
        if let cst_v1::Bind::Module {
            name,
            sig_annot,
            body,
            ..
        } = b
        {
            let mut child_path = mod_path.to_vec();
            child_path.push(name.name.clone());
            if let Some(sa) = sig_annot {
                if let Ok(resolved) =
                    resolve_sig(&sa.sig_.0, &child_path.join("."), surfaces, &child_path)
                {
                    discover_functor_members_in_decls(&resolved.decls, &child_path, surfaces, env)?;
                }
            }
            if let ast_v1::ModExpr::Struct { binds: inner, .. } = &*body.0 {
                let inner_refs: Vec<&cst_v1::Bind> = inner.iter().map(|sb| sb.0.as_ref()).collect();
                discover_sealed_functors_walk_binds(&inner_refs, &child_path, surfaces, env)?;
            }
        }
    }
    Ok(())
}

fn discover_functor_members_in_decls<'a>(
    decls: &[&'a cst_v1::StructDeclV1],
    mod_path: &[String],
    surfaces: &'a SurfaceEnv<'a>,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    let module_name = mod_path.join(".");
    let mut declared_functors: Vec<String> = Vec::new();
    for d in decls {
        if let ast_v1::Decl::Module { kw, name, sig_, .. } = &*d.0 {
            if let ast_v1::SigExpr::Functor { dom, cod, .. } = &**sig_ {
                handle_functor_sig_member(
                    kw.0,
                    &name.name,
                    dom,
                    cod,
                    mod_path,
                    surfaces,
                    env,
                    &module_name,
                )?;
                declared_functors.push(name.name.clone());
            } else {
                let mut child_path = mod_path.to_vec();
                child_path.push(name.name.clone());
                if let Ok(inner_resolved) =
                    resolve_sig(sig_, &child_path.join("."), surfaces, &child_path)
                {
                    discover_functor_members_in_decls(
                        &inner_resolved.decls,
                        &child_path,
                        surfaces,
                        env,
                    )?;
                }
            }
        }
    }
    let prefix = format!("{module_name}.");
    for fpath in surfaces.functors.keys() {
        if let Some(rest) = fpath.strip_prefix(prefix.as_str()) {
            if !rest.contains('.') && !declared_functors.iter().any(|f| f == rest) {
                env.hidden_functors
                    .insert(fpath.clone(), module_name.clone());
            }
        }
    }
    Ok(())
}

// ============================================================================
// The instantiation store — one [`InstantiatedApp`]
// per frozen application of a SEALED functor, materialized before phase A
// runs.
// ============================================================================

fn build_instantiations<'a>(
    deps: &'a [&'a cst_v1::FileV1],
    surfaces: &'a SurfaceEnv<'a>,
    env: &StaticEnv,
) -> Result<Vec<InstantiatedApp>, TypeError> {
    let mut out = Vec::new();
    for file in deps.iter().copied() {
        let cst_v1::FileV1::Library { name, binds, .. } = file else {
            continue;
        };
        let mod_path = vec![name.name.clone()];
        let bind_refs: Vec<&cst_v1::Bind> = binds.iter().collect();
        collect_instantiations_in_binds(&bind_refs, &mod_path, surfaces, env, &mut out)?;
    }
    Ok(out)
}

fn collect_instantiations_in_binds<'a>(
    binds: &[&'a cst_v1::Bind],
    mod_path: &[String],
    surfaces: &'a SurfaceEnv<'a>,
    env: &StaticEnv,
    out: &mut Vec<InstantiatedApp>,
) -> Result<(), TypeError> {
    for b in binds.iter().copied() {
        if let cst_v1::Bind::Module { name, body, .. } = b {
            let mut child_path = mod_path.to_vec();
            child_path.push(name.name.clone());
            match &*body.0 {
                ast_v1::ModExpr::App { func, arg: _ } => {
                    let span = mod_chain_span_v1(func);
                    if let Some(Some(res)) = surface::frozen_app_target(surfaces, mod_path, span) {
                        if let Some(sealed) = env.sealed_functors.get(&res.functor_path).cloned() {
                            if let Some(fdef) = surfaces.functors.get(&res.functor_path) {
                                if let Some(body_binds) = functor::functor_body_binds(fdef.body) {
                                    let arg_segs: Vec<String> =
                                        res.arg_path.split('.').map(str::to_string).collect();
                                    let substituted_binds = functor::substitute_binds(
                                        body_binds,
                                        &sealed.param,
                                        &arg_segs,
                                    )
                                    .map_err(|e| lower_error_to_type_error(&e))?;
                                    let cod_substituted = functor::subst_sig_expr_for_param(
                                        &sealed.cod,
                                        &sealed.param,
                                        &arg_segs,
                                    )
                                    .map_err(|e| lower_error_to_type_error(&e))?;
                                    sig_subtype::substitute_result_sig(
                                        &cod_substituted,
                                        sealed.span,
                                    )
                                    .map_err(|e| {
                                        functor_codomain_error(&mod_path.join("."), &name.name, e)
                                    })?;
                                    // `include` in a sealed                                    // functor's codomain IS supported (the
                                    // decls are cloned out immediately, so
                                    // no self-reference risk); `with type`
                                    // there is an explicit reject instead —
                                    // `InstantiatedApp` is fully OWNED (no
                                    // lifetime parameter), but a `Refine<'a>`
                                    // borrows the LOCAL/substituted `cst_v1`
                                    // tree, so it can't ride along without an
                                    // owned `Refine` shape this zero-demand
                                    // corner doesn't warrant.
                                    let cod_resolved = resolve_sig(
                                        &cod_substituted,
                                        &child_path.join("."),
                                        surfaces,
                                        &sealed.def_site,
                                    )?;
                                    if let Some(refine) = cod_resolved.refines.first() {
                                        return Err(simple_error(
                                            Some(refine.span),
                                            format!(
                                                "functor `{}`'s declared codomain signature \
                                                 uses `with type` — refining a sealed \
                                                 functor's codomain is not enforced",
                                                res.functor_path
                                            ),
                                        ));
                                    }
                                    let cod_decls: Vec<cst_v1::StructDeclV1> =
                                        cod_resolved.decls.into_iter().cloned().collect();
                                    let owned_body: Vec<cst_v1::Bind> =
                                        substituted_binds.into_iter().map(|sb| *sb.0).collect();
                                    let child_tyenv = TypeNameEnv::default().child(
                                        &child_path,
                                        owned_body.iter(),
                                        surfaces,
                                    );
                                    out.push(InstantiatedApp {
                                        app_path: child_path.clone(),
                                        cod_decls,
                                        body_binds: owned_body,
                                        tyenv: child_tyenv,
                                        span: sealed.span,
                                    });
                                }
                            }
                        }
                    }
                }
                ast_v1::ModExpr::Struct { binds: inner, .. } => {
                    let inner_refs: Vec<&cst_v1::Bind> =
                        inner.iter().map(|sb| sb.0.as_ref()).collect();
                    collect_instantiations_in_binds(&inner_refs, &child_path, surfaces, env, out)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn mod_chain_span_v1(c: &ast_v1::ModChainV1) -> Span {
    match c {
        ast_v1::ModChainV1::Long(t) => t.span,
        ast_v1::ModChainV1::Single(t) => t.span,
    }
}

fn lower_error_to_type_error(e: &lower::LowerError) -> TypeError {
    simple_error(Some(e.span), format!("{}: {}", e.construct, e.hint))
}

// ============================================================================
// Syntactic (span-ignoring) structural equality over// `ast_v1` decl trees, driving `handle_signature_decl`'s and
// `handle_functor_sig_member`'s identity checks. Deliberately hand-written
// rather than `Unparse`-to-token-stream (not a dependency of this crate) —
// equivalent soundness-wise (Parse/Unparse round-trip: identical trees
// minus spans ⟺ identical token streams), and conservative: syntactic
// identity ⊂ semantic equality ⟹ never wrong-accepts, may reject some
// alpha-variants upstream would accept (out of scope per the module header).
// ============================================================================

fn decls_eq_ignoring_span(a: &[cst_v1::StructDeclV1], b: &[cst_v1::StructDeclV1]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| decl_eq(&x.0, &y.0))
}

fn decl_eq(a: &ast_v1::Decl, b: &ast_v1::Decl) -> bool {
    match (a, b) {
        (
            ast_v1::Decl::Val {
                stage: s1,
                name: n1,
                quant: q1,
                ty: t1,
                ..
            },
            ast_v1::Decl::Val {
                stage: s2,
                name: n2,
                quant: q2,
                ty: t2,
                ..
            },
        ) => {
            // `val ~x : t` and `val x : t` declare DIFFERENT members (one is
            // written at stage 0, the other at the document stage), so a
            // comparator that ignored the stage would call two unequal
            // signatures equal.
            decl_stage_eq(s1.as_ref(), s2.as_ref())
                && n1.name == n2.name
                && tyvar_list_eq(q1, q2)
                && type_expr_eq(t1, t2)
        }
        (
            ast_v1::Decl::ValHorzCmd {
                cmd: c1,
                quant: q1,
                ty: t1,
                ..
            },
            ast_v1::Decl::ValHorzCmd {
                cmd: c2,
                quant: q2,
                ty: t2,
                ..
            },
        ) => c1.name == c2.name && tyvar_list_eq(q1, q2) && type_expr_eq(t1, t2),
        (
            ast_v1::Decl::ValVertCmd {
                cmd: c1,
                quant: q1,
                ty: t1,
                ..
            },
            ast_v1::Decl::ValVertCmd {
                cmd: c2,
                quant: q2,
                ty: t2,
                ..
            },
        ) => c1.name == c2.name && tyvar_list_eq(q1, q2) && type_expr_eq(t1, t2),
        (
            ast_v1::Decl::TypeOpaque {
                name: n1, kind: k1, ..
            },
            ast_v1::Decl::TypeOpaque {
                name: n2, kind: k2, ..
            },
        ) => n1.name == n2.name && kind_eq(k1, k2),
        (ast_v1::Decl::Type { binds: b1, .. }, ast_v1::Decl::Type { binds: b2, .. }) => {
            type_binds_eq(&b1.0, &b2.0)
        }
        (
            ast_v1::Decl::Module {
                name: n1, sig_: s1, ..
            },
            ast_v1::Decl::Module {
                name: n2, sig_: s2, ..
            },
        ) => n1.name == n2.name && sig_expr_eq(s1, s2),
        (
            ast_v1::Decl::Signature {
                name: n1, sig_: s1, ..
            },
            ast_v1::Decl::Signature {
                name: n2, sig_: s2, ..
            },
        ) => n1.name == n2.name && sig_expr_eq(s1, s2),
        (ast_v1::Decl::Include { sig_: s1, .. }, ast_v1::Decl::Include { sig_: s2, .. }) => {
            sig_expr_eq(s1, s2)
        }
        _ => false,
    }
}

/// Which stage a `val` decl's qualifier declares. Absent is stage 1, `~` is
/// stage 0, `persistent ~` is the persistent stage
/// (`parser_v1.mly:600-603`) — three distinct values out of an
/// `Option<BindStageV1>`, which is why the comparators below go through this
/// rather than deriving `PartialEq` on the token.
fn decl_stage(s: Option<&cst_v1::BindStageV1>) -> types::Stage {
    match s {
        None => types::Stage::Stage1,
        Some(st) if st.persistent.is_some() => types::Stage::Persistent0,
        Some(_) => types::Stage::Stage0,
    }
}

/// Do two `val` decls declare the same stage? (Signature-vs-signature; a
/// signature against an IMPLEMENTATION is [`stage_conforms`], which is a
/// subsumption rather than an equality.)
fn decl_stage_eq(a: Option<&cst_v1::BindStageV1>, b: Option<&cst_v1::BindStageV1>) -> bool {
    decl_stage(a) == decl_stage(b)
}

/// May a binding written at `implemented` satisfy a signature declaring
/// `declared`? Upstream's `signatureSubtyping.ml:286-298`, verbatim: a
/// `persistent` binding is reachable from every stage and so satisfies any
/// declaration, and the other two stages must match exactly. Notably NOT
/// reflexive-by-widening in the other direction — a stage-0 binding does not
/// satisfy a `persistent` declaration, because what the signature promises
/// (nameable from stage 1 too) is more than the binding can do.
///
/// Both stage checks in this file go through here, because upstream runs both
/// through the same rows: the sealed-binding check (`check_program`'s spine
/// walk) and the layer check a PARENT signature performs when it re-declares a
/// nested child's member ([`process_link_member`]). In the latter the
/// "implemented" side is the child's own DECLARED stage — upstream's
/// `subtype_concrete_with_concrete` matches exactly this on `(stage1, stage2)`
/// with `ssig1` the inner signature.
fn stage_conforms(implemented: types::Stage, declared: types::Stage) -> bool {
    use types::Stage::*;
    matches!(
        (implemented, declared),
        (Persistent0, _) | (Stage0, Stage0) | (Stage1, Stage1)
    )
}

fn tyvar_list_eq(a: &[TypeVarTok], b: &[TypeVarTok]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.name == y.name)
}

fn kind_eq(a: &ast_v1::KindV1, b: &ast_v1::KindV1) -> bool {
    a.first.name == b.first.name
        && a.rest.len() == b.rest.len()
        && a.rest
            .iter()
            .zip(b.rest.iter())
            .all(|(x, y)| x.base.name == y.base.name)
}

fn sig_expr_eq(a: &ast_v1::SigExpr, b: &ast_v1::SigExpr) -> bool {
    match (a, b) {
        (
            ast_v1::SigExpr::Functor {
                param: p1,
                dom: d1,
                cod: c1,
                ..
            },
            ast_v1::SigExpr::Functor {
                param: p2,
                dom: d2,
                cod: c2,
                ..
            },
        ) => p1.name == p2.name && sig_expr_eq(d1, d2) && sig_expr_eq(c1, c2),
        (
            ast_v1::SigExpr::WithType {
                base: b1,
                path: p1,
                binds: t1,
                ..
            },
            ast_v1::SigExpr::WithType {
                base: b2,
                path: p2,
                binds: t2,
                ..
            },
        ) => sig_bot_eq(b1, b2) && opt_mod_chain_eq(p1, p2) && type_binds_eq(&t1.0, &t2.0),
        (ast_v1::SigExpr::Bot(x), ast_v1::SigExpr::Bot(y)) => sig_bot_eq(x, y),
        _ => false,
    }
}

fn sig_bot_eq(a: &ast_v1::SigBotV1, b: &ast_v1::SigBotV1) -> bool {
    match (a, b) {
        (ast_v1::SigBotV1::Path(x), ast_v1::SigBotV1::Path(y)) => {
            x.mods == y.mods && x.name == y.name
        }
        (ast_v1::SigBotV1::Var(x), ast_v1::SigBotV1::Var(y)) => x.name == y.name,
        (ast_v1::SigBotV1::Sig { decls: x, .. }, ast_v1::SigBotV1::Sig { decls: y, .. }) => {
            decls_eq_ignoring_span(x, y)
        }
        _ => false,
    }
}

fn opt_mod_chain_eq(a: &Option<ast_v1::ModChainV1>, b: &Option<ast_v1::ModChainV1>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => mod_chain_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

fn mod_chain_eq(a: &ast_v1::ModChainV1, b: &ast_v1::ModChainV1) -> bool {
    match (a, b) {
        (ast_v1::ModChainV1::Long(x), ast_v1::ModChainV1::Long(y)) => {
            x.mods == y.mods && x.name == y.name
        }
        (ast_v1::ModChainV1::Single(x), ast_v1::ModChainV1::Single(y)) => x.name == y.name,
        _ => false,
    }
}

fn type_binds_eq(a: &cst_v1::TypeBindsV1, b: &cst_v1::TypeBindsV1) -> bool {
    type_bind_single_eq(&a.first, &b.first)
        && a.ands.len() == b.ands.len()
        && a.ands
            .iter()
            .zip(b.ands.iter())
            .all(|(x, y)| type_bind_single_eq(&x.bind, &y.bind))
}

fn type_bind_single_eq(a: &cst_v1::TypeBindSingleV1, b: &cst_v1::TypeBindSingleV1) -> bool {
    a.name.name == b.name.name
        && tyvar_list_eq(&a.tyvars, &b.tyvars)
        && type_body_eq(&a.body, &b.body)
}

fn type_body_eq(a: &cst_v1::TypeBodyV1, b: &cst_v1::TypeBodyV1) -> bool {
    match (a, b) {
        (
            cst_v1::TypeBodyV1::Variant {
                first: f1,
                rest: r1,
                ..
            },
            cst_v1::TypeBodyV1::Variant {
                first: f2,
                rest: r2,
                ..
            },
        ) => {
            variant_def_eq(f1, f2)
                && r1.len() == r2.len()
                && r1
                    .iter()
                    .zip(r2.iter())
                    .all(|(x, y)| variant_def_eq(&x.def, &y.def))
        }
        (cst_v1::TypeBodyV1::Synonym(x), cst_v1::TypeBodyV1::Synonym(y)) => type_expr_eq(x, y),
        _ => false,
    }
}

fn variant_def_eq(a: &cst_v1::VariantDefV1, b: &cst_v1::VariantDefV1) -> bool {
    a.ctor.name == b.ctor.name
        && match (&a.of_ty, &b.of_ty) {
            (Some(x), Some(y)) => type_expr_eq(&x.ty, &y.ty),
            (None, None) => true,
            _ => false,
        }
}

fn type_expr_eq(a: &ast_v1::TypeExpr, b: &ast_v1::TypeExpr) -> bool {
    match (a, b) {
        (
            ast_v1::TypeExpr::Fun {
                dom: d1, cod: c1, ..
            },
            ast_v1::TypeExpr::Fun {
                dom: d2, cod: c2, ..
            },
        ) => type_prod_eq(d1, d2) && type_expr_eq(c1, c2),
        (ast_v1::TypeExpr::Atom(p1), ast_v1::TypeExpr::Atom(p2)) => type_prod_eq(p1, p2),
        (
            ast_v1::TypeExpr::OptRowFun {
                opt_dom: o1,
                dom: d1,
                cod: c1,
                ..
            },
            ast_v1::TypeExpr::OptRowFun {
                opt_dom: o2,
                dom: d2,
                cod: c2,
                ..
            },
        ) => type_opt_dom_eq(o1, o2) && type_prod_eq(d1, d2) && type_expr_eq(c1, c2),
        _ => false,
    }
}

fn type_opt_dom_eq(a: &ast_v1::TypeOptDomV1, b: &ast_v1::TypeOptDomV1) -> bool {
    a.inner.entries.len() == b.inner.entries.len()
        && a.inner
            .entries
            .iter()
            .zip(b.inner.entries.iter())
            .all(|(x, y)| x.label.name == y.label.name && type_expr_eq(&x.ty.0, &y.ty.0))
        && a.inner.row_tail.is_some() == b.inner.row_tail.is_some()
}

fn type_prod_eq(a: &ast_v1::TypeProd, b: &ast_v1::TypeProd) -> bool {
    type_app_eq(&a.first, &b.first)
        && a.rest.len() == b.rest.len()
        && a.rest
            .iter()
            .zip(b.rest.iter())
            .all(|(x, y)| type_app_eq(&x.ty, &y.ty))
}

fn type_app_eq(a: &ast_v1::TypeApp, b: &ast_v1::TypeApp) -> bool {
    match (a, b) {
        (
            ast_v1::TypeApp::InlineCmdTy { args: a1, .. },
            ast_v1::TypeApp::InlineCmdTy { args: a2, .. },
        )
        | (
            ast_v1::TypeApp::BlockCmdTy { args: a1, .. },
            ast_v1::TypeApp::BlockCmdTy { args: a2, .. },
        )
        | (
            ast_v1::TypeApp::MathCmdTy { args: a1, .. },
            ast_v1::TypeApp::MathCmdTy { args: a2, .. },
        ) => {
            a1.len() == a2.len()
                && a1.iter().zip(a2.iter()).all(|(x, y)| {
                    opt_type_cmd_opt_dom_eq(&x.opts, &y.opts) && type_expr_eq(&x.ty.0, &y.ty.0)
                })
        }
        (
            ast_v1::TypeApp::AppliedLong {
                ctor: c1,
                first: f1,
                rest: r1,
            },
            ast_v1::TypeApp::AppliedLong {
                ctor: c2,
                first: f2,
                rest: r2,
            },
        ) => {
            c1.mods == c2.mods
                && c1.name == c2.name
                && type_atom_eq(f1, f2)
                && r1.len() == r2.len()
                && r1.iter().zip(r2.iter()).all(|(x, y)| type_atom_eq(x, y))
        }
        (
            ast_v1::TypeApp::Applied {
                ctor: c1,
                first: f1,
                rest: r1,
            },
            ast_v1::TypeApp::Applied {
                ctor: c2,
                first: f2,
                rest: r2,
            },
        ) => {
            c1.name == c2.name
                && type_atom_eq(f1, f2)
                && r1.len() == r2.len()
                && r1.iter().zip(r2.iter()).all(|(x, y)| type_atom_eq(x, y))
        }
        (ast_v1::TypeApp::Atom(x), ast_v1::TypeApp::Atom(y)) => type_atom_eq(x, y),
        _ => false,
    }
}

fn opt_type_cmd_opt_dom_eq(
    a: &Option<ast_v1::TypeCmdOptDomV1>,
    b: &Option<ast_v1::TypeCmdOptDomV1>,
) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            x.entries.len() == y.entries.len()
                && x.entries
                    .iter()
                    .zip(y.entries.iter())
                    .all(|(p, q)| p.label.name == q.label.name && type_expr_eq(&p.ty.0, &q.ty.0))
        }
        (None, None) => true,
        _ => false,
    }
}

fn type_atom_eq(a: &ast_v1::TypeAtom, b: &ast_v1::TypeAtom) -> bool {
    match (a, b) {
        (ast_v1::TypeAtom::Paren { inner: x, .. }, ast_v1::TypeAtom::Paren { inner: y, .. }) => {
            type_expr_eq(&x.0, &y.0)
        }
        (ast_v1::TypeAtom::Record { inner: x, .. }, ast_v1::TypeAtom::Record { inner: y, .. }) => {
            x.fields.len() == y.fields.len()
                && x.fields
                    .iter()
                    .zip(y.fields.iter())
                    .all(|(p, q)| p.name.name == q.name.name && type_expr_eq(&p.ty.0, &q.ty.0))
        }
        (ast_v1::TypeAtom::Var(x), ast_v1::TypeAtom::Var(y)) => x.name == y.name,
        (ast_v1::TypeAtom::LongName(x), ast_v1::TypeAtom::LongName(y)) => {
            x.mods == y.mods && x.name == y.name
        }
        (ast_v1::TypeAtom::Name(x), ast_v1::TypeAtom::Name(y)) => x.name == y.name,
        _ => false,
    }
}

/// The ONE `Decl` arm signature enforcement does not process for real
/// (`Module`/`Signature` go through their own handlers): a `Decl::Include`
/// reaching `prescan_seal_types`'s match is unreachable in practice, since
/// `resolve_sig` splices it away first — a defensive fallback, never a
/// silent narrowing or a panic.
fn non_val_decl_error(module_name: &str, decl: &ast_v1::Decl) -> TypeError {
    let (span, what): (Span, String) = match decl {
        ast_v1::Decl::Include { kw, .. } => (
            kw.0,
            "internal — an `include` decl reached signature enforcement unspliced \
             (expected `resolve_sig` to flatten it first)"
                .to_string(),
        ),
        // `prescan_seal_types` only ever calls this for `Include`.
        _ => (
            Span::default(),
            "this signature declaration is not enforced yet".to_string(),
        ),
    };
    simple_error(
        Some(span),
        format!("module `{module_name}`'s signature: {what}"),
    )
}

/// Include-aware — a `Bind::Include` with a resolved
/// frozen target contributes the target surface's type members as
/// [`ImplTypeInfo`] with [`ImplTypeBody::Synonym`] — NEVER `Variant`, even
/// when the target's own `t` is a variant: the include's copy IS a synonym,
/// and a seal on `P` must NOT hide the TARGET's constructors (they still
/// belong to, and are exported by, `M`).
fn build_impl_type_table(
    binds: &[&cst_v1::Bind],
    mod_path: &[String],
    surfaces: &SurfaceEnv,
) -> HashMap<String, ImplTypeInfo> {
    let mut table = HashMap::new();
    for b in binds.iter().copied() {
        match b {
            cst_v1::Bind::Type { first, ands, .. } => {
                insert_impl_type(&mut table, first);
                for a in ands {
                    insert_impl_type(&mut table, &a.bind);
                }
            }
            cst_v1::Bind::Include { kw, body } => {
                if let ast_v1::ModExpr::Var(_) = &*body.0 {
                    if let Some(Some(target)) =
                        surface::frozen_include_target(surfaces, mod_path, kw.0)
                    {
                        if let Some(target_surf) = surfaces.modules.get(target) {
                            for (tname, arity) in &target_surf.types {
                                table.insert(
                                    tname.clone(),
                                    ImplTypeInfo {
                                        arity: *arity,
                                        body: ImplTypeBody::Synonym,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    table
}

fn insert_impl_type(table: &mut HashMap<String, ImplTypeInfo>, s: &cst_v1::TypeBindSingleV1) {
    let arity = s.tyvars.len();
    let body = match &s.body {
        cst_v1::TypeBodyV1::Variant { first, rest, .. } => {
            let mut ctors = vec![first.ctor.name.clone()];
            ctors.extend(rest.iter().map(|bv| bv.def.ctor.name.clone()));
            ImplTypeBody::Variant(ctors)
        }
        cst_v1::TypeBodyV1::Synonym(_) => ImplTypeBody::Synonym,
    };
    table.insert(s.name.name.clone(), ImplTypeInfo { arity, body });
}

fn flatten_type_binds(binds: &cst_v1::TypeBindsErasedV1) -> Vec<&cst_v1::TypeBindSingleV1> {
    let mut out = vec![&binds.0.first];
    for a in &binds.0.ands {
        out.push(&a.bind);
    }
    out
}

/// `kind`-checking: every base must spell `"o"` — this
/// port only supports first-order kinds; no other kind exists.
fn check_kind_all_o(kind: &ast_v1::KindV1) -> Result<(), TypeError> {
    if kind.first.name != "o" {
        return Err(unsupported_kind_error(&kind.first.name, kind.first.span));
    }
    for r in &kind.rest {
        if r.base.name != "o" {
            return Err(unsupported_kind_error(&r.base.name, r.base.span));
        }
    }
    Ok(())
}

fn unsupported_kind_error(k: &str, span: Span) -> TypeError {
    simple_error(
        Some(span),
        format!("unsupported kind `{k}` — only `o`-kinds exist in this port's first-order slice"),
    )
}

fn width_type_missing_error(module_name: &str, name: &str, span: Span) -> TypeError {
    simple_error(
        Some(span),
        format!(
            "module `{module_name}` signature declares `type {name}` but its \
             `struct .. end` body never defines it"
        ),
    )
}

fn arity_mismatch_error(
    module_name: &str,
    name: &str,
    span: Span,
    declared_arity: usize,
    impl_arity: usize,
) -> TypeError {
    simple_error(
        Some(span),
        format!(
            "module `{module_name}` signature declares `type {name}` with arity \
             {declared_arity} but its implementation defines it with arity {impl_arity}"
        ),
    )
}

/// `with type t` declared with the WRONG arity
/// against the signature's OWN `type t :: kind` — upstream's
/// `KindContradiction` (`moduleTypechecker.ml:465-470`).
fn refine_arity_error(
    module_name: &str,
    name: &str,
    span: Span,
    refine_arity: usize,
    declared_arity: usize,
) -> TypeError {
    simple_error(
        Some(span),
        format!(
            "module `{module_name}`'s signature: `with type {name}` has arity \
             {refine_arity} but the signature declares `type {name}` with arity \
             {declared_arity}"
        ),
    )
}

/// A `with type` refinement whose τ is a
/// variant literal — the standing "re-declaring a variant in a
/// signature is not supported" rule, extended to refinement.
fn refine_variant_body_error(module_name: &str, name: &str, span: Span) -> TypeError {
    simple_error(
        Some(span),
        format!(
            "module `{module_name}`'s signature: a `with type {name}` refinement cannot \
             introduce constructors — re-declaring a variant in a signature is not \
             enforced yet (Sub-slice 2d-3b's rule)"
        ),
    )
}

// ============================================================================
// Phase B: the external-reference rewrite.
// ============================================================================

/// Rewrite EVERY `program.type_decls`/`synonym_decls` through
/// [`rename_type_expr`] (scope = the decl's own qualified name), UNLESS no
/// seal declared any type at all, in which case `None` is returned and the
/// caller passes the ORIGINAL decls through untouched — preserving the
/// bit-parity argument on every seal-free program.
fn maybe_rewrite_program_types<'s>(
    program: &Program<'s>,
    env: &StaticEnv,
) -> Result<Option<(Vec<UserSynonymDecl>, Vec<UserTypeDecl>)>, TypeError> {
    if env.types.is_empty() && env.hidden_types.is_empty() {
        return Ok(None);
    }
    let mut synonym_decls = Vec::with_capacity(program.synonym_decls.len());
    for usd in &program.synonym_decls {
        let ctx = RenameCtx {
            scope: &usd.name,
            force_stamp_owner: None,
            enforce_self_containment: false,
        };
        let body = rename_type_expr(&usd.body, &ctx, env)?;
        synonym_decls.push(UserSynonymDecl {
            name: usd.name.clone(),
            params: usd.params.clone(),
            body,
        });
    }
    let mut type_decls = Vec::with_capacity(program.type_decls.len());
    for utd in &program.type_decls {
        let ctx = RenameCtx {
            scope: &utd.name,
            force_stamp_owner: None,
            enforce_self_containment: false,
        };
        let ctors = utd
            .ctors
            .iter()
            .map(|(cname, payload)| {
                let payload = payload
                    .as_ref()
                    .map(|ty| rename_type_expr(ty, &ctx, env))
                    .transpose()?;
                Ok((cname.clone(), payload))
            })
            .collect::<Result<Vec<_>, TypeError>>()?;
        type_decls.push(UserTypeDecl {
            name: utd.name.clone(),
            params: utd.params.clone(),
            ctors,
        });
    }
    Ok(Some((synonym_decls, type_decls)))
}

/// Parameters shared by every call into the [`rename_type_expr`] family
/// (phases B through C2):
///
/// - `scope`: qualified name of "whoever is asking" — a decl's own name
///   (phase B, C1) or a sealing module's path (phase C2's val/command decls).
/// - `force_stamp_owner`: when `Some(p)`, a reference owned by `p` is
///   stamped even though the owner-scope rule would keep it concrete —
///   phase C2's "own" map (a seal's OWN abstract types must still be opaque
///   on the COMMITTED scheme side). `None` everywhere else.
/// - `enforce_self_containment`: phase C2 only — a bare reference to THIS
///   module's own HIDDEN type is a self-containment error rather than left
///   concrete. `false` elsewhere (an impl's own internal type references
///   are never restricted by its own hiding — hiding is an EXTERNAL
///   concept).
struct RenameCtx<'a> {
    scope: &'a str,
    force_stamp_owner: Option<&'a str>,
    enforce_self_containment: bool,
}

/// The exhaustive `cst::ast::TypeExpr → TypeExpr` renamer (phase B):
/// every type-name atom, bare or applied-head, is resolved
/// against `env.types`/`env.hidden_types` by [`rename_type_name`]. No `_`
/// arm anywhere in this family — a future `cst::ast` type-grammar arm
/// breaks the build here, not silently.
fn rename_type_expr(
    ty: &cst::ast::TypeExpr,
    ctx: &RenameCtx,
    env: &StaticEnv,
) -> Result<cst::ast::TypeExpr, TypeError> {
    Ok(match ty {
        cst::ast::TypeExpr::Fun {
            opts,
            dom,
            arrow,
            cod,
        } => cst::ast::TypeExpr::Fun {
            opts: opts
                .iter()
                .map(|o| {
                    Ok(cst::ast::OptArrowDom {
                        ty: rename_type_prod(&o.ty, ctx, env)?,
                        arrow: o.arrow.clone(),
                    })
                })
                .collect::<Result<_, TypeError>>()?,
            dom: rename_type_prod(dom, ctx, env)?,
            arrow: arrow.clone(),
            cod: Box::new(rename_type_expr(cod, ctx, env)?),
        },
        cst::ast::TypeExpr::Atom(p) => cst::ast::TypeExpr::Atom(rename_type_prod(p, ctx, env)?),
        // `?(l1 : ty1, …) dom -> cod` has no
        // type NAME of its own at this level, so this arm just recurses
        // into every `ty`/`dom`/`cod` sub-expression, like `Fun` above.
        cst::ast::TypeExpr::OptRowFun {
            opt_dom,
            dom,
            arrow,
            cod,
        } => cst::ast::TypeExpr::OptRowFun {
            opt_dom: cst::ast::CstTypeOptDom {
                q: opt_dom.q.clone(),
                paren: opt_dom.paren.clone(),
                entries: opt_dom
                    .entries
                    .iter()
                    .map(|e| {
                        Ok(cst::ast::CstTypeOptEntry {
                            label: e.label.clone(),
                            colon: e.colon.clone(),
                            ty: cst::TyErased(Box::new(rename_type_expr(&e.ty.0, ctx, env)?)),
                            comma: e.comma.clone(),
                        })
                    })
                    .collect::<Result<_, TypeError>>()?,
            },
            dom: rename_type_prod(dom, ctx, env)?,
            arrow: arrow.clone(),
            cod: Box::new(rename_type_expr(cod, ctx, env)?),
        },
    })
}

fn rename_type_prod(
    p: &cst::ast::TypeProd,
    ctx: &RenameCtx,
    env: &StaticEnv,
) -> Result<cst::ast::TypeProd, TypeError> {
    Ok(cst::ast::TypeProd {
        first: rename_type_app(&p.first, ctx, env)?,
        rest: p
            .rest
            .iter()
            .map(|s| {
                Ok(cst::ast::StarType {
                    star: s.star.clone(),
                    ty: rename_type_app(&s.ty, ctx, env)?,
                })
            })
            .collect::<Result<_, TypeError>>()?,
    })
}

fn rename_type_app(
    a: &cst::ast::TypeApp,
    ctx: &RenameCtx,
    env: &StaticEnv,
) -> Result<cst::ast::TypeApp, TypeError> {
    // A bare atom (no constructor): rename it directly.
    if a.rest.is_empty() {
        return Ok(cst::ast::TypeApp {
            head: rename_type_atom(&a.head, ctx, env)?,
            rest: Vec::new(),
        });
    }
    // `arg1 … argN ctor`: arguments rename as ordinary 0-ary atoms; the
    // final constructor renames through `rename_type_name` with the
    // correct arity so the abstract-type arity-check still matches. A
    // `Mod.t` constructor is a qualified reference (unrelated to this
    // pass's dotted-string encoding) and passes through.
    let n = a.rest.len();
    let arity = n; // total arguments = 1 (head) + (n - 1) = n
    let head = rename_type_atom(&a.head, ctx, env)?;
    let mut rest = Vec::with_capacity(n);
    for atom in &a.rest[..n - 1] {
        rest.push(rename_type_atom(atom, ctx, env)?);
    }
    let ctor = match &a.rest[n - 1] {
        cst::ast::TypeAtom::Name(name) => {
            cst::ast::TypeAtom::Name(rename_type_name(&name.name, name.span, ctx, env, arity)?)
        }
        other => rename_type_atom(other, ctx, env)?,
    };
    rest.push(ctor);
    Ok(cst::ast::TypeApp { head, rest })
}

fn rename_type_atom(
    a: &cst::ast::TypeAtom,
    ctx: &RenameCtx,
    env: &StaticEnv,
) -> Result<cst::ast::TypeAtom, TypeError> {
    Ok(match a {
        cst::ast::TypeAtom::Cmd { list, args, kind } => cst::ast::TypeAtom::Cmd {
            list: list.clone(),
            args: args
                .iter()
                .map(|it| {
                    Ok(cst::ast::TypeCmdArgItem {
                        // A `?(l:τ,…)` label type can itself name a sealed
                        // abstract type — recurse each field's `ty` like
                        // the slot's own mandatory `ty` below, or a
                        // seal-pierce leak follows.
                        opt_labels: it
                            .opt_labels
                            .iter()
                            .map(|f| {
                                Ok(cst::ast::TypeCmdOptField {
                                    label: f.label.clone(),
                                    colon: f.colon.clone(),
                                    ty: cst::TyErased(Box::new(rename_type_expr(
                                        &f.ty.0, ctx, env,
                                    )?)),
                                    comma: f.comma.clone(),
                                })
                            })
                            .collect::<Result<_, TypeError>>()?,
                        ty: cst::TyErased(Box::new(rename_type_expr(&it.ty.0, ctx, env)?)),
                        opt: it.opt.clone(),
                        semi: it.semi.clone(),
                    })
                })
                .collect::<Result<_, TypeError>>()?,
            kind: kind.clone(),
        },
        cst::ast::TypeAtom::Paren { paren, inner } => cst::ast::TypeAtom::Paren {
            paren: paren.clone(),
            inner: cst::TyErased(Box::new(rename_type_expr(&inner.0, ctx, env)?)),
        },
        cst::ast::TypeAtom::Record { rec, fields } => cst::ast::TypeAtom::Record {
            rec: rec.clone(),
            fields: fields
                .iter()
                .map(|f| {
                    Ok(cst::ast::TypeRecordField {
                        name: f.name.clone(),
                        colon: f.colon.clone(),
                        ty: cst::TyErased(Box::new(rename_type_expr(&f.ty.0, ctx, env)?)),
                        semi: f.semi.clone(),
                    })
                })
                .collect::<Result<_, TypeError>>()?,
        },
        cst::ast::TypeAtom::Var(v) => cst::ast::TypeAtom::Var(v.clone()),
        cst::ast::TypeAtom::Name(n) => {
            cst::ast::TypeAtom::Name(rename_type_name(&n.name, n.span, ctx, env, 0)?)
        }
        // `Mod.t` — see `TypeApp::AppliedMod`'s arm above: pass through
        // unchanged, not this pass's own dotted-string encoding.
        cst::ast::TypeAtom::NameMod(n) => cst::ast::TypeAtom::NameMod(n.clone()),
        // An open record's row-variable tail names no TYPE (a different
        // namespace `rename_type_name` never sees), so only the field
        // types need renaming; `bar`/`var` pass through unchanged.
        cst::ast::TypeAtom::RecordOpen { orec, inner } => cst::ast::TypeAtom::RecordOpen {
            orec: orec.clone(),
            inner: cst::ast::CstRecordOpenInner {
                fields: inner
                    .fields
                    .iter()
                    .map(|f| {
                        Ok(cst::ast::CstRecordOpenField {
                            name: f.name.clone(),
                            colon: f.colon.clone(),
                            ty: cst::TyErased(Box::new(rename_type_expr(&f.ty.0, ctx, env)?)),
                            comma: f.comma.clone(),
                        })
                    })
                    .collect::<Result<_, TypeError>>()?,
                bar: inner.bar.clone(),
                var: inner.var.clone(),
            },
        },
    })
}

/// The one place a type-name atom's fate is decided (phase B):
///
/// 1. Not a dotted name at all (`"int"`, a builtin/unqualified nominal) —
///    never a sealed key; left unchanged.
/// 2. "Under" `owner`'s scope (the `starts_with("M.N.")` rule) and NOT
///    force-stamped: this is a self-reference. If `enforce_self_
///    containment` and the type is `hidden_types` (defined by the impl but
///    never declared in THIS sig), a self-containment error ("mentions its
///    type … without declaring it"); otherwise
///    left unchanged (concrete).
/// 3. Otherwise (external, or force-stamped self): a `hidden_types` hit is
///    the "not exported" error; an `Abstract` hit renames to its stamp
///    (arity-checked); a `Transparent` hit — or no entry at all (unsealed)
///    — is left unchanged (transparent resolution happens later, through
///    the ordinary synonym table — the θ-for-free argument).
fn rename_type_name(
    name: &str,
    span: Span,
    ctx: &RenameCtx,
    env: &StaticEnv,
    used_arity: usize,
) -> Result<VarTok, TypeError> {
    let Some((owner, local)) = name.rsplit_once('.') else {
        return Ok(VarTok {
            name: name.to_string(),
            span,
        });
    };
    let under = ctx.scope == owner || ctx.scope.starts_with(&format!("{owner}."));
    // `force_stamp_owner` only overrides "stay concrete" for a GENUINELY
    // ABSTRACT type — a transparent or undeclared-but-not-abstract own-type
    // reference must stay concrete regardless, or a non-abstract own
    // reference would wrongly route into the external/hidden-types
    // dispatch below.
    let is_abstract_here = matches!(
        env.types.get(name).map(|d| &d.opacity),
        Some(TypeOpacity::Abstract { .. })
    );
    let forced = ctx.force_stamp_owner == Some(owner) && is_abstract_here;
    if under && !forced {
        if ctx.enforce_self_containment {
            if let Some(hidden_owner) = env.hidden_types.get(name) {
                return Err(simple_error(
                    Some(span),
                    format!(
                        "module `{hidden_owner}`'s signature mentions its type `{local}` \
                         without declaring it"
                    ),
                ));
            }
        }
        return Ok(VarTok {
            name: name.to_string(),
            span,
        });
    }
    if let Some(hidden_owner) = env.hidden_types.get(name) {
        return Err(simple_error(
            Some(span),
            format!(
                "type `{local}` exists in module `{hidden_owner}` but is not exported \
                 by its signature"
            ),
        ));
    }
    if let Some(decl) = env.types.get(name) {
        if let TypeOpacity::Abstract { stamped } = &decl.opacity {
            if decl.arity != used_arity {
                return Err(simple_error(
                    Some(span),
                    format!(
                        "type `{local}` (module `{owner}`) is declared with arity \
                         {} but used with {used_arity} argument(s)",
                        decl.arity
                    ),
                ));
            }
            return Ok(VarTok {
                name: stamped.clone(),
                span,
            });
        }
    }
    Ok(VarTok {
        name: name.to_string(),
        span,
    })
}

// ============================================================================
// Phase C: the seal-table val/type half.
// ============================================================================

fn phase_c_finish<'s>(
    pending: &[PendingSeal],
    links: &[PendingLink],
    ck: &mut Checker<'s>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    for seal in pending {
        for pt in &seal.pending_transparent {
            check_transparent_type(pt, seal, ck, mint, env)?;
        }

        // `seal.value_names`/`other_names` are already
        // include-spliced (phase A) — reusing them here (rather than
        // recomputing off `seal.binds`) is what makes width-checking and
        // hiding see an included member as defined.
        let value_names = &seal.value_names;
        let other_names = &seal.other_names;
        let mut declared: Vec<String> = Vec::new();
        for d in &seal.sig_decls {
            match &*d.0 {
                ast_v1::Decl::Val {
                    kw,
                    stage,
                    name,
                    quant,
                    ty,
                    ..
                } => {
                    process_seal_member(
                        kw.0,
                        &name.name,
                        name.span,
                        decl_stage(stage.as_ref()),
                        quant,
                        ty,
                        CmdShape::None,
                        seal,
                        value_names,
                        other_names,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(name.name.clone());
                }
                ast_v1::Decl::ValHorzCmd {
                    kw, cmd, quant, ty, ..
                } => {
                    process_seal_member(
                        kw.0,
                        &cmd.name,
                        cmd.span,
                        // No stage slot on a command decl (`parser_v1.mly:
                        // 604-607` takes no `bind_stage`), so a command
                        // member is declared at the document stage.
                        types::Stage::Stage1,
                        quant,
                        ty,
                        CmdShape::Inline,
                        seal,
                        value_names,
                        other_names,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(cmd.name.clone());
                }
                ast_v1::Decl::ValVertCmd {
                    kw, cmd, quant, ty, ..
                } => {
                    process_seal_member(
                        kw.0,
                        &cmd.name,
                        cmd.span,
                        types::Stage::Stage1,
                        quant,
                        ty,
                        CmdShape::Block,
                        seal,
                        value_names,
                        other_names,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(cmd.name.clone());
                }
                // `TypeOpaque`/`Type` handled by the `pending_transparent`
                // loop above; `Module`/`Signature`/`Include` already
                // errored in phase A — nothing left for any other arm.
                _ => {}
            }
        }
        for vn in value_names {
            if !declared.contains(vn) {
                let qualified = lower::qualify_type_key(&seal.mod_path, vn);
                // A PARENT-imposed synthetic seal defers                // hiding to the parent's own trigger instead of committing
                // `hidden` immediately — immediate commit would break a
                // SIBLING use of the omitted member elsewhere in the
                // parent's subtree, before the parent's trigger fires.
                match &seal.parent_trigger {
                    Some((trigger, owner)) => {
                        let entry = env
                            .member_revoke_triggers
                            .entry(trigger.clone())
                            .or_insert_with(|| (owner.clone(), Vec::new()));
                        entry.1.push(qualified);
                    }
                    None => {
                        env.hidden.insert(qualified, seal.module_name.clone());
                    }
                }
            }
        }
    }

    // All `PendingSeal`s ran first; NOW the `PendingLink`s    // — links only READ+UPDATE existing `env.seals` entries, so
    // parent-before-child prescan order stops mattering here.
    for link in links {
        let mut declared: Vec<String> = Vec::new();
        for d in &link.decls {
            match &*d.0 {
                ast_v1::Decl::Val {
                    kw,
                    name,
                    stage,
                    quant,
                    ty,
                    ..
                } => {
                    process_link_member(
                        kw.0,
                        &name.name,
                        name.span,
                        decl_stage(stage.as_ref()),
                        quant,
                        ty,
                        link,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(name.name.clone());
                }
                ast_v1::Decl::ValHorzCmd {
                    kw, cmd, quant, ty, ..
                } => {
                    process_link_member(
                        kw.0,
                        &cmd.name,
                        cmd.span,
                        // No stage slot on a command decl (`parser_v1.mly:
                        // 604-607`), exactly as in `process_seal_member`.
                        types::Stage::Stage1,
                        quant,
                        ty,
                        link,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(cmd.name.clone());
                }
                ast_v1::Decl::ValVertCmd {
                    kw, cmd, quant, ty, ..
                } => {
                    process_link_member(
                        kw.0,
                        &cmd.name,
                        cmd.span,
                        types::Stage::Stage1,
                        quant,
                        ty,
                        link,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(cmd.name.clone());
                }
                _ => {}
            }
        }
        // Members `N` exports that `S_N` omits go to the parent revoke
        // list when this link sits under a synthetic, parent-imposed seal
        // chain; otherwise (`link.parent_trigger: None`, `M` IS the
        // top-level seal) hiding is IMMEDIATE, like a real `PendingSeal`'s
        // own `env.hidden` commit.
        let prefix = format!("{}.", link.child_path.join("."));
        let omitted: Vec<String> = env
            .seals
            .keys()
            .filter(|k| k.starts_with(prefix.as_str()) && !k[prefix.len()..].contains('.'))
            .filter(|k| !declared.iter().any(|d| d == &k[prefix.len()..]))
            .cloned()
            .collect();
        if !omitted.is_empty() {
            match &link.parent_trigger {
                Some(trigger) => {
                    let entry = env
                        .member_revoke_triggers
                        .entry(trigger.clone())
                        .or_insert_with(|| (link.parent_name.clone(), Vec::new()));
                    entry.1.extend(omitted);
                }
                None => {
                    for m in omitted {
                        // The member is likely ALREADY in `env.seals`
                        // (from the child's own real inner seal) — phase D
                        // checks `seals` before `hidden`, so this must
                        // also RETRACT the `seals` entry, or the inner
                        // seal would win regardless.
                        env.seals.remove(&m);
                        env.hidden.insert(m, link.parent_name.clone());
                    }
                }
            }
        }
    }
    Ok(())
}

/// The [`PendingLink`] twin of [`process_seal_member`] —/// lower `S_N`'s declared type like a real seal member would, then check
/// INNER ⊑ OUTER (the child's own committed scheme must subsume what the
/// parent additionally declares) and REPLACE the committed scheme with the
/// outer one (rigid/stamp_marker stay INNER, since the spine still enforces
/// the child's own seal); soundness: inferred ⊑ inner (spine) ∧ inner ⊑
/// outer (link) ⟹ inferred ⊑ outer.
#[allow(clippy::too_many_arguments)]
fn process_link_member<'s>(
    _kw_span: Span,
    member_name: &str,
    member_span: Span,
    outer_stage: types::Stage,
    quant: &[TypeVarTok],
    ty: &ast_v1::TypeExpr,
    link: &PendingLink,
    ck: &mut Checker<'s>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    let qualified = lower::qualify_type_key(&link.child_path, member_name);
    let sub_module = link.child_path.join(".");
    let Some(inner) = env.seals.get(&qualified).cloned() else {
        return Err(simple_error(
            Some(member_span),
            format!(
                "module `{}`'s signature declares `val {member_name}` in sub-module `{sub_module}`, \
                 but `{sub_module}`'s own signature does not export `{member_name}`",
                link.parent_name,
            ),
        ));
    };

    // The STAGE half of the layer check — upstream folds this into the same
    // `subtype_concrete_with_concrete`/`ConcStructure` case that checks the
    // type, reporting `NotASubtypeAboutValueStage` on failure (`dev-0-1-0
    // signatureSubtyping.ml:279-298`). Without it a parent's `val ~y` and a
    // child's `val y` never meet — the types unify, so the disagreement
    // would be silently accepted.
    //
    // `inner.stage` is NOT overwritten the way the scheme is below — it
    // stays a check input only (the spine compares the binding against it),
    // which keeps the same transitive soundness argument: bound ⊑ inner
    // (spine) ∧ inner ⊑ outer (here) ⟹ bound ⊑ outer.
    if !stage_conforms(inner.stage, outer_stage) {
        return Err(link_stage_mismatch_error(
            link,
            member_name,
            &inner,
            outer_stage,
        ));
    }

    let cst_ty = lower::lower_type_expr(ty, &link.tyenv).map_err(|e| {
        simple_error(
            Some(e.span),
            format!(
                "module `{}`'s signature: {} ({})",
                link.parent_name, e.construct, e.hint
            ),
        )
    })?;
    let ctx = RenameCtx {
        scope: &sub_module,
        force_stamp_owner: None,
        enforce_self_containment: false,
    };
    let outer_ty = rename_type_expr(&cst_ty, &ctx, env)?;

    let stamp = mint.next();
    let stamp_marker = format!("#{stamp}");
    let mut rigid_map: HashMap<String, MonoType> = HashMap::new();
    for tv in quant {
        rigid_map.insert(
            tv.name.clone(),
            MonoType::Variant(format!("'{}{stamp_marker}", tv.name), Vec::new()),
        );
    }
    let outer_rigid_raw = typecheck::lower_type_expr(&outer_ty, &rigid_map, RustyfiVersion::V0_1);
    let outer_rigid = ck.expand_synonyms_in(&outer_rigid_raw)?;

    sig_subtype::val_subsumes(ck.ctx_mut(), &inner.scheme, &outer_rigid, &stamp_marker)
        .map_err(|e| link_mismatch_error(link, member_name, &inner, &outer_rigid, e))?;

    let mut scheme_vars = Vec::with_capacity(quant.len());
    let mut scheme_map: HashMap<String, MonoType> = HashMap::new();
    for tv in quant {
        let v = types::new_ty_var(0);
        scheme_map.insert(tv.name.clone(), MonoType::Var(v.clone()));
        scheme_vars.push(v);
    }
    let scheme_raw = typecheck::lower_type_expr(&outer_ty, &scheme_map, RustyfiVersion::V0_1);
    let scheme_body = ck.expand_synonyms_in(&scheme_raw)?;
    let outer_scheme = PolyType::from_vars(scheme_vars, Vec::new(), scheme_body);

    if let Some(entry) = env.seals.get_mut(&qualified) {
        entry.scheme = outer_scheme;
    }
    Ok(())
}

/// The stage twin of [`link_mismatch_error`] and the link twin of
/// [`stage_mismatch_error`]: a sub-module member is sealed at one stage and
/// the PARENT's signature re-declares it at another. Both stages are named
/// via [`types::Stage::as_str`], so the reader can see which `~` to add or
/// drop.
fn link_stage_mismatch_error(
    link: &PendingLink,
    member: &str,
    inner: &DeclaredVal,
    outer_stage: types::Stage,
) -> TypeError {
    let sub_module = link.child_path.join(".");
    simple_error(
        Some(inner.span),
        format!(
            "module `{}` does not match its signature: sub-module `{sub_module}`'s value \
             `{member}` is sealed at {} but `{}`'s signature declares it at {}",
            link.parent_name,
            inner.stage.as_str(),
            link.parent_name,
            outer_stage.as_str(),
        ),
    )
}

fn link_mismatch_error(
    link: &PendingLink,
    member: &str,
    inner: &DeclaredVal,
    outer_rigid: &MonoType,
    err: SubsumeError,
) -> TypeError {
    let sub_module = link.child_path.join(".");
    match err {
        SubsumeError::Mismatch(unify_err) => TypeError {
            span: Some(inner.span),
            message: format!(
                "module `{}` does not match its signature: sub-module `{sub_module}`'s value \
                 `{member}` is sealed at type {} but `{}`'s signature declares {outer_rigid}",
                link.parent_name, inner.scheme, link.parent_name,
            ),
            source: Some(unify_err),
        },
        SubsumeError::EscapedSkolem => TypeError {
            span: Some(inner.span),
            message: format!(
                "module `{}` does not match its signature: sub-module `{sub_module}`'s value \
                 `{member}` is less polymorphic than `{}`'s signature declares",
                link.parent_name, link.parent_name,
            ),
            source: None,
        },
    }
}

/// Phase C step 1, "transparent `type t 'a… = τ` equality": lower BOTH
/// sides with the SAME positional rigid tyvar map, expand through the
/// session synonym table, and `unify` — fully rigid, so `unify` degenerates
/// to structural equality. The sig's declared τ is first passed through
/// [`rename_type_expr`] so an external abstract reference inside it
/// resolves to the SAME stamp the impl's already-registered synonym body
/// does.
fn check_transparent_type<'s>(
    pt: &PendingTransparent,
    seal: &PendingSeal,
    ck: &mut Checker<'s>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    let stamp = mint.next();
    let marker = format!("#{stamp}");
    let mut rigid_map: HashMap<String, MonoType> = HashMap::new();
    for tv in pt.quant {
        rigid_map.insert(
            tv.name.clone(),
            MonoType::Variant(format!("'{}{marker}", tv.name), Vec::new()),
        );
    }

    let cst_ty = lower::lower_type_expr(pt.ty, &seal.tyenv).map_err(|e| {
        simple_error(
            Some(e.span),
            format!(
                "module `{}`'s signature: {} ({})",
                seal.module_name, e.construct, e.hint
            ),
        )
    })?;
    let ctx = RenameCtx {
        scope: &pt.qualified,
        force_stamp_owner: None,
        enforce_self_containment: false,
    };
    let renamed_ty = rename_type_expr(&cst_ty, &ctx, env)?;
    let declared_raw = typecheck::lower_type_expr(&renamed_ty, &rigid_map, RustyfiVersion::V0_1);
    let declared_body = ck.expand_synonyms_in(&declared_raw)?;

    let impl_args: Vec<MonoType> = pt
        .quant
        .iter()
        .map(|tv| rigid_map[&tv.name].clone())
        .collect();
    let impl_body = ck.expand_synonyms_in(&MonoType::Variant(pt.qualified.clone(), impl_args))?;

    unify(&declared_body, &impl_body).map_err(|e| TypeError {
        span: Some(pt.span),
        message: format!(
            "module `{}` does not match its signature: type `{}` is declared `= {declared_body}` \
             but its implementation defines `= {impl_body}`",
            seal.module_name,
            local_name(&pt.qualified),
        ),
        source: Some(e),
    })?;

    env.types.insert(
        pt.qualified.clone(),
        DeclaredType {
            arity: pt.quant.len(),
            opacity: TypeOpacity::Transparent(declared_body),
            span: pt.span,
        },
    );
    Ok(())
}

/// Which command-type shape (if any) a `Decl::Val`-family member must
/// unify to, post-expansion (the shape guard).
#[derive(Clone, Copy)]
enum CmdShape {
    None,
    Inline,
    Block,
}

/// Phase C steps 2-3: the shared width/tyvar-closure/lower/rename/
/// stamp/subsumption-prep pipeline for `Decl::Val`/`ValHorzCmd`/`ValVertCmd`
/// — plus the `ext`/`own` [`RenameCtx`] rewrite (the two opacity-entry
/// points) and, for a command decl, the post-expansion shape guard.
#[allow(clippy::too_many_arguments)]
fn process_seal_member<'s>(
    kw_span: Span,
    member_name: &str,
    member_span: Span,
    stage: types::Stage,
    quant: &[TypeVarTok],
    ty: &ast_v1::TypeExpr,
    shape: CmdShape,
    seal: &PendingSeal,
    value_names: &[String],
    other_names: &[String],
    ck: &mut Checker<'s>,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    if !value_names.iter().any(|v| v == member_name) {
        let is_type_or_module = other_names.iter().any(|o| o == member_name);
        return Err(width_missing_error(
            &seal.module_name,
            member_name,
            member_span,
            is_type_or_module,
        ));
    }
    check_tyvar_closure(ty, quant)?;

    let cst_ty = lower::lower_type_expr(ty, &seal.tyenv).map_err(|e| {
        simple_error(
            Some(e.span),
            format!(
                "module `{}`'s signature: {} ({})",
                seal.module_name, e.construct, e.hint
            ),
        )
    })?;

    // `ext` (rigid/check side): THIS seal's own types stay concrete; OTHER
    // sealed modules' abstract types become stamps; a bare own-type name
    // the sig never declared is a self-containment error, but only when
    // this seal opted into type control (`declares_any_type`).
    let ext_ctx = RenameCtx {
        scope: &seal.module_name,
        force_stamp_owner: None,
        enforce_self_containment: seal.declares_any_type,
    };
    let ext_ty = rename_type_expr(&cst_ty, &ext_ctx, env)?;
    // `own` (scheme/committed side): `ext` PLUS this seal's OWN abstract
    // types also become stamps (every scheme escaping the
    // seal mentions only the stamp).
    let own_ctx = RenameCtx {
        scope: &seal.module_name,
        force_stamp_owner: Some(&seal.module_name),
        enforce_self_containment: false,
    };
    let own_ty = rename_type_expr(&cst_ty, &own_ctx, env)?;

    let stamp = mint.next();
    let stamp_marker = format!("#{stamp}");
    let mut scheme_vars = Vec::with_capacity(quant.len());
    let mut scheme_map: HashMap<String, MonoType> = HashMap::new();
    let mut rigid_map: HashMap<String, MonoType> = HashMap::new();
    for tv in quant {
        let v = types::new_ty_var(0);
        scheme_map.insert(tv.name.clone(), MonoType::Var(v.clone()));
        scheme_vars.push(v);
        rigid_map.insert(
            tv.name.clone(),
            MonoType::Variant(format!("'{}{stamp_marker}", tv.name), Vec::new()),
        );
    }

    let scheme_raw = typecheck::lower_type_expr(&own_ty, &scheme_map, RustyfiVersion::V0_1);
    let scheme_body = ck.expand_synonyms_in(&scheme_raw)?;
    let rigid_raw = typecheck::lower_type_expr(&ext_ty, &rigid_map, RustyfiVersion::V0_1);
    let rigid_body = ck.expand_synonyms_in(&rigid_raw)?;

    match shape {
        // Math commands share the `\` sigil with inline commands (`val
        // \frac : math […]` is a `ValHorzCmd`, precedent
        // `typecheck.rs:1441-1443`). Soundness holds regardless: a sig
        // declaring `math […]` for an inline binding still fails at
        // subsumption/unify — this guard is only the early, better-message
        // filter.
        CmdShape::Inline
            if !matches!(&scheme_body, MonoType::InlineCmd(_) | MonoType::MathCmd(_)) =>
        {
            return Err(simple_error(
                Some(kw_span),
                format!(
                    "module `{}`'s signature: a `val {member_name} :` decl needs an \
                     `inline [...]` or `math [...]` command type",
                    seal.module_name
                ),
            ));
        }
        CmdShape::Block if !matches!(&scheme_body, MonoType::BlockCmd(_)) => {
            return Err(simple_error(
                Some(kw_span),
                format!(
                    "module `{}`'s signature: a `val {member_name} :` decl needs a \
                     `block [...]` command type",
                    seal.module_name
                ),
            ));
        }
        _ => {}
    }

    let scheme = PolyType::from_vars(scheme_vars, Vec::new(), scheme_body);
    let qualified = lower::qualify_type_key(&seal.mod_path, member_name);
    env.seals.insert(
        qualified,
        DeclaredVal {
            name: member_name.to_string(),
            scheme,
            rigid: rigid_body,
            span: kw_span,
            stamp_marker,
            stage,
        },
    );
    Ok(())
}

fn width_missing_error(
    module_name: &str,
    member: &str,
    span: Span,
    is_type_or_module: bool,
) -> TypeError {
    let message = if is_type_or_module {
        format!(
            "module `{module_name}` signature declares `val {member} : ..` but its \
             `struct .. end` body defines `{member}` as a type/module, not a value"
        )
    } else {
        format!(
            "module `{module_name}` signature declares `val {member} : ..` but its \
             `struct .. end` body never defines `{member}`"
        )
    };
    simple_error(Some(span), message)
}

/// The struct's defined member names, split into value members (every
/// `Decl::Val`-family width-check candidate) and type/module/signature
/// names, kept separate for `width_missing_error`'s nicer wording.
///
/// include-aware.** At a `Bind::Include` with a resolved
/// frozen target, extends `values`/`others` with the target surface's own
/// member names, IN PLACE at the include's position — preserving
/// interleaved source order, since the ctor-hide/revocation trigger key is
/// "the LAST value member in source order". This function and
/// `v1/lower.rs`'s lowering splice both read the SAME frozen target
/// surface, in the SAME order, so the two stay in lock-step.
fn struct_member_names_spliced(
    binds: &[&cst_v1::Bind],
    mod_path: &[String],
    surfaces: &SurfaceEnv,
) -> (Vec<String>, Vec<String>) {
    let mut values = Vec::new();
    let mut others = Vec::new();
    for b in binds.iter().copied() {
        match b {
            cst_v1::Bind::Value { name, .. } => values.push(name.name.clone()),
            cst_v1::Bind::ValueInline { cmd, .. } => values.push(any_horz_name(cmd)),
            cst_v1::Bind::ValueBlock { cmd, .. } => values.push(any_vert_name(cmd)),
            cst_v1::Bind::ValueMath { cmd, .. } => values.push(any_horz_name(cmd)),
            cst_v1::Bind::ValueRec { first, ands, .. } => {
                values.push(first.name.name.clone());
                for a in ands {
                    values.push(a.clause.name.name.clone());
                }
            }
            cst_v1::Bind::ValueMutable { name, .. } => values.push(name.name.clone()),
            cst_v1::Bind::Type { first, ands, .. } => {
                others.push(first.name.name.clone());
                for a in ands {
                    others.push(a.bind.name.name.clone());
                }
            }
            cst_v1::Bind::Module { name, .. } => others.push(name.name.clone()),
            cst_v1::Bind::Signature { name, .. } => others.push(name.name.clone()),
            cst_v1::Bind::Include { kw, body } => {
                if let ast_v1::ModExpr::Var(_) = &*body.0 {
                    if let Some(Some(target)) =
                        surface::frozen_include_target(surfaces, mod_path, kw.0)
                    {
                        if let Some(target_surf) = surfaces.modules.get(target) {
                            values.extend(target_surf.vals.iter().cloned());
                            others.extend(target_surf.types.iter().map(|(n, _)| n.clone()));
                            others.extend(target_surf.mods.iter().map(|(n, _)| n.clone()));
                            others.extend(target_surf.sigs.iter().cloned());
                        }
                    }
                }
            }
        }
    }
    (values, others)
}

fn any_horz_name(cmd: &AnyHorzCmdTok) -> String {
    match cmd {
        AnyHorzCmdTok::Plain(t) => t.name.clone(),
        AnyHorzCmdTok::Mod(t) => t.name.clone(),
    }
}

fn any_vert_name(cmd: &AnyVertCmdTok) -> String {
    match cmd {
        AnyVertCmdTok::Plain(t) => t.name.clone(),
        AnyVertCmdTok::Mod(t) => t.name.clone(),
    }
}

// ---- the v1 tyvar walker ---------------------------------------------------

/// Every distinct type-variable occurrence in `ty`, with its span — the v1
/// twin of `typecheck.rs::collect_type_vars`, operating over `ast_v1::
/// TypeExpr` instead of `cst::ast::TypeExpr`.
fn collect_v1_type_vars(ty: &ast_v1::TypeExpr, out: &mut Vec<(String, Span)>) {
    match ty {
        ast_v1::TypeExpr::Fun { dom, cod, .. } => {
            collect_v1_type_vars_prod(dom, out);
            collect_v1_type_vars(cod, out);
        }
        ast_v1::TypeExpr::Atom(p) => collect_v1_type_vars_prod(p, out),
        // A labeled-optional domain's entry types are nested type
        // positions — walk each one, same as `dom`/`cod` (the row-variable
        // tail, if any, is a different namespace this walker doesn't
        // track).
        ast_v1::TypeExpr::OptRowFun {
            opt_dom, dom, cod, ..
        } => {
            for e in &opt_dom.inner.entries {
                collect_v1_type_vars(&e.ty.0, out);
            }
            collect_v1_type_vars_prod(dom, out);
            collect_v1_type_vars(cod, out);
        }
    }
}

fn collect_v1_type_vars_prod(p: &ast_v1::TypeProd, out: &mut Vec<(String, Span)>) {
    collect_v1_type_vars_app(&p.first, out);
    for st in &p.rest {
        collect_v1_type_vars_app(&st.ty, out);
    }
}

fn collect_v1_type_vars_app(a: &ast_v1::TypeApp, out: &mut Vec<(String, Span)>) {
    match a {
        ast_v1::TypeApp::Applied { first, rest, .. }
        | ast_v1::TypeApp::AppliedLong { first, rest, .. } => {
            collect_v1_type_vars_atom(first, out);
            for r in rest {
                collect_v1_type_vars_atom(r, out);
            }
        }
        // Command-type argument slots are full `TypeExpr`s        // — walk each one. A slot's `?(l:τ,…)` optional-label bundle is the
        // same kind of nested type position — walk its field types too, or
        // a quantified var used ONLY inside a bundle would go unregistered.
        ast_v1::TypeApp::InlineCmdTy { args, .. }
        | ast_v1::TypeApp::BlockCmdTy { args, .. }
        | ast_v1::TypeApp::MathCmdTy { args, .. } => {
            for a in args {
                if let Some(dom) = &a.opts {
                    for e in &dom.entries {
                        collect_v1_type_vars(&e.ty.0, out);
                    }
                }
                collect_v1_type_vars(&a.ty.0, out);
            }
        }
        ast_v1::TypeApp::Atom(at) => collect_v1_type_vars_atom(at, out),
    }
}

fn collect_v1_type_vars_atom(a: &ast_v1::TypeAtom, out: &mut Vec<(String, Span)>) {
    match a {
        ast_v1::TypeAtom::Paren { inner, .. } => collect_v1_type_vars(&inner.0, out),
        // A closed record type's field types are nested type positions —
        // walk each one, same as `InlineCmdTy`/`BlockCmdTy`'s `args` above.
        ast_v1::TypeAtom::Record { inner, .. } => {
            for f in &inner.fields {
                collect_v1_type_vars(&f.ty.0, out);
            }
        }
        ast_v1::TypeAtom::Var(tv) => out.push((tv.name.clone(), tv.span)),
        // `M.t` is an ABSOLUTE reference — it can never        // spell one of THIS decl's own quantified type variables.
        ast_v1::TypeAtom::LongName(_) => {}
        ast_v1::TypeAtom::Name(_) => {}
    }
}

/// Every type variable `ty` mentions must be bound by `quant`
/// — deliberately stricter than `typecheck.rs::lower_type_atom`'s
/// permissive fresh-var fallback (permissiveness here would silently weaken
/// the seal).
fn check_tyvar_closure(ty: &ast_v1::TypeExpr, quant: &[TypeVarTok]) -> Result<(), TypeError> {
    let mut found = Vec::new();
    collect_v1_type_vars(ty, &mut found);
    for (name, span) in found {
        if !quant.iter().any(|q| q.name == name) {
            return Err(simple_error(
                Some(span),
                format!(
                    "type variable '{name} is not bound by this `val` declaration's \
                     quantifier list"
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolStore;
    use crate::{elaborate, primitives};
    use rustyfi_syntax::{leaf::KwIn, parse_file_v1};

    /// Elaborate a bare V0_1 document expression (no dependency libraries)
    /// into a `Program`, the way `l3_per_binding_tests::elaborate_src` does
    /// for 0.0.6 sources.
    fn elaborate_doc_only<'s>(store: &'s SymbolStore, doc_src: &str) -> Program<'s> {
        let doc_file = parse_file_v1(doc_src).unwrap_or_else(|e| panic!("v1 parse failed: {e}"));
        let body = lower::lower_document_v1(&doc_file)
            .unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
        let eoi = match &doc_file {
            cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
            _ => unreachable!("doc_src must parse as a FileV1::Document"),
        };
        let file = rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude: Vec::new(),
            in_kw: Some(KwIn(Span::default())),
            body: Some(body),
            eoi,
        };
        let env0 = primitives::base_env_with_version(RustyfiVersion::V0_1);
        let scope = elaborate::Scope::new(store, env0.names());
        elaborate::elaborate_program(&file, &scope).unwrap_or_else(|e| panic!("elaborate: {e}"))
    }

    /// Elaborate a dependency library (`module … = struct … end`) plus a
    /// document body together, the same assembly `lib.rs`/`v01_modules.rs`
    /// tests use.
    fn elaborate_with_lib<'s>(
        store: &'s SymbolStore,
        lib_src: &str,
        doc_src: &str,
    ) -> (cst_v1::FileV1, Program<'s>) {
        let lib_file = parse_file_v1(lib_src).unwrap_or_else(|e| panic!("lib parse failed: {e}"));
        let prelude =
            lower::lower_file_v1(&lib_file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let doc_file = parse_file_v1(doc_src).unwrap_or_else(|e| panic!("doc parse failed: {e}"));
        let body = lower::lower_document_v1(&doc_file)
            .unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
        let eoi = match &doc_file {
            cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
            _ => unreachable!("doc_src must parse as a FileV1::Document"),
        };
        let file = rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: Some(KwIn(Span::default())),
            body: Some(body),
            eoi,
        };
        let env0 = primitives::base_env_with_version(RustyfiVersion::V0_1);
        let scope = elaborate::Scope::new(store, env0.names());
        let program = elaborate::elaborate_program(&file, &scope)
            .unwrap_or_else(|e| panic!("elaborate: {e}"));
        (lib_file, program)
    }

    /// `check_program` ≡ `typecheck_verbose_with_version` on seal-free
    /// inputs, driven at the module-checker layer. `deps` is empty: no
    /// module system is involved, isolating the parity claim to step 0's
    /// session-setup order.
    fn assert_parity_no_deps(doc_src: &str) {
        let store = SymbolStore::new();
        let program = elaborate_doc_only(&store, doc_src);
        let whole = typecheck::typecheck_verbose_with_version(&program, RustyfiVersion::V0_1);
        let per_binding = check_program(&[], &program);
        match (whole, per_binding) {
            (Ok(w1), Ok(w2)) => assert_eq!(w1, w2, "warnings differ for {doc_src:?}"),
            (Err(e1), Err(e2)) => {
                assert_eq!(
                    format!("{e1}"),
                    format!("{e2}"),
                    "error strings differ for {doc_src:?}"
                )
            }
            (Ok(w), Err(e)) => panic!(
                "{doc_src:?}: whole-program accepted (warnings={w:?}), check_program rejected: {e}"
            ),
            (Err(e), Ok(w)) => panic!(
                "{doc_src:?}: whole-program rejected ({e}), check_program accepted (warnings={w:?})"
            ),
        }
    }

    #[test]
    fn seal_free_parity_no_deps() {
        let cases: &[&str] = &[
            "let x = 1 in x + 1",
            "let x = 1 in x + true",
            "let id = fun x -> x in (id 1, id true)",
            "let rec even n = if n <= 0 then true else odd (n - 1) \
             and odd n = if n <= 0 then false else even (n - 1) in even 4",
            "let mutable c <- 0 in c <- !c + 1",
        ];
        for src in cases {
            assert_parity_no_deps(src);
        }
    }

    /// The module-bearing twin of the whole-program/per-binding parity test
    /// above: an UNSEALED dependency exercises the
    /// driver's `with_all` fallback arm against the same whole-program
    /// comparison. The module ALSO declares a variant type,
    /// a synonym, and a command — exercising phase B's empty-map fast path
    /// even though `program.type_decls`/`synonym_decls` are non-empty.
    #[test]
    fn seal_free_parity_with_unsealed_module() {
        let lib_src = "module M = struct\n\
                       val x = 1\n\
                       val f y = y\n\
                       type t = | A of int | B\n\
                       type sz = int\n\
                       val inline ctx \\show x = read-inline ctx (embed-string (arabic x))\n\
                       end";
        for doc_src in [
            "M.x + 1",
            "M.f true",
            "M.x + M.f 1",
            "let y = A 1 in match y with A n -> n | B -> 0 end",
        ] {
            let store = SymbolStore::new();
            let (lib_file, program) = elaborate_with_lib(&store, lib_src, doc_src);
            let whole = typecheck::typecheck_verbose_with_version(&program, RustyfiVersion::V0_1);
            let per_binding = check_program(&[&lib_file], &program);
            match (whole, per_binding) {
                (Ok(w1), Ok(w2)) => assert_eq!(w1, w2, "warnings differ for {doc_src:?}"),
                (Err(e1), Err(e2)) => assert_eq!(
                    format!("{e1}"),
                    format!("{e2}"),
                    "error strings differ for {doc_src:?}"
                ),
                (Ok(w), Err(e)) => panic!(
                    "{doc_src:?}: whole-program accepted ({w:?}), check_program rejected: {e}"
                ),
                (Err(e), Ok(w)) => panic!(
                    "{doc_src:?}: whole-program rejected ({e}), check_program accepted ({w:?})"
                ),
            }
        }
    }

    /// Pins [`rewrite_hidden_error`]'s string coupling: the exact
    /// unbound-variable format `typecheck.rs`'s `Ast::Var` arm produces
    /// must still trigger the rewrite. If this test fails, that message
    /// was reworded — fix the PREFIX/SUFFIX constants in the SAME commit.
    #[test]
    fn hidden_rewrite_matches_typecheck_unbound_var_format() {
        let mut static_env = StaticEnv::default();
        static_env
            .hidden
            .insert("M.secret".to_string(), "M".to_string());
        let err = TypeError {
            span: Some(Span::default()),
            message: "internal error: unbound variable 'M.secret' reached the typechecker"
                .to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(err, &static_env);
        assert!(
            rewritten.message.contains("exists in module `M`")
                && rewritten.message.contains("not exported by its signature"),
            "{}",
            rewritten.message
        );
    }

    /// A DIFFERENT unbound-name error (not recorded in `hidden`) passes
    /// through unrewritten.
    #[test]
    fn hidden_rewrite_ignores_unrelated_unbound_names() {
        let static_env = StaticEnv::default();
        let err = TypeError {
            span: None,
            message: "internal error: unbound variable 'q' reached the typechecker".to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(err, &static_env);
        assert_eq!(
            rewritten.message,
            "internal error: unbound variable 'q' reached the typechecker"
        );
    }

    /// A HIDDEN command member's use, both inline
    /// and block, must rewrite through `static_env.hidden` too (the earlier
    /// check only matched the plain-variable format).
    #[test]
    fn hidden_rewrite_matches_command_formats() {
        let mut static_env = StaticEnv::default();
        static_env
            .hidden
            .insert("M.\\hidden".to_string(), "M".to_string());
        static_env
            .hidden
            .insert("M.+hidden".to_string(), "M".to_string());
        let inline_err = TypeError {
            span: None,
            message: "internal error: unbound inline command 'M.\\hidden' reached the typechecker"
                .to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(inline_err, &static_env);
        assert!(
            rewritten.message.contains("exists in module `M`")
                && rewritten.message.contains("not exported by its signature"),
            "{}",
            rewritten.message
        );
        let block_err = TypeError {
            span: None,
            message: "internal error: unbound block command 'M.+hidden' reached the typechecker"
                .to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(block_err, &static_env);
        assert!(
            rewritten.message.contains("exists in module `M`")
                && rewritten.message.contains("not exported by its signature"),
            "{}",
            rewritten.message
        );
    }

    /// A hidden constructor's use, both expression
    /// and pattern sites, rewrites through `static_env.hidden_ctors`.
    #[test]
    fn hidden_rewrite_matches_ctor_formats() {
        let mut static_env = StaticEnv::default();
        static_env.hidden_ctors.insert(
            "T".to_string(),
            HiddenCtor {
                module: "M".to_string(),
                type_name: "M.t".to_string(),
            },
        );
        let expr_err = TypeError {
            span: None,
            message: "unknown constructor 'T'".to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(expr_err, &static_env);
        assert!(
            rewritten
                .message
                .contains("constructor `T` belongs to type `t`")
                && rewritten
                    .message
                    .contains("module `M`'s signature seals abstract"),
            "{}",
            rewritten.message
        );
        let pat_err = TypeError {
            span: None,
            message: "unknown constructor 'T' in a pattern".to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(pat_err, &static_env);
        assert!(
            rewritten
                .message
                .contains("constructor `T` belongs to type `t`")
                && rewritten
                    .message
                    .contains("module `M`'s signature seals abstract"),
            "{}",
            rewritten.message
        );
    }

    /// `strip_stamps` removes every maximal `#N`
    /// run and nothing else.
    #[test]
    fn strip_stamps_removes_only_hash_digit_runs() {
        assert_eq!(strip_stamps("M.t#3 vs int"), "M.t vs int");
        assert_eq!(strip_stamps("'a#12 -> 'a#12"), "'a -> 'a");
        assert_eq!(strip_stamps("no stamps here"), "no stamps here");
        assert_eq!(
            strip_stamps("a # b"),
            "a # b",
            "a bare '#' with no digits is left alone"
        );
    }

    // ========================================================================
    // The functor parameter-signature check +    // generativity-by-path pins.
    // ========================================================================

    /// Shared functor fixture for the tests below: `Make = fun (Key : Ord) ->
    /// struct val cmp2 x = Key.compare x x end`, applied to two
    /// differently-shaped arguments (`IntOrd`/`FlagOrd`) — the parameter
    /// type is forced by the REAL `Key.compare` reference, a genuine
    /// application constraint, not a possibly-unenforced ascription.
    const FUNCTOR_LIB: &str = "\
module Lib = struct
  signature Ord = sig
    type t :: o
    val compare : t -> t -> int
  end
  module IntOrd = struct
    type t = int
    val compare x y = x - y
  end
  module FlagOrd = struct
    type t = | Yes | No
    val compare x y = 0
    val y () = Yes
  end
  module Make = fun (Key : Ord) -> struct
    val cmp2 x = Key.compare x x
  end
  module A = Make IntOrd
  module B = Make FlagOrd
end
";

    /// A functor applied to an argument that satisfies its
    /// parameter signature — `check_program` accepts.
    #[test]
    fn t_chk1_functor_param_sig_accept() {
        let store = SymbolStore::new();
        let (lib_file, program) = elaborate_with_lib(&store, FUNCTOR_LIB, "Lib.A.cmp2 1");
        check_program(&[&lib_file], &program)
            .expect("IntOrd satisfies Ord — check_program accepts");
    }

    /// Generativity: `Make IntOrd` and `Make FlagOrd` are two
    /// INDEPENDENT instantiations — `Lib.A.cmp2`'s parameter type is forced
    /// to `IntOrd.t` (= `int`) by its own substituted `Key.compare`
    /// reference, which does NOT accept a `FlagOrd.t`-typed value (via
    /// `Lib.FlagOrd.y ()`, since a bare qualified ctor has no
    /// expression-grammar support here) — even though both instantiations
    /// share the IDENTICAL functor body, pinning that each application is
    /// checked against its OWN substituted argument.
    #[test]
    fn t_chk3_functor_generativity_distinct_instantiations_do_not_cross_unify() {
        let store = SymbolStore::new();
        let (lib_file, program) =
            elaborate_with_lib(&store, FUNCTOR_LIB, "Lib.A.cmp2 (Lib.FlagOrd.y ())");
        let err = check_program(&[&lib_file], &program)
            .expect_err("A's `cmp2` requires an IntOrd-shaped argument, not FlagOrd's `Yes`");
        assert!(!err.message.is_empty());
    }

    /// The core acceptance-negative test: a functor applied to an
    /// argument MISSING a member its parameter signature requires
    /// (`BadOrd` has no `compare`) is REJECTED with a functor-framed
    /// message — even though the functor body never itself calls
    /// `compare`, so only the dedicated width check
    /// (`check_functor_applications`) catches it.
    #[test]
    fn t_chk2_functor_param_sig_mismatch_missing_member_is_rejected() {
        let lib = "\
module Lib = struct
  signature Ord = sig
    type t :: o
    val compare : t -> t -> int
  end
  module BadOrd = struct
    type t = int
  end
  module Make = fun (Key : Ord) -> struct
    val f (x : Key.t) = x
  end
  module A = Make BadOrd
end
";
        let store = SymbolStore::new();
        let (lib_file, program) = elaborate_with_lib(&store, lib, "1");
        let err = check_program(&[&lib_file], &program)
            .expect_err("BadOrd is missing `compare` — Ord's width check must reject");
        assert!(
            err.message.contains("does not match functor") && err.message.contains("compare"),
            "{}",
            err.message
        );
    }
}
