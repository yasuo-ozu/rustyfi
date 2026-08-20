//! Sub-slice 2d-1 (`…/tmp/slice2d-sealing.md` §2.3, §4.2), extended by
//! Sub-slice 2d-2 (`…/tmp/slice2d2-opaque-types.md`): the per-binding
//! module-checking driver — the V0_1 replacement for `typecheck::
//! typecheck_with_version` at `lib.rs`'s V0_1 pipeline (upstream analogue:
//! `moduleTypechecker.ml:596 typecheck_module` + `coerce_signature:375`,
//! collapsed onto the flat-spine model `elaborate.rs` already produces).
//!
//! **Architecture (2d-1 spec §3.3, extended by 2d-2 spec §3): a spine walk
//! keyed by a cst_v1-derived seal table, now built in two passes.** The
//! *signature* information comes from the `cst_v1` trees (they are the only
//! place `:>` survives — `v1/lower.rs` erases it by design, 2d-1 spec §0
//! fact 3), while the *expressions* checked are the elaborated spine's
//! (`program.body`'s `Let`-chain, walked exactly like `typecheck.rs`'s
//! `l3_per_binding_tests::drive_manually`). [`check_program`]'s algorithm,
//! in full (2d-2 spec §3's phase letters):
//!
//! - **Phase A — syntactic seal pre-scan, no [`Checker`]**
//!   ([`phase_a_prescan`]/[`prescan_seal_types`]): walk every dependency's
//!   `cst_v1::FileV1::Library`, recursing through nested `Bind::Module`s
//!   with the identical `mod_path`/`TypeNameEnv` threading `v1/lower.rs`'s
//!   `lower_module_bind` uses. At each sealed module: build the impl-type
//!   table from its `Bind::Type`s; process every `Decl::TypeOpaque` (mint a
//!   fresh stamp via [`StampMint`], width/kind/arity-check, populate
//!   [`StaticEnv::types`]) and `Decl::Type` (width-check, queue a
//!   [`PendingTransparent`] equality check for phase C — no [`Checker`]
//!   exists yet to run it); mark every un-declared impl type
//!   [`StaticEnv::hidden_types`]; compute the ctor-hide list (§2.2) and
//!   record its deferred trigger — the qualified alias of the module's LAST
//!   value member ([`StaticEnv::ctor_hide_triggers`]), or an immediate hide
//!   for a zero-value-member module. `Decl::Val`/`ValHorzCmd`/`ValVertCmd`
//!   are NOT processed here (phase C); `Module`/`Signature`/`Include` decls
//!   still error via [`non_val_decl_error`].
//! - **Phase B — session setup with the external-reference rewrite**
//!   (inside [`check_program`] itself, using [`maybe_rewrite_program_types`]):
//!   mirrors 2d-1's session-setup statement order (`Checker::empty` →
//!   synonyms → `check_cycles` → builtins → variants) exactly, except: when
//!   phase A found ANY sealed type (`StaticEnv::types`/`hidden_types`
//!   non-empty), every `UserSynonymDecl`/`UserTypeDecl` is first passed
//!   through the [`rename_type_expr`] walker (the §2.5 leak fix: a
//!   `type s = M.t` elsewhere in the program must not resolve straight
//!   through `M`'s seal) — an EMPTY-map fast path passes the ORIGINAL decls
//!   through untouched (2d-1's T9 bit-parity argument, extended). Then any
//!   `immediate_hides` (zero-value-member modules) fire via
//!   [`Checker::hide_ctors`].
//! - **Phase C — the seal-table val/type half** ([`phase_c_finish`]): with
//!   the checker session now live, resolve every queued
//!   [`PendingTransparent`] equality check ([`check_transparent_type`] —
//!   rigid-lower both sides with a shared positional tyvar map and `unify`,
//!   store `Transparent(MonoType)`), then re-walk each seal's `Decl::Val`/
//!   `ValHorzCmd`/`ValVertCmd` items ([`process_seal_member`]): width +
//!   tyvar-closure-check as 2d-1, PLUS the new `ext`/`own` [`RenameCtx`]
//!   rewrite before lowering (external sealed-type references become
//!   stamps on BOTH sides; THIS seal's own abstract types stay concrete on
//!   the rigid/check side but become stamps on the committed/scheme side —
//!   2d-2 spec §2.1's "opacity enters at exactly two places"), a command-
//!   type shape guard for `ValHorzCmd`/`ValVertCmd`, then `seals` insert as
//!   2d-1. Un-declared value members join `hidden` as before.
//! - **Phase D — the spine walk with interception** (the main `loop` in
//!   [`check_program`]): `Ast::LetIn`'s alias-commit case looks itself up in
//!   `seals`/`hidden` and either subsumption-checks + commits the DECLARED
//!   scheme (sealing), skips the commit entirely (hiding), or falls through
//!   to the ordinary `with_all` commit — THEN, if this alias is a ctor-hide
//!   trigger, fires [`Checker::hide_ctors`] (§2.2's deferred deregistration:
//!   elaboration emits every member's alias contiguously and in source
//!   order, so the trigger member's commit is the first program point after
//!   which nothing inside the sealing module executes).
//!
//! **Sealing has zero runtime residue** (2d-1 spec §0 fact 3, unchanged by
//! 2d-2): this module reads signature information from the pre-lowering
//! `cst_v1` trees purely for type-checking; the elaborated/compiled/
//! evaluated program never differs from its unsealed twin.
//!
//! **Every outgoing message is stamp-stripped** ([`strip_stamps`], 2d-2 spec
//! §2.6): applied once, at [`check_program`]'s own boundary (folding a
//! `TypeError`'s `source` into its `message` first, since `UnifyError`'s
//! `Display` — a frozen `unify.rs`/`types.rs` surface — is where a raw
//! `M.t#3` would otherwise leak), so no `#N` stamp ever reaches a user-
//! facing string regardless of which phase produced the error.
//!
//! **Sub-slice 2d-3 (`…/tmp/slice2d3-module-sig-decls.md`), landed here:**
//! named signatures resolve at every ascription site ([`resolve_sig_decls`]
//! → `v1/surface.rs::find_sig`, threaded from the same `SurfaceEnv`
//! `v1/lower.rs` builds): a `:> S` / `:> A.B.S` seal re-elaborates the
//! resolved decls through the SAME phase-A/C pipeline an inline `sig … end`
//! would, so opaque types stamp FRESH per site (generativity — spec §2.2,
//! `v01_sealing.rs`'s N7). An unresolved name is a precise "unknown
//! signature name" error. Module ALIASES (`module M = N`, `module M = N :>
//! S`) are handled entirely in `v1/lower.rs` (member-copy expansion) — an
//! UNSEALED alias's copies then type-check as ordinary bindings through the
//! spine walk below, with no special handling needed here.
//!
//! **Sub-slice 2d-3b (`…/tmp/slice2d3b-2f2-sigmembers.md` §3), landed here:**
//! nested-module sig MEMBERS (`Decl::Module { N : S_N }`, recursively matched
//! against the struct's own `Bind::Module { N }` — [`ImplView`] parameterizes
//! [`prescan_seal_types`] over a struct-literal's binds vs. an alias/coerce
//! target's syntactic surface, so an unsealed child gets a SYNTHETIC child
//! seal with PARENT-DEFERRED hiding (`member_revoke_triggers`, §3.4) and a
//! sealed child gets a [`PendingLink`] layer-check instead of clobbering the
//! child's own `env.seals` entries); `Decl::Signature` members (syntactic
//! token-identity equality — semantic `sig_equal` up to alpha-variance is
//! explicitly deferred, §3.5); and seal-narrowing on an alias BODY
//! (`module M :> S = N`, `module M = N :> S` — [`walk_nested_seals_a`]'s
//! four-way body match).
//!
//! **Sub-slice 2f-2b (`…/tmp/slice2d3b-2f2-sigmembers.md` §5), landed here:**
//! functor sig-members inside a `:> sig … end` umbrella (`Decl::Module` whose
//! declared type is a `SigExpr::Functor`) — [`StaticEnv::sealed_functors`]
//! records the declared domain/codomain; every frozen application of such a
//! functor is checked against the declared domain and its RESULT sealed with
//! the declared codomain substituted `[param := arg]` (fresh stamps per
//! application — generative, §0.4's caveat), via a small owned "instantiation
//! store" ([`InstantiatedApp`]) built in a phase A0 pre-pass so the
//! synthesized codomain decls/body outlive the borrowed-from-`deps`
//! `PendingSeal`/`PendingLink` machinery they otherwise reuse unchanged.
//!
//! **Still explicitly out of scope** (§10 of the spec — permanent, sound
//! placeholders, not "not yet enforced"): semantic (alpha-variant)
//! `Decl::Signature` equality; structural (non-identical) functor-domain
//! subtyping; higher-order functors (curried sig-members —
//! [`sig_subtype::SigSubtypeError::NestedFunctorSubstitution`], now
//! reachable); a functor-sig ASCRIPTION directly on a struct bind
//! (`module M :> (X:S)->S2 = struct…`); `Decl::Module`/`Signature` inside a
//! functor's PARAMETER signature; relative-sibling references INSIDE
//! signature bodies (Sub-slice 2f-2a's absolutizer deliberately skips
//! signature bodies); `include` of a sealed functor's application result.
//!
//! **Sub-slice 2e-1, landed here:** `include M` (`Bind::Include`, struct-
//! include — `v1/lower.rs`'s job for real, this module's is bookkeeping)
//! needs NOTHING from the spine walk itself — its member copies elaborate
//! to ordinary qualified `Let`/`Type` binds, which the existing seal
//! machinery already covers. What this module DOES learn about: seal
//! *bookkeeping* sees included members as defined —
//! [`struct_member_names_spliced`] (the include-aware twin of the retired
//! `struct_member_names`) and [`build_impl_type_table`] (also include-
//! aware; an included type member is always [`ImplTypeBody::Synonym`],
//! never `Variant` — a seal must not hide the INCLUDING module's included
//! ctors, since they still belong to, and are exported by, the target)
//! both consult `surfaces`/`v1/surface.rs::frozen_include_target`, never
//! re-resolving.
//!
//! **Sub-slice 2e-2, landed here:** SIG-side `include` (`Decl::Include`)
//! and `with type` (`SigExpr::WithType`) — both live entirely inside
//! signature RESOLUTION ([`resolve_sig`], replacing 2d-3's
//! `resolve_sig_decls`): an `include S` decl is recursively FLATTENED into
//! the enclosing signature's own decl list (a resolved-table-key cycle
//! guard rejects a self-referential chain; a duplicate post-splice is a
//! hard [`check_sig_conflicts`] error), so [`prescan_seal_types`] and every
//! other consumer never sees a `Decl::Include` at all — the whole
//! [`non_val_decl_error`] `Include` row is now purely defensive dead code.
//! `with type` is collected as a [`surface::Refine`] alongside the
//! flattened decls and intercepts [`prescan_seal_types`]'s `TypeOpaque` arm
//! BEFORE it mints a stamp (the Abstract → Transparent rewrite). A functor
//! PARAMETER signature stays checked name/arity-only (2f-1's own posture,
//! unchanged) — a `with type` there is an explicit reject
//! ([`check_functor_applications`]), never silently unenforced.

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
/// outgoing error message (module doc comment, §2.6) — the one thing this
/// wrapper adds over [`check_program_inner`].
pub(crate) fn check_program<'s>(
    deps: &[&cst_v1::FileV1],
    program: &Program<'s>,
) -> Result<Vec<MatchWarning>, TypeError> {
    check_program_inner(deps, program).map_err(strip_stamps_error)
}

fn check_program_inner<'s, 'a>(
    deps: &'a [&'a cst_v1::FileV1],
    program: &Program<'s>,
) -> Result<Vec<MatchWarning>, TypeError> {
    // Sub-slice 2d-3 §2.2/§2.1: the syntactic surface + named-signature
    // table, rebuilt from the SAME `deps` `v1/lower.rs` built its own copy
    // from (pure + cheap; single implementation, `v1/surface.rs`). Feeds
    // named-signature resolution at every ascription site (`sig_decls_of`)
    // and alias-body width surfaces (`ImplView::Alias`, phase C).
    let mut surfaces = SurfaceEnv::default();
    for file in deps.iter().copied() {
        surface::build_file_surface(file, &mut surfaces);
    }

    // ---- phase A: syntactic seal pre-scan (no `Checker`) ----
    let mut static_env = StaticEnv::default();
    let mut mint = StampMint::default();
    let mut immediate_hides: Vec<(String, String)> = Vec::new();

    // Sub-slice 2f-2b (spec §5.1): a lightweight early pass discovering
    // every functor sig-member (`sealed_functors`/`hidden_functors`) BEFORE
    // `check_functor_applications`/phase A0 need to consult them — the main
    // `phase_a_prescan` walk below re-visits the same `Decl::Module`-functor
    // arms and re-registers `sealed_functors` identically (idempotent); its
    // OWN `hidden_functors` computation (per seal, alongside type-hiding)
    // is therefore redundant with — but consistent with — this early pass.
    discover_sealed_functors(deps, &surfaces, &mut static_env)?;

    // Sub-slice 2f-1 §2.2, extended by 2f-2b §5.1 step 5: every frozen
    // functor-application site's parameter-signature check (purely
    // syntactic, off `surfaces` alone) PLUS the hidden-functor check (off
    // `static_env.hidden_functors`, populated by the discovery pass just
    // above) — see `check_functor_applications`'s own doc comment for why
    // this deliberately does NOT otherwise reuse the seal machinery's
    // `Checker`/`StampMint` state.
    check_functor_applications(&surfaces, &static_env)?;

    // Sub-slice 2f-2b §5.2-4: the instantiation store — materialize every
    // sealed-functor application's substituted codomain + body BEFORE
    // `pending`/`links` (plain lexical outliving: `PendingSeal`/
    // `PendingLink` below borrow FROM this local, never the other way).
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
    // The static-env tables (`seals`, `hidden`, `ctor_hide_triggers`, …) are
    // keyed by member-name TEXT — they are built from signature declarations,
    // not from the elaborated tree — so the spine walk resolves each binder
    // symbol back to its text to probe them.
    let store = program.store;
    let mut env = typecheck::base_type_env_with_version(store, RustyfiVersion::V0_1);
    let mut ast: &Ast<'s> = &program.body;
    loop {
        ast = match ast {
            Ast::LetIn(name, value, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::Let { name: *name, value }),
                    &static_env,
                )?;
                let name_text = store.resolve(*name);
                env = match static_env.seals.get(name_text) {
                    // the alias binding of a SEALED member: subsumption-
                    // check, then commit the DECLARED scheme (§4.2 steps
                    // 4-5 — sealing).
                    Some(decl) => {
                        let (_, inferred) = &schemes[0];
                        sig_subtype::val_subsumes(
                            ck.ctx_mut(),
                            inferred,
                            &decl.rigid,
                            &decl.stamp_marker,
                        )
                        .map_err(|e| seal_mismatch_error(name_text, decl, inferred, e))?;
                        env.with(*name, decl.scheme.clone())
                    }
                    // the alias binding of a HIDDEN member: commit NOTHING.
                    None if static_env.hidden.contains_key(name_text) => env,
                    // every ordinary binding (locals, unsealed aliases,
                    // opens).
                    None => env.with_all(schemes),
                };
                // Sub-slice 2d-2 §2.2: this alias may be a deferred
                // ctor-hide trigger (the sealing module's LAST value
                // member) — fire it AFTER the commit above, so the
                // module's own members (which just finished checking)
                // still saw the concrete ctors.
                if let Some(hides) = static_env.ctor_hide_triggers.get(name_text) {
                    ck.hide_ctors(hides);
                }
                // Sub-slice 2d-3b §3.4: this alias may ALSO be a deferred
                // parent-imposed MEMBER-revocation trigger (a `Decl::Module`
                // sig member whose sub-signature omitted values the child
                // itself exports) — fire it AFTER the commit above too, so
                // the child's own members still saw the un-narrowed
                // bindings while they themselves were checking. The
                // `hidden` insert happens ONLY NOW, not in phase A-C —
                // inserting earlier would trip the phase-D skip-commit arm
                // just above and break sibling visibility.
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
                env = env.with_all(schemes);
                body
            }
            Ast::LetRecIn(bindings, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::LetRec(bindings)),
                    &static_env,
                )?;
                env = env.with_all(schemes);
                body
            }
            Ast::LetMutableIn(name, init, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::LetMutable { name: *name, init }),
                    &static_env,
                )?;
                env = env.with_all(schemes);
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
/// through [`rewrite_hidden_error`] (§4.2 step 6).
fn catch_hidden<T>(r: Result<T, TypeError>, static_env: &StaticEnv) -> Result<T, TypeError> {
    r.map_err(|e| rewrite_hidden_error(e, static_env))
}

/// §4.2 step 6, extended by 2d-2 §3 phase D: any `TypeError` propagating out
/// of the spine walk passes through this filter. Five exact message formats
/// are pinned (both from THIS module's own unit tests and from
/// `typecheck.rs`'s call sites, at the time of writing):
///
/// - the plain unbound-variable format (`typecheck.rs`'s `Ast::Var` arm,
///   `:1564`) and the unbound-inline/-block-command formats (`check_itext`/
///   `check_btext`, `:1981`/`:2038`) — all three consult
///   [`StaticEnv::hidden`], which now also carries hidden COMMAND members
///   (2d-1 only matched the plain-variable format, so a hidden command
///   member used to leak the "internal error" wording — closed here, spec
///   test U20);
/// - the two "unknown constructor" formats (`infer_ctor`/`bind_pattern`,
///   `:1833`/`:1923`) — consult [`StaticEnv::hidden_ctors`] (2d-2, §2.2).
///
/// Every other error passes through unchanged.
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

/// 2d-2 spec §2.6: remove every maximal `#[0-9]+` run from a diagnostic
/// string. `#` is unlexable in either version's identifier grammar and no
/// type `Display` otherwise emits it, so this rewrite cannot mangle a
/// legitimate rendering — it can only ever strip a stamp suffix `module_
/// check.rs` itself minted (`StampMint`'s doc comment).
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

/// [`check_program`]'s outermost error transform: fold `source` (a frozen
/// `UnifyError`'s `Display`, which may itself render a stamped nominal) into
/// `message` before stripping, so the ENTIRE rendered diagnostic — not just
/// the hand-written `message` half — is stamp-free (spec §2.6, acceptance
/// item 5). This necessarily drops the `source` chain (`std::error::Error::
/// source`); nothing in the V0_1 pipeline inspects it beyond `Display`.
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
// Phase A: the syntactic seal pre-scan (no `Checker`) — spec §3 phase A.
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
/// checker-needed equality check (spec §3 phase A step 3 / phase C step 1).
/// Borrows straight from the `cst_v1` tree (`deps` outlives the whole
/// `check_program` call), so no cloning of the declared type expression is
/// needed.
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
    /// Sub-slice 2e-2 §2.2: FLATTENED (every `Decl::Include` recursively
    /// spliced away) — owned, since a `resolve_sig` call may collect decls
    /// from more than one borrowed source (a literal `sig … end`'s own
    /// slice AND, through a splice, a NAMED sig's own decls elsewhere in
    /// `deps`) with no single contiguous slice to point at.
    sig_decls: Vec<&'a cst_v1::StructDeclV1>,
    mod_path: Vec<String>,
    tyenv: TypeNameEnv,
    module_name: String,
    pending_transparent: Vec<PendingTransparent<'a>>,
    /// Whether this seal's sig declares AT LEAST ONE `type`/`type ::` decl
    /// of its own — self-containment enforcement (spec §3 phase C step 2)
    /// is opt-in on this flag. A sig with ZERO type decls never restricted
    /// type visibility in the first place (2d-1's pre-2d-2 behavior, pinned
    /// by `v01_sealing.rs`'s T6: `val f : t -> t`/`val mk : t` bare-
    /// referencing an entirely undeclared own type stays accepted); once a
    /// sig declares even one type, it has opted into explicit type control,
    /// and any OTHER bare own-type reference it left undeclared is a real
    /// gap (U13).
    declares_any_type: bool,
    /// Sub-slice 2e-1 §2.1 step 5: this seal's member-name lists, INCLUDE-
    /// SPLICED (via [`struct_member_names_spliced`]) and computed ONCE here
    /// in phase A — [`phase_c_finish`] reads these back instead of
    /// recomputing (`struct_member_names(&seal.binds)` would miss any
    /// included members), which also avoids re-threading `surfaces` into
    /// phase C at all.
    value_names: Vec<String>,
    other_names: Vec<String>,
    /// Sub-slice 2d-3b (spec §3.3-6): `Some((trigger, owner))` for a
    /// SYNTHETIC child seal created by a parent's `Decl::Module` member (an
    /// unsealed or alias/app-bodied child narrowed by an ENCLOSING seal,
    /// never the child's own `:>`) — un-declared value members go to
    /// `StaticEnv::member_revoke_triggers` under `trigger` (diagnostics
    /// naming `owner`, the TRUE revoking ancestor — NOT this seal's own
    /// `module_name`, which is the CHILD being narrowed) instead of
    /// `StaticEnv::hidden` (phase C), and ctor hides join `trigger`'s
    /// `ctor_hide_triggers` entry instead of this seal's own last-value
    /// alias (phase A) — both deferred to the PARENT's own spine point
    /// (§3.3-6's "hiding is parent-deferred" rule). `None` for every real
    /// `:>`/named-sig/instantiated-functor-result seal (today's ONLY case
    /// before 2d-3b) — immediate hiding, unchanged.
    parent_trigger: Option<(String, String)>,
}

/// Sub-slice 2d-3b (`…/tmp/slice2d3b-2f2-sigmembers.md` §3.1): what a
/// signature is checked AGAINST — parameterizes [`prescan_seal_types`] over
/// a struct literal's own binds (2d-1/2d-2's only case, and every
/// `Decl::Module` recursion's synthetic-child-over-an-unsealed-struct case)
/// vs. an alias/coerce/application-result body's already seal-filtered
/// syntactic surface (§3.2's alias-body narrowing; §3.3's synthetic-child-
/// over-an-alias case). A functor application's instantiated result (2f-2b
/// §5.2) reuses `Struct` over its OWNED, substituted body binds (borrowed
/// from the phase-A0 instantiation store, §"instantiation store" below) —
/// no third variant needed.
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

/// Sub-slice 2d-3b §3.3/§3.5: the impl's own module names and signature
/// names, SEPARATELY (distinct from `impl_view_member_names`'s conflated
/// `other_names`, kept as-is for backward-compatible width-error wording
/// elsewhere) — needed to width-check a `Decl::Module`/`Decl::Signature`
/// member and to compute `hidden_sigs`.
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

/// Sub-slice 2d-3b §3.3 step 3: what a `Decl::Module { N : S_N }` member's
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
    /// ever runs, §3.3 step 3's defensive note) — cannot recurse.
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
            // A dependency is always a Library (the loader's
            // `DocumentAsDependency` check already rejects anything else
            // before this is ever reached, mirroring `v1/lower.rs::
            // lower_file_v1`'s own defensive `LowerError` arm).
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
    // Sub-slice 2f-2b (spec §5.2-4): phase A0's per-application abstract
    // codomain seals — one synthetic `PendingSeal` per instantiated,
    // sealed-functor application, immediate hiding (the codomain seal IS
    // the application's own boundary, §5.2-3). Sub-slice 2e-2: a `with
    // type` on a sealed functor's declared codomain is rejected at
    // `collect_instantiations_in_binds`'s own construction site (the
    // instantiation store is fully OWNED, so a borrowed `Refine<'a>` can't
    // ride along — §8-4's ownership note); `cod_decls` itself is already
    // FLATTENED there, so no `refines` slice is threaded through here.
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

/// Recurse through every nested `Bind::Module { .. }` looking for further
/// seals — independent of whether THIS level is itself sealed (2d-1 spec
/// §4.5 test T7). The phase-A twin of 2d-1's `walk_nested_seals`.
///
/// Sub-slice 2d-3b §3.2: a NON-struct module body carrying its OWN seal
/// (`module M :> S = N`, `sig_annot: Some` over a `Var` body; `module M = N
/// :> S`, a `Coerce` body) is now narrowed too — resolved to the target's
/// syntactic surface (`ImplView::Surface`), fed through the SAME
/// `prescan_seal_types` pipeline a struct literal gets. A `Coerce` body
/// whose OUTER `sig_annot` is ALSO present (`module M :> S1 = N :> S2`) is a
/// SEAL CHAIN: the INNER `S2` (on the `Coerce`) becomes the real
/// `PendingSeal` (what the spine enforces via `env.seals`), the OUTER `S1`
/// (the `sig_annot`) becomes a [`PendingLink`] layer on top — innermost
/// first. An `App`/`Functor` body is 2f-2b territory (its own seal, if any,
/// is the per-application codomain seal computed in phase A0 above) — left
/// untouched here.
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
                // The seal-chain rule: the `Coerce`'s OWN `sig_` is always
                // the innermost layer (the real `PendingSeal`); an OUTER
                // `sig_annot`, if present, is an additional link on top —
                // only pushed when the inner narrowing actually applied
                // (`narrow_alias_body`'s own literal-sig scope guard): a
                // link referencing a seal that was never registered would
                // itself misreport as a width error.
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
                                // `M` ITSELF is the "owner" here (its OWN
                                // `:>` is doing the narrowing over `N`'s —
                                // unlike `handle_nested_module_decl`'s
                                // `Decl::Module` case, there is no
                                // grandparent imposing this from further
                                // out).
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
                // 2f-2b territory: an `App`-bodied module's own seal (if
                // any) is the per-application codomain seal (phase A0); a
                // direct functor-literal ascription is an unsupported §10
                // shape. Neither needs anything from this walk.
            }
        }
    }
    Ok(())
}

/// A literal `sig … end` body — the ONLY shape [`narrow_alias_body`]/
/// [`handle_nested_module_decl`]'s `ViaSurface` arm apply full narrowing to
/// (§3.2/§3.3's scope guard, below).
fn sig_is_literal_inline(s: &ast_v1::SigExpr) -> bool {
    matches!(s, ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Sig { .. }))
}

/// Sub-slice 2d-3b §3.2: resolve an alias/coerce body's TARGET to its
/// (possibly seal-filtered) syntactic surface and feed it through
/// `prescan_seal_types` as an `ImplView::Surface` — shared by
/// `walk_nested_seals_a`'s `Var` and `Coerce` arms. An UNRESOLVED target
/// already died in lowering before `check_program` ever runs — defensively
/// skip (never invent).
///
/// **Scope guard**: only applied when `sig` is a LITERAL `sig … end`
/// (`sig_is_literal_inline`); a NAMED signature reference (`Var`/`Path`) is
/// left un-narrowed here — the too-permissive (sound) direction, matching
/// the pre-2d-3b posture for this one shape. Reason: a named sig's own bare
/// type references (e.g. an external type visible only via an `include` at
/// the sig's OWN DEFINITION site) need that DEFINITION site's own
/// [`TypeNameEnv`], which this module has no way to reconstruct for an
/// arbitrary already-elaborated dependency module without a much deeper
/// def-site-tyenv-cache change — out of 2d-3b's scope (pinned by the
/// pre-existing `i9`/`i9b` regression tests, which this exact guard keeps
/// green). A literal inline `sig … end` never has this problem: every type
/// it can reference is either its OWN declared member (handled via
/// [`TypeNameEnv::child_from_names`] off the target's [`ModSurface`]) or an
/// already-absolute `LONG_LOWER` name.
/// Returns whether narrowing was actually applied (the `Coerce` arm's own
/// caller uses this to decide whether an outer seal-chain link is safe to
/// register at all).
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
    // The TARGET's own surface — NOT `alias_path`'s own registered entry
    // (`env.modules[alias_path]`), which `v1/surface.rs::build_binds`
    // already seal-filters by whichever annotation(s) apply to the ALIAS
    // itself (this `sig`, for a `Var` body; possibly a DIFFERENT, OUTER
    // `sig_annot` too, for a `Coerce` body under a seal-chain) — using the
    // self-referential entry here would apply the wrong (or a doubly-
    // narrowed) filter to THIS check.
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

/// One parent-sig layer over a child that ALREADY has its own seal entries
/// (its own `:>`, or an inner seal-chain layer): phase C checks inner ⊑
/// outer per member and REPLACES the committed scheme with the outer one
/// (spec §3.4). Never registers a second `env.seals` entry at the same
/// qualified key — `env.seals`/`env.types` are plain `HashMap`s keyed by
/// qualified name, so a second registration would silently clobber the
/// first (the exact cross-contamination `check_functor_applications`'s doc
/// comment already refuses for a different reason).
struct PendingLink<'a> {
    /// `"M.N"` as segments.
    child_path: Vec<String>,
    /// `"M"`, diagnostics.
    parent_name: String,
    /// `S_N` (or, for a seal chain, the OUTER `S1`), resolved and FLATTENED
    /// (Sub-slice 2e-2 §2.2 — same ownership note as [`PendingSeal::
    /// sig_decls`]). A non-empty `with type` refinement on this layer is
    /// rejected at construction time (§2.3's scope note: a `PendingLink`
    /// layer only re-checks `Decl::Val`-family members, never type decls —
    /// see [`reject_link_refines`]).
    decls: Vec<&'a cst_v1::StructDeclV1>,
    /// For lowering `S_N`'s val types at `M.N`.
    tyenv: TypeNameEnv,
    /// The `Decl::Module`/`Coerce`'s own span — diagnostics-reserved (every
    /// current link error already carries a more precise per-member span).
    #[allow(dead_code)]
    span: Span,
    /// Sub-slice 2d-3b §3.4: the PARENT's own deferred-revoke trigger key
    /// (`Some` when this link's `child_path` sits under a synthetic,
    /// parent-imposed seal chain — practically: always `None` on the
    /// direct `module M :> S1 = N :> S2` chain shape, since `M` itself is
    /// the top-level seal there; reserved for a `Decl::Module` sig member
    /// whose own child is ITSELF sealed under a further-nested parent
    /// seal, a corner 2d-3b's D-tests do not exercise but this field keeps
    /// sound by construction: `None` here means "member omissions become
    /// this link's own module's `hidden` immediately", matching every
    /// tested shape).
    parent_trigger: Option<String>,
}

/// Sub-slice 2f-2b (`…/tmp/slice2d3b-2f2-sigmembers.md` §5.2-4): one
/// materialized sealed-functor application — the OWNED, substituted
/// codomain decls + body binds a per-application abstract-result seal is
/// built from. Fully owned (no lifetime parameter): `deps`-borrowed
/// `PendingSeal`/`PendingLink` reuse it by borrowing FROM this store, which
/// is declared (in `check_program_inner`) BEFORE `pending`/`links` — plain
/// lexical outliving, no arena crate, no self-reference (§5.2-4's "the
/// instantiation store").
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
/// body it seals (spec §3 phase A): width-check + kind/arity-check +
/// stamp-mint every `Decl::TypeOpaque`; width-check + queue every
/// `Decl::Type`; mark every un-named impl type hidden; compute the ctor-hide
/// list and its deferred trigger. `Decl::Module`/`Decl::Signature` members
/// recurse/compare per Sub-slice 2d-3b §3.3/§3.5 (a `Decl::Module` whose
/// declared type is itself a `SigExpr::Functor` is a 2f-2b functor sig-
/// member, §5.1, dispatched separately). Sub-slice 2e-2 §2.3: `refines`
/// (already off `decls` — [`resolve_sig`]'s caller, since `decls` itself
/// always arrives pre-FLATTENED, so `Decl::Include` never reaches this
/// function's own match) intercepts a matching `Decl::TypeOpaque`'s stamp
/// mint — the Abstract → Transparent rewrite BEFORE stamping.
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
    // Sub-slice 2e-2 §2.3 step 3: refine names a `TypeOpaque` decl actually
    // consumed — anything left over at the end of the loop is a precise
    // error (undeclared name, or a second refine of an already-transparent
    // one, upstream's `CannotRestrictTransparentType`/`UndefinedTypeName`).
    // Tracked by INDEX into `refines`, not by name: two chained refines of
    // the SAME name (e.g. a named sig's OWN stored refine plus an outer
    // `S2 with type t = …` at the use site) are two DISTINCT entries, and
    // only the FIRST (in `refines`' order) is ever consumed — the second
    // must still fall through to the "leftover" check below and error
    // there (upstream's ordered-first-match `CannotRestrictTransparentType`
    // semantics, W6's chained-refine pin).
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
                // Sub-slice 2e-2 §2.3 step 1: a `with type` refinement on
                // THIS opaque decl (first match wins — a second refine of
                // the same name falls through to the "leftover refines"
                // check below and errors there, upstream's ordered-first-
                // match semantics) skips the stamp mint entirely — push
                // exactly what a literal transparent decl at this position
                // would have queued instead.
                if let Some((idx, refine)) = refines
                    .iter()
                    .enumerate()
                    .find(|(_, r)| r.name == name.name)
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
            // Sub-slice 2e-2: `decls` always arrives pre-flattened
            // ([`resolve_sig`]'s job) — a `Decl::Include` reaching THIS
            // match is unreachable in practice; a defensive
            // (non-panicking) error rather than `unreachable!()`.
            other @ ast_v1::Decl::Include { .. } => {
                return Err(non_val_decl_error(&module_name, other));
            }
        }
    }

    // Sub-slice 2e-2 §2.3 step 3: any refine no `Decl::TypeOpaque` consumed
    // — either it names a type this signature declares TRANSPARENTLY
    // already (upstream `CannotRestrictTransparentType`) or a name the
    // signature never declares at all (upstream `UndefinedTypeName`).
    for (idx, refine) in refines.iter().enumerate() {
        if consumed_refine_idx.contains(&idx) {
            continue;
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

    // Type hiding (spec §3 phase A step 4): every impl type NOT named by
    // any sig type decl.
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

    // Sub-slice 2d-3b §3.5: signature-member hiding — every struct
    // `signature S = ..` bind this seal never declared.
    let (_, sig_names) = impl_view_mod_and_sig_names(&impl_view);
    for sn in &sig_names {
        if !declared_sigs.contains(sn) {
            env.hidden_sigs
                .insert(lower::qualify_type_key(mod_path, sn), module_name.clone());
        }
    }

    // Sub-slice 2f-2b §5.1 step 5: functor hiding — every DIRECT-child
    // functor (`surfaces.functors`, keyed by full qualified path) this
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

    // Seal point (spec §3 phase A step 5), extended by 2d-3b §3.3-6:
    // parent-imposed hiding is deferred to the PARENT's own trigger.
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

/// Sub-slice 2d-3b §3.3: recursive matching for a non-functor
/// `Decl::Module { N : S_N }` member.
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
    // Sub-slice 2e-2 §9's handoff note: routing this through the SAME
    // `resolve_sig` funnel means `module N : S_incl` (a nested sig member
    // whose own declared signature includes/refines) just works.
    let s_n_resolved = resolve_sig(s_n, &child_path.join("."), surfaces, &child_path)?;
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
            // Sub-slice 2d-3b §3.3, the SAME scope guard as
            // `narrow_alias_body` (its own doc comment explains why): only
            // a LITERAL `sig … end` for `S_N` gets a synthetic child seal
            // here — width is already verified above regardless.
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

/// Sub-slice 2d-3b §3.5: a `Decl::Signature` member — syntactic token-
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

/// Sub-slice 2f-2b §5.1: a functor sig-member (`Decl::Module { Make :
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

/// Sub-slice 2d-3b §3.4: the trigger key for a parent-imposed synthetic
/// child seal — the qualified alias of the PARENT's LAST value member
/// ACROSS ITS WHOLE SUBTREE, in source order (elaboration emits every
/// member's alias contiguously in source order, the 2d-2 §2.2 invariant).
/// `None` when the subtree has zero value members (nothing ever commits, so
/// the ordinary immediate-hiding path — `prescan_seal_types`'s own
/// `value_names.is_empty()` fallback — already suffices).
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

/// Record one ctor-hide entry, updating both the deferred `hide_list` (fed
/// to `Checker::hide_ctors` at this seal's trigger) and `env.hidden_ctors`
/// (consulted by [`rewrite_hidden_error`] for the precise diagnostic — safe
/// to populate immediately, unlike the hide itself, since it only affects
/// error TEXT and is only ever consulted after `self.ctors` genuinely no
/// longer has the entry).
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

/// Sub-slice 2e-2 §2.2/§2.3: what an ascription's `SigExpr` resolves to —
/// the FLATTENED decl list (every `Decl::Include` recursively replaced by
/// the included signature's own decls, in place — [`resolve_sig`]) plus
/// every `with type` refinement collected along the way, in application
/// order ([`surface::Refine`], owned by `v1/surface.rs` since a NAMED
/// signature's own stored refinements — §2.3's "named-sig storage" — are
/// inherited from there too).
struct ResolvedSig<'a> {
    decls: Vec<&'a cst_v1::StructDeclV1>,
    refines: Vec<surface::Refine<'a>>,
}

/// Resolve one ascription's [`ast_v1::SigExpr`] to its FULLY FLATTENED decl
/// list (Sub-slice 2e-2 §2.2, replacing 2d-3's `resolve_sig_decls`, whose
/// non-`Include`/non-`WithType` behavior this function reproduces exactly):
/// a literal `sig … end` is itself, minus every `Decl::Include` it
/// (recursively) contains, spliced in place; a named `Var(S)`/`Path(A.B.S)`
/// resolves outward from `site_path` through `surfaces.sigs`
/// (`v1/surface.rs::find_sig`) — the resolved decls then feed the SAME
/// prescan/phase-C pipeline an inline body would, so opaque types stamp
/// FRESH at THIS site (generativity — spec §2.2, test N7). An unresolved
/// name is the precise "unknown signature name" error; an `include` cycle
/// (through names — literal nesting is finite, §8 risk 7) is a precise
/// error too; a duplicate declaration post-splice is a hard
/// `ConflictInSignature`-shaped error (§2.2's "one linear pass",
/// [`check_sig_conflicts`]); `with type` collects [`surface::Refine`]s
/// (§2.3) rather than resolving further; a functor signature stays its 2f
/// placeholder.
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
        // `S with type t = τ` (§2.2/§2.3): resolve the base (a `SigBotV1` —
        // never itself a `with`, the grammar's own left-recursion note),
        // then APPEND this node's own refines.
        ast_v1::SigExpr::WithType {
            base,
            path: None,
            binds,
            ..
        } => {
            let mut resolved = resolve_sig_bot(base, module_name, surfaces, site_path, visited)?;
            resolved.refines.extend(surface::collect_refines(binds));
            Ok(resolved)
        }
        // `S with M type t = τ` (a sub-module refinement): needs
        // `Decl::Module` members to be elaborated structures — 2d-3b's own
        // recursive-structure territory (§7, §9's handoff note).
        ast_v1::SigExpr::WithType {
            path: Some(chain), ..
        } => Err(simple_error(
            Some(mod_chain_span_v1(chain)),
            format!(
                "module `{module_name}`'s signature: `with type` on a sub-module's type \
                 needs module members in signatures — Sub-slice 2d-3b"
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
/// RESOLVED table key (§8 risk 7 — keying the written suffix would
/// false-positive on two differently-pathed same-suffix sigs), splice its
/// own decls, and inherit its own STORED refines (§2.3's named-sig
/// composition, W6).
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

/// §2.2's flattening: a non-`Include` decl passes through; a `Decl::Include
/// { sig_ }` resolves `sig_` and splices its (recursively flattened) decls
/// AND refines in place — order preserved (deterministic error order, and
/// the spliced opaque decls sit exactly where a later `TypeOpaque`
/// interception, §2.3, or the ctor-hide/revocation "last value member"
/// trigger would expect them, 2e-1's own splice-position argument extended
/// to signatures).
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

/// §2.2's conflict check (upstream `ConflictInSignature`, `staticEnv.ml:
/// 404-428`): one linear pass over the FLATTENED decl list, per-category
/// name sets (vals incl. command keys / types / modules / signatures —
/// mirrors `surface.rs::filter_surface`'s own category walk) — a repeat
/// within a category is a hard error at the SECOND decl's span. Only
/// spliced lists can realistically trip this in practice, but it applies
/// uniformly (a literal sig with two DIRECT `val x` decls now also rejects
/// — the ONE previously-accepted-input behavior 2e-2 tightens, §8 risk 5;
/// pinned by S4).
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
            // unreachable in practice; a defensive (non-panicking) error
            // rather than `unreachable!()`, per this module's "no panics
            // anywhere" posture.
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

/// The SHALLOW twin of [`resolve_sig`] — Sub-slice 2d-3's original
/// single-dereference behavior, UNCHANGED: a literal `sig … end` is
/// itself (`Decl::Include` inside it is NOT flattened); a named `Var`/
/// `Path` resolves outward exactly once. Used ONLY by the two structural
/// (syntactic token-identity) comparators that pre-date 2e-2 and never
/// needed semantic splicing — [`handle_signature_decl`]'s `Decl::Signature`
/// member identity check and [`handle_functor_sig_member`]'s declared-vs-
/// impl parameter-signature identity check — both already handle a literal
/// `Decl::Include` node fine (`decls_eq_ignoring_span`'s own `sig_expr_eq`
/// compares it structurally, like any other decl); a `with type` node,
/// which those two comparators have no way to compare meaningfully (they
/// only compare NAMES/decls, never refines), is an explicit, precise
/// reject here instead — never silently ignored.
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

/// Sub-slice 2e-2 §2.3(b)-6/step-1's PendingLink-scope note: a
/// [`PendingLink`] layer only re-checks `Decl::Val`-family members
/// (`process_link_member`), never type decls — a `with type` refinement
/// on that layer would be silently unenforced rather than really applied,
/// so it is an explicit reject instead (never silence).
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
// Sub-slice 2f-1 §2.2: the functor-application parameter-signature check.
// ============================================================================

/// Walk every frozen `ModExpr::App` resolution ([`surface::AppResolution`],
/// `v1/surface.rs::SurfaceEnv::app_targets`) and width/arity-check the
/// argument's [`ModSurface`] against the functor's parameter signature `S`.
///
/// **Deliberately NAME/ARITY-only, no [`Checker`]/[`StampMint`]/
/// [`StaticEnv`] involvement** — a real ascription re-check (as the spec's
/// §2.2 "factor `check_module_against_sig` out of `prescan_seal_types`"
/// literally proposes) would have to register the SAME qualified member
/// keys (`"Int.compare"`) `prescan_seal_types`/`process_seal_member` already
/// use for the ARGUMENT's own (possibly separate, real) `:>` seal, if it has
/// one — `env.seals`/`env.types` are plain `HashMap`s keyed by that
/// qualified name, so a second registration at the SAME key would silently
/// clobber the argument's own real seal (whichever prescan runs last wins),
/// a genuine cross-contamination risk this port will not take. This check
/// therefore stays entirely within `surfaces` (built once, read-only from
/// here on): it catches a MISSING member/wrong-arity type precisely, with a
/// functor-framed message (T-chk2's primary shape); an argument providing a
/// member at the wrong VALUE TYPE is instead caught naturally, later, when
/// the instantiated body's own use of that member fails to type-check
/// through the ordinary elaborator (a less specific message, but
/// `check_program` still rejects — this module's own "REJECTS, never
/// wrong-accepts" posture, doc comment above).
fn check_functor_applications<'a>(
    surfaces: &'a SurfaceEnv<'a>,
    env: &StaticEnv,
) -> Result<(), TypeError> {
    for (_site_path, _span, resolution) in &surfaces.app_targets {
        let Some(res) = resolution else { continue };
        // Sub-slice 2f-2b §5.1 step 5: an application of a functor an
        // umbrella seal never exported.
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
        // Sub-slice 2e-2 §(b)-6's forced decision: functor parameter
        // signatures are checked NAME/ARITY-only (2f-1's own posture,
        // `check_module_against_sig`'s own doc comment) — a `with type`
        // there is name-invisible (a refinement never changes the name
        // set), so it would be SILENTLY unenforced beyond arity if allowed
        // through. Explicit reject instead, never silence.
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

/// The width/arity half of §2.2's ascription-equivalent check: does
/// `arg_surf` (the argument module's own [`ModSurface`]) provide every
/// member `decls` (the parameter signature `S`'s resolved decl list)
/// declares, at a matching type-arity? A `Decl::Module`/`Signature`/
/// `Include` in a functor's parameter signature is a precise deferred error
/// (no demand package's `Ord`/`Settings`-shaped parameter sig ever declares
/// one).
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
// Sub-slice 2f-2b (`…/tmp/slice2d3b-2f2-sigmembers.md` §5.1): the early
// functor sig-member discovery pass — must run BEFORE `check_functor_
// applications`/phase A0's instantiation store need `StaticEnv::
// sealed_functors`/`hidden_functors`. The main `phase_a_prescan` walk
// re-visits the same `Decl::Module`-functor arms and re-registers
// `sealed_functors` identically (idempotent) via the SAME
// [`handle_functor_sig_member`]; only `hidden_functors` is computed here
// exclusively (mirrored, not duplicated, inside `prescan_seal_types` too —
// see that function's own hidden-functor loop).
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
            // Sub-slice 2e-2: best-effort (swallowed, never `?`) — this
            // pass's own job is EARLY functor-member discovery; a
            // resolution failure here (including a genuinely malformed
            // sig) still surfaces for REAL later, when `phase_a_prescan`
            // re-runs the identical `resolve_sig` call and does NOT
            // swallow it. Using the DEEP resolver (rather than a shallow
            // one) closes a gap the pre-2e-2 code never had to consider: a
            // functor sig-member reachable only through a spliced
            // `include` is now discovered too.
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
// Sub-slice 2f-2b §5.2-4: the instantiation store — one [`InstantiatedApp`]
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
                                    // Sub-slice 2e-2: `include` in a sealed
                                    // functor's codomain IS supported (the
                                    // decls are cloned out immediately, so
                                    // no self-reference risk); `with type`
                                    // there is an explicit reject instead —
                                    // `InstantiatedApp` is fully OWNED (no
                                    // lifetime parameter, §8-4's ownership
                                    // note), and a `Refine<'a>` borrows the
                                    // (here, LOCAL/substituted) `cst_v1`
                                    // tree, so it cannot ride along without
                                    // an owned `Refine` shape this
                                    // zero-demand corner does not warrant.
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
// Sub-slice 2d-3b §3.5: syntactic (span-ignoring) structural equality over
// `ast_v1` decl trees — the comparator `handle_signature_decl`'s
// `Decl::Signature` identity check and `handle_functor_sig_member`'s
// declared-vs-impl parameter-signature identity check both drive. Deliberately
// NOT the full `Unparse`-to-token-stream comparator the spec's own wording
// suggests (`syan`'s `Unparse` trait is not a direct dependency of this
// crate, and adding one would touch `Cargo.toml`/`Cargo.lock` — both frozen
// for this increment); this hand-written structural walk is EQUIVALENT
// soundness-wise (Parse/Unparse round-trip: identical structural trees minus
// spans ⟺ identical token streams), just implemented locally. Sound in the
// same direction as the spec's own comparator: syntactic identity ⊂
// semantic equality ⟹ never wrong-accepts, may rejects some alpha-variants
// upstream would accept (documented, §10 row).
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

/// Do two `val` decls declare the same stage? Absent is stage 1, `~` is
/// stage 0, `persistent ~` is the persistent stage
/// (`parser_v1.mly:600-603`) — three distinct values out of an
/// `Option<BindStageV1>`, which is why this is not a plain `==`.
fn decl_stage_eq(a: Option<&cst_v1::BindStageV1>, b: Option<&cst_v1::BindStageV1>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.persistent.is_some() == y.persistent.is_some(),
        _ => false,
    }
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

/// §4.7's placeholder table, now down to the ONE `Decl` arm that stays
/// unenforced after Sub-slice 2d-3b (`Module`/`Signature` members are now
/// processed for real — see [`handle_nested_module_decl`]/
/// [`handle_signature_decl`]/[`handle_functor_sig_member`]).
/// Sub-slice 2e-2: after the sig-include flattening lands, a `Decl::Include`
/// reaching `prescan_seal_types`'s own decl match is unreachable in
/// practice (`resolve_sig` always splices it away first) — this arm is now
/// a purely DEFENSIVE fallback (never a silent narrowing, never a panic),
/// not a feature placeholder.
fn non_val_decl_error(module_name: &str, decl: &ast_v1::Decl) -> TypeError {
    let (span, what): (Span, String) = match decl {
        ast_v1::Decl::Include { kw, .. } => (
            kw.0,
            "internal — an `include` decl reached signature enforcement unspliced \
             (expected `resolve_sig` to flatten it first)"
                .to_string(),
        ),
        // `prescan_seal_types` only ever calls this for `Include`; kept
        // total (no `unreachable!`) as a defensive fallback.
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

/// Sub-slice 2e-1 §2.1 step 5: include-aware — a `Bind::Include` with a
/// resolved frozen target contributes each of the target surface's type
/// members as an [`ImplTypeInfo`] with [`ImplTypeBody::Synonym`] — NEVER
/// `Variant`, even when the target's own `t` is a variant: the include's
/// copy IS a synonym (`type ⟨P⟩.t = M.t`, `alias_member_decls`'s output),
/// and — decisively — a seal on `P` abstracting an included variant type
/// must NOT hide the TARGET's constructors (they still belong to, and are
/// exported by, `M`; hiding them here would break `M`'s own users). This is
/// the sound direction: upstream would hide `P`'s own ctor re-exports while
/// leaving `M`'s originals untouched; the port's flat ctor namespace can
/// only keep the originals.
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

/// `kind` (§3.1's kind-checking rules): every base must spell `"o"` — this
/// port's first-order slice has no other kind.
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

/// Sub-slice 2e-2 §2.3 step 1: `with type t` declared with the WRONG arity
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

/// Sub-slice 2e-2 §2.3 step 1 / §7: a `with type` refinement whose τ is a
/// variant literal — 2d-3b's own standing "re-declaring a variant in a
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
// Phase B: the external-reference rewrite (spec §2.5, §3 phase B).
// ============================================================================

/// Rewrite EVERY `program.type_decls`/`synonym_decls` through
/// [`rename_type_expr`] (scope = the decl's own qualified name — the
/// generic owner-scope rule, §2.5), UNLESS no seal declared any type at all
/// (`env.types` and `env.hidden_types` both empty), in which case `None` is
/// returned and the caller passes the ORIGINAL `program` decls through
/// untouched — preserving 2d-1's T9 bit-parity argument on every seal-free
/// program (spec §3 phase B, "empty-map fast path").
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
/// (spec §2.5/§3 phase B-C2):
///
/// - `scope`: the qualified name of "whoever is asking" — a decl's own
///   qualified name (phase B, phase C1's transparent-equality check), or a
///   sealing module's own path (phase C2's `val`/command decls).
/// - `force_stamp_owner`: when `Some(p)`, a reference whose owner is `p`
///   gets stamped even though the owner-scope rule would otherwise keep it
///   concrete — phase C2's "own" map (a seal's OWN abstract types must
///   still be opaque on the COMMITTED scheme side). `None` everywhere else
///   (including phase C2's "ext"/rigid-side map).
/// - `enforce_self_containment`: phase C2 only (spec §3 phase C step 2) — a
///   bare reference to THIS module's own type, when that type is hidden
///   (impl defines it, sig never declared it), is a self-containment error
///   rather than silently left concrete. `false` for phase B's general pass
///   and phase C1's transparent-equality check (an impl's OWN internal type
///   references are never restricted by its OWN hiding — hiding is purely
///   an EXTERNAL-boundary concept).
struct RenameCtx<'a> {
    scope: &'a str,
    force_stamp_owner: Option<&'a str>,
    enforce_self_containment: bool,
}

/// The exhaustive `cst::ast::TypeExpr → TypeExpr` renamer (spec §2.5/§3
/// phase B): every type-name atom, bare or applied-head, is resolved
/// against `env.types`/`env.hidden_types` by [`rename_type_name`]. No `_`
/// arm anywhere in this family — a future `cst::ast` type-grammar arm
/// breaks the build here, not silently (spec §8 risk 4).
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
        // optional-arg-rows increment 2: `?(l1 : ty1, …) dom -> cod` — no
        // type NAME of its own to resolve at this level (`opt_dom`'s entries
        // are `label : ty` pairs, not type references), so this arm just
        // recurses into every `ty`/`dom`/`cod` sub-expression, exactly like
        // `Fun`'s `opts`/`dom`/`cod` above.
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
    // `arg1 … argN ctor`: the arguments (`head` + all but the last of `rest`)
    // rename as ordinary 0-ary atoms; the final constructor renames through
    // `rename_type_name` with the correct arity so the abstract-type
    // arity-check (`used_arity`) still matches. A `Mod.t` constructor is a
    // genuinely qualified reference (unrelated to this sealing pass's own
    // dotted-string encoding), passed through like the old `AppliedMod`.
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
                        // optional-arg-rows increment 3a: a `?(l:τ,…)` label
                        // type could itself name a sealed abstract type
                        // (`?(deco : M.t)`) — recurse each field's `ty` the
                        // same as the slot's own mandatory `ty` below, or a
                        // seal-pierce leak follows (spec §8/§14 risk 2).
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
        // optional-arg-rows increment 2: an open record type's row-variable
        // tail names no TYPE (it's a row variable, a different namespace
        // entirely — `rename_type_name` never sees it), so only the field
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

/// The one place a type-name atom's fate is decided (spec §2.5/§3 phase B):
///
/// 1. Not a dotted name at all (`"int"`, a builtin/unqualified nominal) —
///    never a sealed key; left unchanged.
/// 2. "Under" `owner`'s scope (spec's `starts_with("M.N.")` rule) and NOT
///    force-stamped: this is a self-reference. If `enforce_self_
///    containment` and the type is `hidden_types` (defined by the impl but
///    never declared in THIS sig), a self-containment error (spec §3 phase
///    C step 2's "mentions its type … without declaring it"); otherwise
///    left unchanged (concrete).
/// 3. Otherwise (external, or force-stamped self): a `hidden_types` hit is
///    the "not exported" error; an `Abstract` hit renames to its stamp
///    (arity-checked); a `Transparent` hit — or no entry at all (unsealed)
///    — is left unchanged (transparent resolution happens later, through
///    the ordinary synonym table, §2.1's θ-for-free argument).
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
    // `force_stamp_owner` only ever overrides the "stay concrete" shortcut
    // for a GENUINELY ABSTRACT type — a transparent, or entirely
    // undeclared-but-not-abstract, own-type reference must stay concrete
    // regardless of the "own" map's force flag (that map's whole point is
    // "also stamp THIS seal's own abstract types on the committed side";
    // it must never accidentally route a non-abstract own reference into
    // the external/hidden-types dispatch below, where a same-owner
    // reference was never meant to land).
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
// Phase C: the seal-table val/type half (spec §3 phase C).
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

        // Sub-slice 2e-1 §2.1 step 5: `seal.value_names`/`other_names` are
        // ALREADY include-spliced (computed once in phase A by
        // `struct_member_names_spliced`) — reusing them here (instead of
        // recomputing off `seal.binds`, which has no way to see a
        // `Bind::Include`'s target members) is what makes width-checking and
        // hiding see an included member as defined, without re-threading
        // `surfaces` into phase C at all.
        let value_names = &seal.value_names;
        let other_names = &seal.other_names;
        let mut declared: Vec<String> = Vec::new();
        for d in &seal.sig_decls {
            match &*d.0 {
                ast_v1::Decl::Val {
                    kw,
                    name,
                    quant,
                    ty,
                    ..
                } => {
                    process_seal_member(
                        kw.0,
                        &name.name,
                        name.span,
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
                // `TypeOpaque`/`Type` already fully handled by phase A/this
                // function's `pending_transparent` loop above;
                // `Module`/`Signature`/`Include` already errored in phase A
                // (so this seal's `Result` would never have reached phase
                // C at all) — nothing left to do for any other arm.
                _ => {}
            }
        }
        for vn in value_names {
            if !declared.contains(vn) {
                let qualified = lower::qualify_type_key(&seal.mod_path, vn);
                // Sub-slice 2d-3b §3.3-6: a PARENT-imposed synthetic child
                // seal defers hiding to the parent's own trigger
                // (`StaticEnv::member_revoke_triggers`, phase D) instead of
                // committing `hidden` immediately — immediate commit here
                // would break the parent-imposed-deferral rule (a SIBLING
                // use of the omitted member, elsewhere in the parent's own
                // subtree but before the parent's trigger fires, must still
                // accept).
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

    // Sub-slice 2d-3b §3.4/§3.6: all `PendingSeal`s ran first (insertion
    // order); NOW the `PendingLink`s — links only READ+UPDATE existing
    // `env.seals` entries (never register a new key), so parent-before-
    // child prescan order stops mattering here.
    for link in links {
        let mut declared: Vec<String> = Vec::new();
        for d in &link.decls {
            match &*d.0 {
                ast_v1::Decl::Val {
                    kw,
                    name,
                    quant,
                    ty,
                    ..
                } => {
                    process_link_member(
                        kw.0, &name.name, name.span, quant, ty, link, ck, mint, env,
                    )?;
                    declared.push(name.name.clone());
                }
                ast_v1::Decl::ValHorzCmd {
                    kw, cmd, quant, ty, ..
                } => {
                    process_link_member(kw.0, &cmd.name, cmd.span, quant, ty, link, ck, mint, env)?;
                    declared.push(cmd.name.clone());
                }
                ast_v1::Decl::ValVertCmd {
                    kw, cmd, quant, ty, ..
                } => {
                    process_link_member(kw.0, &cmd.name, cmd.span, quant, ty, link, ck, mint, env)?;
                    declared.push(cmd.name.clone());
                }
                _ => {}
            }
        }
        // §3.4's last bullet: members `N` exports that `S_N` omits → the
        // parent revoke list when this link itself sits under a synthetic,
        // parent-imposed seal (deferred, like a `PendingSeal`'s own
        // `parent_trigger`); otherwise (the direct `module M :> S1 = N :>
        // S2` chain shape, `link.parent_trigger: None`) `M` IS the real
        // top-level seal — nothing else depends on seeing the un-narrowed
        // member first, so hiding is IMMEDIATE, exactly like a real (non-
        // synthetic) `PendingSeal`'s own `env.hidden` commit.
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
                        // The member is likely ALREADY present in
                        // `env.seals` (registered by the child's own REAL,
                        // inner seal) — phase D's dispatch checks `seals`
                        // BEFORE `hidden`, so an immediate hide here must
                        // also RETRACT the `seals` entry, or the inner
                        // seal's commit would win regardless.
                        env.seals.remove(&m);
                        env.hidden.insert(m, link.parent_name.clone());
                    }
                }
            }
        }
    }
    Ok(())
}

/// Sub-slice 2d-3b §3.4: the [`PendingLink`] twin of [`process_seal_member`]
/// — lower `S_N`'s (the outer, imposed) declared type exactly like a real
/// seal member would, then check INNER ⊑ OUTER (`env.seals[qualified]`'s
/// already-committed scheme, from the child's OWN real seal, must subsume
/// what the parent additionally declares) and, on success, REPLACE the
/// committed scheme with the outer one (rigid/stamp_marker stay the INNER
/// ones — the spine still enforces the child's OWN seal against the real
/// inference; soundness: inferred ⊑ inner (spine) ∧ inner ⊑ outer (link) ⟹
/// inferred ⊑ outer, what escapes through the parent).
fn process_link_member<'s>(
    _kw_span: Span,
    member_name: &str,
    member_span: Span,
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

/// Spec §3 phase C step 1 / §2.1's "transparent `type t 'a… = τ` equality":
/// lower BOTH sides with the SAME positional rigid tyvar map (one fresh
/// [`StampMint`] draw), expand both through the session synonym table, and
/// `unify` — with both sides fully rigid, `unify` degenerates to structural
/// equality. The sig's declared τ is first passed through [`rename_type_expr`]
/// (scope = this type's own qualified name) so an external abstract
/// reference inside it (`type s = N.t`) resolves to the SAME stamp the
/// impl's already-registered (and already leak-fix-rewritten, phase B)
/// synonym body does.
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
/// unify to, post-expansion (spec §2.3's shape guard).
#[derive(Clone, Copy)]
enum CmdShape {
    None,
    Inline,
    Block,
}

/// Spec §3 phase C steps 2-3: the shared width/tyvar-closure/lower/rename/
/// stamp/subsumption-prep pipeline for `Decl::Val`/`ValHorzCmd`/`ValVertCmd`
/// — the same per-decl work 2d-1's `process_seal` did, plus the `ext`/`own`
/// [`RenameCtx`] rewrite (§2.1's two opacity-entry-points) and, for a
/// command decl, the post-expansion shape guard (§2.3).
#[allow(clippy::too_many_arguments)]
fn process_seal_member<'s>(
    kw_span: Span,
    member_name: &str,
    member_span: Span,
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

    // `ext` (rigid/check side): THIS seal's own types stay concrete;
    // OTHER sealed modules' abstract types become stamps; a bare own-type
    // name the sig never declared is a self-containment error — but ONLY
    // when this seal opted into type control at all (`declares_any_type`,
    // `PendingSeal`'s doc comment; T6's pinned zero-type-decl accept).
    let ext_ctx = RenameCtx {
        scope: &seal.module_name,
        force_stamp_owner: None,
        enforce_self_containment: seal.declares_any_type,
    };
    let ext_ty = rename_type_expr(&cst_ty, &ext_ctx, env)?;
    // `own` (scheme/committed side): `ext` PLUS this seal's OWN abstract
    // types also become stamps (§2.1 point 1 — every scheme escaping the
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
        // Math commands share the `\` sigil with inline commands (no
        // separate sig keyword upstream — `val \frac : math […]` is a
        // `ValHorzCmd` exactly like `val \it : inline […]`), precedent
        // `command_scheme`'s alias pass-through (`typecheck.rs:1441-1443`).
        // Soundness is preserved: a sig declaring `math […]` for an inline
        // binding (or vice versa) still fails at subsumption/unify
        // (`UnifyError::Mismatch`) — this guard is only the early,
        // better-message filter (math-package completion M1).
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

/// The struct's defined member names, split into (value members — every
/// `Decl::Val`-family width-check candidate — and type/module/signature
/// names, kept separate purely for `width_missing_error`'s nicer wording).
/// The same accessors `v1/lower.rs::lower_bind_v1` clones.
///
/// **Sub-slice 2e-1 §2.1 step 5, include-aware.** At a `Bind::Include` with
/// a resolved frozen target, extends `values`/`others` with the target
/// surface's own member names, IN PLACE at the include's position —
/// preserving interleaved source order, since the ctor-hide/revocation
/// trigger key is "the LAST value member in source order" (§2.1 step 5) and
/// elaboration's contiguous-alias assumption both count spliced members at
/// the include's position. This function, `v1/surface.rs::build_binds`'s
/// splice, and `v1/lower.rs`'s lowering splice all read the SAME frozen
/// target surface, in the SAME order — never re-resolving — so the three
/// stay in lock-step by construction.
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

// ---- the v1 tyvar walker (spec §4.2 step 1b) -------------------------------

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
        // optional-arg-rows increment 2: a labeled-optional domain's entry
        // types are nested type positions — walk each one, same as `dom`/
        // `cod` (the row-variable tail, if any, is a ROW var, a different
        // namespace this walker doesn't track — `check_tyvar_closure` below
        // only enforces closure over TYPE vars).
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
        // Command-type argument slots (Sub-slice 2d-2) are full `TypeExpr`s
        // — walk each one, same as any other nested type position. A slot's
        // `?(l:τ,…)` optional-label bundle (optional-arg-rows increment 3a)
        // is the same kind of nested type position — walk its field types
        // too, or a quantified type variable used ONLY inside a bundle
        // (`?(l : 'a)`) would go unregistered.
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
        // walk each one, same as any other (mirrors `InlineCmdTy`/
        // `BlockCmdTy`'s `args` walk above).
        ast_v1::TypeAtom::Record { inner, .. } => {
            for f in &inner.fields {
                collect_v1_type_vars(&f.ty.0, out);
            }
        }
        ast_v1::TypeAtom::Var(tv) => out.push((tv.name.clone(), tv.span)),
        // `M.t` (Sub-slice 2d-2) is an ABSOLUTE reference — it can never
        // spell one of THIS decl's own quantified type variables.
        ast_v1::TypeAtom::LongName(_) => {}
        ast_v1::TypeAtom::Name(_) => {}
    }
}

/// §4.2 step 1b: every type variable `ty` mentions must be bound by `quant`
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
    /// document body together, the same assembly `lib.rs::
    /// compile_document_v1_with_trials` and `tests/v01_modules.rs::
    /// elaborate_with_lib` both use.
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

    /// T9: `check_program` ≡ `typecheck_verbose_with_version` on seal-free
    /// inputs — same shape as `typecheck.rs`'s `l3_per_binding_tests::
    /// assert_equivalent`, driven at the module-checker layer instead of
    /// the raw per-binding API. `deps` is empty: no module system is
    /// involved at all, isolating the parity claim to step 0's session-setup
    /// order (spec §4.2 step 0's rationale).
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

    /// T9's module-bearing twin: an UNSEALED dependency library exercises
    /// the driver's `with_all` fallback arm (an alias present in neither
    /// `seals` nor `hidden`) against the same whole-program comparison.
    /// Sub-slice 2d-2's U16 extension: the unsealed module ALSO declares a
    /// variant type, a synonym, and a command — exercising phase B's
    /// empty-map fast path (this module has no seal at all, so `static_env.
    /// types`/`hidden_types` stay empty for the WHOLE program) even though
    /// `program.type_decls`/`synonym_decls` are non-empty.
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

    /// Pins [`rewrite_hidden_error`]'s string coupling from BOTH sides (spec
    /// §6 risk 2): the exact unbound-variable format
    /// `typecheck.rs::infer`'s `Ast::Var` arm produces (`typecheck.rs:1383`
    /// at the time of writing) must still trigger the rewrite. If this test
    /// starts failing, `typecheck.rs`'s message was reworded — fix
    /// `rewrite_hidden_error`'s PREFIX/SUFFIX constants in the SAME commit.
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

    /// Sub-slice 2d-2 U20 pin: a HIDDEN command member's use, both inline
    /// and block, must rewrite through `static_env.hidden` too (2d-1 only
    /// matched the plain-variable format).
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

    /// Sub-slice 2d-2 U9 pin: a hidden constructor's use, both expression
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

    /// Sub-slice 2d-2 §2.6 pin: `strip_stamps` removes every maximal `#N`
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
    // Sub-slice 2f-1 (`…/tmp/slice2f-functors.md` §5): the functor
    // parameter-signature check + generativity-by-path pins.
    // ========================================================================

    /// Shared functor fixture for T-chk1/T-chk3: `Make = fun (Key : Ord) ->
    /// struct val cmp2 x = Key.compare x x end`, applied to two differently-
    /// shaped concrete arguments (`IntOrd`/`FlagOrd`). `use`'s parameter
    /// type is forced by the REAL `Key.compare` reference (substituted to
    /// `IntOrd.compare`/`FlagOrd.compare` per application) — a genuine
    /// application constraint, not a possibly-unenforced type ascription.
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

    /// T-chk1 (spec §5): a functor applied to an argument that satisfies its
    /// parameter signature — `check_program` accepts.
    #[test]
    fn t_chk1_functor_param_sig_accept() {
        let store = SymbolStore::new();
        let (lib_file, program) = elaborate_with_lib(&store, FUNCTOR_LIB, "Lib.A.cmp2 1");
        check_program(&[&lib_file], &program)
            .expect("IntOrd satisfies Ord — check_program accepts");
    }

    /// T-chk3 (spec §5, generativity): `Make IntOrd` and `Make FlagOrd` are
    /// two INDEPENDENT instantiations — `Lib.A.cmp2`'s parameter type is
    /// forced to `IntOrd.t` (= `int`) by its own substituted `Key.compare`
    /// reference, which does NOT accept a `FlagOrd.t`-typed value
    /// (`Lib.FlagOrd.y ()`, exposing `FlagOrd`'s `Yes` ctor through a plain
    /// function, since a BARE qualified ctor reference like
    /// `Lib.FlagOrd.Yes` has no expression-grammar support in this port,
    /// `v1/lower.rs`'s "ctor carve-out" doc comment) — even though both
    /// instantiations share the IDENTICAL functor body source, pinning
    /// that each application is checked against its OWN substituted
    /// argument, never a merged/confused one.
    #[test]
    fn t_chk3_functor_generativity_distinct_instantiations_do_not_cross_unify() {
        let store = SymbolStore::new();
        let (lib_file, program) =
            elaborate_with_lib(&store, FUNCTOR_LIB, "Lib.A.cmp2 (Lib.FlagOrd.y ())");
        let err = check_program(&[&lib_file], &program)
            .expect_err("A's `cmp2` requires an IntOrd-shaped argument, not FlagOrd's `Yes`");
        assert!(!err.message.is_empty());
    }

    /// T-chk2 (spec §5, the core acceptance-negative test): a functor
    /// applied to an argument MISSING a member its parameter signature
    /// requires (`BadOrd` has no `compare`) is REJECTED with a
    /// functor-framed "does not match … parameter signature" message —
    /// even though the functor body here never itself calls `compare` (so
    /// elaboration alone would never catch this; the dedicated width check,
    /// `check_functor_applications`, is what rejects it).
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
