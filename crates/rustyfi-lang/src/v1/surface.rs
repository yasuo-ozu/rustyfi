//! Sub-slice 2d-3 (`…/tmp/slice2d3-module-sig-decls.md` §2.1/§2.2/§4-A): the
//! **purely syntactic** module-surface + named-signature table module
//! aliases and named-signature ascriptions are resolved against.
//!
//! Deliberately independent of both `v1/lower.rs` (which only *consumes* it,
//! for alias member-copy expansion, §2.1) and `v1/module_check.rs` (which
//! *also* only consumes it, for width/type checks against an alias-bodied
//! sub-module and for named-signature resolution, §2.2/§2.3) — each of those
//! two callers builds its OWN [`SurfaceEnv`] from the same `deps` slice via
//! [`build_file_surface`], so there is exactly one implementation of "what
//! does this module export" and "what does this name denote", never two that
//! could drift. No [`crate::typecheck::Checker`], no `crate::types` import —
//! this module never runs a type check, it only walks names.
//!
//! **Infallible by design.** Every resolution helper here returns `Option`,
//! never `Result`: a miss (unknown module/signature name, an unfilterable
//! sig shape like `with type`/a functor signature) is reported by the
//! CALLER, in whichever precise wording that caller's own diagnostics
//! already use (`v1/lower.rs`'s alias `LowerError`, `v1/module_check.rs`'s
//! `TypeError`s) — `surface.rs` itself never invents user-facing text. Where
//! a resolution is used only to compute the FILTER (a sealed module's
//! exported surface, §2.1's "seal filter"), a miss/unfilterable shape simply
//! leaves the surface UNFILTERED (full width) — safe, because the same
//! ascription is independently, precisely re-checked by
//! `v1/module_check.rs`; `surface.rs` never causes a wrong ACCEPT, only (at
//! worst) a temporarily-too-wide surface that a later real check rejects.

use crate::v1::lower::qualify_type_key;
use rustyfi_syntax::cst_v1::{self, ast as ast_v1};
use rustyfi_syntax::leaf::{AnyHorzCmdTok, AnyVertCmdTok, TypeVarTok};
use rustyfi_syntax::span::Span;
use std::collections::{HashMap, HashSet};

/// One module's exported surface: names + arities only (§2.1) — no types,
/// no schemes; the semantic side (schemes, opacity, stamps) stays entirely
/// in `v1/static_env.rs`/`v1/module_check.rs`. Already **seal-filtered**
/// (the intersection of what a `struct .. end` defines and what its `:>`
/// declares, §2.1's "seal filter") when this is a sealed/coerced/annotated
/// module; the full defined surface otherwise.
#[derive(Clone, Debug, Default)]
pub(crate) struct ModSurface {
    /// Exported value-member names in source order (sigil'd command keys
    /// included: `"x"`, `"\cmd"`, `"+cmd"`).
    pub(crate) vals: Vec<String>,
    /// Exported type members: `(name, tyvar arity)`.
    pub(crate) types: Vec<(String, usize)>,
    /// Exported nested modules, recursively (each already seal-filtered by
    /// its OWN annotation — NOT further narrowed by an ancestor's
    /// `Decl::Module` restriction; see this module's doc comment on
    /// `filter_surface`'s scope limitation).
    pub(crate) mods: Vec<(String, ModSurface)>,
    /// Exported signature-member names (`signature S = ..` binds this
    /// module itself defines and does not hide).
    pub(crate) sigs: Vec<String>,
}

/// One `signature S = sigexpr` bind's definition, resolved to its underlying
/// `sig … end` decl list (§2.2's eager registration — a `Var`/`Path`-shaped
/// right-hand side is itself resolved at registration time, so every
/// `SigDef` bottoms out at a real decl list, borrowed straight from the
/// parsed `cst_v1` tree the caller's `deps` slice outlives). `decls` is
/// deliberately NOT flattened through any `Decl::Include` it may itself
/// contain — expansion is per-USE (`v1/module_check.rs::resolve_sig`'s own
/// job, and this module's `sig_decl_views` for the filter side), exactly
/// like Example 2 of the sig-include spec ("`Big`'s registration stores its
/// literal decls — the include NOT yet expanded").
#[derive(Clone, Debug)]
pub(crate) struct SigDef<'a> {
    pub(crate) decls: &'a [cst_v1::StructDeclV1],
    /// Sub-slice 2e-2 §2.3 named-sig storage: `with type` refinements this
    /// signature's OWN right-hand side carries (`signature S2 = S with type
    /// t = τ`) — `find_sig`'s consumers ([`crate::v1::module_check::
    /// resolve_sig`]) inherit these alongside `decls`, so a refinement
    /// composes across `signature` bind boundaries (W6). Empty for every
    /// plain (non-`with type`-bodied) `signature` bind.
    pub(crate) refines: Vec<Refine<'a>>,
    /// The module path the `signature` bind appeared at — diagnostics only
    /// (not read by anything in this module today).
    #[allow(dead_code)]
    pub(crate) def_path: Vec<String>,
}

/// Sub-slice 2e-2 (`…/tmp/slice2e-include-withtype.md` §2.2/§2.3): one
/// `with type t 'a… = τ` refinement, collected off a [`ast_v1::SigExpr::
/// WithType`] node's `binds` chain (or a named signature's OWN stored
/// refinement, inherited through [`SigDef::refines`]). Deliberately
/// UNVALIDATED here (`body` is kept whole — `Variant` or `Synonym`): whether
/// a variant-bodied refinement is illegal (it is — §7's "cannot introduce
/// constructors" row) is the CONSUMER's decision
/// ([`crate::v1::module_check`]'s `prescan_seal_types` `TypeOpaque`
/// interception), so the SAME check applies uniformly whether a refine
/// arrives via an inline `with type` or is inherited through a named
/// `signature S2 = S with type …` bind — this module never invents
/// user-facing text (module doc comment's standing posture).
#[derive(Clone, Debug)]
pub(crate) struct Refine<'a> {
    pub(crate) name: String,
    pub(crate) tyvars: &'a [TypeVarTok],
    pub(crate) body: &'a cst_v1::TypeBodyV1,
    pub(crate) span: Span,
}

/// Sub-slice 2f-1 (`…/tmp/slice2f-functors.md` §2.6): one `module Make =
/// fun (X : S) -> body` bind's definition, stored SYNTACTICALLY — a functor
/// is never lowered to a runtime value (like [`SigDef`] above, or a named
/// `signature`), so this is the whole of what `Make` denotes. Borrowed
/// straight from the parsed `cst_v1` tree (`deps` outlives the whole pass).
#[derive(Clone, Debug)]
pub(crate) struct FunctorDef<'a> {
    /// The parameter's bare name (`"X"`/`"Key"`).
    pub(crate) param: String,
    /// The parameter signature `S` — an ordinary sig (`SigBotV1::Var`/
    /// `Path`/`Sig`), never itself a `SigExpr::Functor` (§0.1's cut: a
    /// functor's PARAMETER is never itself higher-order in the demand).
    pub(crate) param_sig: &'a ast_v1::SigExpr,
    /// The un-instantiated body — re-lowered fresh, at a distinct path, by
    /// EVERY application (generativity, §2.4).
    pub(crate) body: &'a ast_v1::ModExpr,
    /// The functor bind's own defining path — where `S`'s named-signature
    /// references (if any) resolve outward from (§2.2).
    pub(crate) def_path: Vec<String>,
}

/// Sub-slice 2f-1 §2.6: the frozen, in-source-order resolution of one
/// `ModExpr::App` site (`Make Arg`) — `Some` only when BOTH `func` resolves
/// to a registered [`FunctorDef`] AND `arg` resolves to a CONCRETE,
/// already-registered [`ModSurface`] AND the functor's body is itself a
/// literal `struct … end` ([`crate::v1::functor::functor_body_binds`]
/// returns `Some`). Anything else (an unknown functor/argument name, an
/// argument that is itself an enclosing functor's OWN parameter — 2f-2's
/// `set.satyg` shape — or a non-struct functor body) freezes `None` here,
/// which `v1/lower.rs` turns into a precise Sub-slice-2f-2-naming
/// `LowerError`, never a panic.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AppResolution {
    pub(crate) functor_path: String,
    pub(crate) arg_path: String,
}

/// Both lookup tables, threaded across every dependency file in load order
/// (§2.1/§2.2's "one pass, file/bind order" rule — see [`build_file_surface`]).
#[derive(Default)]
pub(crate) struct SurfaceEnv<'a> {
    /// Full module path (`"Lib"`, `"Lib.N"`) → its exported surface.
    pub(crate) modules: HashMap<String, ModSurface>,
    /// Full signature-name path (`"Lib.S"`, `"Lib.M.S"`) → its definition.
    pub(crate) sigs: HashMap<String, SigDef<'a>>,
    /// An alias-bodied (`ModExpr::Var`/`Coerce`) `Bind::Module`'s own
    /// resolved target, recorded ONCE, AT BUILD TIME — keyed by the
    /// ALIAS's own qualified path (`"Lib.M"`), value = the target's
    /// qualified path if resolution succeeded, `None` if it failed (unknown
    /// name, or a FORWARD reference — §2.5's "may only target an earlier
    /// module" rule, §7's `L3` row). Load-bearing for `v1/lower.rs`'s
    /// `lower_module_alias`: [`build_file_surface`] builds the WHOLE file's
    /// surface in one pass before lowering ever starts, so by the time
    /// lowering runs, `modules` already contains EVERY module in the file
    /// (including ones defined LATER in source order) — re-resolving from
    /// scratch at that point would wrongly accept a forward reference. This
    /// map freezes the ordering-sensitive yes/no answer as it stood exactly
    /// when the alias bind was itself walked.
    pub(crate) alias_targets: HashMap<String, Option<String>>,
    /// Sub-slice 2e-1 §2.1 step 1: frozen in-source-order resolutions of
    /// `include` binds, keyed by (the includer's qualified path, the
    /// `include` keyword's own `Span`). A `Vec` with LINEAR lookup, not a
    /// `HashMap`: `Span` is `Eq` but not `Hash` (span.rs:11-15 — keeping
    /// `rustyfi-syntax` diff-empty in 2e), and a module has at most a
    /// handful of includes. `None` = resolution failed (unknown name, or a
    /// FORWARD reference — the same §2.5 "earlier module only" rule
    /// `alias_targets` enforces); a non-`Var` include body (`Struct`/
    /// `Coerce`/`App`/`Functor`) records NOTHING here — `v1/lower.rs`
    /// dispatches those straight to their own precise errors without ever
    /// consulting this table. [`frozen_include_target`] is the read side.
    pub(crate) include_targets: Vec<(String, Span, Option<String>)>,
    /// Sub-slice 2f-1 §2.6: full functor path (`"Map.Make"`, `"Code.Make"`)
    /// -> its definition. Registered by a `Bind::Module` whose body is a
    /// `ModExpr::Functor` literal — contributing NO `modules` entry (a
    /// functor name is not a usable module, mirroring `Bind::Signature`'s
    /// `sigs`-only registration).
    pub(crate) functors: HashMap<String, FunctorDef<'a>>,
    /// Sub-slice 2f-1 §2.6: the [`AppResolution`] twin of `include_targets`
    /// — frozen in-source-order resolutions of every `ModExpr::App` site,
    /// keyed by (the enclosing scope's qualified path, the App's own
    /// FUNCTOR-CHAIN span — a genuine lexed token span, unique per
    /// application, and the same span both a `Bind::Module`'s and a
    /// `Bind::Include`'s App body exposes via `func`). `None` = resolution
    /// failed (unknown functor/argument, a forward reference, the functor
    /// name actually denoting a struct, a non-struct functor body, or —
    /// `set.satyg`'s shape — the argument being an ENCLOSING functor's own
    /// parameter rather than a concrete module; all Sub-slice 2f-2).
    /// [`frozen_app_target`] is the read side.
    pub(crate) app_targets: Vec<(String, Span, Option<AppResolution>)>,
}

/// Sub-slice 2f-2a (`…/tmp/slice2d3b-2f2-sigmembers.md` §4.1): an innermost-
/// last stack of (functor parameter name -> the application's argument,
/// already resolved to its ABSOLUTE dotted path) active while walking a
/// functor body for one application — empty everywhere else. Lets a
/// parameter used as a functor ARGUMENT inside the body (`module Impl =
/// Map.Make Elem`, set.satyg's shape) resolve, instead of only a parameter
/// used as a plain qualified-reference head (2f-1's own scope). Names only —
/// no owned-tree substitution happens here (that stays `v1/functor.rs`'s
/// job at LOWERING time); this stack only steers `resolve_module`/
/// `resolve_functor` lookups during the SURFACE walk.
pub(crate) type ParamSubst = Vec<(String, String)>;

/// Splice `subst`'s answer (innermost/last entry wins) in place of
/// `rendered`'s leading dotted segment, keeping any further segments —
/// the string-keyed twin of `v1/functor.rs::HeadRewrite`'s splice rule
/// (test F-consistency pins the two never drifting apart).
fn subst_chain(rendered: &str, subst: &ParamSubst) -> String {
    let mut parts = rendered.splitn(2, '.');
    let head = parts.next().unwrap_or(rendered);
    let rest = parts.next();
    for (param, arg) in subst.iter().rev() {
        if head == param {
            return match rest {
                Some(r) => format!("{arg}.{r}"),
                None => arg.clone(),
            };
        }
    }
    rendered.to_string()
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

/// Register one whole dependency file's module tree + named-signature binds
/// into `env` — the ONLY entry point (`v1/lower.rs`/`v1/module_check.rs` both
/// call this, never the per-bind helpers below directly). A `FileV1::
/// Document` contributes nothing (a document is never a dependency; the
/// loader already rejects that combination before either caller reaches this
/// module — same defensive posture as `v1/lower.rs::lower_file_v1`).
///
/// Must be called for dependency files in the SAME order lowering/
/// `module_check` process them — an alias may only target an EARLIER module
/// (§2.5), and this builder's single left-to-right walk is exactly what
/// makes that ordering rule enforceable (a forward reference simply doesn't
/// resolve, §7's `L3` row).
pub(crate) fn build_file_surface<'a>(file: &'a cst_v1::FileV1, env: &mut SurfaceEnv<'a>) {
    if let cst_v1::FileV1::Library { name, sig_annot, binds, .. } = file {
        let path = vec![name.name.clone()];
        let bind_refs: Vec<&cst_v1::Bind> = binds.iter().collect();
        let raw = build_binds(&bind_refs, &path, env, &Vec::new());
        let filtered = match sig_annot {
            Some(sa) => filter_surface(raw, &sa.sig_.0, env, &path),
            None => raw,
        };
        env.modules.insert(path.join("."), filtered);
    }
}

/// Walk one `struct .. end` body's binds (or a library's top-level `binds`),
/// in source order, computing its raw (unfiltered) [`ModSurface`] and
/// registering every nested module/named-signature it defines into `env` as
/// it goes (so a LATER sibling alias can already resolve an EARLIER one,
/// §2.5).
fn build_binds<'a>(
    binds: &[&'a cst_v1::Bind],
    path: &[String],
    env: &mut SurfaceEnv<'a>,
    subst: &ParamSubst,
) -> ModSurface {
    let mut surf = ModSurface::default();
    for b in binds.iter().copied() {
        match b {
            cst_v1::Bind::Value { name, .. } => surf.vals.push(name.name.clone()),
            cst_v1::Bind::ValueInline { cmd, .. } => surf.vals.push(any_horz_name(cmd)),
            cst_v1::Bind::ValueBlock { cmd, .. } => surf.vals.push(any_vert_name(cmd)),
            cst_v1::Bind::ValueMath { cmd, .. } => surf.vals.push(any_horz_name(cmd)),
            cst_v1::Bind::ValueRec { first, ands, .. } => {
                surf.vals.push(first.name.name.clone());
                for a in ands {
                    surf.vals.push(a.clause.name.name.clone());
                }
            }
            cst_v1::Bind::ValueMutable { name, .. } => surf.vals.push(name.name.clone()),
            cst_v1::Bind::Type { first, ands, .. } => {
                surf.types.push((first.name.name.clone(), first.tyvars.len()));
                for a in ands {
                    surf.types.push((a.bind.name.name.clone(), a.bind.tyvars.len()));
                }
            }
            cst_v1::Bind::Module { name, sig_annot: _, body, .. } if matches!(&*body.0, ast_v1::ModExpr::Functor { .. }) => {
                // Sub-slice 2f-1 §2.6: a functor DEFINITION contributes NO
                // usable module at all — register the `FunctorDef` under
                // `child_path` and `continue`, WITHOUT ever touching
                // `env.modules`/`surf.mods` (mirrors `Bind::Signature`'s
                // `sigs`-only registration, a few arms below). Handled as
                // its own guarded arm — not inside the shared `base_surf`
                // match below — precisely so the shared post-match
                // `env.modules.insert`/`surf.mods.push` (every OTHER body
                // shape's common tail) never fires for a functor name.
                let ast_v1::ModExpr::Functor { param: fparam, dom, body: fbody, .. } = &*body.0 else {
                    unreachable!("guarded by the arm's own `matches!` pattern")
                };
                let mut child_path = path.to_vec();
                child_path.push(name.name.clone());
                env.functors.insert(
                    child_path.join("."),
                    FunctorDef {
                        param: fparam.name.clone(),
                        param_sig: dom.as_ref(),
                        body: fbody.as_ref(),
                        def_path: path.to_vec(),
                    },
                );
            }
            cst_v1::Bind::Module { name, sig_annot, body, .. } => {
                let mut child_path = path.to_vec();
                child_path.push(name.name.clone());
                // Sub-slice 2e-1 gap fix: the alias/coerce target's own
                // qualified path, if this bind resolved one — carried past
                // the `sig_annot` filter below so `register_sig_reexports`
                // can re-export the TARGET's named signatures under `Alias.S`
                // (2d-3's `Alias.S` re-export was never actually wired up;
                // see [`register_sig_reexports`]'s doc comment).
                let mut alias_target_path: Option<String> = None;
                let base_surf = match &*body.0 {
                    ast_v1::ModExpr::Struct { binds: inner, .. } => {
                        let inner_binds: Vec<&cst_v1::Bind> =
                            inner.iter().map(|sb| sb.0.as_ref()).collect();
                        build_binds(&inner_binds, &child_path, env, subst)
                    }
                    // Alias body: resolve the target NOW, in source order —
                    // a forward reference (a target defined LATER in the
                    // same body) misses here even though `env.modules` will
                    // eventually contain it, and that in-order yes/no answer
                    // is FROZEN into `alias_targets` so `v1/lower.rs` can't
                    // wrongly re-resolve it against the fully-built env
                    // (§2.5's "earlier module only" rule; see
                    // `SurfaceEnv::alias_targets`'s doc comment). Sub-slice
                    // 2f-2a: `subst_chain` first — the alias target may
                    // itself be the ENCLOSING functor's own parameter
                    // (`module Impl = Elem`-shaped, no demand package needs
                    // this exact shape, but it costs nothing extra once the
                    // stack exists).
                    ast_v1::ModExpr::Var(chain) => {
                        let resolved = resolve_module(env, path, &subst_chain(&chain.render(), subst))
                            .map(|(t, s)| (t, s.clone()));
                        alias_target_path = resolved.as_ref().map(|(t, _)| t.clone());
                        env.alias_targets
                            .insert(child_path.join("."), alias_target_path.clone());
                        resolved.map(|(_, s)| s).unwrap_or_default()
                    }
                    ast_v1::ModExpr::Coerce { name: target, sig_, .. } => {
                        let resolved = resolve_module(env, path, &subst_chain(&target.name, subst))
                            .map(|(t, s)| (t, s.clone()));
                        alias_target_path = resolved.as_ref().map(|(t, _)| t.clone());
                        env.alias_targets
                            .insert(child_path.join("."), alias_target_path.clone());
                        let target_surf =
                            resolved.map(|(_, s)| s).unwrap_or_default();
                        filter_surface(target_surf, sig_, env, &child_path)
                    }
                    // Sub-slice 2f-1 §2.6: a functor APPLICATION (`module M =
                    // Make Arg`) — resolve both operands, and (only when the
                    // functor's body is itself a literal `struct … end`)
                    // compute the result surface by walking the body's OWN
                    // binds at `child_path` (member NAMES never mention the
                    // parameter — only their REFERENCES do, §2.1 — so no
                    // substitution is needed here, only at lowering). Freeze
                    // the (possibly failed) resolution in `app_targets`
                    // regardless. Sub-slice 2f-2a: `subst` is threaded so
                    // `arg` (or `func`) may itself be the ENCLOSING functor's
                    // own parameter — set.satyg's `Map.Make Elem` shape.
                    ast_v1::ModExpr::App { func, arg } => {
                        build_app_result_surface(env, path, &child_path, func, arg, subst)
                    }
                    // Exhaustiveness only: the guarded arm above already
                    // intercepts every `Functor`-bodied `Bind::Module`
                    // before this (separate) match is ever reached.
                    ast_v1::ModExpr::Functor { .. } => {
                        unreachable!("a functor-bodied Bind::Module is intercepted by the guarded arm above")
                    }
                };
                let filtered = match sig_annot {
                    Some(sa) => filter_surface(base_surf, &sa.sig_.0, env, &child_path),
                    None => base_surf,
                };
                if let Some(target_path) = &alias_target_path {
                    register_sig_reexports(env, &child_path, target_path, &filtered);
                }
                env.modules.insert(child_path.join("."), filtered.clone());
                surf.mods.push((name.name.clone(), filtered));
            }
            cst_v1::Bind::Signature { name, sig_, .. } => {
                let key = qualify_type_key(path, &name.name);
                // Sub-slice 2e-2 §2.3 named-sig storage: `resolve_sig_rhs`
                // additionally resolves a `WithType`-bodied RHS (`signature
                // S2 = S with type t = τ`) — pre-2e-2 this arm's `sig_decls`
                // call returned `None` for `WithType`, so `S2` silently
                // never registered at all (a later `:> S2` missed with the
                // generic "unknown signature name" error, §8 risk 8's
                // "silent-unregistration wart" — closed here).
                if let Some((decls, refines)) = resolve_sig_rhs(&sig_.0, env, path) {
                    env.sigs.insert(key, SigDef { decls, refines, def_path: path.to_vec() });
                }
                surf.sigs.push(name.name.clone());
            }
            // `include M` (Sub-slice 2e-1, struct-include, §2.1 steps 1-2):
            // splice M's ENTIRE exported surface into THIS level, directly
            // (`surf.*.extend`, no synthetic sub-module wrapper — contrast
            // the alias arms above, which register a NEW name) — at THIS
            // include's position in source order, which is automatic here
            // since `surf` is built by this very loop as it walks `binds` in
            // order. Resolution is frozen in `include_targets`, exactly like
            // `alias_targets`: a forward reference misses even though
            // `env.modules` eventually contains the target. Only a `Var`
            // body resolves for real here; `App` (Sub-slice 2f-1: `include
            // Make Arg`, map.satyg's shape) resolves via `app_targets`
            // instead and splices the INSTANTIATED result surface — member
            // NAMES only, same "no substitution needed at this layer"
            // argument as the `Bind::Module` App arm above. `Struct`/
            // `Coerce`/`Functor` bodies record nothing here — `v1/lower.rs`
            // dispatches those to their own precise errors before ever
            // consulting either table, so the includer's surface being
            // too-narrow in those cases is safe (module doc comment's
            // standing posture).
            cst_v1::Bind::Include { kw, body } => match &*body.0 {
                ast_v1::ModExpr::Var(chain) => {
                    let resolved = resolve_module(env, path, &subst_chain(&chain.render(), subst))
                        .map(|(t, s)| (t, s.clone()));
                    let target_path = resolved.as_ref().map(|(t, _)| t.clone());
                    env.include_targets.push((path.join("."), kw.0, target_path));
                    if let Some((target_path, target_surf)) = resolved {
                        register_sig_reexports(env, path, &target_path, &target_surf);
                        surf.vals.extend(target_surf.vals);
                        surf.types.extend(target_surf.types);
                        surf.mods.extend(target_surf.mods);
                        surf.sigs.extend(target_surf.sigs);
                    }
                }
                ast_v1::ModExpr::App { func, arg } => {
                    // Splice UNWRAPPED at `path` itself (the includer's OWN
                    // level, not a fresh child path) — mirrors the `Var` arm
                    // above splicing `target_surf`, except the "target" is
                    // the functor's OWN body walked fresh at `path`.
                    let result_surf = build_app_result_surface(env, path, path, func, arg, subst);
                    surf.vals.extend(result_surf.vals);
                    surf.types.extend(result_surf.types);
                    surf.mods.extend(result_surf.mods);
                    surf.sigs.extend(result_surf.sigs);
                }
                ast_v1::ModExpr::Struct { .. }
                | ast_v1::ModExpr::Coerce { .. }
                | ast_v1::ModExpr::Functor { .. } => {}
            },
        }
    }
    surf
}

fn mod_chain_span(c: &ast_v1::ModChainV1) -> Span {
    match c {
        ast_v1::ModChainV1::Long(t) => t.span,
        ast_v1::ModChainV1::Single(t) => t.span,
    }
}

/// Sub-slice 2f-1 §2.6: shared by the `Bind::Module`/`Bind::Include` `App`
/// arms above — resolve `func`/`arg` outward from `enclosing_path`, freeze
/// the (possibly-failed) resolution into `env.app_targets` keyed by
/// `(enclosing_path, func`'s own span`)`, and — only on a full resolution
/// (functor found, argument found CONCRETE, functor body struct-shaped) —
/// return the result surface computed by walking the functor body's own
/// binds at `result_path` (the `Bind::Module` arm's fresh child path, or the
/// `Bind::Include` arm's own `path` — splicing vs. wrapping is the caller's
/// concern, this helper only computes "what the instantiation exports").
///
/// Sub-slice 2f-2a (spec §4.1/§4.3): `subst` is the stack ACTIVE AT THIS
/// APPLICATION SITE (so `func`/`arg` resolve as the enclosing scope would
/// see them — possibly themselves an outer functor's own parameter); the
/// body walk below pushes `(fdef.param, arg's resolved absolute path)` onto
/// a COPY of that stack, so a reference inside the body to ITS OWN
/// parameter resolves too — nested applications stack (`set.satyg`'s
/// `Map.Make Elem` shape, walked while instantiating `Set.Make`).
fn build_app_result_surface<'a>(
    env: &mut SurfaceEnv<'a>,
    enclosing_path: &[String],
    result_path: &[String],
    func: &ast_v1::ModChainV1,
    arg: &ast_v1::ModChainV1,
    subst: &ParamSubst,
) -> ModSurface {
    let functor_resolved: Option<(String, FunctorDef<'a>)> =
        resolve_functor(env, enclosing_path, &subst_chain(&func.render(), subst)).map(|(p, f)| (p, f.clone()));
    let arg_resolved: Option<(String, ModSurface)> =
        resolve_module(env, enclosing_path, &subst_chain(&arg.render(), subst)).map(|(p, s)| (p, s.clone()));
    let app_span = mod_chain_span(func);
    let body_binds: Option<&[cst_v1::StructBindV1]> = functor_resolved
        .as_ref()
        .and_then(|(_, fdef)| crate::v1::functor::functor_body_binds(fdef.body));
    let resolution = match (&functor_resolved, &arg_resolved, &body_binds) {
        (Some((fpath, _)), Some((apath, _)), Some(_)) => Some(AppResolution {
            functor_path: fpath.clone(),
            arg_path: apath.clone(),
        }),
        _ => None,
    };
    env.app_targets
        .push((enclosing_path.join("."), app_span, resolution.clone()));
    match (body_binds, &functor_resolved, &arg_resolved) {
        (Some(binds), Some((_, fdef)), Some((apath, _))) if resolution.is_some() => {
            let bind_refs: Vec<&cst_v1::Bind> = binds.iter().map(|sb| sb.0.as_ref()).collect();
            let mut body_subst = subst.clone();
            body_subst.push((fdef.param.clone(), apath.clone()));
            build_binds(&bind_refs, result_path, env, &body_subst)
        }
        _ => ModSurface::default(),
    }
}

/// Sub-slice 2f-1 §2.6: outward search against `env.functors` — the
/// [`resolve_module`] twin for functor names.
pub(crate) fn resolve_functor<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    site_path: &[String],
    chain: &str,
) -> Option<(String, &'b FunctorDef<'a>)> {
    for candidate in outward_candidates(site_path, chain) {
        if let Some(f) = env.functors.get(&candidate) {
            return Some((candidate, f));
        }
    }
    None
}

/// Sub-slice 2f-1 §2.6: the [`frozen_include_target`] twin for `ModExpr::App`
/// sites — consult THIS rather than re-running [`resolve_functor`]/
/// [`resolve_module`], keyed by (the enclosing scope's qualified path, the
/// App's own functor-chain `Span`).
pub(crate) fn frozen_app_target<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    enclosing_path: &[String],
    span: Span,
) -> Option<&'b Option<AppResolution>> {
    let key = enclosing_path.join(".");
    env.app_targets
        .iter()
        .find(|(p, s, _)| *p == key && *s == span)
        .map(|(_, _, t)| t)
}

fn joined(mods: &[String], name: &str) -> String {
    if mods.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", mods.join("."), name)
    }
}

/// Resolve a [`ast_v1::SigBotV1`] to its (single-level — a `Var`/`Path` is
/// resolved outward exactly ONCE, `Decl::Include` NOT flattened, matching
/// [`SigDef::decls`]'s own deferred-expansion posture) decl list plus any
/// `refines` the resolved definition itself carries. Shared by
/// [`resolve_sig_rhs`] (the named-sig registration side, §2.3) and
/// [`sig_bot_decl_views`] delegates to its OWN (Include-flattening) sibling
/// instead — this shallow helper is for the two call sites that need a
/// single dereference, not a full splice.
fn sig_bot_decls<'a>(
    bot: &'a ast_v1::SigBotV1,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
) -> Option<(&'a [cst_v1::StructDeclV1], Vec<Refine<'a>>)> {
    match bot {
        ast_v1::SigBotV1::Sig { decls, .. } => Some((decls.as_slice(), Vec::new())),
        ast_v1::SigBotV1::Var(t) => find_sig(env, site_path, &t.name).map(|d| (d.decls, d.refines.clone())),
        ast_v1::SigBotV1::Path(t) => {
            let suffix = joined(&t.mods, &t.name);
            find_sig(env, site_path, &suffix).map(|d| (d.decls, d.refines.clone()))
        }
    }
}

/// Sub-slice 2e-2 §2.3 named-sig storage: what `signature S2 = <rhs>`
/// registers (`build_binds`'s `Bind::Signature` arm) — the RHS's own
/// (unflattened) decl list, plus any `with type` refinements the RHS itself
/// carries: its OWN `binds` chain (via [`collect_refines`]) when the RHS is
/// a bare `WithType` node, PLUS — when the RHS's base names ANOTHER
/// registered signature — that signature's OWN stored `refines` (so a
/// refinement chain composes across `signature` bind boundaries, W6). A
/// `with ⟨path⟩ type` RHS (`path: Some(_)`) is left UNREGISTERED (`None`) —
/// this module never invents the precise 2d-3b error text; a later use sees
/// the generic "unknown signature name" miss instead (safe, just less
/// precise — §8 risk 8's accepted trade-off for a zero-demand corner).
fn resolve_sig_rhs<'a>(
    sig: &'a ast_v1::SigExpr,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
) -> Option<(&'a [cst_v1::StructDeclV1], Vec<Refine<'a>>)> {
    match sig {
        ast_v1::SigExpr::Bot(bot) => sig_bot_decls(bot, env, site_path),
        ast_v1::SigExpr::WithType { base, path: None, binds, .. } => {
            let (decls, mut refines) = sig_bot_decls(base, env, site_path)?;
            refines.extend(collect_refines(binds));
            Some((decls, refines))
        }
        ast_v1::SigExpr::WithType { path: Some(_), .. } => None,
        ast_v1::SigExpr::Functor { .. } => None,
    }
}

/// Sub-slice 2e-2 §2.2/§2.3: `with type t 'a… = τ and u… = σ` → one
/// [`Refine`] per chain link, DEFERRED (§2.3's rationale — the body is kept
/// whole, `Variant` or `Synonym`, so the SAME validation applies uniformly
/// whether a refine arrives via an inline [`ast_v1::SigExpr::WithType`] or
/// is INHERITED through a named `signature S2 = S with type …` bind).
/// `pub(crate)`: shared by `v1/module_check.rs::resolve_sig`'s own inline
/// `WithType` arm.
pub(crate) fn collect_refines(binds: &cst_v1::TypeBindsErasedV1) -> Vec<Refine<'_>> {
    let inner = &binds.0;
    let mut out = vec![refine_from_single(&inner.first)];
    for a in &inner.ands {
        out.push(refine_from_single(&a.bind));
    }
    out
}

fn refine_from_single(single: &cst_v1::TypeBindSingleV1) -> Refine<'_> {
    Refine {
        name: single.name.name.clone(),
        tyvars: &single.tyvars,
        body: &single.body,
        span: single.name.span,
    }
}

/// Sub-slice 2e-2 §2.2's `surface.rs` twin: the Vec-returning splice that
/// recursively flattens `Decl::Include` (with a resolved-table-key cycle
/// guard) — used by [`filter_surface`] so the seal filter works THROUGH an
/// included signature's declarations instead of bailing to "filter
/// nothing". `WithType` resolves to its base's decls here too (§2.2's
/// closing note: "a refinement never changes the exported NAME set").
/// `None` on a miss/unfilterable shape/cycle — the same safe "filter
/// nothing" posture the pre-2e-2 bail already had (this module never
/// invents user-facing text; the REAL error, if any, fires independently in
/// `v1/module_check.rs::resolve_sig`).
fn sig_decl_views<'a>(
    sig: &'a ast_v1::SigExpr,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
) -> Option<Vec<&'a cst_v1::StructDeclV1>> {
    let mut visited = Vec::new();
    sig_decl_views_visited(sig, env, site_path, &mut visited)
}

fn sig_decl_views_visited<'a>(
    sig: &'a ast_v1::SigExpr,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Option<Vec<&'a cst_v1::StructDeclV1>> {
    match sig {
        ast_v1::SigExpr::Bot(bot) => sig_bot_decl_views(bot, env, site_path, visited),
        ast_v1::SigExpr::WithType { base, .. } => sig_bot_decl_views(base, env, site_path, visited),
        ast_v1::SigExpr::Functor { .. } => None,
    }
}

fn sig_bot_decl_views<'a>(
    bot: &'a ast_v1::SigBotV1,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Option<Vec<&'a cst_v1::StructDeclV1>> {
    match bot {
        ast_v1::SigBotV1::Sig { decls, .. } => splice_decl_views(decls, env, site_path, visited),
        ast_v1::SigBotV1::Var(t) => named_sig_decl_views(&t.name, env, site_path, visited),
        ast_v1::SigBotV1::Path(t) => {
            let suffix = joined(&t.mods, &t.name);
            named_sig_decl_views(&suffix, env, site_path, visited)
        }
    }
}

fn named_sig_decl_views<'a>(
    name: &str,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Option<Vec<&'a cst_v1::StructDeclV1>> {
    let (key, def) = find_sig_keyed(env, site_path, name)?;
    if visited.contains(&key) {
        return None;
    }
    visited.push(key);
    let out = splice_decl_views(def.decls, env, site_path, visited);
    visited.pop();
    out
}

fn splice_decl_views<'a>(
    decls: &'a [cst_v1::StructDeclV1],
    env: &SurfaceEnv<'a>,
    site_path: &[String],
    visited: &mut Vec<String>,
) -> Option<Vec<&'a cst_v1::StructDeclV1>> {
    let mut out = Vec::new();
    for d in decls {
        if let ast_v1::Decl::Include { sig_, .. } = &*d.0 {
            out.extend(sig_decl_views_visited(sig_, env, site_path, visited)?);
        } else {
            out.push(d);
        }
    }
    Some(out)
}

/// The seal filter (§2.1): `raw`'s vals/types/mods/sigs, intersected with
/// whatever `sig` actually declares (name-only — width/depth mistakes are
/// `module_check`'s to report). `sig` resolved via [`sig_decl_views`]
/// (Sub-slice 2e-2: now flattens `include` too); a miss or an unfilterable
/// shape (a functor sig, or a cycle) returns `raw` UNCHANGED (full width) —
/// safe, per the module doc comment.
fn filter_surface<'a>(
    raw: ModSurface,
    sig: &'a ast_v1::SigExpr,
    env: &SurfaceEnv<'a>,
    site_path: &[String],
) -> ModSurface {
    let Some(decls) = sig_decl_views(sig, env, site_path) else {
        return raw;
    };
    let mut dv: HashSet<String> = HashSet::new();
    let mut dt: HashSet<String> = HashSet::new();
    let mut dm: HashSet<String> = HashSet::new();
    let mut ds: HashSet<String> = HashSet::new();
    for d in decls {
        match &*d.0 {
            ast_v1::Decl::Val { name, .. } => {
                dv.insert(name.name.clone());
            }
            ast_v1::Decl::ValHorzCmd { cmd, .. } => {
                dv.insert(cmd.name.clone());
            }
            ast_v1::Decl::ValVertCmd { cmd, .. } => {
                dv.insert(cmd.name.clone());
            }
            ast_v1::Decl::TypeOpaque { name, .. } => {
                dt.insert(name.name.clone());
            }
            ast_v1::Decl::Type { binds, .. } => {
                dt.insert(binds.0.first.name.name.clone());
                for a in &binds.0.ands {
                    dt.insert(a.bind.name.name.clone());
                }
            }
            ast_v1::Decl::Module { name, .. } => {
                dm.insert(name.name.clone());
            }
            ast_v1::Decl::Signature { name, .. } => {
                ds.insert(name.name.clone());
            }
            // Sub-slice 2e-2: `sig_decl_views` already flattened every
            // `Decl::Include` above — unreachable in practice; kept as a
            // defensive no-op (never a silent narrowing) rather than a
            // `_` wildcard, so a future `Decl` arm still breaks the build.
            ast_v1::Decl::Include { .. } => {}
        }
    }
    ModSurface {
        vals: raw.vals.into_iter().filter(|v| dv.contains(v)).collect(),
        types: raw.types.into_iter().filter(|(n, _)| dt.contains(n)).collect(),
        mods: raw.mods.into_iter().filter(|(n, _)| dm.contains(n)).collect(),
        sigs: raw.sigs.into_iter().filter(|n| ds.contains(n)).collect(),
    }
}

/// Outward search (§2.1/§2.2's shared rule): try `site_path` joined with
/// `suffix`, then each successively shorter prefix of `site_path`, then bare
/// `suffix` — first hit wins (lexical shadowing, innermost first).
fn outward_candidates(site_path: &[String], suffix: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(site_path.len() + 1);
    for i in (0..=site_path.len()).rev() {
        if i == 0 {
            out.push(suffix.to_string());
        } else {
            out.push(format!("{}.{}", site_path[..i].join("."), suffix));
        }
    }
    out
}

/// Resolve a module alias/path target (`chain`, e.g. `"N"` or `"A.B.C"`)
/// outward from `site_path` against `env.modules` — §2.1's alias-expansion
/// rule, and (unqualified) §2.3-7's `ImplView::Alias` target lookup. Returns
/// the resolved qualified path alongside the surface (the qualified path is
/// what a nested-module copy's absolute references qualify against).
pub(crate) fn resolve_module<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    site_path: &[String],
    chain: &str,
) -> Option<(String, &'b ModSurface)> {
    for candidate in outward_candidates(site_path, chain) {
        if let Some(s) = env.modules.get(&candidate) {
            return Some((candidate, s));
        }
    }
    None
}

/// The frozen (build-time, in-source-order) resolution of an alias-bodied
/// `Bind::Module` at `alias_path` (`["Lib","M"]`): `Some(Some(target))` if
/// it resolved to `target`, `Some(None)` if resolution FAILED (unknown/
/// forward reference), `None` if `alias_path` names no alias bind at all
/// (a struct-literal or functor body). `v1/lower.rs`'s `lower_module_alias`
/// consults THIS rather than re-running [`resolve_module`], so the
/// ordering-sensitive answer (§2.5) isn't relitigated against the
/// fully-built env — see [`SurfaceEnv::alias_targets`]'s doc comment.
pub(crate) fn frozen_alias_target<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    alias_path: &[String],
) -> Option<&'b Option<String>> {
    env.alias_targets.get(&alias_path.join("."))
}

/// Sub-slice 2e-1 §2.1 step 1: the [`frozen_alias_target`] twin for
/// `include` binds — consult THIS rather than re-running [`resolve_module`],
/// keyed by (the includer's qualified path, the `include` keyword's own
/// `Span` — a genuine lexed token span, unique per include). `Some(Some(t))`
/// = resolved to `t`; `Some(None)` = resolution FAILED (unknown/forward
/// reference); `None` = this exact (path, span) was never recorded (a
/// non-`Var` include body, or a caller passing the wrong span/path
/// entirely — impossible on the real pipeline, since `build_file_surface`
/// always runs before lowering and walks every `include` bind exactly once).
pub(crate) fn frozen_include_target<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    includer_path: &[String],
    kw_span: Span,
) -> Option<&'b Option<String>> {
    let key = includer_path.join(".");
    env.include_targets
        .iter()
        .find(|(p, s, _)| *p == key && *s == kw_span)
        .map(|(_, _, t)| t)
}

/// Sub-slice 2e-1: shared by the alias `Var`/`Coerce` arms (the found 2d-3
/// gap fix — an aliased module's named signatures were never re-exported,
/// so `:> Alias.S` used to miss even though `Alias`'s value/type members
/// resolved fine) and the `include` arm (§2.1 step 2's own requirement) —
/// for each signature name `s` the target surface exports, look up the
/// target's OWN registered [`SigDef`] (keyed `"target_path.s"`) and
/// re-register a CLONE under `at_path`'s own qualified key
/// ([`SigDef`] is `Clone`; `def_path` stays the ORIGINAL definer's,
/// diagnostics-only) — so `at_path.s` resolves through [`find_sig`] exactly
/// like a directly-defined `signature s = ..` bind would. A miss (the
/// target's own build somehow never registered it — should not happen for a
/// name genuinely in `surface.sigs`) is silently skipped, matching this
/// module's standing "never invent, only report via the caller" posture.
fn register_sig_reexports<'a>(
    env: &mut SurfaceEnv<'a>,
    at_path: &[String],
    target_path: &str,
    surface: &ModSurface,
) {
    for s in &surface.sigs {
        let target_key = format!("{target_path}.{s}");
        if let Some(def) = env.sigs.get(&target_key).cloned() {
            env.sigs.insert(qualify_type_key(at_path, s), def);
        }
    }
}

/// Resolve a named-signature reference outward from `site_path` — `suffix`
/// is `"S"` for a bare [`ast_v1::SigBotV1::Var`] or the joined
/// `"A.B.S"` for a [`ast_v1::SigBotV1::Path`] (§2.2's resolution rule).
/// `pub(crate)`: `v1/module_check.rs` calls this directly at every real
/// ascription site (the one place a miss must become a precise, user-facing
/// "unknown signature name" error — this module never invents that text).
pub(crate) fn find_sig<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    site_path: &[String],
    suffix: &str,
) -> Option<&'b SigDef<'a>> {
    find_sig_keyed(env, site_path, suffix).map(|(_, d)| d)
}

/// The [`find_sig`] twin that ALSO returns the matched candidate's fully
/// qualified TABLE KEY — needed by `v1/module_check.rs::resolve_named_sig`'s
/// cycle guard (Sub-slice 2e-2 §2.2: a re-entry must key on the RESOLVED
/// name, not the written suffix, or two differently-pathed same-suffix sigs
/// would false-positive, §8 risk 7) — `pub(crate)` for that one external
/// caller, mirroring [`find_sig`]'s own visibility.
pub(crate) fn find_sig_keyed<'a, 'b>(
    env: &'b SurfaceEnv<'a>,
    site_path: &[String],
    suffix: &str,
) -> Option<(String, &'b SigDef<'a>)> {
    for candidate in outward_candidates(site_path, suffix) {
        if let Some(d) = env.sigs.get(&candidate) {
            return Some((candidate, d));
        }
    }
    None
}

/// The joined-suffix formula [`find_sig`]'s callers use for a
/// [`ast_v1::SigBotV1::Path`] — `pub(crate)` so `v1/module_check.rs` doesn't
/// need its own copy of the "mods.join(\".\") + \".\" + name" formula.
pub(crate) fn sig_path_suffix(mods: &[String], name: &str) -> String {
    joined(mods, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_syntax::parse_file_v1;

    fn parse(src: &str) -> cst_v1::FileV1 {
        parse_file_v1(src).unwrap_or_else(|e| panic!("v1 parse failed: {e}"))
    }

    #[test]
    fn plain_struct_surface_lists_every_member() {
        let file = parse(
            "module Lib = struct\n\
             val x = 1\n\
             val f y = y\n\
             type t = int\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let surf = env.modules.get("Lib").expect("Lib registered");
        assert_eq!(surf.vals, vec!["x".to_string(), "f".to_string()]);
        assert_eq!(surf.types, vec![("t".to_string(), 0)]);
    }

    #[test]
    fn sealed_surface_is_filtered_to_declared_names() {
        let file = parse(
            "module Lib :> sig val x : int end = struct\n\
             val x = 1\n\
             val secret = 2\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let surf = env.modules.get("Lib").expect("Lib registered");
        assert_eq!(surf.vals, vec!["x".to_string()]);
    }

    #[test]
    fn unfilterable_sig_passes_the_full_surface_through() {
        let file = parse(
            "module Lib :> sig include S end = struct\n\
             val x = 1\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let surf = env.modules.get("Lib").expect("Lib registered");
        assert_eq!(surf.vals, vec!["x".to_string()]);
    }

    #[test]
    fn nested_module_resolves_outward_before_falling_back_to_bare() {
        let file = parse(
            "module Lib = struct\n\
             module Inner = struct val y = 2 end\n\
             module Alias = Inner\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let (resolved, surf) =
            resolve_module(&env, &["Lib".to_string()], "Inner").expect("Inner resolves");
        assert_eq!(resolved, "Lib.Inner");
        assert_eq!(surf.vals, vec!["y".to_string()]);
        let alias = env.modules.get("Lib.Alias").expect("Alias registered");
        assert_eq!(alias.vals, vec!["y".to_string()]);
    }

    #[test]
    fn named_signature_registers_and_resolves_by_qualified_key() {
        let file = parse(
            "module Lib = struct\n\
             signature S = sig val x : int end\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let def = find_sig(&env, &["Lib".to_string()], "S").expect("S resolves");
        assert_eq!(def.decls.len(), 1);
    }

    /// Sub-slice 2e-1 §2.1 steps 1-2: `include Base` splices Base's ENTIRE
    /// surface (vals + types) into the includer's own raw surface, at the
    /// include's position in source order — no synthetic sub-module name.
    #[test]
    fn include_splices_target_surface_at_its_source_position() {
        let file = parse(
            "module P = struct\n\
             module Base = struct val x = 1 val f y = y type t = int end\n\
             include Base\n\
             val extra = 1\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let surf = env.modules.get("P").expect("P registered");
        assert_eq!(
            surf.vals,
            vec!["x".to_string(), "f".to_string(), "extra".to_string()],
            "spliced vals sit BEFORE `extra`, matching source order"
        );
        assert_eq!(surf.types, vec![("t".to_string(), 0)]);
    }

    /// Sub-slice 2e-1 §2.1 step 1: an `include` naming an unknown/forward
    /// module freezes a `Some(None)` (not a silent miss) in
    /// `include_targets` — the `v1/lower.rs` consumer turns that into a
    /// precise `LowerError`.
    #[test]
    fn include_of_an_unknown_target_freezes_a_none_resolution() {
        let file = parse("module P = struct\ninclude Nope\nend");
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let surf = env.modules.get("P").expect("P registered");
        assert!(surf.vals.is_empty(), "an unresolved include splices nothing");
        assert_eq!(env.include_targets.len(), 1, "the miss is FROZEN, not merely absent");
        let (path, kw_span, target) = &env.include_targets[0];
        assert_eq!(path, "P");
        assert_eq!(target, &None);
        assert_eq!(
            frozen_include_target(&env, &["P".to_string()], *kw_span),
            Some(&None)
        );
    }

    /// Sub-slice 2e-1 §2.1 step 2 + the 2d-3 gap fix: `include Basic`
    /// re-exports `Basic`'s named signatures under the includer's OWN path
    /// (`P.Ord` resolves after `include Basic` names `Ord`).
    #[test]
    fn include_reexports_the_target_named_signature() {
        let file = parse(
            "module P = struct\n\
             module Basic = struct\n\
             signature Ord = sig type t :: o end\n\
             end\n\
             include Basic\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let def = find_sig(&env, &["P".to_string()], "Ord").expect("P.Ord resolves");
        assert_eq!(def.decls.len(), 1);
    }

    /// Sub-slice 2e-1 §2.1 step 2: an included target's surface arrives
    /// PRE-FILTERED when the target itself is sealed — a hidden member
    /// never splices.
    #[test]
    fn include_of_a_sealed_target_only_splices_its_declared_members() {
        let file = parse(
            "module P = struct\n\
             module Base :> sig val x : int end = struct\n\
             val x = 1\n\
             val secret = 2\n\
             end\n\
             include Base\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let surf = env.modules.get("P").expect("P registered");
        assert_eq!(surf.vals, vec!["x".to_string()]);
    }

    /// Sub-slice 2e-1's found 2d-3 gap fix: a plain alias (`module A2 =
    /// Basic`) ALSO re-exports the target's named signatures under its own
    /// path (`A2.Ord` resolves) — 2d-3 never wired this up for aliases; the
    /// same `register_sig_reexports` helper now fixes both flavors.
    #[test]
    fn alias_reexports_the_target_named_signature() {
        let file = parse(
            "module Lib = struct\n\
             module Basic = struct\n\
             signature Ord = sig type t :: o end\n\
             end\n\
             module A2 = Basic\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let def = find_sig(&env, &["Lib".to_string()], "A2.Ord").expect("Lib.A2.Ord resolves");
        assert_eq!(def.decls.len(), 1);
    }

    /// T-surf1 (Sub-slice 2f-1 spec §5): a functor DEFINITION registers a
    /// `FunctorDef` and contributes NO `modules` entry of its own; an
    /// APPLICATION to a concrete, already-registered argument computes the
    /// result surface by walking the functor body's own binds (member
    /// NAMES only — no substitution needed at this layer). Adapted from the
    /// spec's literal snippet by giving `A` a real definition (§4.B's own
    /// text requires the argument to resolve before a result surface is
    /// computed at all — an unresolved argument leaves the result empty,
    /// same posture as an unresolved alias/include target).
    #[test]
    fn functor_def_registers_no_member_and_application_computes_the_result_surface() {
        let file = parse(
            "module M = struct\n\
             module A = struct val a = 1 end\n\
             module F = fun (X : S) -> struct val y = X.a end\n\
             module R = F A\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        assert!(env.functors.contains_key("M.F"), "M.F must be a registered functor");
        assert!(!env.modules.contains_key("M.F"), "a functor name is never a usable module");
        let r = env.modules.get("M.R").expect("M.R registered");
        assert_eq!(r.vals, vec!["y".to_string()]);
    }

    /// T-surf2 (spec §5): an application naming an UNKNOWN functor freezes a
    /// failed (`Some(&None)`) resolution in `app_targets`, never a silent
    /// miss — the same posture `include_targets` already established.
    #[test]
    fn application_of_an_unknown_functor_freezes_a_failed_resolution() {
        let file = parse("module M = struct\nmodule R = Unknown A\nend");
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        // `M.R` still gets an (EMPTY) `modules` entry — the same posture an
        // unresolved alias/`Coerce` target already has (§2.5's established
        // "register empty, the caller decides what a miss means" rule);
        // what's under test here is that the MISS itself is frozen, not a
        // silent re-resolution risk.
        let r = env.modules.get("M.R").expect("M.R still registered, just empty");
        assert!(r.vals.is_empty() && r.mods.is_empty());
        assert_eq!(env.app_targets.len(), 1, "the miss is FROZEN, not merely absent");
        let (path, span, _) = &env.app_targets[0];
        assert_eq!(path, "M");
        assert_eq!(frozen_app_target(&env, &["M".to_string()], *span), Some(&None));
    }

    /// F-surf1 (Sub-slice 2f-2a spec §4.1/§4.3, formerly T-surf2 — this test
    /// PINNED THE GAP 2f-2a closes, and now pins the fix): a PARAMETER-
    /// argument application (`set.satyg`'s shape — the app's argument is the
    /// ENCLOSING functor's own bound parameter, not a directly-registered
    /// module) now RESOLVES, via the `ParamSubst` stack `build_app_result_
    /// surface` threads while walking `Outer`'s instantiated body: `X` (`F
    /// X`'s argument) substitutes to `Outer`'s own application's argument —
    /// `Base`, already resolved to `M.Base` — exactly as if `F` had been
    /// applied to `Base` directly. `Outer`'s body (`module R = F X`) is
    /// inert until `Outer` itself is applied (a functor's DEFINITION
    /// contributes nothing to the surface — only an APPLICATION walks its
    /// body, §2.6), so this fixture applies `Outer` to a real, resolvable
    /// argument (`Base`) to make that inner application actually get walked.
    #[test]
    fn application_whose_argument_is_the_enclosing_parameter_resolves_through_the_subst_stack() {
        let file = parse(
            "module M = struct\n\
             module F = fun (Y : S2) -> struct val g y = y end\n\
             module Base = struct end\n\
             module Outer = fun (X : S) -> struct module R = F X end\n\
             module Applied = Outer Base\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        let (path, span, resolution) = env
            .app_targets
            .iter()
            .find(|(p, _, _)| p == "M.Applied")
            .expect("the inner `F X` application (inside Outer's instantiated body) is frozen");
        assert_eq!(
            resolution,
            &Some(AppResolution { functor_path: "M.F".to_string(), arg_path: "M.Base".to_string() }),
            "F X's argument X (Outer's own parameter) substitutes to Outer's application's \
             argument, M.Base"
        );
        assert_eq!(
            frozen_app_target(&env, &["M".to_string(), "Applied".to_string()], *span),
            Some(&Some(AppResolution { functor_path: "M.F".to_string(), arg_path: "M.Base".to_string() }))
        );
        let _ = path;
    }

    /// F-surf1b: an application whose argument is the enclosing parameter
    /// but the ENCLOSING functor is never applied at all stays frozen-`None`
    /// — the parameter never resolves to anything outside an application
    /// (2f-2a's scope guard, spec §4.1).
    #[test]
    fn application_whose_argument_is_the_enclosing_parameter_stays_unresolved_when_never_applied() {
        let file = parse(
            "module M = struct\n\
             module F = fun (Y : S2) -> struct val g y = y end\n\
             module Outer = fun (X : S) -> struct module R = F X end\n\
             end",
        );
        let mut env = SurfaceEnv::default();
        build_file_surface(&file, &mut env);
        assert!(
            env.app_targets.is_empty(),
            "Outer's body is never walked at all unless Outer itself is applied"
        );
    }
}
