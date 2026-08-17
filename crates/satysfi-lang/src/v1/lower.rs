//! Structural CST-to-CST transcription: `cst_v1::ast` -> `cst::ast`
//! (`docs/plans/satysfi-0-1-0-support.md` §3, the finale spec's §1-§3).
//!
//! **Strategy (§1 of the finale spec).** Rather than widening
//! `elaborate.rs`'s ~30 expression-lowering helpers to also walk
//! `cst_v1::ast` nodes (which would re-implement the entire recursive walk —
//! operator-precedence climbing, itemize regrouping, pattern currying — over
//! a second node type), this module converts a parsed [`cst_v1::FileV1`]
//! into ordinary [`cst::TopBinding`]s / [`cst::ast::Expr`], and the caller
//! (`compile_document_v1` in `lib.rs`) assembles one synthetic [`cst::File`]
//! — exactly the shape the CLI's `merge_program` already produces for
//! 0.0.6 — and hands it to the **untouched** `elaborate::elaborate_program`
//! -> `typecheck_with_version(V0_1)` -> `compile`/`eval` pipeline.
//!
//! **Why this is near-mechanical.** `cst_v1::ast` imports the very same
//! `satysfi_syntax::leaf` token types `cst::ast` uses — `VarTok`, `DefEqTok`,
//! `ParenGroup<()>`, `LengthTok`, keyword leaves, … are literally identical
//! types on both sides, so most of the transcription below *moves/clones
//! tokens*, it does not re-encode them. The one genuinely non-1:1 seam is
//! the `CmdTail` bridge (§3.3, [`lower_cmd_tail`]): `cst_v1` kept the older
//! "one application-chain `Expr`" argument encoding, while `cst.rs` has
//! since moved to a flat `AppArg` list.
//!
//! **What this module deliberately does NOT lower** (§3.4 of the finale
//! spec — a real user-facing [`LowerError`], never a panic):
//!
//! - SATySFi 0.1 labeled-optional arguments (`?(l = e, …)`) and parameter
//!   bundles ARE lowered now (optional-arg-rows increment 1):
//!   [`AppArg::Bundled`]/[`AppArg::BundledCtor`] and [`Param::opts`] bridge
//!   to the additive `cst::ast::AppArg::Bundled`/`Expr::FunRows` nodes; the
//!   0.0.6 `?:`/`?*` positional markers no longer exist under V0_1 (the
//!   lexer emits only `?` = `OptionalType`). A bundle on an inline/block/
//!   math *command* parameter, and `?(…)` on the *type* level, remain a
//!   `LowerError` (roadmap increments 2/3).
//! - 0.1 math (`Atomic::MathText`, `InlineElem::EmbedMath`, and the whole
//!   math-element layer) — lowered since the math-split spec (L6): the
//!   deferral above turned out unnecessary. `${…}`'s *value* is version-
//!   independent ([`satysfi_syntax::SatysfiVersion::math_is_split`] only
//!   changes the surrounding TYPE/prim tables, `typecheck.rs`'s
//!   `name_to_mono`), so `Atomic::MathText`/`InlineElem::EmbedMath`
//!   transcribe structurally like every other node here — see
//!   `lower_math_elem_cst`/`lower_math_bot`/`lower_math_script`/
//!   `lower_math_group_arg`/`lower_math_arg`, below.
//! - A module-qualified command name (`\Mod.cmd`/`+Mod.cmd`) in *binding*
//!   position — the cst target field (`TopBinding::LetInline::cmd` /
//!   `LetBlock::cmd`) is a bare `HorzCmdTok`/`VertCmdTok`.
//! - An applied type constructor with more than one argument (`type t =
//!   pair int int`) — the 0.0.6 cst target (`cst::ast::TypeApp`) is
//!   single-argument postfix (`cst.rs:1297-1312`); 0.1's prefix `TypeApp` is
//!   n-ary, so the prefix→postfix bridge ([`lower_type_app`]) accepts arity
//!   0/1 and rejects arity ≥ 2 with a real `LowerError` (Sub-slice 2b, §3.4/
//!   §8 of the sub-slice 2b spec).
//!
//! **`type` names are pre-qualified during lowering** (Sub-slice 2b,
//! resolving slice2 spec §5 item 7): a module-local `type` name is rewritten
//! to its `qualify_key`-identical fully-qualified string (`"M.t"`) at
//! exactly three sites — the declared name, synonym/payload type
//! references, and applied-constructor heads (all in [`lower_type_single`]/
//! [`lower_type_atom`]/[`lower_type_app`]) — via [`TypeNameEnv`], threaded
//! through [`lower_bind_v1`]/[`lower_module_bind`]. Constructor names stay
//! unqualified (the carve-out; see [`TypeNameEnv`]'s doc comment). This is
//! confined to `v1/lower.rs`: `elaborate.rs`/`typecheck.rs` see the already-
//! dotted string as an ordinary opaque nominal key. Sub-slice 2d-2's
//! `LONG_LOWER` type names (`M.t`, [`ast_v1::TypeAtom::LongName`]/
//! [`ast_v1::TypeApp::AppliedLong`]) are the fourth site, but a DIFFERENT
//! formula: [`qualify_type_key`] applied to the token's own `mods`/`name`
//! directly, bypassing `TypeNameEnv::qualify` entirely — the reference is
//! already absolute (a dotted head), so there is nothing to resolve against
//! the LOCAL module's `type` names. Whether the resulting string names a
//! concrete type, an abstract stamp, or an error is undecidable HERE
//! (lowering runs before any seal table exists — 2d-1's zero-residue rule);
//! `v1/module_check.rs` resolves it later by string key.
//!
//! **Real modules, qualified exports (Sub-slice 2a).** [`lower_file_v1`] on
//! `FileV1::Library { module_kw, name, eq, struct_kw, binds, end_kw, .. }`
//! wraps `binds` in a single [`cst::TopBinding::Module`] — byte-shaped like
//! a 0.0.6 package file's own `module … = struct … end` — instead of
//! splicing them flat. `elaborate.rs`'s untouched `walk_bindings` Module arm
//! (`elaborate.rs:546-606`) then exports every binding **qualified**
//! (`Mod.x`, `export_alias`), and — same as every 0.0.6 module — only the
//! qualified names remain visible after the module closes: a V0_1 document
//! reaches a dependency's bindings via `Mod.x`, `\Mod.cmd`/`+Mod.cmd`, or
//! `let open Mod in`, never bare. Nested `module N = struct … end` binds
//! lower the same way (`lower_bind_v1`'s `Bind::Module` arm), sharing
//! `lower_module_bind` with the top-level case.
//!
//! **Sub-slice 2c/2d-1: module/signature grammar, ascription enforced.**
//! `cst_v1`'s grammar covers the FULL module/signature surface (functors,
//! `:>` coercion, `signature`/`include`, `sig … end`, `with type`) — but
//! this module still lowers only [`ast_v1::ModExpr::Struct`] bodies to real
//! semantics (2a's [`lower_module_bind`], unchanged). A `sig_annot` on a
//! struct-literal module is (as of 2d-1) simply DROPPED here — it carries
//! no runtime information at all, so lowering a sealed module produces the
//! byte-identical `TopBinding::Module` its unsealed twin would; enforcement
//! is entirely `v1/module_check.rs`'s job, reading the same annotation back
//! off the original `cst_v1` tree (`v1/static_env.rs` + `v1/sig_subtype.rs`
//! + `v1/module_check.rs`). Everything else reaching lowering still yields
//! a precise [`LowerError`] naming its owning sub-slice: module
//! aliases/paths and non-struct-literal `:>` coercion are 2d-3/2f (a `:>`
//! whose LEFT side isn't a `struct … end` literal, or whose right side
//! resolves to a bare module value, needs the static module environment
//! this port doesn't build until then), functors are 2f, `signature`/
//! `include` binds are 2d-3/2e. `Decl`/`SigExpr` never reach lowering on
//! their own — they occur only under a `sig_annot`/`Signature` position,
//! and (for binds) this module still never walks them structurally (no
//! `lower_decl`/`lower_sig_expr` exist here); `v1/module_check.rs` is the
//! one place that does, working directly from `cst_v1`.
//!
//! **The seal rule (load-bearing ordering).** `module M :> S = struct …
//! end` lowers its struct BODY FIRST (through 2a's `lower_module_bind`,
//! exactly as if there were no annotation), and only THEN — as of Sub-slice
//! 2d-1 — the annotation itself is lowered to NOTHING: `sig_annot` is
//! simply ignored by lowering (zero runtime residue; a sealed source lowers
//! to the byte-identical `TopBinding::Module { sig: None, .. }` its
//! unsealed twin does). Enforcement (width/depth `val` matching, sealing,
//! hiding) lives entirely in `v1/module_check.rs`, which reads the
//! annotation straight off the `cst_v1` tree this module never touches for
//! that purpose — see that module's doc comment. This still means a body
//! error (0.1 math, `?:` args, arity-≥2 type apps, …) surfaces with its own
//! precise message, unaffected by whether the module carries a `:>` at all
//! — pinned by `sig_annot_body_still_lowers_first` (§5.3 test 2 of the
//! sub-slice 2c spec).

use satysfi_syntax::cst;
use satysfi_syntax::cst_v1::{self, ast as ast_v1};
use satysfi_syntax::leaf::*;
use satysfi_syntax::Span;

/// A 0.1 construct Slice 1 deliberately does not lower yet. A real user
/// error (not a panic): points at the construct and the roadmap.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{span}: SATySFi 0.1 construct not supported yet in this port's Slice 1: {construct} ({hint})")]
pub struct LowerError {
    pub construct: &'static str,
    pub hint: &'static str,
    pub span: Span,
}

fn unsupported(construct: &'static str, hint: &'static str, span: Span) -> LowerError {
    LowerError {
        construct,
        hint,
        span,
    }
}

/// Module-path pre-qualification of 0.1 `type` names (Sub-slice 2b; see
/// slice2-module-system.md §5 item 7). Maps a locally-visible bare type
/// name to its fully-qualified nominal key, using EXACTLY
/// `elaborate::qualify_key`'s scheme (`elaborate.rs:289-295`): `"M.N.t"`
/// for `type t` inside `module M = struct module N = … end`.
/// `elaborate::qualify_key` itself is private to that module, so
/// [`qualify_type_key`] reproduces the identical formula here rather than
/// importing it — the two must never drift; both are one-liners.
///
/// **Ctor carve-out:** constructor names (`Known`, `Some`) are NEVER looked
/// up here — they stay surfaced unqualified (`UserTypeDecl.ctors`,
/// `elaborate.rs:147-154`), referenced as bare `Atomic::Ctor`/`PatBot::Ctor`
/// strings. 0.1 spells a qualified ctor `M.Ctor` via `LONG_UPPER`, which has
/// no token until Sub-slice 2c, so a pre-qualified ctor could not be
/// referenced from anywhere; cross-package ctor collisions remain a
/// documented latent 0.0.6-inherited limitation, resolved in 2d alongside
/// the static type env.
/// `pub(crate)`: Sub-slice 2d-1's `v1/module_check.rs` reuses this exact
/// type (and [`TypeNameEnv::child`]/`qualify`) to pre-qualify a sig's own
/// `val` type annotations against the SAME module-local `type` names the
/// struct body resolves against — a single-implementation guarantee that
/// the two walks (body lowering here, sig elaboration there) can never
/// drift apart on what a bare type name means (spec §4.2 steps 1c/1e).
#[derive(Clone, Default)]
pub(crate) struct TypeNameEnv(std::collections::HashMap<String, String>);

/// `elaborate::qualify_key`'s formula (`elaborate.rs:289-295`), reproduced
/// here since that function is private to the `elaborate` module — see
/// [`TypeNameEnv`]'s doc comment. `pub(crate)`: also the formula
/// `v1/module_check.rs` uses to key `StaticEnv::seals`/`hidden` by qualified
/// member name (spec §4.2 step 1e) — value names and type names share the
/// same qualification scheme.
pub(crate) fn qualify_type_key(mod_path: &[String], local: &str) -> String {
    if mod_path.is_empty() {
        local.to_string()
    } else {
        format!("{}.{}", mod_path.join("."), local)
    }
}

impl TypeNameEnv {
    pub(crate) fn qualify(&self, bare: &str) -> String {
        self.0.get(bare).cloned().unwrap_or_else(|| bare.to_string())
    }

    /// Child env for one module body: parent mappings (outer types stay
    /// visible inside, per ordinary module scoping) overlaid with this
    /// body's own `type` names — collected by a pre-scan over ALL of the
    /// body's `Bind::Type` arms (first + ands) BEFORE any bind is
    /// lowered, so forward and mutual references inside the module map
    /// correctly. Inner declarations shadow outer same-named ones.
    /// `mod_path` is the FULL path at which `binds` lives (i.e. already
    /// includes this module's own name — the caller, [`lower_module_bind`],
    /// computes it before calling this).
    pub(crate) fn child<'a>(
        &self,
        mod_path: &[String],
        binds: impl Iterator<Item = &'a cst_v1::Bind>,
    ) -> Self {
        let mut map = self.0.clone();
        for b in binds {
            if let cst_v1::Bind::Type { first, ands, .. } = b {
                map.insert(
                    first.name.name.clone(),
                    qualify_type_key(mod_path, &first.name.name),
                );
                for a in ands {
                    map.insert(
                        a.bind.name.name.clone(),
                        qualify_type_key(mod_path, &a.bind.name.name),
                    );
                }
            }
        }
        TypeNameEnv(map)
    }
}

/// Lower one dependency library (`module Name = struct binds end`) to a
/// single real [`cst::TopBinding::Module`] — names exported **qualified**
/// (Sub-slice 2a; see the module doc comment). `FileV1::Document` input is a
/// caller bug (the loader's `DocumentAsDependency` check already rejects it
/// before this is ever reached): a `LowerError`, not a panic.
pub fn lower_file_v1(file: &cst_v1::FileV1) -> Result<Vec<cst::TopBinding>, LowerError> {
    match file {
        cst_v1::FileV1::Library {
            module_kw,
            name,
            sig_annot: _,
            eq,
            struct_kw,
            binds,
            end_kw,
            ..
        } => {
            // The seal rule (module doc comment): lower the struct body
            // FIRST — exactly as if there were no annotation. As of
            // Sub-slice 2d-1, `sig_annot` is then simply DROPPED: it is
            // enforced by `v1/module_check.rs`, which reads it back off the
            // original `cst_v1` tree, never lowered (zero runtime residue).
            let module = lower_module_bind(
                module_kw,
                name,
                eq,
                struct_kw,
                binds,
                end_kw,
                &[],
                &TypeNameEnv::default(),
            )?;
            Ok(vec![module])
        }
        cst_v1::FileV1::Document { eoi, .. } => Err(unsupported(
            "a document file used as a dependency library",
            "the loader's DocumentAsDependency check should have rejected \
             this before lowering ever ran",
            eoi.0,
        )),
    }
}

/// Shared by [`lower_file_v1`] (the top-level library, whose `binds` field
/// is a plain `Vec<Bind>`) and `lower_bind_v1`'s `Bind::Module` arm (a
/// nested `module N = struct … end` bind, whose `binds` field is a
/// `Vec<StructBindV1>` — see [`cst_v1::StructBindV1`]'s doc comment for why
/// the two differ). Both produce a `cst::TopBinding::Module` with `sig:
/// None` (no signature annotation until Sub-slice 2c/2d) wrapping the
/// lowered binds. `mod_path` is the path of the ENCLOSING scope (`[]` at
/// the top level); this function extends it with `name` itself before
/// pre-scanning `binds` for `type` names ([`TypeNameEnv::child`], Sub-slice
/// 2b's pre-qualification, §4) and lowering each bind against the extended
/// env/path — `flat_map`, preserving source order, since a `Type` `and`-
/// chain now lowers to N consecutive `TopBinding`s (§3.0).
fn lower_module_bind<'a>(
    module_kw: &KwModule,
    name: &CtorTok,
    eq: &DefEqTok,
    struct_kw: &KwStruct,
    binds: impl IntoIterator<Item = &'a cst_v1::Bind> + Clone,
    end_kw: &KwEnd,
    mod_path: &[String],
    tyenv: &TypeNameEnv,
) -> Result<cst::TopBinding, LowerError> {
    let mut child_path = mod_path.to_vec();
    child_path.push(name.name.clone());
    let child_tyenv = tyenv.child(&child_path, binds.clone().into_iter());
    let mut decls = Vec::new();
    for b in binds {
        for tb in lower_bind_v1(b, &child_path, &child_tyenv)? {
            decls.push(cst::StructDecl(Box::new(tb)));
        }
    }
    Ok(cst::TopBinding::Module {
        kw: module_kw.clone(),
        name: name.clone(),
        sig: None,
        eq: eq.clone(),
        struct_kw: struct_kw.clone(),
        decls,
        end_kw: end_kw.clone(),
    })
}

/// Lower the entry document's body expression. `FileV1::Library` input is
/// the mirror-image caller bug (the loader's `LibraryAsEntry` check already
/// rejects it): a `LowerError`, not a panic.
pub fn lower_document_v1(file: &cst_v1::FileV1) -> Result<cst::ast::Expr, LowerError> {
    match file {
        cst_v1::FileV1::Document { body, .. } => lower_expr(body),
        cst_v1::FileV1::Library { end_kw, .. } => Err(unsupported(
            "a library file used as the entry document",
            "the loader's LibraryAsEntry check should have rejected this \
             before lowering ever ran",
            end_kw.0,
        )),
    }
}

// ---- Bind ---------------------------------------------------------------

/// `mod_path` is the path of the SCOPE `b` itself lives in (not including
/// any module `b` might itself introduce — see [`lower_module_bind`]'s doc
/// comment); `tyenv` is that same scope's `type`-name pre-qualification env
/// (§4). Returns a `Vec` (not a single `TopBinding`) since Sub-slice 2b's
/// `Type` arm's `and`-chain lowers to N consecutive `TopBinding::Type`s
/// (§3.0) — 1-element for every other arm.
fn lower_bind_v1(
    b: &cst_v1::Bind,
    mod_path: &[String],
    tyenv: &TypeNameEnv,
) -> Result<Vec<cst::TopBinding>, LowerError> {
    match b {
        cst_v1::Bind::Value {
            kw,
            name,
            params,
            eq,
            body,
        } => {
            let (ps, value) = lower_param_units(params, lower_expr(body)?)?;
            Ok(vec![cst::TopBinding::Let(cst::TopLet {
                let_kw: KwLet(kw.0),
                name: name.clone(),
                params: ps,
                eq: eq.clone(),
                value,
            })])
        }
        cst_v1::Bind::ValueInline {
            kw,
            ctx,
            cmd,
            params,
            eq,
            body,
            ..
        } => Ok(vec![cst::TopBinding::LetInline {
            kw: KwLetHorz(kw.0),
            ctx: ctx.clone(),
            cmd: plain_horz(cmd)?,
            params: lower_command_params(params)?,
            eq: eq.clone(),
            value: lower_expr(body)?,
        }]),
        cst_v1::Bind::ValueBlock {
            kw,
            ctx,
            cmd,
            params,
            eq,
            body,
            ..
        } => Ok(vec![cst::TopBinding::LetBlock {
            kw: KwLetVert(kw.0),
            ctx: ctx.clone(),
            cmd: plain_vert(cmd)?,
            params: lower_command_params(params)?,
            eq: eq.clone(),
            value: lower_expr(body)?,
        }]),
        cst_v1::Bind::ValueMath {
            kw,
            ctx,
            cmd,
            params,
            scripts,
            eq,
            body,
            ..
        } => Ok(vec![lower_value_math(kw, ctx, cmd, params, scripts, eq, body)?]),
        cst_v1::Bind::ValueRec { kw, first, ands, .. } => Ok(vec![cst::TopBinding::LetRec {
            kw: KwLetRec(kw.0),
            first: lower_rec_clause(first)?,
            ands: ands
                .iter()
                .map(|a| {
                    Ok(cst::ast::AndBinding {
                        and_kw: a.and_kw.clone(),
                        binding: lower_rec_clause(&a.clause)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
        }]),
        cst_v1::Bind::ValueMutable {
            kw,
            name,
            arrow,
            value,
            ..
        } => Ok(vec![cst::TopBinding::LetMutable {
            kw: KwLetMutable(kw.0),
            name: name.clone(),
            arrow: arrow.clone(),
            value: lower_expr(value)?,
        }]),
        cst_v1::Bind::Type { kw, first, ands } => {
            let mut out = Vec::with_capacity(1 + ands.len());
            out.push(lower_type_single(kw, first, tyenv)?);
            for a in ands {
                out.push(lower_type_single(kw, &a.bind, tyenv)?);
            }
            Ok(out)
        }
        cst_v1::Bind::Module {
            module_kw,
            name,
            sig_annot: _,
            eq,
            body,
        } => match &*body.0 {
            ast_v1::ModExpr::Struct { struct_kw, binds, end_kw } => {
                // 2a's path, one eraser hop deeper: the struct-literal body
                // lowers to a real TopBinding::Module regardless of the
                // annotation (the seal rule, module doc comment) — as of
                // Sub-slice 2d-1, `sig_annot` is then simply DROPPED here;
                // `v1/module_check.rs` enforces it straight off the
                // original `cst_v1` tree.
                let module = lower_module_bind(
                    module_kw,
                    name,
                    eq,
                    struct_kw,
                    binds.iter().map(|b| b.0.as_ref()),
                    end_kw,
                    mod_path,
                    tyenv,
                )?;
                Ok(vec![module])
            }
            ast_v1::ModExpr::Var(chain) => Err(unsupported(
                "a module alias/path binding (`module M = N`)",
                "module aliases need Sub-slice 2d-3's static module \
                 environment (name -> members); until then spell the \
                 module out as a struct literal",
                mod_chain_span(chain),
            )),
            ast_v1::ModExpr::App { func, .. } => Err(unsupported(
                "a functor application (`module M = F X`)",
                "functors are Sub-slice 2f (elaboration-time \
                 instantiation)",
                mod_chain_span(func),
            )),
            ast_v1::ModExpr::Functor { fun_kw, .. } => Err(unsupported(
                "a functor literal (`fun (X : S) -> ...`)",
                "functors are Sub-slice 2f (elaboration-time \
                 instantiation)",
                fun_kw.0,
            )),
            ast_v1::ModExpr::Coerce { coerce, .. } => Err(unsupported(
                "a module coercion (`M :> S`)",
                "sealing a NAME (not a struct literal) needs Sub-slice \
                 2d-3's static module environment — a `:>` directly on a \
                 `struct .. end` literal is Sub-slice 2d-1 and already \
                 enforced",
                coerce.0,
            )),
        },
        cst_v1::Bind::Signature { kw, .. } => Err(unsupported(
            "a `signature S = ...` binding",
            "signature names live in Sub-slice 2d-3's static environment \
             (consumed by 2d-3 ascription and 2e `include`) — parsed but \
             not yet enforced",
            kw.0,
        )),
        cst_v1::Bind::Include { kw, .. } => Err(unsupported(
            "an `include M` binding",
            "include splices a module's bindings at elaboration time and \
             needs Sub-slice 2d-3's static environment — Sub-slice 2e",
            kw.0,
        )),
    }
}

/// Any reasonable span off a [`ast_v1::ModChainV1`] — used only to point a
/// [`LowerError`] at the offending module-path/functor-operand token.
fn mod_chain_span(c: &ast_v1::ModChainV1) -> Span {
    match c {
        ast_v1::ModChainV1::Long(t) => t.span,
        ast_v1::ModChainV1::Single(t) => t.span,
    }
}

/// The shared clause bridge for `val rec`/`let rec` — the `RecBinding`
/// reshape the module doc anticipated, field-by-field against the real
/// `cst::ast::RecBinding` (`cst.rs:753-762`).
fn lower_rec_clause(c: &ast_v1::RecClauseV1) -> Result<cst::ast::RecBinding, LowerError> {
    let value_expr = lower_expr(&c.value.0)?;
    // `RecBinding.params` is `Vec<PatBot>` (not `Vec<Param>`). All-plain
    // clauses lower their patterns directly (byte-identical). A clause with a
    // `?(l = x, …)` bundle desugars to `params: []` + a nested lambda-chain
    // value — the LetRec lambda-body invariant holds because the chain head
    // is itself a lambda.
    let (params, value) = if c.params.iter().all(|p| p.opts.is_none()) {
        let ps = c
            .params
            .iter()
            .map(|p| lower_param_body(&p.body))
            .collect::<Result<_, _>>()?;
        (ps, value_expr)
    } else {
        let (_, chain) = lower_param_units(&c.params, value_expr)?;
        (Vec::new(), chain)
    };
    Ok(cst::ast::RecBinding {
        // `BindName` == `BindName`: shared leaf-level type, clone not
        // re-encode (see the module doc comment).
        name: c.name.clone(),
        // No `: ty` form in 0.1's `bind_value_nonrec`.
        ascription: None,
        // No multi-clause sugar in 0.1.
        leading_bar: None,
        params,
        eq: c.eq.clone(),
        value: erase_expr(value),
        // `RecClause` continuations are a 0.0.6-only surface (cst.rs:779-786).
        extra: Vec::new(),
    })
}

// ---- type binds (Sub-slice 2b) -----------------------------------------

fn lower_type_single(
    kw: &KwType,
    s: &cst_v1::TypeBindSingleV1,
    tyenv: &TypeNameEnv,
) -> Result<cst::TopBinding, LowerError> {
    Ok(cst::TopBinding::Type(cst::TypeDecl {
        // One shared `type` span per chain.
        kw: kw.clone(),
        // Field REORDER: 0.1 postfix (`type t 'a`) → cst prefix slot.
        tyvars: s.tyvars.clone(),
        name: VarTok {
            name: tyenv.qualify(&s.name.name),
            span: s.name.span,
        },
        eq: s.eq.clone(),
        body: match &s.body {
            cst_v1::TypeBodyV1::Variant { leading_bar, first, rest } => cst::TypeDeclBody::Variant {
                leading_bar: leading_bar.clone(),
                first: lower_variant_def(first, tyenv)?,
                rest: rest
                    .iter()
                    .map(|b| {
                        Ok(cst::BarVariantDef {
                            bar: b.bar.clone(),
                            def: lower_variant_def(&b.def, tyenv)?,
                        })
                    })
                    .collect::<Result<_, LowerError>>()?,
            },
            cst_v1::TypeBodyV1::Synonym(ty) => cst::TypeDeclBody::Synonym(lower_type_expr(ty, tyenv)?),
        },
    }))
}

fn lower_variant_def(v: &cst_v1::VariantDefV1, tyenv: &TypeNameEnv) -> Result<cst::VariantDef, LowerError> {
    Ok(cst::VariantDef {
        // Ctors stay UNQUALIFIED — §4's carve-out.
        ctor: v.ctor.clone(),
        of_ty: v
            .of_ty
            .as_ref()
            .map(|o| {
                Ok(cst::OfType {
                    of_kw: o.of_kw.clone(),
                    ty: lower_type_expr(&o.ty, tyenv)?,
                })
            })
            .transpose()?,
    })
}

pub(crate) fn lower_type_expr(
    t: &ast_v1::TypeExpr,
    tyenv: &TypeNameEnv,
) -> Result<cst::ast::TypeExpr, LowerError> {
    Ok(match t {
        ast_v1::TypeExpr::Fun { dom, arrow, cod } => cst::ast::TypeExpr::Fun {
            // 0.1 has no `?->` domain-suffix syntax at all (that's 0.0.6's
            // own fused sigil, dropped in 0.1) — `opts` here is always empty.
            // The 0.1 `?(…) ->` PREFIX form is `ast_v1::TypeExpr::OptRowFun`,
            // a separate arm below (optional-arg-rows increment 2).
            opts: Vec::new(),
            dom: lower_type_prod(dom, tyenv)?,
            arrow: arrow.clone(),
            cod: Box::new(lower_type_expr(cod, tyenv)?),
        },
        ast_v1::TypeExpr::Atom(p) => cst::ast::TypeExpr::Atom(lower_type_prod(p, tyenv)?),
        // `?(l1 : ty1, … [| ?'r]) dom -> cod` (optional-arg-rows increment
        // 2). A row-variable tail is parsed but rejected here: it needs
        // signature-level row quantification (`rowquant`/`quant`,
        // `parser_v1.mly:631-633`) — L4/2d territory, not this increment
        // (contrast the record-type row-tail below, which THIS increment
        // DOES complete — a bare record type has no `quant`-list obligation
        // to satisfy).
        ast_v1::TypeExpr::OptRowFun { opt_dom, dom, arrow, cod } => {
            if let Some(tail) = &opt_dom.inner.row_tail {
                return Err(unsupported(
                    "a row-variable tail in an optional-argument type domain (`| ?'r`)",
                    "row quantification arrives with signature enforcement — \
                     roadmap L4 / Sub-slice 2d",
                    tail.var.span,
                ));
            }
            if opt_dom.inner.entries.is_empty() {
                return Err(unsupported(
                    "an empty `?()` optional-argument type domain",
                    "a `?(…)` domain must bind at least one label",
                    opt_dom.q.0,
                ));
            }
            cst::ast::TypeExpr::OptRowFun {
                opt_dom: cst::ast::CstTypeOptDom {
                    q: opt_dom.q.clone(),
                    paren: clone_paren(&opt_dom.paren),
                    entries: opt_dom
                        .inner
                        .entries
                        .iter()
                        .map(|e| {
                            Ok(cst::ast::CstTypeOptEntry {
                                label: e.label.clone(), // labels are NOT type names
                                colon: e.colon.clone(),
                                ty: cst::TyErased(Box::new(lower_type_expr(&e.ty.0, tyenv)?)),
                                comma: e.comma.clone(),
                            })
                        })
                        .collect::<Result<_, LowerError>>()?,
                },
                dom: lower_type_prod(dom, tyenv)?,
                arrow: arrow.clone(),
                cod: Box::new(lower_type_expr(cod, tyenv)?),
            }
        }
    })
}

fn lower_type_prod(p: &ast_v1::TypeProd, tyenv: &TypeNameEnv) -> Result<cst::ast::TypeProd, LowerError> {
    Ok(cst::ast::TypeProd {
        first: lower_type_app(&p.first, tyenv)?,
        rest: p
            .rest
            .iter()
            .map(|s| {
                Ok(cst::ast::StarType {
                    star: s.star.clone(),
                    ty: lower_type_app(&s.ty, tyenv)?,
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

/// The PREFIX → POSTFIX bridge (§3.5 item 1): 0.1 `list int` ↔ cst `int
/// list`. A real `LowerError` (not a panic) on arity ≥ 2 — the cst target
/// (`cst::ast::TypeApp`) is single-argument by design (`cst.rs:1297-1312`,
/// "N-ary applied constructors are not [supported]"). Sub-slice 2d-2 adds
/// `InlineCmdTy`/`BlockCmdTy` (→ `cst::ast::TypeAtom::Cmd`, wrapped in
/// `TypeApp::Atom` — a command type is never itself "applied") and
/// `AppliedLong` (the same prefix→postfix bridge as `Applied`, minus
/// `tyenv.qualify`: a `LONG_LOWER` head is already absolute, spec §2.4).
fn lower_type_app(a: &ast_v1::TypeApp, tyenv: &TypeNameEnv) -> Result<cst::ast::TypeApp, LowerError> {
    match a {
        ast_v1::TypeApp::InlineCmdTy { kw, list, args } => {
            Ok(cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Cmd {
                list: list.clone(),
                args: lower_type_cmd_args(args, tyenv)?,
                kind: cst::ast::CmdTypeKind::Inline(HorzCmdTypeTok(kw.0)),
            }))
        }
        ast_v1::TypeApp::BlockCmdTy { kw, list, args } => {
            Ok(cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Cmd {
                list: list.clone(),
                args: lower_type_cmd_args(args, tyenv)?,
                kind: cst::ast::CmdTypeKind::Block(VertCmdTypeTok(kw.0)),
            }))
        }
        // `math […]` (math-package completion M1). `lower_type_atom`'s Cmd
        // arm in `typecheck.rs` already produces `MonoType::MathCmd` for
        // `CmdTypeKind::Math` and `unify.rs` already dispatches it — no
        // typecheck/unify change needed for the head itself.
        ast_v1::TypeApp::MathCmdTy { kw, list, args } => {
            Ok(cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Cmd {
                list: list.clone(),
                args: lower_type_cmd_args(args, tyenv)?,
                kind: cst::ast::CmdTypeKind::Math(MathCmdTypeTok(kw.0)),
            }))
        }
        ast_v1::TypeApp::AppliedLong { ctor, first, rest } => {
            if let Some(extra) = rest.first() {
                return Err(unsupported(
                    "an applied type constructor with more than one argument",
                    "the 0.0.6 cst target (`TypeApp`) is single-argument \
                     (cst.rs:1297-1312) — widen it only when a real package \
                     needs arity ≥ 2",
                    type_atom_span(extra),
                ));
            }
            Ok(cst::ast::TypeApp::Applied {
                arg: lower_type_atom(first, tyenv)?,
                // NO `tyenv.qualify`: a `LONG_LOWER` head (`M.t`) is already
                // an absolute dotted reference — qualifying it again would
                // be wrong (and `qualify` only ever rewrites BARE names
                // anyway, so this is purely a documentation point, not a
                // behavioral guard).
                ctor: VarTok {
                    name: qualify_type_key(&ctor.mods, &ctor.name),
                    span: ctor.span,
                },
            })
        }
        ast_v1::TypeApp::Applied { ctor, first, rest } => {
            if let Some(extra) = rest.first() {
                return Err(unsupported(
                    "an applied type constructor with more than one argument",
                    "the 0.0.6 cst target (`TypeApp`) is single-argument \
                     (cst.rs:1297-1312) — widen it only when a real package \
                     needs arity ≥ 2",
                    type_atom_span(extra),
                ));
            }
            Ok(cst::ast::TypeApp::Applied {
                arg: lower_type_atom(first, tyenv)?,
                ctor: VarTok {
                    name: tyenv.qualify(&ctor.name),
                    span: ctor.span,
                },
            })
        }
        ast_v1::TypeApp::Atom(at) => Ok(cst::ast::TypeApp::Atom(lower_type_atom(at, tyenv)?)),
    }
}

/// Each `[…]`-bracketed command-type slot lowers to one
/// `cst::ast::TypeCmdArgItem` (`opt: None` — 2d-2's grammar has no `?`
/// suffix of its own; `semi: None` — these synthetic items are never
/// re-unparsed, only fed to `elaborate`/`typecheck`, so the `;`-separator
/// token is immaterial). `opt_labels` (optional-arg-rows increment 3a)
/// carries this slot's `?(l:τ,…)` prefix bundle, if any — a flat,
/// *surface-order* list; `typecheck.rs`'s `lower_type_atom` `Cmd` arm is
/// responsible for sorting it into the closed map's canonical order (kept
/// unsorted here, matching every other lowering site in this file, which
/// never itself imposes a canonical order on anything — that's a
/// typecheck-time concern).
fn lower_type_cmd_args(
    args: &[ast_v1::TypeCmdArgItemV1],
    tyenv: &TypeNameEnv,
) -> Result<Vec<cst::ast::TypeCmdArgItem>, LowerError> {
    args.iter()
        .map(|a| {
            Ok(cst::ast::TypeCmdArgItem {
                opt_labels: match &a.opts {
                    None => Vec::new(),
                    Some(dom) => {
                        if dom.entries.is_empty() {
                            return Err(unsupported(
                                "an empty `?()` command-type optional-label bundle",
                                "a `?(…)` bundle must bind at least one label",
                                dom.q.0,
                            ));
                        }
                        dom.entries
                            .iter()
                            .map(|e| {
                                Ok(cst::ast::TypeCmdOptField {
                                    label: e.label.clone(),
                                    colon: e.colon.clone(),
                                    ty: cst::TyErased(Box::new(lower_type_expr(&e.ty.0, tyenv)?)),
                                    comma: e.comma.clone(),
                                })
                            })
                            .collect::<Result<_, LowerError>>()?
                    }
                },
                ty: cst::TyErased(Box::new(lower_type_expr(&a.ty.0, tyenv)?)),
                opt: None,
                semi: None,
            })
        })
        .collect()
}

fn lower_type_atom(a: &ast_v1::TypeAtom, tyenv: &TypeNameEnv) -> Result<cst::ast::TypeAtom, LowerError> {
    Ok(match a {
        ast_v1::TypeAtom::Paren { paren, inner } => cst::ast::TypeAtom::Paren {
            paren: paren.clone(),
            inner: cst::TyErased(Box::new(lower_type_expr(&inner.0, tyenv)?)),
        },
        // Closed form (`row_tail: None`) transcribes to the existing
        // `cst::ast::TypeAtom::Record`, byte-identical to before this
        // increment. Open form (`row_tail: Some(_)`, optional-arg-rows
        // increment 2) transcribes to the additive
        // `cst::ast::TypeAtom::RecordOpen` instead — a fresh row variable at
        // the `typecheck.rs` end, not the SAME variable across occurrences
        // (this increment models one open record type at a time, not
        // cross-signature shared-row polymorphism).
        ast_v1::TypeAtom::Record { rec, inner } if inner.row_tail.is_none() => {
            cst::ast::TypeAtom::Record {
                rec: rec.clone(),
                fields: inner
                    .fields
                    .iter()
                    .map(|f| {
                        Ok(cst::ast::TypeRecordField {
                            name: f.name.clone(),   // labels are NOT type names —
                            colon: f.colon.clone(), // no `tyenv.qualify` on `name`
                            ty: cst::TyErased(Box::new(lower_type_expr(&f.ty.0, tyenv)?)),
                            // `,` dropped (`semi: None`) — synthetic tree is
                            // never unparsed; `lower_record_field`/
                            // `lower_type_cmd_args` precedent.
                            semi: None,
                        })
                    })
                    .collect::<Result<_, LowerError>>()?,
            }
        }
        ast_v1::TypeAtom::Record { rec, inner } => {
            let tail = inner.row_tail.as_ref().expect("guarded by the arm above");
            cst::ast::TypeAtom::RecordOpen {
                rec: rec.clone(),
                inner: cst::ast::CstRecordOpenInner {
                    fields: inner
                        .fields
                        .iter()
                        .map(|f| {
                            Ok(cst::ast::CstRecordOpenField {
                                name: f.name.clone(),
                                colon: f.colon.clone(),
                                ty: cst::TyErased(Box::new(lower_type_expr(&f.ty.0, tyenv)?)),
                                comma: None,
                            })
                        })
                        .collect::<Result<_, LowerError>>()?,
                    bar: tail.bar.clone(),
                    var: tail.var.clone(),
                },
            }
        }
        ast_v1::TypeAtom::Var(v) => cst::ast::TypeAtom::Var(v.clone()),
        // NO `tyenv.qualify` — see `AppliedLong`'s doc comment above; the
        // same "already absolute" argument applies bare.
        ast_v1::TypeAtom::LongName(t) => cst::ast::TypeAtom::Name(VarTok {
            name: qualify_type_key(&t.mods, &t.name),
            span: t.span,
        }),
        ast_v1::TypeAtom::Name(n) => cst::ast::TypeAtom::Name(VarTok {
            name: tyenv.qualify(&n.name),
            span: n.span,
        }),
    })
}

/// Any reasonable span off a [`ast_v1::TypeAtom`] — used only to point the
/// arity-≥2 [`LowerError`] at the offending extra argument.
fn type_atom_span(a: &ast_v1::TypeAtom) -> Span {
    match a {
        ast_v1::TypeAtom::Paren { paren, .. } => paren.open.0,
        ast_v1::TypeAtom::Record { rec, .. } => rec.open.0,
        ast_v1::TypeAtom::Var(v) => v.span,
        ast_v1::TypeAtom::LongName(t) => t.span,
        ast_v1::TypeAtom::Name(n) => n.span,
    }
}

fn plain_horz(name: &AnyHorzCmdTok) -> Result<HorzCmdTok, LowerError> {
    match name {
        AnyHorzCmdTok::Plain(t) => Ok(t.clone()),
        AnyHorzCmdTok::Mod(t) => Err(unsupported(
            "a module-qualified command name in binding position",
            "the cst target field (`LetInline::cmd`) is a bare `HorzCmdTok` \
             — not valid 0.1 syntax",
            t.span,
        )),
    }
}

fn plain_vert(name: &AnyVertCmdTok) -> Result<VertCmdTok, LowerError> {
    match name {
        AnyVertCmdTok::Plain(t) => Ok(t.clone()),
        AnyVertCmdTok::Mod(t) => Err(unsupported(
            "a module-qualified command name in binding position",
            "the cst target field (`LetBlock::cmd`) is a bare `VertCmdTok` \
             — not valid 0.1 syntax",
            t.span,
        )),
    }
}

/// Lower a `param_unit` list plus the (already-lowered) binding body into a
/// `(cst params, cst value)` pair (upstream `curry_lambda_abstraction`, one
/// `UTFunction` per `param_unit`).
///
/// - **Every unit plain** (`opts: None`): the params lower directly and
///   `body` is returned unchanged — byte-identical to the pre-optional-arg
///   path.
/// - **Any unit bundled** (`?(l = x, …)`): the whole list right-folds into a
///   nested `FunRows`/`Fun` lambda chain, and the returned param list is
///   empty — so the target `TopLet`/`RecBinding`/`LetIn` shape stays frozen
///   (a bundled binding is `params: []` + a lambda-chain value). This is the
///   `f p = e ≡ f = fun p -> e` identity applied per unit.
fn lower_param_units(
    params: &[cst_v1::Param],
    body: cst::ast::Expr,
) -> Result<(Vec<cst::ast::Param>, cst::ast::Expr), LowerError> {
    if params.iter().all(|p| p.opts.is_none()) {
        let ps = params
            .iter()
            .map(|p| Ok(cst::ast::Param::Pat(lower_param_body(&p.body)?)))
            .collect::<Result<_, LowerError>>()?;
        return Ok((ps, body));
    }
    let mut chain = body;
    for p in params.iter().rev() {
        let param_pat = lower_param_body(&p.body)?;
        chain = match &p.opts {
            Some(opts) => cst::ast::Expr::FunRows {
                kw: KwFun(opts.q.0),
                opts: lower_opt_binders(opts)?,
                param: param_pat,
                arrow: ArrowTok(opts.q.0),
                body: Box::new(chain),
            },
            None => cst::ast::Expr::Fun {
                kw: KwFun(Span::default()),
                params: vec![param_pat],
                arrow: ArrowTok(Span::default()),
                body: Box::new(chain),
            },
        };
    }
    Ok((Vec::new(), chain))
}

/// Lower an inline/block/math command binding's own `Param` list, preserving
/// order 1:1 (each `cst_v1::Param` maps to exactly one `cst::ast::Param` —
/// unlike the value-level `FunRows` desugar, which right-folds a bundled
/// unit into a lambda chain and returns an EMPTY param list, a command
/// binding's `params` vec carries order straight into `curry_cmd_params_v1`).
/// A plain (non-bundled) unit lowers to `Param::Pat` as before (optional-
/// arg-rows increment 1 and earlier); a `?(l = x, …)`-bundled unit
/// (optional-arg-rows increment 3a) lowers to the additive
/// `cst::ast::Param::Bundled`, consumed by `elaborate.rs`'s bundle-aware
/// `curry_cmd_params_v1`. Shared by `ValueInline`/`ValueBlock` (which accept
/// a bundle freely) AND `ValueMath` (`lower_value_math` rejects a bundle
/// itself, BEFORE calling this — math command parameter bundles are
/// optional-arg-rows increment 3b, `?(name=…)` on `val math ctx \derive`).
fn lower_command_params(
    params: &[cst_v1::Param],
) -> Result<Vec<cst::ast::Param>, LowerError> {
    params
        .iter()
        .map(|p| match &p.opts {
            None => Ok(cst::ast::Param::Pat(lower_param_body(&p.body)?)),
            Some(opts) => Ok(cst::ast::Param::Bundled {
                opts: lower_opt_binders(opts)?,
                body: lower_param_body(&p.body)?,
            }),
        })
        .collect()
}

/// Transcribe a `?(l = x, …)` parameter-binder bundle to its cst twin. An
/// empty bundle (`?()`) is a lowering error.
fn lower_opt_binders(opts: &ast_v1::OptParamsV1) -> Result<cst::ast::CstOptBinders, LowerError> {
    if opts.entries.is_empty() {
        return Err(unsupported(
            "an empty `?()` optional-parameter bundle",
            "a `?(…)` bundle must bind at least one label",
            opts.q.0,
        ));
    }
    Ok(cst::ast::CstOptBinders {
        q: opts.q.clone(),
        paren: clone_paren(&opts.paren),
        entries: opts
            .entries
            .iter()
            .map(|e| cst::ast::CstOptBinderEntry {
                label: e.label.clone(),
                eq: e.eq.clone(),
                var: e.var.clone(),
                comma: e.comma.clone(),
            })
            .collect(),
    })
}

/// Transcribe a `?(l = e, …)` application-argument bundle to its cst twin
/// (entry values are full expressions, lowered + erased). Empty (`?()`) is an
/// error.
fn lower_opt_args(opts: &ast_v1::OptArgsV1) -> Result<cst::ast::CstOptArgs, LowerError> {
    if opts.entries.is_empty() {
        return Err(unsupported(
            "an empty `?()` optional-argument bundle",
            "a `?(…)` bundle must supply at least one label",
            opts.q.0,
        ));
    }
    Ok(cst::ast::CstOptArgs {
        q: opts.q.clone(),
        paren: clone_paren(&opts.paren),
        entries: opts
            .entries
            .iter()
            .map(|e| {
                Ok(cst::ast::CstOptArgEntry {
                    label: e.label.clone(),
                    eq: e.eq.clone(),
                    value: erase_expr(lower_expr(&e.value.0)?),
                    comma: e.comma.clone(),
                })
            })
            .collect::<Result<_, LowerError>>()?,
    })
}

/// Clone an empty `ParenGroup<()>` marker (open/close tokens only). The
/// `#[group]`-payload slot is `()`, so this just carries the delimiter spans.
fn clone_paren(p: &ParenGroup<()>) -> ParenGroup<()> {
    ParenGroup {
        open: p.open.clone(),
        slot: (),
        close: p.close.clone(),
    }
}

// ---- `val math` (math-split spec §4.3): the target — the EXISTING
// `cst::TopBinding::LetMath` — is deliberately NOT extended with ctx/scripts
// fields (a back-compat break on the 0.0.6 grammar the same derive parses);
// instead the ctx/sub/sup binders are synthesized directly into the bind's
// own VALUE as ordinary `cst::ast::Expr::Fun`/application nodes, reusing the
// bind's own spans. The synthesis helpers below (`var_atomic`/`atom_expr`/
// `paren_atomic`/`apply_chain`/`fun1`) build exactly the small slice of
// `cst::ast::Expr` shapes this needs — a bare variable reference, a
// parenthesized sub-expression, an application chain, and a one-parameter
// lambda — by hand, since this is the one place `v1/lower.rs` needs to
// PRODUCE `cst::ast::Expr` nodes rather than transcribe them from a parsed
// `cst_v1::ast::Expr`. ----

fn var_tok(name: &str, span: Span) -> VarTok {
    VarTok {
        name: name.to_string(),
        span,
    }
}

fn var_atomic(name: &str, span: Span) -> cst::ast::Atomic {
    cst::ast::Atomic::Var(var_tok(name, span))
}

/// `( expr )` as an `Atomic::Paren` — needed to embed an arbitrary `Expr`
/// (the `val math` bind's own body) as ONE application argument, since
/// `AppArg::Atom.atom: Atomic` can't hold a full `Expr` directly.
fn paren_atomic(expr: cst::ast::Expr, span: Span) -> cst::ast::Atomic {
    cst::ast::Atomic::Paren {
        paren: ParenGroup {
            open: LParenTok(span),
            slot: (),
            close: RParenTok(span),
        },
        inner: Box::new(cst::ast::ParenBody {
            first: cst::ExprErased(Box::new(expr)),
            rest: Vec::new(),
        }),
    }
}

/// `head arg1 arg2 …` as one `Expr::Ops(OpChain)` node — a curried
/// application chain with `args.len()` atomic arguments, one `AppExpr`.
fn apply_chain(head: cst::ast::Atomic, args: Vec<cst::ast::Atomic>) -> cst::ast::Expr {
    let app_args = args
        .into_iter()
        .map(|atom| cst::ast::AppArg::Atom {
            excl: None,
            atom,
            accesses: Vec::new(),
        })
        .collect();
    cst::ast::Expr::Ops(cst::ast::OpChain {
        head: cst::ast::AppExpr {
            minus: None,
            excl: None,
            head,
            head_accesses: Vec::new(),
            args: app_args,
        },
        tail: Vec::new(),
        before: None,
    })
}

/// `fun <param> -> <body>` — a one-parameter lambda.
fn fun1(param_name: &str, span: Span, body: cst::ast::Expr) -> cst::ast::Expr {
    cst::ast::Expr::Fun {
        kw: KwFun(span),
        params: vec![cst::ast::PatBot::Var(var_tok(param_name, span))],
        arrow: ArrowTok(span),
        body: Box::new(body),
    }
}

/// Lower one `Bind::ValueMath` (math-split spec §4.1/§4.2/§4.3): `val math
/// <ctx> \cmd <param>* [with <sub> <sup>] = <body>`. Target: the existing
/// `cst::TopBinding::LetMath` — `elaborate_let_math` (unchanged) curries the
/// user's own `params` around the synthesized `fun ctx -> fun sub -> fun sup
/// -> …` chain built here and emits `Ast::LetMathIn`; `Checker::infer_
/// binding`'s `LetMath` arm (`typecheck.rs`) then branches on `self.version`
/// to apply `math_command_scheme_v01` instead of the 0.0.6 rule — no
/// elaborate/eval edit needed at all.
///
/// - **with `with sub sup`**: the user's own binders become the `sub`/`sup`
///   lambda parameters directly; the body is used as written (upstream
///   WithScripts: closure run with ctx, body's result returned raw).
/// - **without**: hidden binders `%sub`/`%sup` (unlexable — `%` starts a
///   comment, so unreachable from real source, the same trick upstream's own
///   `"%context"` uses), and the body wrapped as `%math-attach-scripts <ctx>
///   (<body>) %sub %sup` (upstream Simple: closure run with ctx, then
///   scripts appended under `enter_script`).
fn lower_value_math(
    kw: &KwVal,
    ctx: &VarTok,
    cmd: &AnyHorzCmdTok,
    params: &[cst_v1::Param],
    scripts: &Option<cst_v1::ScriptsParamV1>,
    eq: &DefEqTok,
    body: &ast_v1::Expr,
) -> Result<cst::TopBinding, LowerError> {
    // Reject a `?(l = x, …)` bundle on a `val math` parameter BEFORE it ever
    // reaches `lower_command_params` (which — optional-arg-rows increment
    // 3a — freely accepts one for `ValueInline`/`ValueBlock`): math command
    // parameter bundles (`val math ctx \derive ?(name = …) …`) are optional-
    // arg-rows increment 3b (needs `math_command_scheme_v01`'s own row
    // harvest, not yet wired — `typecheck.rs`'s doc comment on that fn).
    if let Some(p) = params.iter().find(|p| p.opts.is_some()) {
        return Err(unsupported(
            "a `?(l = e)` labeled optional on a `val math` command parameter",
            "math command parameter bundles need `math_command_scheme_v01`'s \
             own optional-label harvest — roadmap increment 3b",
            p.opts.as_ref().unwrap().q.0,
        ));
    }
    let body = lower_expr(body)?;
    let span = eq.0;
    let (sub_name, sup_name, wrapped_body) = match scripts {
        Some(sp) => (sp.sub.name.clone(), sp.sup.name.clone(), body),
        None => (
            "%sub".to_string(),
            "%sup".to_string(),
            apply_chain(
                var_atomic("%math-attach-scripts", span),
                vec![
                    var_atomic(&ctx.name, ctx.span),
                    paren_atomic(body, span),
                    var_atomic("%sub", span),
                    var_atomic("%sup", span),
                ],
            ),
        ),
    };
    let value = fun1(
        &ctx.name,
        ctx.span,
        fun1(&sub_name, span, fun1(&sup_name, span, wrapped_body)),
    );
    Ok(cst::TopBinding::LetMath {
        kw: KwLetMath(kw.0),
        cmd: plain_horz(cmd)?,
        params: lower_command_params(params)?,
        eq: eq.clone(),
        value,
    })
}

// ---- Expr ---------------------------------------------------------------

fn lower_expr(e: &ast_v1::Expr) -> Result<cst::ast::Expr, LowerError> {
    match e {
        ast_v1::Expr::LetRecIn {
            let_kw, first, ands, in_kw, body, ..
        } => Ok(cst::ast::Expr::LetRecIn {
            // Span-lossy on `rec` (synthetic tree, never unparsed).
            kw: KwLetRec(let_kw.0),
            first: lower_rec_clause(first)?,
            ands: ands
                .iter()
                .map(|a| {
                    Ok(cst::ast::AndBinding {
                        and_kw: a.and_kw.clone(),
                        binding: lower_rec_clause(&a.clause)?,
                    })
                })
                .collect::<Result<_, LowerError>>()?,
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::LetMutableIn {
            let_kw,
            name,
            arrow,
            init,
            in_kw,
            body,
            ..
        } => Ok(cst::ast::Expr::LetMutableIn {
            kw: KwLetMutable(let_kw.0),
            name: name.clone(),
            arrow: arrow.clone(),
            init: Box::new(lower_expr(init)?),
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::LetIn {
            kw,
            name,
            params,
            eq,
            value,
            in_kw,
            body,
        } => {
            let (ps, value_expr) = lower_param_units(params, lower_expr(value)?)?;
            Ok(cst::ast::Expr::LetIn {
                kw: kw.clone(),
                name: name.clone(),
                params: ps,
                eq: eq.clone(),
                value: Box::new(value_expr),
                in_kw: in_kw.clone(),
                body: Box::new(lower_expr(body)?),
            })
        }
        ast_v1::Expr::LetPatternIn {
            kw,
            pat,
            eq,
            value,
            in_kw,
            body,
        } => Ok(cst::ast::Expr::LetPatternIn {
            kw: kw.clone(),
            pat: erase_pat(lower_pattern(pat)?),
            eq: eq.clone(),
            value: Box::new(lower_expr(value)?),
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::OpenIn {
            open_kw,
            name,
            in_kw,
            body,
            ..
        } => Ok(cst::ast::Expr::OpenIn {
            kw: open_kw.clone(),
            name: name.clone(),
            in_kw: in_kw.clone(),
            body: Box::new(lower_expr(body)?),
        }),
        ast_v1::Expr::If {
            kw,
            cond,
            then_kw,
            then_branch,
            else_kw,
            else_branch,
        } => Ok(cst::ast::Expr::If {
            kw: kw.clone(),
            cond: Box::new(lower_expr(cond)?),
            then_kw: then_kw.clone(),
            then_branch: Box::new(lower_expr(then_branch)?),
            else_kw: else_kw.clone(),
            else_branch: Box::new(lower_expr(else_branch)?),
        }),
        ast_v1::Expr::Fun {
            kw,
            params,
            arrow,
            body,
        } => {
            let body_expr = lower_expr(body)?;
            if params.iter().all(|p| p.opts.is_none()) {
                Ok(cst::ast::Expr::Fun {
                    kw: kw.clone(),
                    params: params
                        .iter()
                        .map(|p| lower_param_body(&p.body))
                        .collect::<Result<_, _>>()?,
                    arrow: arrow.clone(),
                    body: Box::new(body_expr),
                })
            } else {
                // Any `?(l = x, …)` bundle: the whole `fun` desugars to a
                // nested `FunRows`/`Fun` lambda chain (returned directly as
                // the lambda expression).
                let (_, chain) = lower_param_units(params, body_expr)?;
                Ok(chain)
            }
        }
        ast_v1::Expr::Match {
            kw,
            scrutinee,
            with_kw,
            leading_bar,
            first,
            rest,
            ..
        } => Ok(cst::ast::Expr::Match {
            kw: kw.clone(),
            scrutinee: Box::new(lower_expr(scrutinee)?),
            with_kw: with_kw.clone(),
            leading_bar: leading_bar.clone(),
            first: lower_match_arm(first)?,
            rest: rest.iter().map(lower_bar_arm).collect::<Result<_, _>>()?,
        }),
        ast_v1::Expr::Overwrite { name, arrow, value } => Ok(cst::ast::Expr::Overwrite {
            name: name.clone(),
            arrow: arrow.clone(),
            value: erase_expr(lower_expr(value)?),
        }),
        ast_v1::Expr::Ops(chain) => Ok(cst::ast::Expr::Ops(lower_op_chain(chain)?)),
    }
}

fn lower_match_arm(a: &ast_v1::MatchArm) -> Result<cst::ast::MatchArm, LowerError> {
    Ok(cst::ast::MatchArm {
        pat: erase_pat(lower_pattern(&a.pat)?),
        guard: None,
        arrow: a.arrow.clone(),
        body: erase_expr(lower_expr(&a.body)?),
    })
}

fn lower_bar_arm(a: &ast_v1::BarArm) -> Result<cst::ast::BarArm, LowerError> {
    Ok(cst::ast::BarArm {
        bar: a.bar.clone(),
        arm: lower_match_arm(&a.arm)?,
    })
}

fn lower_op_chain(c: &ast_v1::OpChain) -> Result<cst::ast::OpChain, LowerError> {
    Ok(cst::ast::OpChain {
        head: lower_app_expr(&c.head)?,
        tail: c.tail.iter().map(lower_op_rhs).collect::<Result<_, _>>()?,
        before: None,
    })
}

fn lower_op_rhs(r: &ast_v1::OpRhs) -> Result<cst::ast::OpRhs, LowerError> {
    Ok(cst::ast::OpRhs {
        op: r.op.clone(),
        rhs: lower_app_expr(&r.rhs)?,
    })
}

fn lower_app_expr(e: &ast_v1::AppExpr) -> Result<cst::ast::AppExpr, LowerError> {
    Ok(cst::ast::AppExpr {
        minus: e.minus.clone(),
        excl: e.excl.clone(),
        head: lower_atomic(&e.head)?,
        head_accesses: e.head_accesses.iter().map(lower_access_seg).collect(),
        args: e.args.iter().map(lower_app_arg).collect::<Result<_, _>>()?,
    })
}

fn lower_access_seg(a: &ast_v1::AccessSeg) -> cst::ast::AccessSeg {
    cst::ast::AccessSeg {
        hash: a.hash.clone(),
        label: a.label.clone(),
    }
}

fn lower_app_arg(a: &ast_v1::AppArg) -> Result<cst::ast::AppArg, LowerError> {
    match a {
        // `?(l = e, …) atom` — a SATySFi 0.1 labeled-optional bundle paired
        // with its following positional argument.
        ast_v1::AppArg::Bundled {
            opts,
            excl,
            atom,
            accesses,
        } => Ok(cst::ast::AppArg::Bundled {
            opts: lower_opt_args(opts)?,
            excl: excl.clone(),
            atom: lower_atomic(atom)?,
            accesses: accesses.iter().map(lower_access_seg).collect(),
        }),
        ast_v1::AppArg::BundledCtor { opts, ctor } => Ok(cst::ast::AppArg::BundledCtor {
            opts: lower_opt_args(opts)?,
            ctor: ctor.clone(),
        }),
        ast_v1::AppArg::Atom {
            excl,
            atom,
            accesses,
        } => Ok(cst::ast::AppArg::Atom {
            excl: excl.clone(),
            atom: lower_atomic(atom)?,
            accesses: accesses.iter().map(lower_access_seg).collect(),
        }),
        ast_v1::AppArg::Ctor(t) => Ok(cst::ast::AppArg::Ctor(t.clone())),
    }
}

fn lower_atomic(a: &ast_v1::Atomic) -> Result<cst::ast::Atomic, LowerError> {
    match a {
        ast_v1::Atomic::Length(t) => Ok(cst::ast::Atomic::Length(t.clone())),
        ast_v1::Atomic::Float(t) => Ok(cst::ast::Atomic::Float(t.clone())),
        ast_v1::Atomic::Int(t) => Ok(cst::ast::Atomic::Int(t.clone())),
        ast_v1::Atomic::Literal(t) => Ok(cst::ast::Atomic::Literal(t.clone())),
        ast_v1::Atomic::True(t) => Ok(cst::ast::Atomic::True(t.clone())),
        ast_v1::Atomic::False(t) => Ok(cst::ast::Atomic::False(t.clone())),
        ast_v1::Atomic::Ctor(t) => Ok(cst::ast::Atomic::Ctor(t.clone())),
        ast_v1::Atomic::Var(t) => Ok(cst::ast::Atomic::Var(t.clone())),
        ast_v1::Atomic::VarWithMod(t) => Ok(cst::ast::Atomic::VarWithMod(t.clone())),
        ast_v1::Atomic::Command { kw, name } => Ok(cst::ast::Atomic::Command {
            kw: kw.clone(),
            name: name.clone(),
        }),
        ast_v1::Atomic::Unit { paren } => Ok(cst::ast::Atomic::Unit { paren: paren.clone() }),
        ast_v1::Atomic::Paren { paren, inner } => Ok(cst::ast::Atomic::Paren {
            paren: paren.clone(),
            inner: Box::new(lower_paren_body(inner)?),
        }),
        ast_v1::Atomic::Record { rec, body } => Ok(cst::ast::Atomic::Record {
            rec: rec.clone(),
            body: lower_record_body(body)?,
        }),
        ast_v1::Atomic::List { list, items } => Ok(cst::ast::Atomic::List {
            list: list.clone(),
            items: items.iter().map(lower_list_item).collect::<Result<_, _>>()?,
        }),
        ast_v1::Atomic::InlineText { igrp, elems } => Ok(cst::ast::Atomic::InlineText {
            igrp: igrp.clone(),
            elems: elems
                .iter()
                .map(lower_inline_elem)
                .collect::<Result<_, _>>()?,
        }),
        ast_v1::Atomic::BlockText { bgrp, elems } => Ok(cst::ast::Atomic::BlockText {
            bgrp: bgrp.clone(),
            elems: elems
                .iter()
                .map(lower_block_elem)
                .collect::<Result<_, _>>()?,
        }),
        // math-split spec §3.1: the `${…}`/`math`-elaboration split lives
        // version-independently in `elaborate.rs`'s SHARED math path — only
        // structural transcription is needed here, exactly like every other
        // `Atomic` arm above.
        ast_v1::Atomic::MathText { mgrp, elems } => Ok(cst::ast::Atomic::MathText {
            mgrp: mgrp.clone(),
            elems: lower_math_elems(elems)?,
        }),
    }
}

fn lower_record_body(b: &ast_v1::RecordBody) -> Result<cst::ast::RecordBody, LowerError> {
    match b {
        ast_v1::RecordBody::Update {
            base,
            with_kw,
            fields,
        } => Ok(cst::ast::RecordBody::Update {
            base: erase_expr(lower_expr(base)?),
            with_kw: with_kw.clone(),
            fields: fields.iter().map(lower_record_field).collect::<Result<_, _>>()?,
        }),
        ast_v1::RecordBody::Fields(fields) => Ok(cst::ast::RecordBody::Fields(
            fields.iter().map(lower_record_field).collect::<Result<_, _>>()?,
        )),
    }
}

fn lower_record_field(f: &ast_v1::RecordField) -> Result<cst::ast::RecordField, LowerError> {
    Ok(cst::ast::RecordField {
        name: f.name.clone(),
        eq: f.eq.clone(),
        value: erase_expr(lower_expr(&f.value)?),
        // The `,` separator is dropped (`semi: None`) — harmless: the
        // synthetic tree this module builds is never unparsed. See the
        // finale spec §11's "Separator-token loss" risk note.
        semi: None,
    })
}

fn lower_paren_body(b: &ast_v1::ParenBody) -> Result<cst::ast::ParenBody, LowerError> {
    Ok(cst::ast::ParenBody {
        first: erase_expr(lower_expr(&b.first)?),
        rest: b.rest.iter().map(lower_comma_expr).collect::<Result<_, _>>()?,
    })
}

fn lower_comma_expr(c: &ast_v1::CommaExpr) -> Result<cst::ast::CommaExpr, LowerError> {
    Ok(cst::ast::CommaExpr {
        comma: c.comma.clone(),
        value: erase_expr(lower_expr(&c.value)?),
    })
}

fn lower_list_item(i: &ast_v1::ListItem) -> Result<cst::ast::ListItem, LowerError> {
    Ok(cst::ast::ListItem {
        value: erase_expr(lower_expr(&i.value)?),
        semi: None,
    })
}

fn lower_inline_elem(e: &ast_v1::InlineElem) -> Result<cst::ast::InlineElem, LowerError> {
    match e {
        ast_v1::InlineElem::Char(t) => Ok(cst::ast::InlineElem::Char(t.clone())),
        ast_v1::InlineElem::Space(t) => Ok(cst::ast::InlineElem::Space(t.clone())),
        ast_v1::InlineElem::Break(t) => Ok(cst::ast::InlineElem::Break(t.clone())),
        ast_v1::InlineElem::Embed { var, semi } => Ok(cst::ast::InlineElem::Embed {
            var: var.clone(),
            semi: semi.clone(),
        }),
        // math-split spec §3.1: mechanical transcription, mirror of
        // `Atomic::MathText` above.
        ast_v1::InlineElem::EmbedMath { mgrp, elems } => Ok(cst::ast::InlineElem::EmbedMath {
            mgrp: mgrp.clone(),
            elems: lower_math_elems(elems)?,
        }),
        ast_v1::InlineElem::Cmd { name, tail } => Ok(cst::ast::InlineElem::Cmd {
            name: name.clone(),
            tail: lower_cmd_tail(tail)?,
        }),
        ast_v1::InlineElem::ItemBullet(t) => Ok(cst::ast::InlineElem::ItemBullet(t.clone())),
        ast_v1::InlineElem::Sep(t) => Ok(cst::ast::InlineElem::Sep(t.clone())),
    }
}

fn lower_block_elem(e: &ast_v1::BlockElem) -> Result<cst::ast::BlockElem, LowerError> {
    match e {
        ast_v1::BlockElem::Embed { var, semi } => Ok(cst::ast::BlockElem::Embed {
            var: var.clone(),
            semi: semi.clone(),
        }),
        ast_v1::BlockElem::Cmd { name, tail } => Ok(cst::ast::BlockElem::Cmd {
            name: name.clone(),
            tail: lower_cmd_tail(tail)?,
        }),
    }
}

/// The `CmdTail` bridge (§3.3 of the finale spec) — the one non-1:1
/// transcription. `cst_v1` kept the OLD "one application-chain `Expr`"
/// argument encoding (`Args { args: ExprErasedV1, semi }`), while `cst.rs`
/// has since moved to the flat `AppArg` list (`Args { first, rest, semi }`).
/// Semantics-preserving by construction: `cst_v1` parses `\cmd{a}{b}` as
/// `AppExpr { head: {a}, args: [{b}] }`, and `cst` parses the same surface
/// as `first = {a}, rest = [{b}]` — this bridge maps one onto the other
/// exactly. The two error arms are unreachable from token streams the
/// `cst_v1` grammar can actually produce in command-tail position (a bare
/// application chain with no operator/negation) — `LowerError` (not
/// `unreachable!()`) keeps a future grammar-drift bug user-visible rather
/// than silently mis-nesting arguments.
fn lower_cmd_tail(t: &ast_v1::CmdTail) -> Result<cst::ast::CmdTail, LowerError> {
    match t {
        ast_v1::CmdTail::Semi(s) => Ok(cst::ast::CmdTail::Semi(s.clone())),
        ast_v1::CmdTail::Args { args, semi } => {
            let ast_v1::Expr::Ops(chain) = &*args.0 else {
                return Err(unsupported(
                    "command arguments that are not a plain application chain",
                    "grammar-drift guard — the cst_v1 grammar cannot actually \
                     produce this shape in command-tail position",
                    Span::default(),
                ));
            };
            if !chain.tail.is_empty() || chain.head.minus.is_some() {
                return Err(unsupported(
                    "an operator or unary negation inside a command argument chain",
                    "grammar-drift guard — the cst_v1 grammar cannot actually \
                     produce this shape in command-tail position",
                    Span::default(),
                ));
            }
            let a = &chain.head;
            let first = cst::AppArgErased(Box::new(cst::ast::AppArg::Atom {
                excl: a.excl.clone(),
                atom: lower_atomic(&a.head)?,
                accesses: a.head_accesses.iter().map(lower_access_seg).collect(),
            }));
            let rest = a
                .args
                .iter()
                .map(|arg| Ok(cst::AppArgErased(Box::new(lower_app_arg(arg)?))))
                .collect::<Result<Vec<_>, LowerError>>()?;
            Ok(cst::ast::CmdTail::Args {
                first,
                rest,
                semi: semi.clone(),
            })
        }
    }
}

// ---- Math (math-split spec §3.1) -----------------------------------------
//
// The cst_v1 math layer is declared shape-identical to `crate::cst::ast`'s
// (cst_v1.rs's own module doc comment: "no 0.1 delta"), so every function
// below is a field-wise transcription, exactly like the rest of this
// module — the ONE non-mechanical seam is `lower_math_arg`'s bridge from
// cst_v1's flat 6-variant `MathArg` (no `?:`/`?*` forms at all — 0.1's
// optional math-command arguments are `?(l = e)` labeled bundles, a
// grammar production this port's `MathArg` node doesn't parse yet, phase 4)
// onto cst.rs's two-level `MathArg::Plain(MathArgBody::…)` shape.

fn lower_math_elems(elems: &[cst_v1::MathErasedV1]) -> Result<Vec<cst::MathErased>, LowerError> {
    elems
        .iter()
        .map(|e| Ok(cst::MathErased(Box::new(lower_math_elem_cst(e)?))))
        .collect()
}

fn lower_math_elem_cst(m: &ast_v1::MathElemCst) -> Result<cst::ast::MathElemCst, LowerError> {
    Ok(cst::ast::MathElemCst {
        base: lower_math_bot(&m.base)?,
        scripts: m
            .scripts
            .iter()
            .map(lower_math_script)
            .collect::<Result<_, _>>()?,
    })
}

fn lower_math_bot(b: &ast_v1::MathBot) -> Result<cst::ast::MathBot, LowerError> {
    Ok(match b {
        ast_v1::MathBot::Cmd { name, args } => cst::ast::MathBot::Cmd {
            // `ast_v1::MathBot::Cmd.name` is ALREADY `AnyMathCmdTok`
            // (math-package completion M4 fix — cst_v1.rs's grammar used to
            // spell this `MathCmdTok` (sigil-only), which silently
            // couldn't parse a `\Mod.cmd` qualified math command at all,
            // even though the shared lexer always emitted
            // `Token::MathCmdWithMod` for one; both cst.rs's 0.0.6 `MathBot`
            // and this node now share the exact same tag), so no `Plain`
            // wrapping is needed here — just carry it through.
            name: name.clone(),
            args: args.iter().map(lower_math_arg).collect::<Result<_, _>>()?,
        },
        ast_v1::MathBot::Chars(t) => cst::ast::MathBot::Chars(t.clone()),
        ast_v1::MathBot::Embed(t) => cst::ast::MathBot::Embed(t.clone()),
        ast_v1::MathBot::Sep(t) => cst::ast::MathBot::Sep(t.clone()),
        ast_v1::MathBot::Group { mgrp, elems } => cst::ast::MathBot::Group {
            mgrp: mgrp.clone(),
            elems: lower_math_elems(elems)?,
        },
    })
}

fn lower_math_script(s: &ast_v1::MathScript) -> Result<cst::ast::MathScript, LowerError> {
    Ok(match s {
        ast_v1::MathScript::Super { hat, group } => cst::ast::MathScript::Super {
            hat: hat.clone(),
            group: lower_math_group_arg(group)?,
        },
        ast_v1::MathScript::Sub { under, group } => cst::ast::MathScript::Sub {
            under: under.clone(),
            group: lower_math_group_arg(group)?,
        },
        ast_v1::MathScript::Primes(t) => cst::ast::MathScript::Primes(t.clone()),
    })
}

fn lower_math_group_arg(g: &ast_v1::MathGroupArg) -> Result<cst::ast::MathGroupArg, LowerError> {
    Ok(match g {
        ast_v1::MathGroupArg::Group { mgrp, elems } => cst::ast::MathGroupArg::Group {
            mgrp: mgrp.clone(),
            elems: lower_math_elems(elems)?,
        },
        ast_v1::MathGroupArg::Bot(b) => {
            cst::ast::MathGroupArg::Bot(Box::new(lower_math_bot(b)?))
        }
    })
}

fn lower_math_arg(a: &ast_v1::MathArg) -> Result<cst::ast::MathArg, LowerError> {
    Ok(cst::ast::MathArg::Plain(match a {
        ast_v1::MathArg::Math { mgrp, elems } => cst::ast::MathArgBody::Math {
            mgrp: mgrp.clone(),
            elems: lower_math_elems(elems)?,
        },
        ast_v1::MathArg::Inline { igrp, elems } => cst::ast::MathArgBody::Inline {
            igrp: igrp.clone(),
            elems: elems
                .iter()
                .map(lower_inline_elem)
                .collect::<Result<_, _>>()?,
        },
        ast_v1::MathArg::Block { bgrp, elems } => cst::ast::MathArgBody::Block {
            bgrp: bgrp.clone(),
            elems: elems
                .iter()
                .map(lower_block_elem)
                .collect::<Result<_, _>>()?,
        },
        ast_v1::MathArg::ParenEscape { paren, inner } => cst::ast::MathArgBody::ParenEscape {
            paren: paren.clone(),
            inner: Box::new(lower_paren_body(inner)?),
        },
        ast_v1::MathArg::ListEscape { list, items } => cst::ast::MathArgBody::ListEscape {
            list: list.clone(),
            items: items.iter().map(lower_list_item).collect::<Result<_, _>>()?,
        },
        ast_v1::MathArg::RecordEscape { rec, body } => cst::ast::MathArgBody::RecordEscape {
            rec: rec.clone(),
            body: lower_record_body(body)?,
        },
    }))
}

// ---- Pattern layer --------------------------------------------------------

fn lower_pattern(p: &ast_v1::Pattern) -> Result<cst::ast::Pattern, LowerError> {
    Ok(cst::ast::Pattern {
        head: lower_pat_cons(&p.head)?,
        as_clause: p.as_clause.as_ref().map(lower_as_clause),
    })
}

fn lower_as_clause(a: &ast_v1::AsClause) -> cst::ast::AsClause {
    cst::ast::AsClause {
        as_kw: a.as_kw.clone(),
        name: a.name.clone(),
    }
}

fn lower_pat_cons(c: &ast_v1::PatCons) -> Result<cst::ast::PatCons, LowerError> {
    Ok(cst::ast::PatCons {
        head: lower_pat_bot(&c.head)?,
        tail: c.tail.iter().map(lower_cons_seg).collect::<Result<_, _>>()?,
    })
}

fn lower_cons_seg(s: &ast_v1::ConsSeg) -> Result<cst::ast::ConsSeg, LowerError> {
    Ok(cst::ast::ConsSeg {
        cons: s.cons.clone(),
        tail: lower_pat_bot(&s.tail)?,
    })
}

fn lower_pat_bot(p: &ast_v1::PatBot) -> Result<cst::ast::PatBot, LowerError> {
    match p {
        ast_v1::PatBot::CtorApplied { ctor, arg } => Ok(cst::ast::PatBot::CtorApplied {
            ctor: ctor.clone(),
            arg: Box::new(lower_pat_bot(arg)?),
        }),
        ast_v1::PatBot::Ctor(t) => Ok(cst::ast::PatBot::Ctor(t.clone())),
        ast_v1::PatBot::Int(t) => Ok(cst::ast::PatBot::Int(t.clone())),
        ast_v1::PatBot::True(t) => Ok(cst::ast::PatBot::True(t.clone())),
        ast_v1::PatBot::False(t) => Ok(cst::ast::PatBot::False(t.clone())),
        ast_v1::PatBot::Str(t) => Ok(cst::ast::PatBot::Str(t.clone())),
        ast_v1::PatBot::Wild(t) => Ok(cst::ast::PatBot::Wild(t.clone())),
        ast_v1::PatBot::Var(t) => Ok(cst::ast::PatBot::Var(t.clone())),
        ast_v1::PatBot::Unit { paren } => Ok(cst::ast::PatBot::Unit { paren: paren.clone() }),
        ast_v1::PatBot::Paren { paren, inner } => Ok(cst::ast::PatBot::Paren {
            paren: paren.clone(),
            inner: Box::new(lower_pattern_paren_body(inner)?),
        }),
        ast_v1::PatBot::List { plist, items } => Ok(cst::ast::PatBot::List {
            plist: plist.clone(),
            items: items
                .iter()
                .map(lower_pat_list_item)
                .collect::<Result<_, _>>()?,
        }),
    }
}

/// A [`ast_v1::Param`]'s trailing shape (optional-arg-rows increment 2):
/// either a plain `patbot` (unchanged path), or a `( pattern : typ )`
/// ascribed pattern. The ascription's `typ` is DROPPED — a documented
/// carve-out, precedent `cst::ast::RecBinding.ascription`'s own
/// parse-and-ignore (`cst.rs:729-737`; enforcing it needs an `Ast`-level
/// ascription node, a typechecker-completion follow-up, not this increment).
/// Once dropped, `( pat : typ )` reduces exactly to a trivially-parenthesized
/// FULL pattern — precisely [`cst::ast::PatBot::Paren`]'s own shape (a single
/// `first` pattern, no `rest`), since the ascribed form's parens were already
/// there in the source.
fn lower_param_body(pb: &ast_v1::ParamBody) -> Result<cst::ast::PatBot, LowerError> {
    match pb {
        ast_v1::ParamBody::Pat(p) => lower_pat_bot(p),
        ast_v1::ParamBody::Ascribed { paren, inner } => Ok(cst::ast::PatBot::Paren {
            paren: paren.clone(),
            inner: Box::new(cst::ast::PatternParenBody {
                first: erase_pat(lower_pattern(&inner.pat)?),
                rest: Vec::new(),
            }),
        }),
    }
}

fn lower_pattern_paren_body(
    b: &ast_v1::PatternParenBody,
) -> Result<cst::ast::PatternParenBody, LowerError> {
    Ok(cst::ast::PatternParenBody {
        first: erase_pat(lower_pattern(&b.first)?),
        rest: b
            .rest
            .iter()
            .map(lower_comma_pattern)
            .collect::<Result<_, _>>()?,
    })
}

fn lower_comma_pattern(c: &ast_v1::CommaPattern) -> Result<cst::ast::CommaPattern, LowerError> {
    Ok(cst::ast::CommaPattern {
        comma: c.comma.clone(),
        value: erase_pat(lower_pattern(&c.value)?),
    })
}

fn lower_pat_list_item(i: &ast_v1::PatListItem) -> Result<cst::ast::PatListItem, LowerError> {
    Ok(cst::ast::PatListItem {
        value: erase_pat(lower_pattern(&i.value)?),
        semi: None,
    })
}

// ---- erasure helpers --------------------------------------------------

fn erase_expr(e: cst::ast::Expr) -> cst::ExprErased {
    cst::ExprErased(Box::new(e))
}

fn erase_pat(p: cst::ast::Pattern) -> cst::PatErased {
    cst::PatErased(Box::new(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_v1(src: &str) -> cst_v1::FileV1 {
        satysfi_syntax::parse_file_v1(src).unwrap_or_else(|e| panic!("v1 parse failed: {e}"))
    }

    /// §6.2: expression-level `let rec … and … in` lowers to a full
    /// `Expr::LetRecIn` (Sub-slice 2b retires the old single-clause
    /// `LowerError`) with the `RecBinding` reshape's 0.1-only fields all
    /// empty/`None` (no ascription/leading-bar/multi-clause sugar exists in
    /// 0.1's `bind_value_nonrec`).
    #[test]
    fn let_rec_document_lowers_with_and_chain() {
        let file = parse_v1(
            "let rec even n = if n <= 0 then true else odd (n - 1)\n\
             and odd n = if n <= 0 then false else even (n - 1) in even 4",
        );
        let ast = lower_document_v1(&file).unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
        let cst::ast::Expr::LetRecIn { first, ands, .. } = ast else {
            panic!("expected Expr::LetRecIn");
        };
        assert_eq!(ands.len(), 1, "one `and` continuation");
        assert!(first.ascription.is_none());
        assert!(first.leading_bar.is_none());
        assert!(first.extra.is_empty());
        assert_eq!(first.name.name, "even");
        assert_eq!(ands[0].binding.name.name, "odd");
    }

    /// §6.2: `val rec … and …` lowers to `TopBinding::LetRec` inside the
    /// module's `decls`.
    #[test]
    fn val_rec_library_lowers_to_top_binding_letrec() {
        let file = parse_v1(
            "module M = struct\n\
             val rec even n = odd n\n\
             and odd n = even n\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(decls.len(), 1);
        let cst::TopBinding::LetRec { first, ands, .. } = &*decls[0].0 else {
            panic!("expected TopBinding::LetRec, got {:?}", decls[0].0);
        };
        assert_eq!(first.name.name, "even");
        assert_eq!(ands.len(), 1);
        assert_eq!(ands[0].binding.name.name, "odd");
    }

    /// §6.2: `val mutable` → `TopBinding::LetMutable`.
    #[test]
    fn val_mutable_lowers_to_top_binding_letmutable() {
        let file = parse_v1("module M = struct\nval mutable c <- 0\nend");
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(decls.len(), 1);
        assert!(
            matches!(&*decls[0].0, cst::TopBinding::LetMutable { name, .. } if name.name == "c"),
            "{:?}",
            decls[0].0
        );
    }

    /// §6.2: `let mutable … in` → `Expr::LetMutableIn`.
    #[test]
    fn let_mutable_in_document_lowers_to_expr_letmutablein() {
        let file = parse_v1("let mutable c <- 0 in c <- !c + 1");
        let ast = lower_document_v1(&file).unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
        assert!(
            matches!(&ast, cst::ast::Expr::LetMutableIn { name, .. } if name.name == "c"),
            "{ast:?}"
        );
    }

    /// §6.2 / §4: `type t = int and u = t` inside `module M` lowers to TWO
    /// consecutive `TopBinding::Type` decls, both names qualified
    /// (`"M.t"`/`"M.u"`), and `u`'s synonym body references the ALREADY-
    /// qualified `"M.t"` — the pre-qualification pin.
    #[test]
    fn type_and_chain_inside_module_qualifies_names_and_synonym_reference() {
        let file = parse_v1(
            "module M = struct\n\
             type t = int\n\
             and u = t\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(decls.len(), 2, "an `and`-chain lowers to N consecutive Type decls");
        let cst::TopBinding::Type(t_decl) = &*decls[0].0 else {
            panic!("expected decls[0] to be a Type decl");
        };
        assert_eq!(t_decl.name.name, "M.t");
        let cst::TopBinding::Type(u_decl) = &*decls[1].0 else {
            panic!("expected decls[1] to be a Type decl");
        };
        assert_eq!(u_decl.name.name, "M.u");
        let cst::TypeDeclBody::Synonym(ty) = &u_decl.body else {
            panic!("expected a synonym body");
        };
        let cst::ast::TypeExpr::Atom(prod) = ty else {
            panic!("expected a bare TypeProd (no arrow)");
        };
        let cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Name(n)) = &prod.first else {
            panic!("expected a bare type name atom");
        };
        assert_eq!(n.name, "M.t", "u's synonym body must reference the QUALIFIED t");
    }

    /// §6.2 / §4: nested-module pre-qualification — `module M = struct type
    /// t = int module N = struct type u = t end end` → `"M.N.u"`'s body
    /// references `"M.t"` (outer types stay visible inside a nested
    /// module, per ordinary module scoping).
    #[test]
    fn nested_module_type_reference_qualifies_to_outer_path() {
        let file = parse_v1(
            "module M = struct\n\
             type t = int\n\
             module N = struct\n\
             type u = t\n\
             end\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(decls.len(), 2);
        let cst::TopBinding::Module {
            name: inner_name,
            decls: inner_decls,
            ..
        } = &*decls[1].0
        else {
            panic!("expected decls[1] to be a nested TopBinding::Module");
        };
        assert_eq!(inner_name.name, "N");
        assert_eq!(inner_decls.len(), 1);
        let cst::TopBinding::Type(u_decl) = &*inner_decls[0].0 else {
            panic!("expected a Type decl");
        };
        assert_eq!(u_decl.name.name, "M.N.u");
        let cst::TypeDeclBody::Synonym(ty) = &u_decl.body else {
            panic!("expected a synonym body");
        };
        let cst::ast::TypeExpr::Atom(prod) = ty else {
            panic!("expected a bare TypeProd");
        };
        let cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Name(n)) = &prod.first else {
            panic!("expected a bare type name atom");
        };
        assert_eq!(n.name, "M.t", "the outer M.t must stay visible/qualified inside N");
    }

    /// §3.4/§6.2: the prefix→postfix `TypeApp` bridge — `type t = option
    /// int` (0.1 prefix, arity 1) lowers to the cst target's postfix
    /// `Applied { arg: int, ctor: option }` shape.
    #[test]
    fn type_app_prefix_to_postfix_bridge() {
        let file = parse_v1("module M = struct\ntype t = option int\nend");
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        let cst::TopBinding::Type(t_decl) = &*decls[0].0 else {
            panic!("expected a Type decl");
        };
        let cst::TypeDeclBody::Synonym(ty) = &t_decl.body else {
            panic!("expected a synonym body");
        };
        let cst::ast::TypeExpr::Atom(prod) = ty else {
            panic!("expected a bare TypeProd");
        };
        let cst::ast::TypeApp::Applied { arg, ctor } = &prod.first else {
            panic!("expected TypeApp::Applied, got {:?}", prod.first);
        };
        assert_eq!(ctor.name, "option");
        assert!(matches!(arg, cst::ast::TypeAtom::Name(n) if n.name == "int"), "{arg:?}");
    }

    /// §3.4/§6.2/§8: an applied type constructor with arity ≥ 2 (`pair int
    /// int`) is a real `LowerError`, not a panic — the cst target
    /// (`cst::ast::TypeApp`) is single-argument by design.
    #[test]
    fn type_app_arity_2_is_a_lower_error() {
        let file = parse_v1("module M = struct\ntype t = pair int int\nend");
        let err = lower_file_v1(&file).unwrap_err();
        assert!(err.to_string().contains("more than one argument"), "{err}");
    }

    /// §9 risk 5: a variant↔variant mutual pair (`type a = A of b and b = B
    /// of a`) lowers to two consecutive `TopBinding::Type` decls — the
    /// forward-reference tolerance `typecheck.rs` provides is exercised at
    /// the typecheck/elaborate layer, not here; this only pins the
    /// lowering shape.
    #[test]
    fn mutual_variant_pair_lowers_to_two_type_decls() {
        let file = parse_v1(
            "module M = struct\n\
             type a = A of b\n\
             and b = B of a\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(decls.len(), 2);
        assert!(matches!(&*decls[0].0, cst::TopBinding::Type(d) if d.name.name == "M.a"));
        assert!(matches!(&*decls[1].0, cst::TopBinding::Type(d) if d.name.name == "M.b"));
    }

    /// G2: `type t = (| x : int, y : bool |)` lowers to `cst::ast::
    /// TypeAtom::Record` with the field list transcribed field-by-field
    /// (labels, colons, and lowered field types) — the pure-transcription
    /// arm in `lower_type_atom`.
    #[test]
    fn type_record_lowers_to_cst_record_atom() {
        let file = parse_v1("module M = struct\ntype t = (| x : int, y : bool |)\nend");
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        let cst::TopBinding::Type(t_decl) = &*decls[0].0 else {
            panic!("expected a Type decl");
        };
        let cst::TypeDeclBody::Synonym(ty) = &t_decl.body else {
            panic!("expected a synonym body");
        };
        let cst::ast::TypeExpr::Atom(prod) = ty else {
            panic!("expected a bare TypeProd");
        };
        let cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Record { fields, .. }) = &prod.first else {
            panic!("expected TypeAtom::Record, got {:?}", prod.first);
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name.name, "x");
        assert!(
            matches!(&*fields[0].ty.0, cst::ast::TypeExpr::Atom(p)
                if matches!(&p.first, cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Name(n)) if n.name == "int")),
            "{:?}",
            fields[0].ty.0
        );
        assert_eq!(fields[1].name.name, "y");
        assert!(
            matches!(&*fields[1].ty.0, cst::ast::TypeExpr::Atom(p)
                if matches!(&p.first, cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Name(n)) if n.name == "bool")),
            "{:?}",
            fields[1].ty.0
        );
    }

    /// A bare local type name used INSIDE a record type field gets the
    /// same `tyenv.qualify` every other type position gets (mirrors
    /// `type_and_chain_inside_module_qualifies_names_and_synonym_reference`
    /// above, but for a field type rather than a synonym body).
    #[test]
    fn type_record_field_type_is_qualified_like_any_other_type_position() {
        let file = parse_v1(
            "module M = struct\n\
             type config = int\n\
             type t = (| c : config |)\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        let cst::TopBinding::Module { decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(decls.len(), 2);
        let cst::TopBinding::Type(t_decl) = &*decls[1].0 else {
            panic!("expected decls[1] to be a Type decl");
        };
        assert_eq!(t_decl.name.name, "M.t");
        let cst::TypeDeclBody::Synonym(ty) = &t_decl.body else {
            panic!("expected a synonym body");
        };
        let cst::ast::TypeExpr::Atom(prod) = ty else {
            panic!("expected a bare TypeProd");
        };
        let cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Record { fields, .. }) = &prod.first else {
            panic!("expected TypeAtom::Record, got {:?}", prod.first);
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name.name, "c");
        let cst::ast::TypeExpr::Atom(field_prod) = &*fields[0].ty.0 else {
            panic!("expected a bare TypeProd for the field type");
        };
        let cst::ast::TypeApp::Atom(cst::ast::TypeAtom::Name(n)) = &field_prod.first else {
            panic!("expected a bare type name atom, got {:?}", field_prod.first);
        };
        assert_eq!(n.name, "M.config", "the field's bare `config` must qualify to M.config");
    }

    /// SATySFi 0.1 dropped the fused `?:`/`?*` optional sigils entirely
    /// (optional-arg-rows increment 1): under V0_1 the lexer emits only `?` =
    /// `OptionalType`, so `?:1`/`?*` now lex as `?` + `:`/`*` — a downstream
    /// PARSE error, no longer reaching the lowerer. Replaced by the labeled
    /// `?(l = e)` bundle.
    #[test]
    fn old_optional_sigils_no_longer_parse() {
        assert!(satysfi_syntax::parse_file_v1("f ?:1").is_err());
        assert!(satysfi_syntax::parse_file_v1("f ?*").is_err());
    }

    /// A `?(l = e, …)` labeled-optional application bundle lowers to
    /// `AppArg::Bundled`; an empty `?()` bundle is a `LowerError`.
    #[test]
    fn empty_opt_arg_bundle_is_a_lower_error() {
        let file = parse_v1("f ?() x");
        let err = lower_document_v1(&file).unwrap_err();
        assert!(err.to_string().contains("optional-argument bundle"), "{err}");
    }

    /// math-split spec §3.1: `${…}` now lowers STRUCTURALLY — the
    /// deferral this test used to pin (`math_text_is_a_lower_error`) turned
    /// out unnecessary, since `${…}`'s *value* is version-independent (only
    /// the surrounding type/prim tables differ, per `typecheck.rs`'s
    /// `name_to_mono`). Round-trips `${x}` through `Atomic::MathText` down
    /// to its one `MathBot::Chars("x")` element.
    #[test]
    fn math_text_lowers_structurally() {
        let file = parse_v1("${x}");
        let ast = lower_document_v1(&file).unwrap_or_else(|e| panic!("lower_document_v1: {e}"));
        let cst::ast::Expr::Ops(chain) = &ast else {
            panic!("expected Expr::Ops, got {ast:?}");
        };
        let cst::ast::Atomic::MathText { elems, .. } = &chain.head.head else {
            panic!("expected Atomic::MathText, got {:?}", chain.head.head);
        };
        assert_eq!(elems.len(), 1, "one math element (`x`)");
        let cst::ast::MathBot::Chars(t) = &elems[0].base else {
            panic!("expected MathBot::Chars, got {:?}", elems[0].base);
        };
        assert_eq!(t.text, "x");
    }

    /// The shared lexer never actually emits a module-qualified command
    /// token (`HorzCmdWithMod`/`VertCmdWithMod`) in program-mode binding
    /// position — only inline/block-text and math areas dotted-scan a
    /// backslash/plus command name (`lexer.rs`'s `lex_program` vs.
    /// `lex_horz`/`lex_vert`/`lex_math`) — so `AnyHorzCmdTok::Mod`/
    /// `AnyVertCmdTok::Mod` are unreachable from any real `parse_file_v1`
    /// input at a `val inline`/`val block` binding's command-name position.
    /// This exercises `plain_horz`/`plain_vert` directly (a hand-built
    /// token, not a parse) so the `LowerError` arm itself is still proven,
    /// same rationale as the `CmdTail` bridge's own unreachable-in-practice
    /// guards (§3.3's doc comment).
    #[test]
    fn mod_qualified_command_name_in_bind_is_a_lower_error() {
        let tok = HorzCmdWithModTok {
            mods: vec!["Mod".to_string()],
            name: "\\emph".to_string(),
            span: Span::default(),
        };
        let err = plain_horz(&AnyHorzCmdTok::Mod(tok)).unwrap_err();
        assert!(err.to_string().contains("module-qualified"), "{err}");

        let tok = VertCmdWithModTok {
            mods: vec!["Mod".to_string()],
            name: "+p".to_string(),
            span: Span::default(),
        };
        let err = plain_vert(&AnyVertCmdTok::Mod(tok)).unwrap_err();
        assert!(err.to_string().contains("module-qualified"), "{err}");
    }

    #[test]
    fn lower_file_v1_on_a_document_is_an_error_not_a_panic() {
        let file = parse_v1("3");
        assert!(lower_file_v1(&file).is_err());
    }

    #[test]
    fn lower_document_v1_on_a_library_is_an_error_not_a_panic() {
        let file = parse_v1("module M = struct\nval x = 1\nend");
        assert!(lower_document_v1(&file).is_err());
    }

    /// Sub-slice 2a: `lower_file_v1` on a `Library` yields exactly ONE
    /// `TopBinding::Module` (not a spliced-flat `Vec` of the inner binds),
    /// with `sig: None` and the right name/decl count.
    #[test]
    fn lower_file_v1_yields_one_real_module_binding() {
        let file = parse_v1(
            "module V01Mini = struct\n\
             val x = 1\n\
             val y = 2\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        assert_eq!(lowered.len(), 1, "one TopBinding::Module, not spliced binds");
        let cst::TopBinding::Module { name, sig, decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module, got {:?}", lowered[0]);
        };
        assert_eq!(name.name, "V01Mini");
        assert!(sig.is_none(), "no signature annotation in Sub-slice 2a");
        assert_eq!(decls.len(), 2);
    }

    /// Sub-slice 2a: a nested `module N = struct … end` bind lowers to a
    /// nested `TopBinding::Module` inside the outer module's `decls`.
    #[test]
    fn lower_file_v1_nested_module_bind_lowers_to_nested_module() {
        let file = parse_v1(
            "module M = struct\n\
             val x = 1\n\
             module N = struct\n\
             val y = 2\n\
             end\n\
             end",
        );
        let lowered = lower_file_v1(&file).unwrap_or_else(|e| panic!("lower_file_v1: {e}"));
        assert_eq!(lowered.len(), 1);
        let cst::TopBinding::Module { name, decls, .. } = &lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(name.name, "M");
        assert_eq!(decls.len(), 2);
        assert!(matches!(&*decls[0].0, cst::TopBinding::Let(_)));
        let cst::TopBinding::Module {
            name: inner_name,
            sig: inner_sig,
            decls: inner_decls,
            ..
        } = &*decls[1].0
        else {
            panic!("expected decls[1] to be a nested TopBinding::Module");
        };
        assert_eq!(inner_name.name, "N");
        assert!(inner_sig.is_none());
        assert_eq!(inner_decls.len(), 1);
    }

    // ---- Sub-slice 2d-1: sealing lowers to NOTHING (zero runtime residue) -

    /// §4.3-E4 / §5.4 test T17 twin 1: a library-level `:>` ascription is no
    /// longer a `LowerError` at all — it lowers to the byte-identical
    /// `TopBinding::Module { sig: None, .. }` shape its unsealed twin does
    /// (enforcement moved entirely to `v1/module_check.rs`, which reads the
    /// annotation back off the original `cst_v1` tree — this module never
    /// sees it again after this point).
    #[test]
    fn sig_annot_on_library_lowers_like_its_unsealed_twin() {
        let sealed = parse_v1("module M :> sig val x : int end = struct\nval x = 1\nend");
        let unsealed = parse_v1("module M = struct\nval x = 1\nend");
        let sealed_lowered = lower_file_v1(&sealed).unwrap_or_else(|e| panic!("sealed: {e}"));
        let unsealed_lowered = lower_file_v1(&unsealed).unwrap_or_else(|e| panic!("unsealed: {e}"));
        assert_eq!(sealed_lowered.len(), 1);
        assert_eq!(unsealed_lowered.len(), 1);
        let cst::TopBinding::Module { sig: sealed_sig, decls: sealed_decls, .. } = &sealed_lowered[0]
        else {
            panic!("expected a TopBinding::Module");
        };
        let cst::TopBinding::Module { sig: unsealed_sig, decls: unsealed_decls, .. } =
            &unsealed_lowered[0]
        else {
            panic!("expected a TopBinding::Module");
        };
        assert!(sealed_sig.is_none(), "the seal must lower to NO cst::SigAnnot at all");
        assert!(unsealed_sig.is_none());
        assert_eq!(sealed_decls.len(), unsealed_decls.len());
        assert!(matches!(&*sealed_decls[0].0, cst::TopBinding::Let(_)));
        assert!(matches!(&*unsealed_decls[0].0, cst::TopBinding::Let(_)));
    }

    /// §4.3-E4 / §5.4 test T17 twin 2: the struct body still lowers
    /// (surfacing its own precise error) whether or not the module carries
    /// a `:>` annotation — a body error is identical either way, since the
    /// annotation is no longer consulted by lowering at all. Body error:
    /// an arity-≥2 type application (`pair int int`) — still unsupported
    /// (§3.4/`lower_type_app`, above) — swapped in from the original 0.1-
    /// math body error the math-split spec resolved (§3.1: `${…}` now
    /// lowers structurally, so it can no longer serve as this test's
    /// "still errors" body).
    #[test]
    fn sig_annot_body_error_is_identical_with_or_without_a_seal() {
        let sealed = parse_v1("module M :> sig end = struct\ntype t = pair int int\nend");
        let unsealed = parse_v1("module M = struct\ntype t = pair int int\nend");
        let sealed_err = lower_file_v1(&sealed).unwrap_err();
        let unsealed_err = lower_file_v1(&unsealed).unwrap_err();
        assert!(sealed_err.to_string().contains("more than one argument"), "{sealed_err}");
        assert_eq!(sealed_err.construct, unsealed_err.construct);
        assert_eq!(sealed_err.hint, unsealed_err.hint);
    }

    /// §4.3-E4 / §5.4 test T17 twin 3: the bind-level twin of
    /// `sig_annot_on_library_lowers_like_its_unsealed_twin` — a nested
    /// `module N :> sig .. end = struct .. end` lowers to the same nested
    /// `TopBinding::Module { sig: None, .. }` shape as its unsealed twin.
    #[test]
    fn nested_module_sig_annot_lowers_like_its_unsealed_twin() {
        let sealed = parse_v1(
            "module M = struct\n\
             module N :> sig val y : int end = struct\n\
             val y = 2\n\
             end\n\
             end",
        );
        let unsealed = parse_v1(
            "module M = struct\n\
             module N = struct\n\
             val y = 2\n\
             end\n\
             end",
        );
        let sealed_lowered = lower_file_v1(&sealed).unwrap_or_else(|e| panic!("sealed: {e}"));
        let unsealed_lowered = lower_file_v1(&unsealed).unwrap_or_else(|e| panic!("unsealed: {e}"));
        let cst::TopBinding::Module { decls: sealed_decls, .. } = &sealed_lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        let cst::TopBinding::Module { decls: unsealed_decls, .. } = &unsealed_lowered[0] else {
            panic!("expected a TopBinding::Module");
        };
        assert_eq!(sealed_decls.len(), 1);
        assert_eq!(unsealed_decls.len(), 1);
        let cst::TopBinding::Module { name: sealed_name, sig: sealed_sig, decls: sealed_inner, .. } =
            &*sealed_decls[0].0
        else {
            panic!("expected decls[0] to be a nested TopBinding::Module");
        };
        let cst::TopBinding::Module {
            name: unsealed_name,
            sig: unsealed_sig,
            decls: unsealed_inner,
            ..
        } = &*unsealed_decls[0].0
        else {
            panic!("expected decls[0] to be a nested TopBinding::Module");
        };
        assert_eq!(sealed_name.name, "N");
        assert_eq!(unsealed_name.name, "N");
        assert!(sealed_sig.is_none(), "the seal must lower to NO cst::SigAnnot at all");
        assert!(unsealed_sig.is_none());
        assert_eq!(sealed_inner.len(), unsealed_inner.len());
    }

    /// §5.3 test 4: a module alias/path binding.
    #[test]
    fn module_alias_is_a_lower_error() {
        let file = parse_v1("module M = struct\nmodule P = N\nend");
        let err = lower_file_v1(&file).unwrap_err();
        assert!(err.to_string().contains("module alias"), "{err}");
    }

    /// §5.3 test 5: a functor application.
    #[test]
    fn functor_application_is_a_lower_error() {
        let file = parse_v1("module M = struct\nmodule P = F X\nend");
        let err = lower_file_v1(&file).unwrap_err();
        assert!(err.to_string().contains("functor application"), "{err}");
    }

    /// §5.3 test 6: a functor literal.
    #[test]
    fn functor_literal_is_a_lower_error() {
        let file = parse_v1(
            "module M = struct\n\
             module F = fun (X : sig val x : int end) -> struct val y = X.x end\n\
             end",
        );
        let err = lower_file_v1(&file).unwrap_err();
        assert!(err.to_string().contains("functor literal"), "{err}");
    }

    /// §5.3 test 7: a bare module coercion `N :> S` (no annotation, no
    /// struct literal).
    #[test]
    fn module_coercion_is_a_lower_error() {
        let file = parse_v1("module M = struct\nmodule P = N :> S\nend");
        let err = lower_file_v1(&file).unwrap_err();
        assert!(err.to_string().contains("coercion"), "{err}");
    }

    /// §5.3 test 8: a `signature S = ...` bind.
    #[test]
    fn signature_bind_is_a_lower_error() {
        let file = parse_v1("module M = struct\nsignature S = sig end\nend");
        let err = lower_file_v1(&file).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("signature"), "{msg}");
        assert!(msg.contains("2d"), "{msg}");
    }

    /// §5.3 test 9: an `include M` bind.
    #[test]
    fn include_bind_is_a_lower_error() {
        let file = parse_v1("module M = struct\ninclude N\nend");
        let err = lower_file_v1(&file).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("include"), "{msg}");
        assert!(msg.contains("2e"), "{msg}");
    }

    /// The CmdTail bridge (§3.3): `\cmd{a}{b}` parses under `cst_v1` as one
    /// application chain (`AppExpr { head: {a}, args: [{b}] }`) and must
    /// lower to the same shape `cst.rs`'s own `\cmd{a}{b}` parse produces
    /// (`CmdTail::Args { first: {a}, rest: [{b}] }`).
    #[test]
    fn cmd_tail_bridge_matches_flat_app_arg_shape() {
        let file = parse_v1(r"{\cmd{a}{b}}");
        let cst_v1::FileV1::Document { body, .. } = file else {
            panic!("expected a document file");
        };
        let ast_v1::Expr::Ops(chain) = body else {
            panic!("expected an operator-chain expression");
        };
        let ast_v1::Atomic::InlineText { elems, .. } = chain.head.head else {
            panic!("expected inline text");
        };
        let ast_v1::InlineElem::Cmd { tail, .. } = &elems[0] else {
            panic!("expected the first element to be a command");
        };
        let lowered = lower_cmd_tail(tail).unwrap();
        let cst::ast::CmdTail::Args { first, rest, .. } = lowered else {
            panic!("expected CmdTail::Args");
        };
        assert_eq!(rest.len(), 1, "\\cmd{{a}}{{b}} has exactly one trailing arg");
        assert!(matches!(
            &*first.0,
            cst::ast::AppArg::Atom {
                atom: cst::ast::Atomic::InlineText { .. },
                ..
            }
        ));
        assert!(matches!(
            &*rest[0].0,
            cst::ast::AppArg::Atom {
                atom: cst::ast::Atomic::InlineText { .. },
                ..
            }
        ));
    }
}
