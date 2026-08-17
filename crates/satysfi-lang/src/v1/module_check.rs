//! Sub-slice 2d-1 (`…/tmp/slice2d-sealing.md` §2.3, §4.2): the per-binding
//! module-checking driver — the V0_1 replacement for `typecheck::
//! typecheck_with_version` at `lib.rs`'s V0_1 pipeline (upstream analogue:
//! `moduleTypechecker.ml:596 typecheck_module` + `coerce_signature:375`,
//! collapsed onto the flat-spine model `elaborate.rs` already produces).
//!
//! **Architecture (spec §3.3): a spine walk keyed by a cst_v1-derived seal
//! table.** The *signature* information comes from the `cst_v1` trees (they
//! are the only place `:>` survives — `v1/lower.rs` erases it by design,
//! §0 fact 3 of the spec), while the *expressions* checked are the
//! elaborated spine's (`program.body`'s `Let`-chain, walked exactly like
//! `typecheck.rs`'s `l3_per_binding_tests::drive_manually`). The seal-point
//! analysis (spec §0) makes the interception provably complete: the only
//! spine bindings whose names contain a dot are the elaborator's own
//! alias/`open` re-bindings (`elaborate.rs`'s `export_alias`/`Expr::
//! OpenIn`), and every path from outside a module to a member flows through
//! its alias commit — so intercepting `Ast::LetIn` alone (never `LetMath`/
//! `LetRec`/`LetMutable`, which a qualified alias never is) is complete.
//!
//! [`check_program`]'s algorithm, in full:
//!
//! 1. **Session setup** (`session setup`, below): mirrors `Checker::
//!    new_with_version`'s exact statement order — parity-critical (spec's
//!    T9, seal-free verdict/error/warning equivalence with the
//!    whole-program path).
//! 2. **Seal-table construction** ([`build_static_env`]): walk every
//!    dependency's `cst_v1::FileV1::Library`, recursing through nested
//!    `Bind::Module`s with the identical `mod_path`/`TypeNameEnv` threading
//!    `v1::lower::lower_module_bind` uses (so the two walks cannot drift).
//!    At each sealed module, [`process_seal`] resolves its `sig .. end`
//!    body's `Decl::Val` items into [`crate::v1::static_env::StaticEnv::
//!    seals`], and every struct member NOT so declared into `hidden`.
//! 3. **The spine walk with interception** (the main `loop` in
//!    [`check_program`]): `Ast::LetIn`'s alias-commit case looks itself up
//!    in `seals`/`hidden` and either subsumption-checks + commits the
//!    DECLARED scheme (sealing), skips the commit entirely (hiding), or
//!    falls through to the ordinary `with_all` commit.
//!
//! **Sealing has zero runtime residue** (spec §0 fact 3): this module reads
//! signature information from the pre-lowering `cst_v1` trees purely for
//! type-checking; the elaborated/compiled/evaluated program never differs
//! from its unsealed twin (pinned by `v1/lower.rs`'s
//! `sig_annot_on_library_lowers_like_its_unsealed_twin`-style tests, §4.3-E
//! item 4 of the spec).

use crate::ast::Ast;
use crate::elaborate::Program;
use crate::types::{self, MonoType, PolyType};
use crate::typecheck::{self, BindingView, Checker, MatchWarning, TypeError};
use crate::v1::lower::{self, TypeNameEnv};
use crate::v1::sig_subtype::{self, SubsumeError};
use crate::v1::static_env::{DeclaredVal, StaticEnv, StampMint};
use satysfi_syntax::cst_v1::{self, ast as ast_v1};
use satysfi_syntax::leaf::{AnyHorzCmdTok, AnyVertCmdTok, TypeVarTok};
use satysfi_syntax::span::Span;
use satysfi_syntax::SatysfiVersion;
use std::collections::HashMap;

/// Check one whole elaborated V0_1 program, per binding, enforcing every
/// `:>` seal found in `deps` (the original `cst_v1` trees). Returns the same
/// warnings the whole-program path would; errors are ordinary
/// `typecheck::TypeError`s (pub fields) so `lib.rs`'s
/// `CompileError::Type(#[from])` covers them unchanged.
pub(crate) fn check_program(
    deps: &[&cst_v1::FileV1],
    program: &Program,
) -> Result<Vec<MatchWarning>, TypeError> {
    // ---- step 0: session setup (order-critical, parity-pinned against
    // `Checker::new_with_version`, `typecheck.rs`) ----
    let mut ck = Checker::empty();
    for usd in &program.synonym_decls {
        ck.declare_synonym(usd);
    }
    ck.check_cycles()?;
    ck.install_builtin_variants(SatysfiVersion::V0_1);
    for utd in &program.type_decls {
        ck.declare_variant(utd)?;
    }

    // ---- step 1: the seal-table construction (the cst_v1 walk) ----
    let static_env = build_static_env(deps, &mut ck)?;

    // ---- steps 3-7: the spine walk with interception ----
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

/// §4.2 step 6: any `TypeError` propagating out of the spine walk passes
/// through this filter. If its message is EXACTLY `typecheck.rs`'s
/// `Ast::Var` unbound-variable format (the single format site,
/// `typecheck.rs:1383` at the time of writing — pinned from both sides by
/// this module's own `hidden_rewrite_matches_typecheck_unbound_var_format`
/// unit test) for some name recorded in `static_env.hidden`, rewrite it into
/// a precise sealing diagnostic naming the owning module. Every other error
/// (including the mirror-image `Overwrite` unbound-*mutable*-variable
/// message, a DIFFERENT format site this deliberately does NOT match) passes
/// through unchanged.
fn rewrite_hidden_error(err: TypeError, static_env: &StaticEnv) -> TypeError {
    const PREFIX: &str = "internal error: unbound variable '";
    const SUFFIX: &str = "' reached the typechecker";
    if let Some(name) = err
        .message
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(SUFFIX))
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

// ============================================================================
// Step 1: the seal-table construction (the cst_v1 walk, spec §4.2 step 1).
// ============================================================================

fn build_static_env(
    deps: &[&cst_v1::FileV1],
    ck: &mut Checker,
) -> Result<StaticEnv, TypeError> {
    let mut static_env = StaticEnv::default();
    let mut mint = StampMint::default();
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
            process_seal(sa, &bind_refs, &mod_path, &tyenv, ck, &mut mint, &mut static_env)?;
        }
        walk_nested_seals(&bind_refs, &mod_path, &tyenv, ck, &mut mint, &mut static_env)?;
    }
    Ok(static_env)
}

/// Recurse through every nested `Bind::Module { .. }` looking for further
/// seals — independent of whether THIS level is itself sealed (spec §4.5
/// test T7: an unsealed outer module can contain a sealed nested one).
/// `Bind::Module` with a non-`Struct` body under a seal cannot occur in
/// 2d-1 (those bodies still `LowerError` before checking ever runs, `v1/
/// lower.rs`'s `Bind::Module` arm) — silently skipped here, matching that.
fn walk_nested_seals(
    binds: &[&cst_v1::Bind],
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    ck: &mut Checker,
    mint: &mut StampMint,
    env: &mut StaticEnv,
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
            process_seal(sa, &inner_binds, &child_path, &child_tyenv, ck, mint, env)?;
        }
        walk_nested_seals(&inner_binds, &child_path, &child_tyenv, ck, mint, env)?;
    }
    Ok(())
}

/// Resolve one `:> sig .. end` annotation against the struct body it seals:
/// width-check + tyvar-closure-check + transcribe + skolemize-by-lowering
/// (§3.4) each `Decl::Val`, insert into `env.seals`, then populate
/// `env.hidden` for every struct member no `Decl::Val` named. Every other
/// `SigExpr`/`SigBotV1`/`Decl` shape is a precise §4.7 placeholder error.
fn process_seal(
    sa: &cst_v1::SigAnnotV1,
    binds: &[&cst_v1::Bind],
    mod_path: &[String],
    tyenv: &TypeNameEnv,
    ck: &mut Checker,
    mint: &mut StampMint,
    env: &mut StaticEnv,
) -> Result<(), TypeError> {
    let module_name = mod_path.join(".");
    let decls: &[cst_v1::StructDeclV1] = match &*sa.sig_.0 {
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Sig { decls, .. }) => decls.as_slice(),
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Var(t)) => {
            return Err(simple_error(
                Some(t.span),
                format!(
                    "module `{module_name}`'s signature: named signatures live in \
                     the static environment — Sub-slice 2d-3"
                ),
            ))
        }
        ast_v1::SigExpr::Bot(ast_v1::SigBotV1::Path(t)) => {
            return Err(simple_error(
                Some(t.span),
                format!(
                    "module `{module_name}`'s signature: named signatures live in \
                     the static environment — Sub-slice 2d-3"
                ),
            ))
        }
        ast_v1::SigExpr::WithType { with_kw, .. } => {
            return Err(simple_error(
                Some(with_kw.0),
                format!(
                    "module `{module_name}`'s signature: `with type` refinement is \
                     not enforced yet — Sub-slice 2e"
                ),
            ))
        }
        ast_v1::SigExpr::Functor { lp, .. } => {
            return Err(simple_error(
                Some(lp.0),
                format!(
                    "module `{module_name}`'s signature: functor signatures are not \
                     enforced yet — Sub-slice 2f"
                ),
            ))
        }
    };

    let (value_names, other_names) = struct_member_names(binds);
    let mut declared: Vec<String> = Vec::new();

    for d in decls {
        match &*d.0 {
            ast_v1::Decl::Val { kw, name, quant, ty, .. } => {
                if !value_names.iter().any(|v| v == &name.name) {
                    let is_type_or_module = other_names.iter().any(|o| o == &name.name);
                    return Err(width_missing_error(
                        &module_name,
                        &name.name,
                        name.span,
                        is_type_or_module,
                    ));
                }
                check_tyvar_closure(ty, quant)?;

                let cst_ty = lower::lower_type_expr(ty, tyenv).map_err(|e| {
                    simple_error(
                        Some(e.span),
                        format!("module `{module_name}`'s signature: {} ({})", e.construct, e.hint),
                    )
                })?;

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

                let scheme_raw = typecheck::lower_type_expr(&cst_ty, &scheme_map);
                let scheme_body = ck.expand_synonyms_in(&scheme_raw)?;
                let rigid_raw = typecheck::lower_type_expr(&cst_ty, &rigid_map);
                let rigid_body = ck.expand_synonyms_in(&rigid_raw)?;

                let scheme = PolyType::from_vars(scheme_vars, Vec::new(), scheme_body);
                let qualified = lower::qualify_type_key(mod_path, &name.name);
                declared.push(name.name.clone());
                env.seals.insert(
                    qualified,
                    DeclaredVal {
                        name: name.name.clone(),
                        scheme,
                        rigid: rigid_body,
                        span: kw.0,
                        stamp_marker,
                    },
                );
            }
            other => return Err(non_val_decl_error(&module_name, other)),
        }
    }

    for vn in &value_names {
        if !declared.contains(vn) {
            let qualified = lower::qualify_type_key(mod_path, vn);
            env.hidden.insert(qualified, module_name.clone());
        }
    }
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

/// §4.7's placeholder table for every `Decl` arm other than `Val`.
fn non_val_decl_error(module_name: &str, decl: &ast_v1::Decl) -> TypeError {
    let (span, what): (Span, &str) = match decl {
        ast_v1::Decl::ValHorzCmd { kw, .. } | ast_v1::Decl::ValVertCmd { kw, .. } => (
            kw.0,
            "command declarations in signatures need command types \
             (`inline [...]`) — Sub-slice 2d-2",
        ),
        ast_v1::Decl::TypeOpaque { kw, .. } => (
            kw.0,
            "opaque `type` declarations in signatures are not enforced yet — \
             Sub-slice 2d-2 (abstract-type stamping)",
        ),
        ast_v1::Decl::Type { kw, .. } => (
            kw.0,
            "transparent `type` declarations in signatures are not enforced \
             yet — Sub-slice 2d-2 (transparent type matching)",
        ),
        ast_v1::Decl::Module { kw, .. } => (
            kw.0,
            "module declarations in signatures are not enforced yet — \
             Sub-slice 2d-3",
        ),
        ast_v1::Decl::Signature { kw, .. } => (
            kw.0,
            "named signature declarations in signatures are not enforced yet \
             — Sub-slice 2d-3",
        ),
        ast_v1::Decl::Include { kw, .. } => (
            kw.0,
            "`include` declarations in signatures are not enforced yet — \
             Sub-slice 2e",
        ),
        ast_v1::Decl::Val { .. } => unreachable!("Decl::Val has its own arm"),
    };
    simple_error(Some(span), format!("module `{module_name}`'s signature: {what}"))
}

/// The struct's defined member names, split into (value members — every
/// `Decl::Val` width-check candidate — and type/module/signature names,
/// kept separate purely for `width_missing_error`'s nicer wording). The same
/// accessors `v1/lower.rs::lower_bind_v1` clones.
fn struct_member_names(binds: &[&cst_v1::Bind]) -> (Vec<String>, Vec<String>) {
    let mut values = Vec::new();
    let mut others = Vec::new();
    for b in binds.iter().copied() {
        match b {
            cst_v1::Bind::Value { name, .. } => values.push(name.name.clone()),
            cst_v1::Bind::ValueInline { cmd, .. } => values.push(any_horz_name(cmd)),
            cst_v1::Bind::ValueBlock { cmd, .. } => values.push(any_vert_name(cmd)),
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
        ast_v1::TypeApp::Applied { first, rest, .. } => {
            collect_v1_type_vars_atom(first, out);
            for r in rest {
                collect_v1_type_vars_atom(r, out);
            }
        }
        ast_v1::TypeApp::Atom(at) => collect_v1_type_vars_atom(at, out),
    }
}

fn collect_v1_type_vars_atom(a: &ast_v1::TypeAtom, out: &mut Vec<(String, Span)>) {
    match a {
        ast_v1::TypeAtom::Paren { inner, .. } => collect_v1_type_vars(&inner.0, out),
        ast_v1::TypeAtom::Var(tv) => out.push((tv.name.clone(), tv.span)),
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
    #[test]
    fn seal_free_parity_with_unsealed_module() {
        let lib_src = "module M = struct\nval x = 1\nval f y = y\nend";
        for doc_src in ["M.x + 1", "M.f true", "M.x + M.f 1"] {
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
}
