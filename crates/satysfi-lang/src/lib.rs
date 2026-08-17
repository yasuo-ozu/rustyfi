//! Abstract syntax tree, elaboration, evaluator, and primitives — the
//! language core of the SATySFi port.

pub mod ast;
pub(crate) mod compile;
pub mod crossref;
pub mod elaborate;
pub mod eval;
pub mod exhaustive;
pub mod prim_types;
pub mod primitives;
pub mod typecheck;
pub mod types;
pub mod unify;
pub mod v1;
pub mod value;

use crossref::{CrossRefs, Verdict};
use satysfi_backend::{placed_line_extent, DecoId, FontMetrics, GraphicsElem, Length, PureHorzBox};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use value::{DocumentValue, Value};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(transparent)]
    Parse(#[from] satysfi_syntax::ParseFileError),
    #[error(transparent)]
    Elaborate(#[from] elaborate::ElabError),
    #[error(transparent)]
    Type(#[from] typecheck::TypeError),
    #[error(transparent)]
    Eval(#[from] eval::EvalError),
    #[error("the file's expression evaluated to {0}, not a document")]
    NotADocument(&'static str),
    #[error(transparent)]
    Lower(#[from] v1::lower::LowerError),
    /// Cross-version import (X1, `docs/plans/design-cross-version-import.md`
    /// §5): a `V0_0_6` dependency spliced into a `V0_1` program referenced
    /// `name`, a builtin primitive/type that is version-forked (bound, or
    /// shaped, differently between `V0_0_6` and `V0_1` —
    /// `primitives::forked_prim_names`/`typecheck::forked_type_names`). The
    /// merged program's single `base_env_with_version(V0_1)` can only bind
    /// ONE closure per name (§3.2's R1), so silently accepting this would
    /// mis-resolve `name` to the WRONG version's primitive; `slice` names
    /// the milestone (`"X1"`) so a later slice's error text can update in
    /// lockstep with what it actually fixes.
    #[error(
        "cross-version import ({slice}): dependency {dep} references `{name}`, a \
         version-forked builtin — {slice} only supports the version-neutral subset \
         of the 0.0.6 corpus (see docs/plans/design-cross-version-import.md)"
    )]
    CrossVersionUnsupportedName {
        name: String,
        dep: String,
        slice: &'static str,
    },
}

/// Compile a `.saty` source string down to a typeset document:
/// lex → parse → elaborate → evaluate.
pub fn compile_document(
    src: &str,
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    compile_document_cst(&file, metrics)
}

/// Compile an already-parsed (possibly loader-merged) file. The multi-file
/// loader concatenates library preludes into one synthetic `cst::File` and
/// enters here.
///
/// Thin wrapper over [`compile_document_cst_with_trials`] that drops the
/// trial count — kept as a stable, unchanged entry point for the CLI
/// (`main.rs`) and every pre-Slice-1 caller.
pub fn compile_document_cst(
    file: &satysfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    compile_document_cst_with_trials(file, metrics).map(|(doc, _trials)| doc)
}

/// Same as [`compile_document_cst`], but also returns how many fixpoint
/// trials it took (docs/plans/hooks-annotations-crossref.md §Cross-references
/// & the fixpoint) — exposed for tests that must confirm the fixpoint
/// actually iterated, not just that it produced the right answer on a lucky
/// first pass.
pub fn compile_document_cst_with_trials(
    file: &satysfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    let env0 = primitives::base_env();
    let scope = elaborate::Scope::new(env0.names());
    let program = elaborate::elaborate_program(file, &scope)?;
    typecheck::typecheck(&program)?;
    // Compile the elaborated body into a closure tree ONCE. Each trial below
    // re-runs this same `compiled` against a fresh env + a fresh (except
    // `crossrefs`) `Interp` — safe because `CompiledExpr::run` takes `&self`
    // and re-executes the whole tree from scratch, reproducing upstream's
    // `eval_main i env_freezed ast` per trial (`main.ml:337-397`).
    let compiled = compile::compile_program(&program.body, &env0);
    eval_document_trials(&compiled, metrics, satysfi_syntax::SatysfiVersion::V0_0_6)
}

/// The compile-once + fixpoint-trial tail shared by the `V0_0_6` and `V0_1`
/// entry points (`compile_document_cst_with_trials` above and
/// `compile_document_v1_with_trials` below). Extracted verbatim from what
/// used to be inline in `compile_document_cst_with_trials` — the only
/// version-sensitive step is the fresh per-trial env
/// (`primitives::base_env_with_version(version)`); everything else
/// (crossrefs persistence, `fire_hooks`, `DocExtras` attach) is identical
/// regardless of which SATySFi generation produced `compiled`.
fn eval_document_trials(
    compiled: &compile::CompiledExpr,
    metrics: &dyn FontMetrics,
    version: satysfi_syntax::SatysfiVersion,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    // The cross-reference table persists across trials — it *is* the
    // fixpoint state (docs/plans/hooks-annotations-crossref.md's Risks:
    // "what resets per trial vs what persists").
    let crossrefs = Rc::new(RefCell::new(CrossRefs::new()));
    let mut trials = 0u32;
    loop {
        trials += 1;
        // Fresh per trial: `let-mutable` store state resets (== upstream's
        // `env_freezed` re-eval), and a fresh `Interp` resets `hooks`/
        // `images` too — only `crossrefs` is threaded through.
        let env = primitives::base_env_with_version(version);
        let mut interp = eval::Interp::new(metrics);
        interp.crossrefs = crossrefs.clone();
        // math-split spec §3.4: threads `version` onto the `Interp` so
        // `read_inline`'s `EmbedMath` fallback arm (no installed math
        // command — unit-test contexts only) can dispatch between
        // `reflect_math_elem`/`reflect_math_elem_v01`. Both callers of this
        // shared tail already receive/pass the real `version`, so this is a
        // single, version-agnostic line.
        interp.version = version;
        let doc = match compiled.run(&env, &mut interp)? {
            Value::Document(doc) => doc,
            other => return Err(CompileError::NotADocument(other.type_name())),
        };
        // Fire every placed page-break hook now that `break_pages` has given
        // every one of them its final page number/point; hooks mutate
        // `crossrefs` (the only place that seam is legally crossed — see
        // `fire_hooks`'s doc comment).
        fire_hooks(&mut interp, &doc)?;
        match crossrefs.borrow_mut().verdict() {
            Verdict::NeedsAnotherTrial => continue,
            Verdict::CanTerminate(_) | Verdict::CountMax => {
                // Attach the final trial's accumulated extras. `doc` is
                // usually uniquely held here; if the program's env still
                // holds a clone, fall back to a one-time deep clone.
                let mut final_doc = Rc::try_unwrap(doc).unwrap_or_else(|rc| (*rc).clone());
                final_doc.extras = satysfi_backend::DocExtras {
                    annotations: std::mem::take(&mut interp.annotations),
                    destinations: std::mem::take(&mut interp.destinations),
                    outline: std::mem::take(&mut interp.outline),
                    page_graphics: std::mem::take(&mut interp.page_graphics),
                    doc_info: interp.doc_info.take(),
                };
                return Ok((Rc::new(final_doc), trials));
            }
        }
    }
}

/// Compile a loader-resolved SATySFi 0.1 program (`LoadOptions { version:
/// V0_1, .. }`): dependency libraries (`files[..n-1]`, loader
/// dependency-first order) are each lowered to one `TopBinding::Module`
/// (qualified exports; Sub-slice 2a — see `v1/lower.rs`'s module doc) via
/// [`v1::lower::lower_file_v1`], the entry (`files[n-1]`, always last —
/// `LoadedProgram::files`'s contract) via [`v1::lower::lower_document_v1`],
/// assembled into ONE synthetic `cst::File` — the same shape the CLI's
/// `merge_program` builds for 0.0.6 — and pushed through the SHARED
/// elaborate -> typecheck(V0_1) -> compile -> fixpoint-eval pipeline.
/// Signature ascriptions (`:>`) are enforced per binding by
/// `v1::module_check::check_program` — Sub-slice 2d-1.
pub fn compile_document_v1(
    files: &[satysfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    compile_document_v1_with_trials(files, metrics).map(|(doc, _trials)| doc)
}

/// Trial-count-reporting sibling, mirroring
/// `compile_document_cst_with_trials` (same rationale: fixture tests that
/// must see the fixpoint iterate).
pub fn compile_document_v1_with_trials(
    files: &[satysfi_loader::LoadedFile],
    metrics: &dyn FontMetrics,
) -> Result<(std::rc::Rc<DocumentValue>, u32), CompileError> {
    use satysfi_syntax::SatysfiVersion;

    // -- assemble the synthetic cst::File (merge_program's V0_1 analogue) --
    let (entry, deps) = files
        .split_last()
        .expect("loader always yields at least the entry file");
    // Only ever called on the entry: under `compile_document_v1`, the entry
    // is ALWAYS `V0_1` (the loader's own contract — `load_legacy`'s Q4 rule
    // only ever downgrades a DEPENDENCY to `V0_0_6`, never the entry; see
    // `LoadedFile::version`'s doc comment). A `V0_0_6` dependency is instead
    // routed through the cross-version splice arm below (X1,
    // `docs/plans/design-cross-version-import.md` §5) — it never reaches
    // this helper.
    fn as_v01(f: &satysfi_loader::LoadedFile) -> &satysfi_syntax::cst_v1::FileV1 {
        match &f.cst {
            satysfi_loader::LoadedCst::V0_1(cst) => cst,
            satysfi_loader::LoadedCst::V0_0_6(_) => unreachable!(
                "as_v01 called on a V0_0_6-parsed file — the entry is always \
                 V0_1 under compile_document_v1, and every V0_0_6 dependency \
                 is routed through the X1 cross-version splice arm instead"
            ),
        }
    }
    // Sub-slice 2d-3 (`…/tmp/slice2d3-module-sig-decls.md` §2.1): one
    // `SurfaceEnv` threaded across every V0_1 dependency in load order, so a
    // module alias/named-signature reference in a LATER-loaded library can
    // resolve an EARLIER one (`module M = OtherLib`, `:> OtherLib.S`).
    // `build_file_surface` runs (pure `cst_v1` walk, no lowering needed)
    // BEFORE each dep is lowered, so that dep's own internal aliases/named
    // signatures resolve too (`v1/surface.rs`'s doc comment).
    //
    // X1 (design-cross-version-import.md §5): `deps` is now a MIXED-version
    // list (`LoadedFile::version`). A `V0_1` dep is lowered exactly as
    // before; a `V0_0_6` dep contributes its `cst::File.prelude` bindings
    // DIRECTLY (they are already `cst::TopBinding`s — §1.1's "no syntactic
    // bridge needed"), positioned dependency-first (loader order, unchanged)
    // — but ONLY after the forked-name guard proves it references no
    // version-forked builtin (§3.2's R1: the single `base_env_with_version
    // (V0_1)` below cannot serve two different arities of the same
    // primitive name, so an unguarded splice would silently mis-resolve
    // one). `dep_csts` collects the V0_1 subset only — `check_program`
    // (below) has no `cst_v1` vocabulary for a `V0_0_6` file (§2.5/R3).
    let mut surfaces = v1::surface::SurfaceEnv::default();
    let mut prelude = Vec::new();
    let mut dep_csts: Vec<&satysfi_syntax::cst_v1::FileV1> = Vec::new();
    for dep in deps {
        match &dep.cst {
            satysfi_loader::LoadedCst::V0_1(cst) => {
                v1::surface::build_file_surface(cst, &mut surfaces);
                prelude.extend(v1::lower::lower_file_v1_with_surfaces(cst, &surfaces)?);
                dep_csts.push(cst);
            }
            satysfi_loader::LoadedCst::V0_0_6(cst) => {
                let free = collect_free_globals(&cst.prelude);
                let forked_v = primitives::forked_prim_names();
                let forked_t = typecheck::forked_type_names();
                if let Some(name) = free
                    .values
                    .intersection(&forked_v)
                    .next()
                    .or_else(|| free.types.intersection(&forked_t).next())
                {
                    return Err(CompileError::CrossVersionUnsupportedName {
                        name: name.clone(),
                        dep: dep.path.display().to_string(),
                        slice: "X1",
                    });
                }
                prelude.extend(cst.prelude.iter().cloned());
            }
        }
    }
    let entry_cst = as_v01(entry);
    let body = v1::lower::lower_document_v1(entry_cst)?;
    let eoi = match entry_cst {
        satysfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("lower_document_v1 already rejected a Library entry"),
    };
    let file = satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: Some(satysfi_syntax::leaf::KwIn(satysfi_syntax::Span::default())),
        body: Some(body),
        eoi,
    };

    // -- the shared pipeline, V0_1-tagged (mirrors
    //    compile_document_cst_with_trials line for line) --
    let env0 = primitives::base_env_with_version(SatysfiVersion::V0_1);
    let scope = elaborate::Scope::new_with_version(env0.names(), SatysfiVersion::V0_1);
    let program = elaborate::elaborate_program(&file, &scope)?;
    v1::module_check::check_program(&dep_csts, &program)?;
    let compiled = compile::compile_program(&program.body, &env0);
    eval_document_trials(&compiled, metrics, SatysfiVersion::V0_1)
}

// ============================================================================
// X1 forked-name guard (design-cross-version-import.md §5): before splicing
// a V0_0_6 dependency's `prelude` into a V0_1 program (above), walk it for
// the free (unqualified, unshadowed) primitive/type names it references and
// hard-reject any that is version-forked. This is what keeps X1 *sound*
// rather than silently wrong (§3.2's R1) — see `compile_document_v1_with_
// trials`'s dep loop for the actual check against `primitives::
// forked_prim_names`/`typecheck::forked_type_names`.
//
// Greenfield: there is no generic CST visitor in this crate (the closest
// precedent is `typecheck.rs`'s `walk_atom`/`walk_expr` quartet, which walks
// only `ast::TypeExpr` for tyvars); this is modeled on it, but covers the
// FULL `cst::TopBinding`/`ast::Expr`/`ast::Pattern`/`ast::TypeExpr` grammar.
// ============================================================================

/// The free, unqualified global names a spliced V0_0_6 dependency's
/// `prelude` references, split by namespace (values/commands vs. types)
/// because they are checked against DIFFERENT forked-name sets. See
/// `collect_free_globals`'s doc comment for the walk itself.
#[derive(Default, Debug)]
struct FreeGlobals {
    /// Value-position occurrences that could resolve to `base_env`:
    /// `Atomic::Var`/`Ctor`/`OpRef`/`Command`, the `Plain` arm of an
    /// `AnyHorz`/`Vert`/`MathCmdTok` reference, and an unqualified
    /// `…Elem::Embed`/`MathBot::Embed`.
    values: BTreeSet<String>,
    /// Type-position occurrences: `TypeAtom::Name` and the `ctor` of
    /// `TypeApp::Applied`.
    types: BTreeSet<String>,
}

/// The binder scope threaded through `collect_free_globals`'s walk: two
/// independent namespaces (values/commands vs. types), each a plain stack of
/// names — pushing a name shadows an outer/global name of the SAME
/// namespace for the extent of whatever construct introduced it (`mark`/
/// `truncate_to` bound that extent, mirroring a lexical block's entry/exit).
///
/// **Soundness note.** This is a rejection GUARD, so over-approximation is
/// the safe direction: failing to push a genuine local binder just makes a
/// local look "free" (over-reporting — at worst an over-*rejection*, never
/// silently accepting something unsound). The one thing the walk must never
/// do is drop a binder scope *too early* / push something that ISN'T really
/// bound at that point, which would hide a genuine reference to a
/// version-forked global (see `Expr::OpenIn`'s arm below, which deliberately
/// binds NOTHING for an `open Mod in …` rather than guess at Mod's members).
#[derive(Default)]
struct XverScope {
    values: Vec<String>,
    types: Vec<String>,
}

impl XverScope {
    fn mark(&self) -> (usize, usize) {
        (self.values.len(), self.types.len())
    }

    fn truncate_to(&mut self, mark: (usize, usize)) {
        self.values.truncate(mark.0);
        self.types.truncate(mark.1);
    }

    fn push_value(&mut self, name: &str) {
        self.values.push(name.to_string());
    }

    fn push_type(&mut self, name: &str) {
        self.types.push(name.to_string());
    }

    fn has_value(&self, name: &str) -> bool {
        self.values.iter().any(|v| v == name)
    }

    fn has_type(&self, name: &str) -> bool {
        self.types.iter().any(|v| v == name)
    }
}

fn emit_value(scope: &XverScope, out: &mut FreeGlobals, name: &str) {
    if !scope.has_value(name) {
        out.values.insert(name.to_string());
    }
}

fn emit_type(scope: &XverScope, out: &mut FreeGlobals, name: &str) {
    if !scope.has_type(name) {
        out.types.insert(name.to_string());
    }
}

/// Enumerate the *free, unqualified* global names a spliced V0_0_6
/// dependency references — `TopBinding`/`ast::Expr`/`ast::Pattern`/
/// `ast::TypeExpr`, each threading a binder scope stack so a locally-bound
/// name shadows a primitive of the same name (per `XverScope`'s doc
/// comment). A module-qualified reference (`Atomic::VarWithMod`,
/// `\Mod.cmd`/`+Mod.cmd`/`#Mod.var`) is deliberately SKIPPED: a primitive or
/// builtin type is only ever reachable by a BARE name, so a qualified
/// reference resolves inside a module and can never collide with a forked
/// primitive (0.0.6 has no qualified *type*-name form at all, so every type
/// reference is in scope for this check).
fn collect_free_globals(prelude: &[satysfi_syntax::cst::TopBinding]) -> FreeGlobals {
    let mut out = FreeGlobals::default();
    let mut scope = XverScope::default();
    for tb in prelude {
        walk_top_binding(tb, &mut scope, &mut out);
    }
    out
}

fn walk_top_binding(tb: &satysfi_syntax::cst::TopBinding, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::TopBinding;
    match tb {
        // Recursive: every clause's own name is bound BEFORE any clause body
        // is walked (and stays bound for every sibling `and` clause too).
        TopBinding::LetRec { first, ands, .. } => {
            scope.push_value(&first.name.name);
            for and in ands {
                scope.push_value(&and.binding.name.name);
            }
            walk_rec_binding_body(first, scope, out);
            for and in ands {
                walk_rec_binding_body(&and.binding, scope, out);
            }
        }
        TopBinding::Let(tl) => {
            let mark = scope.mark();
            for p in &tl.params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(&tl.value, scope, out);
            scope.truncate_to(mark);
            scope.push_value(&tl.name.name);
        }
        TopBinding::LetInline { ctx, cmd, params, value, .. } => {
            let mark = scope.mark();
            if let Some(c) = ctx {
                scope.push_value(&c.name);
            }
            for p in params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(value, scope, out);
            scope.truncate_to(mark);
            scope.push_value(&cmd.name);
        }
        TopBinding::LetBlock { ctx, cmd, params, value, .. } => {
            let mark = scope.mark();
            if let Some(c) = ctx {
                scope.push_value(&c.name);
            }
            for p in params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(value, scope, out);
            scope.truncate_to(mark);
            scope.push_value(&cmd.name);
        }
        TopBinding::LetMath { cmd, params, value, .. } => {
            let mark = scope.mark();
            for p in params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(value, scope, out);
            scope.truncate_to(mark);
            scope.push_value(&cmd.name);
        }
        TopBinding::Type(td) => {
            walk_type_decl(td, scope, out);
            scope.push_type(&td.name.name);
        }
        TopBinding::LetMutable { name, value, .. } => {
            walk_expr(value, scope, out);
            scope.push_value(&name.name);
        }
        TopBinding::Module { sig, decls, .. } => {
            if let Some(sig) = sig {
                walk_sig_annot(sig, scope, out);
            }
            // A nested module's own decls get a scope extent of their own —
            // its LOCAL bindings must not leak to a sibling top binding
            // outside the module.
            let mark = scope.mark();
            for d in decls {
                walk_top_binding(&d.0, scope, out);
            }
            scope.truncate_to(mark);
            // The module's own NAME (a `CtorTok`, uppercase-initial) is a
            // third namespace this guard doesn't track — it can never
            // collide with a lowercase primitive/type name.
        }
        // `open Mod` unqualified-imports Mod's members — unknowable
        // statically here (no elaboration has run yet), so this
        // conservatively binds NOTHING new: see `XverScope`'s doc comment
        // for why that is the safe direction (may over-reject, never hides
        // a real forked-name reference).
        TopBinding::Open { .. } => {}
    }
}

/// Walk one `RecBinding`'s own params/value/`extra` clauses (shared by
/// `TopBinding::LetRec` and `Expr::LetRecIn`) — every clause's parameters are
/// scoped to that clause alone.
fn walk_rec_binding_body(
    rb: &satysfi_syntax::cst::ast::RecBinding,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    if let Some(asc) = &rb.ascription {
        walk_type_expr(&asc.ty, scope, out);
    }
    let mark = scope.mark();
    for p in &rb.params {
        walk_patbot_binder(p, scope, out);
    }
    walk_expr(&rb.value.0, scope, out);
    scope.truncate_to(mark);
    for clause in &rb.extra {
        let mark = scope.mark();
        for p in &clause.params {
            walk_patbot_binder(p, scope, out);
        }
        walk_expr(&clause.value.0, scope, out);
        scope.truncate_to(mark);
    }
}

fn walk_param_binder(p: &satysfi_syntax::cst::ast::Param, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::Param;
    match p {
        Param::Optional { name, .. } => scope.push_value(&name.name),
        Param::Pat(pb) => walk_patbot_binder(pb, scope, out),
        Param::Bundled { opts, body } => {
            for e in &opts.entries {
                scope.push_value(&e.var.name);
            }
            walk_patbot_binder(body, scope, out);
        }
    }
}

/// Walk a full `patas` (a pattern plus its optional `as name` binding) in
/// BINDER mode: every `Var`/`AsClause.name` is pushed (never emitted); every
/// `Ctor`/`CtorApplied.ctor` is a REFERENCE — emitted for completeness (the
/// corpus's constructors are neutral, but this keeps the walk total).
fn walk_pattern_binder(pat: &satysfi_syntax::cst::ast::Pattern, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_patcons_binder(&pat.head, scope, out);
    if let Some(ac) = &pat.as_clause {
        scope.push_value(&ac.name.name);
    }
}

fn walk_patcons_binder(pc: &satysfi_syntax::cst::ast::PatCons, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_patbot_binder(&pc.head, scope, out);
    for seg in &pc.tail {
        walk_patbot_binder(&seg.tail, scope, out);
    }
}

fn walk_patbot_binder(pb: &satysfi_syntax::cst::ast::PatBot, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::PatBot;
    match pb {
        PatBot::CtorApplied { ctor, arg } => {
            emit_value(scope, out, &ctor.name);
            walk_patbot_binder(arg, scope, out);
        }
        PatBot::Ctor(ctor) => emit_value(scope, out, &ctor.name),
        PatBot::Int(_) | PatBot::True(_) | PatBot::False(_) | PatBot::Str(_) | PatBot::Wild(_) => {}
        PatBot::Var(v) => scope.push_value(&v.name),
        PatBot::Unit { .. } => {}
        PatBot::Paren { inner, .. } => {
            walk_pattern_binder(&inner.first.0, scope, out);
            for cp in &inner.rest {
                walk_pattern_binder(&cp.value.0, scope, out);
            }
        }
        PatBot::List { items, .. } => {
            for it in items {
                walk_pattern_binder(&it.value.0, scope, out);
            }
        }
    }
}

fn walk_type_decl(td: &satysfi_syntax::cst::TypeDecl, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::TypeDeclBody;
    match &td.body {
        TypeDeclBody::Variant { first, rest, .. } => {
            walk_variant_def(first, scope, out);
            for bv in rest {
                walk_variant_def(&bv.def, scope, out);
            }
        }
        TypeDeclBody::Synonym(ty) => walk_type_expr(ty, scope, out),
    }
}

fn walk_variant_def(vd: &satysfi_syntax::cst::VariantDef, scope: &mut XverScope, out: &mut FreeGlobals) {
    // `vd.ctor` DECLARES a new constructor — not a reference, nothing to
    // emit for it.
    if let Some(of_ty) = &vd.of_ty {
        walk_type_expr(&of_ty.ty, scope, out);
    }
}

fn walk_sig_annot(sig: &satysfi_syntax::cst::SigAnnot, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::SigItem;
    for item in &sig.items {
        match item {
            SigItem::ValHorzCmd { ty, .. }
            | SigItem::ValVertCmd { ty, .. }
            | SigItem::Val { ty, .. }
            | SigItem::DirectHorzCmd { ty, .. }
            | SigItem::DirectVertCmd { ty, .. } => walk_type_expr(ty, scope, out),
            SigItem::Type { .. } => {}
        }
    }
}

fn walk_expr(e: &satysfi_syntax::cst::ast::Expr, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::Expr;
    match e {
        Expr::LetRecIn { first, ands, body, .. } => {
            let mark = scope.mark();
            scope.push_value(&first.name.name);
            for and in ands {
                scope.push_value(&and.binding.name.name);
            }
            walk_rec_binding_body(first, scope, out);
            for and in ands {
                walk_rec_binding_body(&and.binding, scope, out);
            }
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::LetIn { name, params, value, body, .. } => {
            let mark = scope.mark();
            for p in params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(value, scope, out);
            scope.truncate_to(mark);
            let mark = scope.mark();
            scope.push_value(&name.name);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::LetPatternIn { pat, value, body, .. } => {
            walk_expr(value, scope, out);
            let mark = scope.mark();
            walk_pattern_binder(&pat.0, scope, out);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::If { cond, then_branch, else_branch, .. } => {
            walk_expr(cond, scope, out);
            walk_expr(then_branch, scope, out);
            walk_expr(else_branch, scope, out);
        }
        Expr::Fun { params, body, .. } => {
            let mark = scope.mark();
            for p in params {
                walk_patbot_binder(p, scope, out);
            }
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::FunRows { opts, param, body, .. } => {
            let mark = scope.mark();
            for e in &opts.entries {
                scope.push_value(&e.var.name);
            }
            walk_patbot_binder(param, scope, out);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::Match { scrutinee, first, rest, .. } => {
            walk_expr(scrutinee, scope, out);
            walk_match_arm(first, scope, out);
            for ba in rest {
                walk_match_arm(&ba.arm, scope, out);
            }
        }
        Expr::LetMutableIn { name, init, body, .. } => {
            walk_expr(init, scope, out);
            let mark = scope.mark();
            scope.push_value(&name.name);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        Expr::LetMathIn { cmd, params, value, body, .. } => {
            let mark = scope.mark();
            for p in params {
                walk_param_binder(p, scope, out);
            }
            walk_expr(value, scope, out);
            scope.truncate_to(mark);
            let mark = scope.mark();
            scope.push_value(&cmd.name);
            walk_expr(body, scope, out);
            scope.truncate_to(mark);
        }
        // `open Mod in body` — see `TopBinding::Open`'s arm for why this
        // binds nothing new.
        Expr::OpenIn { body, .. } => walk_expr(body, scope, out),
        Expr::WhileDo { cond, body, .. } => {
            walk_expr(cond, scope, out);
            walk_expr(body, scope, out);
        }
        Expr::Overwrite { name, value, .. } => {
            emit_value(scope, out, &name.name);
            walk_expr(&value.0, scope, out);
        }
        Expr::Ops(chain) => walk_opchain(chain, scope, out),
    }
}

fn walk_match_arm(arm: &satysfi_syntax::cst::ast::MatchArm, scope: &mut XverScope, out: &mut FreeGlobals) {
    let mark = scope.mark();
    walk_pattern_binder(&arm.pat.0, scope, out);
    if let Some(g) = &arm.guard {
        walk_expr(&g.cond.0, scope, out);
    }
    walk_expr(&arm.body.0, scope, out);
    scope.truncate_to(mark);
}

fn walk_opchain(oc: &satysfi_syntax::cst::ast::OpChain, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_appexpr(&oc.head, scope, out);
    for r in &oc.tail {
        walk_appexpr(&r.rhs, scope, out);
    }
    if let Some(bt) = &oc.before {
        walk_expr(&bt.body.0, scope, out);
    }
}

fn walk_appexpr(ae: &satysfi_syntax::cst::ast::AppExpr, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_atomic(&ae.head, scope, out);
    // `head_accesses`: `#label` record-field accesses — field labels, not
    // globals, skip.
    for arg in &ae.args {
        walk_apparg(arg, scope, out);
    }
}

fn walk_apparg(a: &satysfi_syntax::cst::ast::AppArg, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::AppArg;
    match a {
        AppArg::Optional { value, .. } => walk_atomic(value, scope, out),
        AppArg::Omission(_) => {}
        AppArg::Atom { atom, .. } => walk_atomic(atom, scope, out),
        AppArg::Ctor(c) => emit_value(scope, out, &c.name),
        AppArg::Bundled { opts, atom, .. } => {
            for e in &opts.entries {
                walk_expr(&e.value.0, scope, out);
            }
            walk_atomic(atom, scope, out);
        }
        AppArg::BundledCtor { opts, ctor } => {
            for e in &opts.entries {
                walk_expr(&e.value.0, scope, out);
            }
            emit_value(scope, out, &ctor.name);
        }
    }
}

fn walk_atomic(a: &satysfi_syntax::cst::ast::Atomic, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::Atomic;
    match a {
        Atomic::Length(_)
        | Atomic::Float(_)
        | Atomic::Int(_)
        | Atomic::Literal(_)
        | Atomic::True(_)
        | Atomic::False(_) => {}
        Atomic::Ctor(c) => emit_value(scope, out, &c.name),
        Atomic::Var(v) => emit_value(scope, out, &v.name),
        // Qualified — resolves inside the module, never against `base_env`.
        Atomic::VarWithMod(_) => {}
        Atomic::OpRef(op) => emit_value(scope, out, &op.name),
        Atomic::Command { name, .. } => walk_any_horz_cmd_ref(name, scope, out),
        Atomic::Unit { .. } => {}
        Atomic::Paren { inner, .. } => walk_paren_body(inner, scope, out),
        Atomic::OpenModule { body, .. } => walk_paren_body(body, scope, out),
        Atomic::Record { body, .. } => walk_record_body(body, scope, out),
        Atomic::List { items, .. } => {
            for it in items {
                walk_expr(&it.value.0, scope, out);
            }
        }
        Atomic::InlineText { elems, .. } => {
            for el in elems {
                walk_inline_elem(el, scope, out);
            }
        }
        Atomic::BlockText { elems, .. } => {
            for el in elems {
                walk_block_elem(el, scope, out);
            }
        }
        Atomic::MathText { elems, .. } => {
            for el in elems {
                walk_math_elem(&el.0, scope, out);
            }
        }
    }
}

fn walk_any_horz_cmd_ref(n: &satysfi_syntax::leaf::AnyHorzCmdTok, scope: &XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::leaf::AnyHorzCmdTok;
    match n {
        AnyHorzCmdTok::Plain(t) => emit_value(scope, out, &t.name),
        AnyHorzCmdTok::Mod(_) => {} // qualified — skip
    }
}

fn walk_any_vert_cmd_ref(n: &satysfi_syntax::leaf::AnyVertCmdTok, scope: &XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::leaf::AnyVertCmdTok;
    match n {
        AnyVertCmdTok::Plain(t) => emit_value(scope, out, &t.name),
        AnyVertCmdTok::Mod(_) => {} // qualified — skip
    }
}

fn walk_any_math_cmd_ref(n: &satysfi_syntax::leaf::AnyMathCmdTok, scope: &XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::leaf::AnyMathCmdTok;
    match n {
        AnyMathCmdTok::Plain(t) => emit_value(scope, out, &t.name),
        AnyMathCmdTok::Mod(_) => {} // qualified — skip
    }
}

fn walk_paren_body(pb: &satysfi_syntax::cst::ast::ParenBody, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_expr(&pb.first.0, scope, out);
    for ce in &pb.rest {
        walk_expr(&ce.value.0, scope, out);
    }
}

fn walk_record_body(rb: &satysfi_syntax::cst::ast::RecordBody, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::RecordBody;
    match rb {
        RecordBody::Update { base, fields, .. } => {
            walk_expr(&base.0, scope, out);
            for f in fields {
                walk_expr(&f.value.0, scope, out);
            }
        }
        RecordBody::Fields(fields) => {
            for f in fields {
                walk_expr(&f.value.0, scope, out);
            }
        }
    }
}

fn walk_inline_elem(el: &satysfi_syntax::cst::ast::InlineElem, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::InlineElem;
    match el {
        InlineElem::Char(_) | InlineElem::Space(_) | InlineElem::Break(_) => {}
        InlineElem::Embed { var, .. } => {
            if var.mods.is_empty() {
                emit_value(scope, out, &var.name);
            }
        }
        InlineElem::EmbedMath { elems, .. } => {
            for m in elems {
                walk_math_elem(&m.0, scope, out);
            }
        }
        InlineElem::Cmd { name, tail } => {
            walk_any_horz_cmd_ref(name, scope, out);
            walk_cmd_tail(tail, scope, out);
        }
        InlineElem::ItemBullet(_) | InlineElem::Sep(_) => {}
    }
}

fn walk_block_elem(el: &satysfi_syntax::cst::ast::BlockElem, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::BlockElem;
    match el {
        BlockElem::Embed { var, .. } => {
            if var.mods.is_empty() {
                emit_value(scope, out, &var.name);
            }
        }
        BlockElem::Cmd { name, tail } => {
            walk_any_vert_cmd_ref(name, scope, out);
            walk_cmd_tail(tail, scope, out);
        }
    }
}

fn walk_cmd_tail(t: &satysfi_syntax::cst::ast::CmdTail, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::CmdTail;
    match t {
        CmdTail::Semi(_) => {}
        CmdTail::Args { first, rest, .. } => {
            walk_apparg(&first.0, scope, out);
            for a in rest {
                walk_apparg(&a.0, scope, out);
            }
        }
    }
}

fn walk_math_elem(m: &satysfi_syntax::cst::ast::MathElemCst, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_math_bot(&m.base, scope, out);
    for s in &m.scripts {
        walk_math_script(s, scope, out);
    }
}

fn walk_math_script(s: &satysfi_syntax::cst::ast::MathScript, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::MathScript;
    match s {
        MathScript::Super { group, .. } | MathScript::Sub { group, .. } => {
            walk_math_group_arg(group, scope, out)
        }
        MathScript::Primes(_) => {}
    }
}

fn walk_math_group_arg(
    g: &satysfi_syntax::cst::ast::MathGroupArg,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use satysfi_syntax::cst::ast::MathGroupArg;
    match g {
        MathGroupArg::Group { elems, .. } => {
            for m in elems {
                walk_math_elem(&m.0, scope, out);
            }
        }
        MathGroupArg::Bot(b) => walk_math_bot(b, scope, out),
    }
}

fn walk_math_bot(b: &satysfi_syntax::cst::ast::MathBot, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::MathBot;
    match b {
        MathBot::Cmd { name, args } => {
            walk_any_math_cmd_ref(name, scope, out);
            for a in args {
                walk_math_arg(a, scope, out);
            }
        }
        MathBot::Chars(_) => {}
        MathBot::Embed(v) => {
            if v.mods.is_empty() {
                emit_value(scope, out, &v.name);
            }
        }
        MathBot::Sep(_) => {}
        MathBot::Group { elems, .. } => {
            for m in elems {
                walk_math_elem(&m.0, scope, out);
            }
        }
    }
}

fn walk_math_arg(a: &satysfi_syntax::cst::ast::MathArg, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::MathArg;
    match a {
        MathArg::Optional { body, .. } => walk_math_arg_body(body, scope, out),
        MathArg::Omission(_) => {}
        MathArg::Plain(body) => walk_math_arg_body(body, scope, out),
    }
}

fn walk_math_arg_body(
    b: &satysfi_syntax::cst::ast::MathArgBody,
    scope: &mut XverScope,
    out: &mut FreeGlobals,
) {
    use satysfi_syntax::cst::ast::MathArgBody;
    match b {
        MathArgBody::Math { elems, .. } => {
            for m in elems {
                walk_math_elem(&m.0, scope, out);
            }
        }
        MathArgBody::Inline { elems, .. } => {
            for el in elems {
                walk_inline_elem(el, scope, out);
            }
        }
        MathArgBody::Block { elems, .. } => {
            for el in elems {
                walk_block_elem(el, scope, out);
            }
        }
        MathArgBody::ParenEscape { inner, .. } => walk_paren_body(inner, scope, out),
        MathArgBody::ListEscape { items, .. } => {
            for it in items {
                walk_expr(&it.value.0, scope, out);
            }
        }
        MathArgBody::RecordEscape { body, .. } => walk_record_body(body, scope, out),
    }
}

fn walk_type_expr(te: &satysfi_syntax::cst::ast::TypeExpr, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::TypeExpr;
    match te {
        TypeExpr::Fun { opts, dom, cod, .. } => {
            for o in opts {
                walk_type_prod(&o.ty, scope, out);
            }
            walk_type_prod(dom, scope, out);
            walk_type_expr(cod, scope, out);
        }
        TypeExpr::Atom(prod) => walk_type_prod(prod, scope, out),
        TypeExpr::OptRowFun { opt_dom, dom, cod, .. } => {
            for e in &opt_dom.entries {
                walk_type_expr(&e.ty.0, scope, out);
            }
            walk_type_prod(dom, scope, out);
            walk_type_expr(cod, scope, out);
        }
    }
}

fn walk_type_prod(tp: &satysfi_syntax::cst::ast::TypeProd, scope: &mut XverScope, out: &mut FreeGlobals) {
    walk_type_app(&tp.first, scope, out);
    for st in &tp.rest {
        walk_type_app(&st.ty, scope, out);
    }
}

fn walk_type_app(ta: &satysfi_syntax::cst::ast::TypeApp, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::TypeApp;
    match ta {
        TypeApp::Applied { arg, ctor } => {
            walk_type_atom(arg, scope, out);
            emit_type(scope, out, &ctor.name);
        }
        TypeApp::Atom(atom) => walk_type_atom(atom, scope, out),
    }
}

fn walk_type_atom(atom: &satysfi_syntax::cst::ast::TypeAtom, scope: &mut XverScope, out: &mut FreeGlobals) {
    use satysfi_syntax::cst::ast::TypeAtom;
    match atom {
        TypeAtom::Cmd { args, .. } => {
            for a in args {
                for l in &a.opt_labels {
                    walk_type_expr(&l.ty.0, scope, out);
                }
                walk_type_expr(&a.ty.0, scope, out);
            }
        }
        TypeAtom::Paren { inner, .. } => walk_type_expr(&inner.0, scope, out),
        TypeAtom::Record { fields, .. } => {
            for f in fields {
                walk_type_expr(&f.ty.0, scope, out);
            }
        }
        // A bound type variable — never a forked-name candidate.
        TypeAtom::Var(_) => {}
        TypeAtom::Name(n) => emit_type(scope, out, &n.name),
        TypeAtom::RecordOpen { inner, .. } => {
            for f in &inner.fields {
                walk_type_expr(&f.ty.0, scope, out);
            }
        }
    }
}

/// One `block-frame-breakable` frame currently between its `FrameStart`/
/// `FrameEnd` markers on the page being walked (§C3).
struct OpenFrame {
    id: DecoId,
    /// The `FrameStart` marker's own `PlacedLine.x` — the frame's left edge.
    x: Length,
    /// The `FrameStart` marker's own baseline — the degenerate-rect fallback
    /// used at close time when NO real line ever appeared between Start/End
    /// (an empty frame). NOT used to seed `top`/`bottom` directly (a
    /// marker's own baseline is just wherever the previous line happened to
    /// end, unrelated to real content extent).
    marker_baseline: Length,
    /// Running (top, bottom) extent in page (y-down) coordinates, `None`
    /// until the first real line is seen between this frame's Start/End.
    top: Option<Length>,
    bottom: Option<Length>,
    /// Insertion order — used to sort same-page fires back into outer-before-
    /// inner document order (see the ordering note below).
    open_seq: usize,
}

/// Fire every placed page-break hook and §D decoration, in document order,
/// now that final page numbers and points are known. THIS is the port's
/// `make_hook` + `handlePdf.ml:234/337`'s invocation (hooks) and
/// `EvHorzFrame`/`EvVertFrame` (decos), relocated to the one place that
/// legally holds `&mut Interp` — the backend produced the geometry (POD
/// `HookId`/`DecoId` tokens riding inside placed boxes, per `hbox.rs`); this
/// reads them back and re-enters the evaluator.
/// docs/plans/hooks-annotations-crossref.md §The callback architecture, §D.
///
/// Sets `interp.current_page` to `Some(i)` for the duration of page `i`'s
/// walk (§0.5's "during page break" window: `register-destination`/
/// `register-link-to-*` — called directly by a hook or, more commonly,
/// transitively by a fired deco closure, e.g. `annot.satyh`'s `\href` —
/// only succeed inside this window) and back to `None` once every page is
/// done.
///
/// **Known scope cuts** (documented deviations, see the plan's Risks):
/// - Frames nested inside a `Tabular` cell or an `EmbeddedBlock`'s stacked
///   lines are NOT discovered by this walk — their placed positions would
///   need the writers' cell/stack arithmetic replicated lang-side. No
///   bundled package puts an `\href`/frame inside one today.
/// - A `block-frame-breakable` frame whose `FrameStart` and `FrameEnd` land
///   on DIFFERENT pages fires nothing (dropped silently at page end) —
///   multi-page fragments (`decoH`/`decoM`/`decoT`) need a body-vs-header/
///   footer split this port's `Page` doesn't carry yet; single-page frames
///   (the common case: `code.satyh` blocks, `itemize` bullets, `annot`'s
///   frames) fire `decoS`.
///
/// `pub` (rather than crate-private) so unit tests can drive it directly
/// against a hand-built `DocumentValue`, without going through a full
/// `compile_document_cst` fixpoint.
pub fn fire_hooks(interp: &mut eval::Interp, doc: &DocumentValue) -> Result<(), eval::EvalError> {
    interp.page_graphics = doc.pages.iter().map(|_| Vec::new()).collect();
    let mut next_open_seq: usize = 0;

    for (i, page) in doc.pages.iter().enumerate() {
        interp.current_page = Some(i);
        let page_number = (i + 1) as i64; // 1-based, = pbinfo#page-number
        let mut open: Vec<OpenFrame> = Vec::new();
        // (open_seq, graphics) per closed block-frame fragment on this page,
        // sorted by open order before being appended to the page's underlay
        // — see the doc comment on the ordering this preserves.
        let mut closings: Vec<(usize, Vec<GraphicsElem>)> = Vec::new();

        for line in &page.lines {
            for (dx, bx) in &line.contents {
                match bx {
                    PureHorzBox::HookPageBreak { id } => {
                        let closure = interp.hooks[id.0].clone();
                        let mut fields = BTreeMap::new();
                        fields.insert("page-number".to_string(), Value::Int(page_number));
                        let pbinfo = Value::Record(fields);
                        // PDF page space is y-up; placed geometry
                        // (`baseline_y`) is page space y-down from the paper
                        // top — the same flip the writers apply.
                        let point = Value::Tuple(vec![
                            Value::Length(line.x + *dx),
                            Value::Length(doc.geometry.paper_height - line.baseline_y),
                        ]);
                        let applied = interp.apply(closure, pbinfo)?;
                        match interp.apply(applied, point)? {
                            Value::Unit => {}
                            other => {
                                return eval::eval_error(format!(
                                    "hook-page-break closure returned {}, expected unit",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    PureHorzBox::Frame { .. } => {
                        fire_inline_frame(interp, doc, i, line.x + *dx, line.baseline_y, bx)?;
                    }
                    PureHorzBox::FrameMarker { id, end: false } => {
                        open.push(OpenFrame {
                            id: *id,
                            x: line.x + *dx,
                            marker_baseline: line.baseline_y,
                            top: None,
                            bottom: None,
                            open_seq: next_open_seq,
                        });
                        next_open_seq += 1;
                    }
                    PureHorzBox::FrameMarker { id, end: true } => {
                        // Close the innermost still-open frame with this id
                        // (well-nested by construction: `prim_block_frame_
                        // breakable` always emits a matched Start/End pair
                        // around its own contents).
                        if let Some(pos) = open.iter().rposition(|f| f.id == *id) {
                            let frame = open.remove(pos);
                            let (pads, width, deco_s) = match &interp.decos[frame.id.0] {
                                eval::DecoEntry::Block { pads, width, decoset } => {
                                    (*pads, *width, decoset[0].clone())
                                }
                                eval::DecoEntry::Inline { .. } => {
                                    return eval::eval_error(
                                        "BUG: inline deco behind a block-frame marker",
                                    )
                                }
                            };
                            // No real line ever appeared between Start/End
                            // (an empty frame) -> degenerate zero-height
                            // rect anchored at the Start marker's own
                            // baseline, rather than a fabricated extent.
                            let top = frame.top.unwrap_or(frame.marker_baseline);
                            let bottom = frame.bottom.unwrap_or(frame.marker_baseline);
                            let frame_top = top - pads.t;
                            let frame_bottom = bottom + pads.b;
                            let pt = (frame.x, doc.geometry.paper_height - frame_bottom);
                            let gr = primitives::apply_deco(
                                interp,
                                deco_s,
                                pt,
                                width,
                                frame_bottom - frame_top,
                                Length::ZERO,
                            )?;
                            closings.push((frame.open_seq, gr));
                        }
                        // An End with no matching Start on this page can't
                        // happen given well-nested markers; ignored if it did.
                    }
                    _ => {}
                }
            }
            // Every REAL line (one with non-marker content) extends every
            // currently-open frame's (top, bottom) — pad Skips don't create
            // a `PlacedLine` at all, so they're naturally excluded here; the
            // ±pad compensation happens once, at close time, above.
            if let Some((height, depth)) = placed_line_extent(line) {
                let top = line.baseline_y - height;
                let bottom = line.baseline_y + depth;
                for f in &mut open {
                    f.top = Some(f.top.map_or(top, |t| t.min(top)));
                    f.bottom = Some(f.bottom.map_or(bottom, |b| b.max(bottom)));
                }
            }
        }
        // Frames still open at page end are dropped (see the doc comment's
        // "Known scope cuts" — no cross-page fragment support yet).

        closings.sort_by_key(|(seq, _)| *seq);
        for (_, gr) in closings {
            interp.page_graphics[i].extend(gr);
        }
    }
    interp.current_page = None;
    Ok(())
}

/// Fire one placed inline frame's deco (and any frames nested in its
/// contents) with its final geometry — the port of `EvHorzFrame`'s
/// `deco (xpos, yposbaseline) wid hgt dpt` (handlePdf.ml:123-129), point
/// pre-flipped to PDF y-up exactly like the hook point above. The returned
/// `graphics list` (absolute page coordinates, `make_frame_deco`'s contract)
/// is accumulated onto this page's underlay.
///
/// `interp.current_page` is already `Some(page)` here (set by `fire_hooks`'s
/// caller), so a deco body calling `register-link-to-uri` (exactly
/// `annot.satyh:11-14`) lands its `Annot` on the right page — this is the
/// entire `\href` unlock.
fn fire_inline_frame(
    interp: &mut eval::Interp,
    doc: &DocumentValue,
    page: usize,
    x: Length,
    baseline_y: Length,
    bx: &PureHorzBox,
) -> Result<(), eval::EvalError> {
    let PureHorzBox::Frame {
        width,
        height,
        depth,
        deco,
        contents,
    } = bx
    else {
        return Ok(());
    };
    let deco_v = match &interp.decos[deco.0] {
        eval::DecoEntry::Inline { deco } => deco.clone(),
        eval::DecoEntry::Block { .. } => {
            return eval::eval_error("BUG: block deco behind an inline frame")
        }
    };
    let pt = (x, doc.geometry.paper_height - baseline_y);
    let gr = primitives::apply_deco(interp, deco_v, pt, *width, *height, *depth)?;
    interp.page_graphics[page].extend(gr);
    for (dx, child) in contents {
        fire_inline_frame(interp, doc, page, x + *dx, baseline_y, child)?;
    }
    Ok(())
}
