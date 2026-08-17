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

use crate::ast::Ast;
use crate::elaborate::{Program, UserSynonymDecl, UserTypeDecl};
use crate::types::{self, MonoType, PolyType};
use crate::typecheck::{self, BindingView, Checker, MatchWarning, TypeError};
use crate::unify::unify;
use crate::v1::lower::{self, TypeNameEnv};
use crate::v1::sig_subtype::{self, SubsumeError};
use crate::v1::static_env::{
    DeclaredType, DeclaredVal, HiddenCtor, StaticEnv, StampMint, TypeOpacity,
};
use satysfi_syntax::cst;
use satysfi_syntax::cst_v1::{self, ast as ast_v1};
use satysfi_syntax::leaf::{AnyHorzCmdTok, AnyVertCmdTok, TypeVarTok, VarTok};
use satysfi_syntax::span::Span;
use satysfi_syntax::SatysfiVersion;
use std::collections::HashMap;

/// Check one whole elaborated V0_1 program, per binding, enforcing every
/// `:>` seal found in `deps` (the original `cst_v1` trees). Returns the same
/// warnings the whole-program path would; errors are ordinary
/// `typecheck::TypeError`s (pub fields) so `lib.rs`'s
/// `CompileError::Type(#[from])` covers them unchanged. Stamp-strips every
/// outgoing error message (module doc comment, §2.6) — the one thing this
/// wrapper adds over [`check_program_inner`].
pub(crate) fn check_program(
    deps: &[&cst_v1::FileV1],
    program: &Program,
) -> Result<Vec<MatchWarning>, TypeError> {
    check_program_inner(deps, program).map_err(strip_stamps_error)
}

fn check_program_inner(
    deps: &[&cst_v1::FileV1],
    program: &Program,
) -> Result<Vec<MatchWarning>, TypeError> {
    // ---- phase A: syntactic seal pre-scan (no `Checker`) ----
    let mut static_env = StaticEnv::default();
    let mut mint = StampMint::default();
    let mut immediate_hides: Vec<(String, String)> = Vec::new();
    let pending = phase_a_prescan(deps, &mut mint, &mut static_env, &mut immediate_hides)?;

    // ---- phase B: session setup with the external-reference rewrite ----
    let mut ck = Checker::empty();
    ck.set_version(SatysfiVersion::V0_1);
    let rewritten = maybe_rewrite_program_types(program, &static_env)?;
    match &rewritten {
        Some((synonym_decls, type_decls)) => {
            for usd in synonym_decls {
                ck.declare_synonym(usd)?;
            }
            ck.check_cycles()?;
            ck.install_builtin_variants(SatysfiVersion::V0_1);
            for utd in type_decls {
                ck.declare_variant(utd)?;
            }
        }
        None => {
            for usd in &program.synonym_decls {
                ck.declare_synonym(usd)?;
            }
            ck.check_cycles()?;
            ck.install_builtin_variants(SatysfiVersion::V0_1);
            for utd in &program.type_decls {
                ck.declare_variant(utd)?;
            }
        }
    }
    if !immediate_hides.is_empty() {
        ck.hide_ctors(&immediate_hides);
    }

    // ---- phase C: the seal-table val/type half ----
    phase_c_finish(&pending, &mut ck, &mut mint, &mut static_env)?;

    // ---- phase D: the spine walk with interception ----
    let mut env = typecheck::base_type_env_with_version(SatysfiVersion::V0_1);
    let mut ast: &Ast = &program.body;
    loop {
        ast = match ast {
            Ast::LetIn(name, value, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::Let { name, value }),
                    &static_env,
                )?;
                env = match static_env.seals.get(name.as_str()) {
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
                        .map_err(|e| seal_mismatch_error(name, decl, inferred, e))?;
                        env.with(name.clone(), decl.scheme.clone())
                    }
                    // the alias binding of a HIDDEN member: commit NOTHING.
                    None if static_env.hidden.contains_key(name.as_str()) => env,
                    // every ordinary binding (locals, unsealed aliases,
                    // opens).
                    None => env.with_all(schemes),
                };
                // Sub-slice 2d-2 §2.2: this alias may be a deferred
                // ctor-hide trigger (the sealing module's LAST value
                // member) — fire it AFTER the commit above, so the
                // module's own members (which just finished checking)
                // still saw the concrete ctors.
                if let Some(hides) = static_env.ctor_hide_triggers.get(name.as_str()) {
                    ck.hide_ctors(hides);
                }
                body
            }
            Ast::LetMathIn(name, value, body) => {
                let schemes = catch_hidden(
                    ck.infer_binding(&env, BindingView::LetMath { name, value }),
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
                    ck.infer_binding(&env, BindingView::LetMutable { name, init }),
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
        ("internal error: unbound variable '", "' reached the typechecker"),
        (
            "internal error: unbound inline command '",
            "' reached the typechecker",
        ),
        (
            "internal error: unbound block command '",
            "' reached the typechecker",
        ),
    ] {
        if let Some(name) = err.message.strip_prefix(prefix).and_then(|rest| rest.strip_suffix(suffix)) {
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
    TypeError { span, message, source: None }
}

/// The bare local name off a qualified nominal (`"M.t"` → `"t"`) — used only
/// for diagnostic text.
fn local_name(qualified: &str) -> &str {
    qualified.rsplit_once('.').map(|(_, t)| t).unwrap_or(qualified)
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
    sig_decls: &'a [cst_v1::StructDeclV1],
    binds: Vec<&'a cst_v1::Bind>,
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
}

fn phase_a_prescan<'a>(
    deps: &'a [&cst_v1::FileV1],
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
) -> Result<Vec<PendingSeal<'a>>, TypeError> {
    let mut pending = Vec::new();
    for file in deps.iter().copied() {
        let cst_v1::FileV1::Library { name, sig_annot, binds, .. } = file else {
            // A dependency is always a Library (the loader's
            // `DocumentAsDependency` check already rejects anything else
            // before this is ever reached, mirroring `v1/lower.rs::
            // lower_file_v1`'s own defensive `LowerError` arm).
            continue;
        };
        let mod_path = vec![name.name.clone()];
        let bind_refs: Vec<&cst_v1::Bind> = binds.iter().collect();
        let tyenv = TypeNameEnv::default().child(&mod_path, bind_refs.iter().copied());
        if let Some(sa) = sig_annot {
            let seal = prescan_seal_types(
                sa,
                bind_refs.clone(),
                &mod_path,
                &tyenv,
                mint,
                env,
                immediate_hides,
            )?;
            pending.push(seal);
        }
        walk_nested_seals_a(&bind_refs, &mod_path, &tyenv, mint, env, immediate_hides, &mut pending)?;
    }
    Ok(pending)
}

/// Recurse through every nested `Bind::Module { .. }` looking for further
/// seals — independent of whether THIS level is itself sealed (2d-1 spec
/// §4.5 test T7). The phase-A twin of 2d-1's `walk_nested_seals`.
fn walk_nested_seals_a<'a>(
    binds: &[&'a cst_v1::Bind],
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
    pending: &mut Vec<PendingSeal<'a>>,
) -> Result<(), TypeError> {
    for b in binds.iter().copied() {
        let cst_v1::Bind::Module { name, sig_annot, body, .. } = b else {
            continue;
        };
        let ast_v1::ModExpr::Struct { binds: inner, .. } = &*body.0 else {
            continue;
        };
        let mut child_path = mod_path.to_vec();
        child_path.push(name.name.clone());
        let inner_binds: Vec<&cst_v1::Bind> = inner.iter().map(|sb| sb.0.as_ref()).collect();
        let child_tyenv = tyenv.child(&child_path, inner_binds.iter().copied());
        if let Some(sa) = sig_annot {
            let seal = prescan_seal_types(
                sa,
                inner_binds.clone(),
                &child_path,
                &child_tyenv,
                mint,
                env,
                immediate_hides,
            )?;
            pending.push(seal);
        }
        walk_nested_seals_a(&inner_binds, &child_path, &child_tyenv, mint, env, immediate_hides, pending)?;
    }
    Ok(())
}

/// Resolve one `:> sig .. end` annotation's TYPE half against the struct
/// body it seals (spec §3 phase A): width-check + kind/arity-check +
/// stamp-mint every `Decl::TypeOpaque`; width-check + queue every
/// `Decl::Type`; mark every un-named impl type hidden; compute the ctor-hide
/// list and its deferred trigger. Every non-Val/-Type `Decl` shape is a
/// precise §4.7 placeholder error, as 2d-1; every non-`Sig` `SigExpr` form
/// likewise.
fn prescan_seal_types<'a>(
    sa: &'a cst_v1::SigAnnotV1,
    binds: Vec<&'a cst_v1::Bind>,
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    mint: &mut StampMint,
    env: &mut StaticEnv,
    immediate_hides: &mut Vec<(String, String)>,
) -> Result<PendingSeal<'a>, TypeError> {
    let module_name = mod_path.join(".");
    let decls = sig_decls_of(&sa.sig_.0, &module_name)?;

    let impl_table = build_impl_type_table(&binds);
    let (value_names, _other_names) = struct_member_names(&binds);

    let mut declared_types: Vec<String> = Vec::new();
    let mut hide_list: Vec<(String, String)> = Vec::new();
    let mut pending_transparent: Vec<PendingTransparent<'a>> = Vec::new();

    for d in decls {
        match &*d.0 {
            ast_v1::Decl::Val { .. } | ast_v1::Decl::ValHorzCmd { .. } | ast_v1::Decl::ValVertCmd { .. } => {}
            ast_v1::Decl::TypeOpaque { kw, name, kind, .. } => {
                declared_types.push(name.name.clone());
                let Some(info) = impl_table.get(&name.name) else {
                    return Err(width_type_missing_error(&module_name, &name.name, name.span));
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
            ast_v1::Decl::Type { binds: type_binds, .. } => {
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
                                     (and exporting its constructors) in a signature is not \
                                     enforced yet — Sub-slice 2d-3"
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
                                         variant in a signature is not enforced yet — \
                                         Sub-slice 2d-3",
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
            other @ (ast_v1::Decl::Module { .. }
            | ast_v1::Decl::Signature { .. }
            | ast_v1::Decl::Include { .. }) => {
                return Err(non_val_decl_error(&module_name, other));
            }
        }
    }

    // Type hiding (spec §3 phase A step 4): every impl type NOT named by
    // any sig type decl.
    for (tname, info) in &impl_table {
        if !declared_types.contains(tname) {
            let qualified = lower::qualify_type_key(mod_path, tname);
            env.hidden_types.insert(qualified.clone(), module_name.clone());
            if let ImplTypeBody::Variant(ctors) = &info.body {
                for c in ctors {
                    record_hide(&mut hide_list, env, &module_name, c, &qualified);
                }
            }
        }
    }

    // Seal point (spec §3 phase A step 5).
    if value_names.is_empty() {
        immediate_hides.extend(hide_list);
    } else {
        let trigger = lower::qualify_type_key(mod_path, value_names.last().unwrap());
        env.ctor_hide_triggers.entry(trigger).or_default().extend(hide_list);
    }

    Ok(PendingSeal {
        sig_decls: decls,
        binds,
        mod_path: mod_path.to_vec(),
        tyenv: tyenv.clone(),
        module_name,
        pending_transparent,
        declares_any_type: !declared_types.is_empty(),
    })
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

fn sig_decls_of<'a>(
    sig: &'a ast_v1::SigExpr,
    module_name: &str,
) -> Result<&'a [cst_v1::StructDeclV1], TypeError> {
    match sig {
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Sig { decls, .. }) => Ok(decls.as_slice()),
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Var(t)) => Err(simple_error(
            Some(t.span),
            format!(
                "module `{module_name}`'s signature: named signatures live in \
                 the static environment — Sub-slice 2d-3"
            ),
        )),
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Path(t)) => Err(simple_error(
            Some(t.span),
            format!(
                "module `{module_name}`'s signature: named signatures live in \
                 the static environment — Sub-slice 2d-3"
            ),
        )),
        ast_v1::SigExpr::WithType { with_kw, .. } => Err(simple_error(
            Some(with_kw.0),
            format!(
                "module `{module_name}`'s signature: `with type` refinement is \
                 not enforced yet — Sub-slice 2e"
            ),
        )),
        ast_v1::SigExpr::Functor { lp, .. } => Err(simple_error(
            Some(lp.0),
            format!(
                "module `{module_name}`'s signature: functor signatures are not \
                 enforced yet — Sub-slice 2f"
            ),
        )),
    }
}

/// §4.7's placeholder table for the three `Decl` arms that stay unenforced
/// after Sub-slice 2d-2 (`ValHorzCmd`/`ValVertCmd`/`TypeOpaque`/`Type` are
/// now processed for real — see [`prescan_seal_types`]/`process_seal_
/// member`/`check_transparent_type`).
fn non_val_decl_error(module_name: &str, decl: &ast_v1::Decl) -> TypeError {
    let (span, what): (Span, String) = match decl {
        ast_v1::Decl::Module { kw, .. } => (
            kw.0,
            "module declarations in signatures are not enforced yet — \
             Sub-slice 2d-3"
                .to_string(),
        ),
        ast_v1::Decl::Signature { kw, .. } => (
            kw.0,
            "named signature declarations in signatures are not enforced yet \
             — Sub-slice 2d-3"
                .to_string(),
        ),
        ast_v1::Decl::Include { kw, .. } => (
            kw.0,
            "`include` declarations in signatures are not enforced yet — \
             Sub-slice 2e"
                .to_string(),
        ),
        // `prescan_seal_types` only ever calls this for the three arms
        // above; kept total (no `unreachable!`) as a defensive fallback.
        _ => (Span::default(), "this signature declaration is not enforced yet".to_string()),
    };
    simple_error(Some(span), format!("module `{module_name}`'s signature: {what}"))
}

fn build_impl_type_table(binds: &[&cst_v1::Bind]) -> HashMap<String, ImplTypeInfo> {
    let mut table = HashMap::new();
    for b in binds.iter().copied() {
        if let cst_v1::Bind::Type { first, ands, .. } = b {
            insert_impl_type(&mut table, first);
            for a in ands {
                insert_impl_type(&mut table, &a.bind);
            }
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
fn maybe_rewrite_program_types(
    program: &Program,
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
        cst::ast::TypeExpr::Fun { opts, dom, arrow, cod } => cst::ast::TypeExpr::Fun {
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
        cst::ast::TypeExpr::OptRowFun { opt_dom, dom, arrow, cod } => cst::ast::TypeExpr::OptRowFun {
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
    Ok(match a {
        cst::ast::TypeApp::Applied { arg, ctor } => cst::ast::TypeApp::Applied {
            arg: rename_type_atom(arg, ctx, env)?,
            ctor: rename_type_name(&ctor.name, ctor.span, ctx, env, 1)?,
        },
        cst::ast::TypeApp::Atom(at) => cst::ast::TypeApp::Atom(rename_type_atom(at, ctx, env)?),
    })
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
                                    ty: cst::TyErased(Box::new(rename_type_expr(&f.ty.0, ctx, env)?)),
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
        // optional-arg-rows increment 2: an open record type's row-variable
        // tail names no TYPE (it's a row variable, a different namespace
        // entirely — `rename_type_name` never sees it), so only the field
        // types need renaming; `bar`/`var` pass through unchanged.
        cst::ast::TypeAtom::RecordOpen { rec, inner } => cst::ast::TypeAtom::RecordOpen {
            rec: rec.clone(),
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
        return Ok(VarTok { name: name.to_string(), span });
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
    let is_abstract_here =
        matches!(env.types.get(name).map(|d| &d.opacity), Some(TypeOpacity::Abstract { .. }));
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
        return Ok(VarTok { name: name.to_string(), span });
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
            return Ok(VarTok { name: stamped.clone(), span });
        }
    }
    Ok(VarTok { name: name.to_string(), span })
}

// ============================================================================
// Phase C: the seal-table val/type half (spec §3 phase C).
// ============================================================================

fn phase_c_finish(
    pending: &[PendingSeal],
    ck: &mut Checker,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    for seal in pending {
        for pt in &seal.pending_transparent {
            check_transparent_type(pt, seal, ck, mint, env)?;
        }

        let (value_names, other_names) = struct_member_names(&seal.binds);
        let mut declared: Vec<String> = Vec::new();
        for d in seal.sig_decls {
            match &*d.0 {
                ast_v1::Decl::Val { kw, name, quant, ty, .. } => {
                    process_seal_member(
                        kw.0,
                        &name.name,
                        name.span,
                        quant,
                        ty,
                        CmdShape::None,
                        seal,
                        &value_names,
                        &other_names,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(name.name.clone());
                }
                ast_v1::Decl::ValHorzCmd { kw, cmd, quant, ty, .. } => {
                    process_seal_member(
                        kw.0,
                        &cmd.name,
                        cmd.span,
                        quant,
                        ty,
                        CmdShape::Inline,
                        seal,
                        &value_names,
                        &other_names,
                        ck,
                        mint,
                        env,
                    )?;
                    declared.push(cmd.name.clone());
                }
                ast_v1::Decl::ValVertCmd { kw, cmd, quant, ty, .. } => {
                    process_seal_member(
                        kw.0,
                        &cmd.name,
                        cmd.span,
                        quant,
                        ty,
                        CmdShape::Block,
                        seal,
                        &value_names,
                        &other_names,
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
        for vn in &value_names {
            if !declared.contains(vn) {
                let qualified = lower::qualify_type_key(&seal.mod_path, vn);
                env.hidden.insert(qualified, seal.module_name.clone());
            }
        }
    }
    Ok(())
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
fn check_transparent_type(
    pt: &PendingTransparent,
    seal: &PendingSeal,
    ck: &mut Checker,
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
            format!("module `{}`'s signature: {} ({})", seal.module_name, e.construct, e.hint),
        )
    })?;
    let ctx = RenameCtx {
        scope: &pt.qualified,
        force_stamp_owner: None,
        enforce_self_containment: false,
    };
    let renamed_ty = rename_type_expr(&cst_ty, &ctx, env)?;
    let declared_raw = typecheck::lower_type_expr(&renamed_ty, &rigid_map, SatysfiVersion::V0_1);
    let declared_body = ck.expand_synonyms_in(&declared_raw)?;

    let impl_args: Vec<MonoType> = pt.quant.iter().map(|tv| rigid_map[&tv.name].clone()).collect();
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
fn process_seal_member(
    kw_span: Span,
    member_name: &str,
    member_span: Span,
    quant: &[TypeVarTok],
    ty: &ast_v1::TypeExpr,
    shape: CmdShape,
    seal: &PendingSeal,
    value_names: &[String],
    other_names: &[String],
    ck: &mut Checker,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    if !value_names.iter().any(|v| v == member_name) {
        let is_type_or_module = other_names.iter().any(|o| o == member_name);
        return Err(width_missing_error(&seal.module_name, member_name, member_span, is_type_or_module));
    }
    check_tyvar_closure(ty, quant)?;

    let cst_ty = lower::lower_type_expr(ty, &seal.tyenv).map_err(|e| {
        simple_error(
            Some(e.span),
            format!("module `{}`'s signature: {} ({})", seal.module_name, e.construct, e.hint),
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

    let scheme_raw = typecheck::lower_type_expr(&own_ty, &scheme_map, SatysfiVersion::V0_1);
    let scheme_body = ck.expand_synonyms_in(&scheme_raw)?;
    let rigid_raw = typecheck::lower_type_expr(&ext_ty, &rigid_map, SatysfiVersion::V0_1);
    let rigid_body = ck.expand_synonyms_in(&rigid_raw)?;

    match shape {
        CmdShape::Inline if !matches!(&scheme_body, MonoType::InlineCmd(_)) => {
            return Err(simple_error(
                Some(kw_span),
                format!(
                    "module `{}`'s signature: a `val {member_name} :` decl needs an \
                     `inline [...]` command type",
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
fn struct_member_names(binds: &[&cst_v1::Bind]) -> (Vec<String>, Vec<String>) {
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
            cst_v1::Bind::Include { .. } => {}
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
        ast_v1::TypeExpr::OptRowFun { opt_dom, dom, cod, .. } => {
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
        ast_v1::TypeApp::InlineCmdTy { args, .. } | ast_v1::TypeApp::BlockCmdTy { args, .. } => {
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
    use crate::{elaborate, primitives};
    use satysfi_syntax::{leaf::KwIn, parse_file_v1};

    /// Elaborate a bare V0_1 document expression (no dependency libraries)
    /// into a `Program`, the way `l3_per_binding_tests::elaborate_src` does
    /// for 0.0.6 sources.
    fn elaborate_doc_only(doc_src: &str) -> Program {
        let doc_file = parse_file_v1(doc_src).unwrap_or_else(|e| panic!("v1 parse failed: {e}"));
        let body = lower::lower_document_v1(&doc_file)
            .unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
        let eoi = match &doc_file {
            cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
            _ => unreachable!("doc_src must parse as a FileV1::Document"),
        };
        let file = satysfi_syntax::cst::File {
            headers: Vec::new(),
            prelude: Vec::new(),
            in_kw: Some(KwIn(Span::default())),
            body: Some(body),
            eoi,
        };
        let env0 = primitives::base_env_with_version(SatysfiVersion::V0_1);
        let scope = elaborate::Scope::new(env0.names());
        elaborate::elaborate_program(&file, &scope).unwrap_or_else(|e| panic!("elaborate: {e}"))
    }

    /// Elaborate a dependency library (`module … = struct … end`) plus a
    /// document body together, the same assembly `lib.rs::
    /// compile_document_v1_with_trials` and `tests/v01_modules.rs::
    /// elaborate_with_lib` both use.
    fn elaborate_with_lib(lib_src: &str, doc_src: &str) -> (cst_v1::FileV1, Program) {
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
        let file = satysfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: Some(KwIn(Span::default())),
            body: Some(body),
            eoi,
        };
        let env0 = primitives::base_env_with_version(SatysfiVersion::V0_1);
        let scope = elaborate::Scope::new(env0.names());
        let program =
            elaborate::elaborate_program(&file, &scope).unwrap_or_else(|e| panic!("elaborate: {e}"));
        (lib_file, program)
    }

    /// T9: `check_program` ≡ `typecheck_verbose_with_version` on seal-free
    /// inputs — same shape as `typecheck.rs`'s `l3_per_binding_tests::
    /// assert_equivalent`, driven at the module-checker layer instead of
    /// the raw per-binding API. `deps` is empty: no module system is
    /// involved at all, isolating the parity claim to step 0's session-setup
    /// order (spec §4.2 step 0's rationale).
    fn assert_parity_no_deps(doc_src: &str) {
        let program = elaborate_doc_only(doc_src);
        let whole = typecheck::typecheck_verbose_with_version(&program, SatysfiVersion::V0_1);
        let per_binding = check_program(&[], &program);
        match (whole, per_binding) {
            (Ok(w1), Ok(w2)) => assert_eq!(w1, w2, "warnings differ for {doc_src:?}"),
            (Err(e1), Err(e2)) => {
                assert_eq!(format!("{e1}"), format!("{e2}"), "error strings differ for {doc_src:?}")
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
            let (lib_file, program) = elaborate_with_lib(lib_src, doc_src);
            let whole = typecheck::typecheck_verbose_with_version(&program, SatysfiVersion::V0_1);
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
        static_env.hidden.insert("M.secret".to_string(), "M".to_string());
        let err = TypeError {
            span: Some(Span::default()),
            message:
                "internal error: unbound variable 'M.secret' reached the typechecker".to_string(),
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
        assert_eq!(rewritten.message, "internal error: unbound variable 'q' reached the typechecker");
    }

    /// Sub-slice 2d-2 U20 pin: a HIDDEN command member's use, both inline
    /// and block, must rewrite through `static_env.hidden` too (2d-1 only
    /// matched the plain-variable format).
    #[test]
    fn hidden_rewrite_matches_command_formats() {
        let mut static_env = StaticEnv::default();
        static_env.hidden.insert("M.\\hidden".to_string(), "M".to_string());
        static_env.hidden.insert("M.+hidden".to_string(), "M".to_string());
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
            HiddenCtor { module: "M".to_string(), type_name: "M.t".to_string() },
        );
        let expr_err = TypeError {
            span: None,
            message: "unknown constructor 'T'".to_string(),
            source: None,
        };
        let rewritten = rewrite_hidden_error(expr_err, &static_env);
        assert!(
            rewritten.message.contains("constructor `T` belongs to type `t`")
                && rewritten.message.contains("module `M`'s signature seals abstract"),
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
            rewritten.message.contains("constructor `T` belongs to type `t`")
                && rewritten.message.contains("module `M`'s signature seals abstract"),
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
        assert_eq!(strip_stamps("a # b"), "a # b", "a bare '#' with no digits is left alone");
    }
}
